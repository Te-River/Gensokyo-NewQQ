//! idmap 写批量：把一次事件内的多键写入合并进同一个写事务（P35）。
//!
//! 用法是 [`IdMapStore::with_write_batch`]：闭包拿到一个 [`IdMapWriteBatch`]，闭包里只调它
//! 自己的方法（**不要**在闭包里调 `store` 上的写方法——redb 同一库只允许一个写事务，同线程
//! 再开一个会等自己提交而卡死）。闭包返回 `Ok` 才提交，返回 `Err` 则未提交事务整批丢弃。
//!
//! **加锁顺序**：两个库的写事务是分别加的（懒打开），批量内必须先身份库后消息库
//! （`retrieve_virtual_value` / `write_config` 在 `store_cache` 之前）。顺序相反的两个批量同时
//! 运行会各持一把锁互等——这是 redb 每库单写者模型的直接后果，调用方必须守同一顺序。
//!
//! 两个库（身份库 [`crate::idmap::DEFAULT_IDENTITY_PATH`] 与消息库）是两个 redb 文件，跨文件
//! 没有真原子：身份库先提交，消息库提交失败时身份的写入保留——与 Go 的两个 bbolt 库各自
//! `Update` 同口径。
//!
//! # 与 Go 的差异（明确列出）
//!
//! - **提交次数**：Go 每个 `StoreIDv2` / `StoreMsgID` / `WriteConfigv2` 各自一次 fsync；这里同
//!   一次事件内的多次写入共用一个事务。**成功路径逐键结果一致**，批内重复解析同一身份命中
//!   本批已写入的行（写事务内读表看得到自己的未提交写入），不会重复分配 vUIN。
//! - **失败语义**：批内任何一步失败时整批回滚；Go 逐条提交时失败点之前的写入会保留。只有
//!   redb 事务失败或库内数据损坏才会走到这里（事件层对这些失败一律降级为原始字符串上报）。

use std::fmt;
use std::time::Duration;

use redb::WriteTransaction;
use tracing::debug;

use super::{IdMapStore, LookupKey, encoding::normalize_identity, rb};
use crate::StorageError;

/// 一次批量写的事务集合（懒打开：只用身份库的批量不会占用消息库的写锁）。
pub struct IdMapWriteBatch<'a> {
    store: &'a IdMapStore,
    identity: Option<WriteTransaction>,
    msg: Option<WriteTransaction>,
    /// 提交成功后要失效的缓存 key（提交前写进来，提交后才用）。
    dirty: Vec<LookupKey>,
}

impl fmt::Debug for IdMapWriteBatch<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdMapWriteBatch")
            .field("identity_open", &self.identity.is_some())
            .field("msg_open", &self.msg.is_some())
            .field("dirty", &self.dirty)
            .finish()
    }
}

impl<'a> IdMapWriteBatch<'a> {
    /// 空批次；事务在第一次用到对应库时才打开。
    fn new(store: &'a IdMapStore) -> Self {
        Self {
            store,
            identity: None,
            msg: None,
            dirty: Vec::new(),
        }
    }

    /// 查身份对应的 vUIN，查不到则在**本批事务内**分配（Go `storeIdentity`）。
    ///
    /// # Errors
    ///
    /// redb 事务失败或计数器/表数据损坏时返回 [`StorageError`]。
    pub fn retrieve_virtual_value(&mut self, identity: &str) -> Result<(String, i64), StorageError> {
        let store = self.store;
        let normalized = normalize_identity(
            identity,
            &store.options().app_id,
            store.options().uin.as_deref(),
        );
        let txn = self.identity_txn()?;
        let (vuin, created) = store.resolve_identity_tx(txn, &normalized)?;
        if created {
            // 正表新增（身份 → 本 vUIN）
            self.dirty.push(LookupKey::Identity(normalized.key.clone()));
            self.dirty.push(LookupKey::Vuin(vuin));
        } else {
            // 命中也会改写 last_seen（resolve_identity_tx），反查结果可能变；正查映射未变
            self.dirty.push(LookupKey::Vuin(vuin));
        }
        if created {
            debug!(identity = %normalized.raw, vuin, "idmap 身份已分配");
        } else {
            debug!(identity = %normalized.raw, vuin, "idmap 身份命中");
        }
        Ok((normalized.raw, vuin))
    }

    /// 分配或复用真实消息 ID 对应的虚拟 ID（Go `StoreMsgID`），TTL 取
    /// [`super::IdMapOptions::msg_ttl`]。
    ///
    /// # Errors
    ///
    /// 消息 ID 为空、redb 事务失败或数据损坏时返回 [`StorageError`]。
    pub fn store_cache(&mut self, msg_id: &str) -> Result<i64, StorageError> {
        let ttl = self.store.options().msg_ttl;
        self.store_cache_with_ttl(msg_id, ttl)
    }

    /// 同 [`IdMapWriteBatch::store_cache`]，但显式指定 TTL（语义见
    /// [`IdMapStore::store_cache_with_ttl`]）。
    ///
    /// # Errors
    ///
    /// 消息 ID 为空、redb 事务失败或数据损坏时返回 [`StorageError`]。
    pub fn store_cache_with_ttl(
        &mut self,
        msg_id: &str,
        ttl: Duration,
    ) -> Result<i64, StorageError> {
        if msg_id.is_empty() {
            return Err(StorageError::Corrupt("消息 ID 不能为空".to_owned()));
        }
        let store = self.store;
        let txn = self.msg_txn()?;
        let (virtual_id, created) = store.store_cache_tx(txn, msg_id, ttl)?;
        if created {
            debug!(msg_id = %msg_id, virtual_id, "idmap 消息 ID 已分配");
        } else {
            debug!(msg_id = %msg_id, virtual_id, "idmap 消息 ID 命中");
        }
        Ok(virtual_id)
    }

    /// 写配置（Go `WriteConfig`），键与 value 编码同 [`IdMapStore::write_config`]。
    ///
    /// # Errors
    ///
    /// redb 事务失败时返回 [`StorageError`]。
    pub fn write_config(
        &mut self,
        section: &str,
        subtype: &str,
        value: &str,
    ) -> Result<(), StorageError> {
        let key = super::config_key(section, subtype);
        let txn = self.identity_txn()?;
        let mut table = txn.open_table(super::CONFIG).map_err(rb)?;
        table.insert(key.as_str(), value.as_bytes()).map_err(rb)?;
        Ok(())
    }

    /// 记录"群 × 用户"的最新消息 ID（Go `StoreLatestMsgID`），空参数静默返回。
    ///
    /// # Errors
    ///
    /// redb 事务失败时返回 [`StorageError`]。
    pub fn store_latest_msg_id(
        &mut self,
        group: &str,
        user: &str,
        msg_id: &str,
    ) -> Result<(), StorageError> {
        if group.is_empty() || user.is_empty() || msg_id.is_empty() {
            return Ok(());
        }
        let store = self.store;
        let ttl = store.options().msg_ttl;
        let txn = self.msg_txn()?;
        super::store_latest_tx(txn, group, user, msg_id, ttl)
    }

    /// 提交两个库的事务并按需失效缓存。
    ///
    /// 顺序：提交身份库 → 前进代际号 → 失效缓存 → 提交消息库。代际号先行关闭「读事务在
    /// 提交前取旧值、提交后回填」的窗口，失效随后清理「提交后仍命中旧缓存」的条目；
    /// 没有缓存 key 受影响时不前进代际号（避免无谓地让并发读放弃回填）。
    fn commit(mut self) -> Result<(), StorageError> {
        if let Some(txn) = self.identity.take() {
            txn.commit().map_err(rb)?;
            if !self.dirty.is_empty() {
                self.store.lookup_cache.bump_epoch();
                self.store.lookup_cache.invalidate(&self.dirty);
            }
        }
        if let Some(txn) = self.msg.take() {
            txn.commit().map_err(rb)?;
        }
        Ok(())
    }

    /// 身份库写事务（首次使用时打开；必须先于消息库，反向锁序会死锁）。
    fn identity_txn(&mut self) -> Result<&WriteTransaction, StorageError> {
        // 只禁止「未开身份库就先开消息库」；write_config 在 msg 事务已开时会再次进入这里。
        debug_assert!(
            self.identity.is_some() || self.msg.is_none(),
            "必须先身份库后消息库"
        );
        if self.identity.is_none() {
            self.identity = Some(self.store.identity_db().begin_write().map_err(rb)?);
        }
        self.identity
            .as_ref()
            .ok_or_else(|| StorageError::Corrupt("身份库写事务未打开".to_owned()))
    }

    /// 消息库写事务（首次使用时打开）。
    fn msg_txn(&mut self) -> Result<&WriteTransaction, StorageError> {
        if self.msg.is_none() {
            self.msg = Some(self.store.msg_db().begin_write().map_err(rb)?);
        }
        self.msg
            .as_ref()
            .ok_or_else(|| StorageError::Corrupt("消息库写事务未打开".to_owned()))
    }
}

impl IdMapStore {
    /// 在同一个写事务里完成多次写入。
    ///
    /// 闭包拿到 [`IdMapWriteBatch`]，只调它的方法；返回 `Ok` 时两个库各自提交，返回 `Err` 时
    /// 未提交的事务整批丢弃（回滚）。缓存失效只在提交成功后发生，因此「写后立即读」看到的
    /// 永远是已提交状态。
    ///
    /// # Errors
    ///
    /// 闭包返回的错误原样上抛；提交失败返回 [`StorageError::Redb`]。
    pub fn with_write_batch<T>(
        &self,
        f: impl FnOnce(&mut IdMapWriteBatch<'_>) -> Result<T, StorageError>,
    ) -> Result<T, StorageError> {
        let mut batch = IdMapWriteBatch::new(self);
        let value = f(&mut batch)?;
        batch.commit()?;
        Ok(value)
    }
}
