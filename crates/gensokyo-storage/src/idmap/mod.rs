//! idmap 本地存储（redb）：身份 ↔ vUIN、消息 ID ↔ 虚拟 ID。
//!
//! 表的定义、key 构造与 value 编码逐条对齐 Go `idmap/map_service.go`，差异见 crate 文档。

mod encoding;
pub mod grpc;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use redb::{
    Database, ReadOnlyTable, ReadableDatabase, ReadableTable, TableDefinition, WriteTransaction,
};
use tracing::{debug, warn};

use crate::StorageError;
use encoding::{decode_i64, encode_i64, normalize_identity, vuin_identity_key, vuin_prefix};

/// 身份库默认文件名（Go `openid-map.db` 的 redb 等价物）。
pub const DEFAULT_IDENTITY_PATH: &str = "openid-map.redb";
/// 消息库默认文件名（Go `msgid-map.db` 的 redb 等价物）。
pub const DEFAULT_MSG_PATH: &str = "msgid-map.redb";
/// 消息 ID 的默认 TTL（Go `GetMsgIDTTLSeconds` 的缺省值）。
pub const DEFAULT_MSG_TTL: Duration = Duration::from_secs(3600);
/// 名称缓存默认 TTL（Go `StoreUserName`/`GetUserName` 的 10 分钟）。
pub const DEFAULT_USER_NAME_TTL: Duration = Duration::from_secs(600);
/// 名称缓存条数达到该值时，写入操作顺带清理过期项（Go 由后台协程每 10 分钟清理）。
pub const USER_NAME_SWEEP_THRESHOLD: usize = 1024;

/// 身份库：正表与反表。
const IDENTITY_TO_VUIN: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("identity_to_vuin");
const VUIN_TO_IDENTITY: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("vuin_to_identity");
const LAST_SEEN: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("last_seen");

/// 身份库：配置 KV（Go `ConfigBucket` = `"config"`），key 为 `"<section>:<subtype>"`，
/// value 是原始字符串字节（Go `WriteConfig` 直接 `b.Put(key, []byte(value))`）。
const CONFIG: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("config");

/// 两个库各自的 `meta` 表（Go 同名 bucket）。
const META: TableDefinition<'static, &'static str, &'static [u8]> = TableDefinition::new("meta");

/// 消息库：正表、反表、过期时间与"群 × 用户 最新消息"。
const MSG_TO_VIRTUAL: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("msg_to_virtual");
const VIRTUAL_TO_MSG: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("virtual_to_msg");
const EXPIRES_AT: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("expires_at");
const LATEST_BY_GROUP_USER: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("latest_by_group_user");
const LATEST_EXPIRES_AT: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("latest_expires_at");

/// Go `identityCounterKey` / `msgCounterKey` / `schemaVersionKey`。
const CURRENT_VUIN_KEY: &str = "current_vuin";
const CURRENT_MSG_KEY: &str = "current_msg";
const SCHEMA_VERSION_KEY: &str = "schema_version";
const SCHEMA_VERSION: &[u8] = b"2";
/// Go `defaultLastSeenScope`。
const LAST_SEEN_SCOPE: &str = "default";

/// [`IdMapOptions`] 的 AppID 占位值：空串。
///
/// [`IdMapStore::open`] 拒绝该值：空 AppID 会让 OpenID 的 key 变成
/// `openid:QQ::<raw>`，与 Go 的 `openid:QQ:<AppID>:<raw>` 永不相等，
/// 且这种 key 一旦写进库就无法再与 Go 对齐。
pub const UNSET_APP_ID: &str = "";

/// [`IdMapStore`] 的打开参数。
#[derive(Debug, Clone)]
pub struct IdMapOptions {
    /// 身份库路径，默认 [`DEFAULT_IDENTITY_PATH`]。
    pub identity_path: PathBuf,
    /// 消息库路径，默认 [`DEFAULT_MSG_PATH`]。
    pub msg_path: PathBuf,
    /// QQ AppID，参与 OpenID 的 key（Go `config.GetAppIDStr()`）。
    pub app_id: String,
    /// Go `idmap_isolation` 开启时的 `uin`；`None` 等价关闭隔离。
    pub uin: Option<String>,
    /// 消息 ID 的 TTL，默认 [`DEFAULT_MSG_TTL`]。
    pub msg_ttl: Duration,
    /// 名称缓存 TTL，默认 [`DEFAULT_USER_NAME_TTL`]。
    pub user_name_ttl: Duration,
}

impl Default for IdMapOptions {
    /// 除 AppID 外全部就绪：AppID 是 [`UNSET_APP_ID`] 占位，
    /// [`IdMapStore::open`] 会拒绝，调用方必须用 [`IdMapOptions::with_app_id`] 显式设置。
    fn default() -> Self {
        Self {
            identity_path: PathBuf::from(DEFAULT_IDENTITY_PATH),
            msg_path: PathBuf::from(DEFAULT_MSG_PATH),
            app_id: UNSET_APP_ID.to_owned(),
            uin: None,
            msg_ttl: DEFAULT_MSG_TTL,
            user_name_ttl: DEFAULT_USER_NAME_TTL,
        }
    }
}

impl IdMapOptions {
    /// 指定两个库的路径，其余取默认值。
    #[must_use]
    pub fn new(identity_path: impl Into<PathBuf>, msg_path: impl Into<PathBuf>) -> Self {
        Self {
            identity_path: identity_path.into(),
            msg_path: msg_path.into(),
            ..Self::default()
        }
    }

    /// 设置 AppID。
    #[must_use]
    pub fn with_app_id(mut self, app_id: impl Into<String>) -> Self {
        self.app_id = app_id.into();
        self
    }

    /// 设置隔离用的 `uin`（等价 Go `idmap_isolation` + `uin`）。
    #[must_use]
    pub fn with_uin(mut self, uin: impl Into<String>) -> Self {
        self.uin = Some(uin.into());
        self
    }

    /// 设置消息 ID 的 TTL。
    #[must_use]
    pub fn with_msg_ttl(mut self, ttl: Duration) -> Self {
        self.msg_ttl = ttl;
        self
    }

    /// 设置名称缓存 TTL。
    #[must_use]
    pub fn with_user_name_ttl(mut self, ttl: Duration) -> Self {
        self.user_name_ttl = ttl;
        self
    }
}

/// 名称缓存项。
#[derive(Debug, Clone)]
struct UserNameEntry {
    name: String,
    stored_at: Instant,
}

/// idmap 本地存储。
///
/// `Send + Sync`：内部两个 redb [`Database`]（各自串行写事务）+ 一个名称缓存互斥锁。
#[derive(Debug)]
pub struct IdMapStore {
    options: IdMapOptions,
    identity_db: Database,
    msg_db: Database,
    user_names: Mutex<HashMap<i64, UserNameEntry>>,
}

impl IdMapStore {
    /// 打开（必要时创建）两个库并建表。
    ///
    /// # Errors
    ///
    /// AppID 为空、路径不可写、两个库指向同一文件、或 redb 打开/建表失败时返回
    /// [`StorageError`]。
    pub fn open(options: IdMapOptions) -> Result<Self, StorageError> {
        if options.app_id.is_empty() {
            return Err(StorageError::Corrupt(
                "IdMapOptions.app_id 不能为空：OpenID key 需要 app_id 参与（Go config.GetAppIDStr()），\
                 空 AppID 会写出 openid:QQ::<raw> 这种与 Go 永不相等且不可迁移的 key"
                    .to_owned(),
            ));
        }
        if options.identity_path == options.msg_path {
            return Err(StorageError::Corrupt(
                "身份库与消息库不能指向同一文件".to_owned(),
            ));
        }
        let identity_db = Database::create(&options.identity_path).map_err(rb)?;
        let msg_db = Database::create(&options.msg_path).map_err(rb)?;
        let store = Self {
            options,
            identity_db,
            msg_db,
            user_names: Mutex::new(HashMap::new()),
        };
        store.init_schema()?;
        debug!(
            identity_path = %store.options.identity_path.display(),
            msg_path = %store.options.msg_path.display(),
            "idmap 存储已打开"
        );
        Ok(store)
    }

    /// 打开参数。
    #[must_use]
    pub fn options(&self) -> &IdMapOptions {
        &self.options
    }

    /// 建表并写入 schema 版本（Go `initNewDBs` 的建 bucket + `schema_version`）。
    ///
    /// 与 Go 一致，每次打开都**无条件**覆盖 `schema_version`；读到未知版本时按 Go
    /// 语义覆盖，但先记 warn（Go 是静默覆盖）。
    fn init_schema(&self) -> Result<(), StorageError> {
        let txn = self.identity_db.begin_write().map_err(rb)?;
        {
            let mut meta = txn.open_table(META).map_err(rb)?;
            let _ = txn.open_table(IDENTITY_TO_VUIN).map_err(rb)?;
            let _ = txn.open_table(VUIN_TO_IDENTITY).map_err(rb)?;
            let _ = txn.open_table(LAST_SEEN).map_err(rb)?;
            let _ = txn.open_table(CONFIG).map_err(rb)?;
            let existing = meta
                .get(SCHEMA_VERSION_KEY)
                .map_err(rb)?
                .map(|guard| guard.value().to_vec());
            if let Some(bytes) = existing
                && bytes != SCHEMA_VERSION
            {
                warn!(
                    found = %String::from_utf8_lossy(&bytes),
                    expected = %String::from_utf8_lossy(SCHEMA_VERSION),
                    "idmap schema_version 未知，按 Go 语义覆盖"
                );
            }
            meta.insert(SCHEMA_VERSION_KEY, SCHEMA_VERSION)
                .map_err(rb)?;
        }
        txn.commit().map_err(rb)?;

        let txn = self.msg_db.begin_write().map_err(rb)?;
        {
            let _ = txn.open_table(META).map_err(rb)?;
            let _ = txn.open_table(MSG_TO_VIRTUAL).map_err(rb)?;
            let _ = txn.open_table(VIRTUAL_TO_MSG).map_err(rb)?;
            let _ = txn.open_table(EXPIRES_AT).map_err(rb)?;
            let _ = txn.open_table(LATEST_BY_GROUP_USER).map_err(rb)?;
            let _ = txn.open_table(LATEST_EXPIRES_AT).map_err(rb)?;
        }
        txn.commit().map_err(rb)?;
        Ok(())
    }

    /// 查身份对应的 vUIN，查不到则分配新 vUIN（Go `storeIdentity`）。
    ///
    /// 返回值的第一项是归一化后的原始身份串（Go `identity.Raw`），等于
    /// [`IdMapStore::retrieve_row_by_id`] 反查到的值。
    ///
    /// # Errors
    ///
    /// redb 事务失败或计数器/表数据损坏时返回 [`StorageError`]。
    pub fn retrieve_virtual_value(&self, identity: &str) -> Result<(String, i64), StorageError> {
        let normalized =
            normalize_identity(identity, &self.options.app_id, self.options.uin.as_deref());
        let txn = self.identity_db.begin_write().map_err(rb)?;
        let (vuin, created) = {
            let existing = {
                let forward = txn.open_table(IDENTITY_TO_VUIN).map_err(rb)?;
                let value = forward.get(normalized.key.as_str()).map_err(rb)?;
                match value {
                    Some(guard) => Some(decode_vuin(guard.value())?),
                    None => None,
                }
            };
            match existing {
                Some(vuin) => {
                    touch_last_seen_tx(&txn, vuin, &normalized.raw)?;
                    (vuin, false)
                }
                None => {
                    // 写事务内读 counter 与已占用的 vUIN：redb 的读事务可与写事务共存，
                    // 读到的是已提交状态，因此不会看到本事务未提交的写入。
                    let current = read_counter_tx(&txn, CURRENT_VUIN_KEY)?;
                    let planned = self.next_free_vuin(current)?;
                    let vuin = allocate_vuin_tx(&txn, planned, &normalized)?;
                    (vuin, true)
                }
            }
        };
        txn.commit().map_err(rb)?;

        if created {
            debug!(identity = %normalized.raw, vuin, "idmap 身份已分配");
        } else {
            debug!(identity = %normalized.raw, vuin, "idmap 身份命中");
        }
        Ok((normalized.raw, vuin))
    }

    /// 只查身份对应的 vUIN，**不分配**（Go `lookupVirtualIdentity` / `RetrieveVirtualValue`
    /// 的新库路径：查不到就是查不到）。
    ///
    /// # Errors
    ///
    /// 身份未登记返回 [`StorageError::NotFound`]；redb 事务失败或正表数据损坏返回对应变体。
    pub fn lookup_virtual_value(&self, identity: &str) -> Result<i64, StorageError> {
        let normalized =
            normalize_identity(identity, &self.options.app_id, self.options.uin.as_deref());
        let txn = self.identity_db.begin_read().map_err(rb)?;
        let forward = txn.open_table(IDENTITY_TO_VUIN).map_err(rb)?;
        match forward.get(normalized.key.as_str()).map_err(rb)? {
            Some(guard) => decode_vuin(guard.value()),
            None => Err(StorageError::NotFound(format!("身份 {}", normalized.raw))),
        }
    }

    /// 从 vUIN 反查身份（Go `retrieveIdentity`）。
    ///
    /// 与 Go 一致：先读 `last_seen`，normalize 后校验正表仍指向该 vUIN，命中即返回；
    /// 否则扫描反表（同一 vUIN 下可能有多个身份 key，Go 迁移遗留），优先返回
    /// `openid:` 前缀或 OpenID 形态的原始串，再回退第一个。
    ///
    /// # Errors
    ///
    /// 查不到时返回 [`StorageError::NotFound`]。
    pub fn retrieve_row_by_id(&self, vuin: i64) -> Result<String, StorageError> {
        if vuin <= 0 {
            return Err(StorageError::NotFound(format!("vUIN={vuin}")));
        }
        let txn = self.identity_db.begin_read().map_err(rb)?;
        {
            let last_seen = txn.open_table(LAST_SEEN).map_err(rb)?;
            let seen = last_seen
                .get(last_seen_key(vuin).as_str())
                .map_err(rb)?
                .map(|guard| guard.value().to_vec());
            if let Some(bytes) = seen {
                let raw = decode_text(&bytes)?;
                if !raw.is_empty() {
                    let normalized =
                        normalize_identity(&raw, &self.options.app_id, self.options.uin.as_deref());
                    let forward = txn.open_table(IDENTITY_TO_VUIN).map_err(rb)?;
                    let mapped = forward
                        .get(normalized.key.as_str())
                        .map_err(rb)?
                        .map(|guard| decode_vuin(guard.value()))
                        .transpose()?;
                    if mapped == Some(vuin) {
                        return Ok(raw);
                    }
                }
            }
        }
        let prefix = vuin_prefix(vuin);
        let table = txn.open_table(VUIN_TO_IDENTITY).map_err(rb)?;

        let mut fallback: Option<String> = None;
        for entry in table.range(prefix.as_str()..).map_err(rb)? {
            let (key, value) = entry.map_err(rb)?;
            let key = key.value();
            if !key.starts_with(prefix.as_str()) {
                break;
            }
            let raw = decode_text(value.value())?;
            let identity_key = key.strip_prefix(prefix.as_str()).unwrap_or_default();
            if identity_key.starts_with("openid:") || (is_openid(&raw) && !raw.starts_with("rUIN-"))
            {
                return Ok(raw);
            }
            if fallback.is_none() {
                fallback = Some(raw);
            }
        }
        fallback.ok_or_else(|| StorageError::NotFound(format!("vUIN={vuin}")))
    }

    /// vUIN 对应的 `last_seen` 原始身份串（Go `readLastSeenTx`）。
    ///
    /// 分配与命中身份时都会写该表（Go `touchLastSeenTx`）。
    ///
    /// # Errors
    ///
    /// redb 事务失败或数据损坏时返回 [`StorageError`]。
    pub fn last_seen(&self, vuin: i64) -> Result<Option<String>, StorageError> {
        if vuin <= 0 {
            return Ok(None);
        }
        let key = last_seen_key(vuin);
        let txn = self.identity_db.begin_read().map_err(rb)?;
        let table = txn.open_table(LAST_SEEN).map_err(rb)?;
        match table.get(key.as_str()).map_err(rb)? {
            Some(guard) => Ok(Some(decode_text(guard.value())?)),
            None => Ok(None),
        }
    }

    /// 分配或复用真实消息 ID 对应的虚拟 ID（Go `StoreMsgID`），TTL 取
    /// [`IdMapOptions::msg_ttl`]。
    ///
    /// 复用命中时刷新过期时间（与 Go 一致）。
    ///
    /// # Errors
    ///
    /// 消息 ID 为空、redb 事务失败或数据损坏时返回 [`StorageError`]。
    pub fn store_cache(&self, msg_id: &str) -> Result<i64, StorageError> {
        self.store_cache_with_ttl(msg_id, self.options.msg_ttl)
    }

    /// 同 [`IdMapStore::store_cache`]，但显式指定 TTL（测试与按请求覆盖用）。
    ///
    /// TTL 按秒截断；截断后 ≤ 0 时按 Go `StoreMsgID` 回退 [`DEFAULT_MSG_TTL`]（1 小时），
    /// 因此 `Duration::ZERO` 与亚秒 TTL 都拿到 1 小时有效期。
    ///
    /// # Errors
    ///
    /// 消息 ID 为空、redb 事务失败或数据损坏时返回 [`StorageError`]。
    pub fn store_cache_with_ttl(&self, msg_id: &str, ttl: Duration) -> Result<i64, StorageError> {
        if msg_id.is_empty() {
            return Err(StorageError::Corrupt("消息 ID 不能为空".to_owned()));
        }
        let expires = expires_after(unix_now(), ttl);
        let cache_key = msg_id.to_owned();

        let txn = self.msg_db.begin_write().map_err(rb)?;
        let (virtual_id, created) = {
            let existing = {
                let forward = txn.open_table(MSG_TO_VIRTUAL).map_err(rb)?;
                let value = forward.get(cache_key.as_str()).map_err(rb)?;
                match value {
                    Some(guard) => Some(decode_vuin(guard.value())?),
                    None => None,
                }
            };
            match existing {
                Some(virtual_id) => {
                    let key = virtual_id.to_string();
                    let expires_bytes = encode_i64(expires);
                    let mut expires_table = txn.open_table(EXPIRES_AT).map_err(rb)?;
                    expires_table
                        .insert(key.as_str(), expires_bytes.as_slice())
                        .map_err(rb)?;
                    let mut reverse = txn.open_table(VIRTUAL_TO_MSG).map_err(rb)?;
                    reverse
                        .insert(key.as_str(), cache_key.as_bytes())
                        .map_err(rb)?;
                    (virtual_id, false)
                }
                None => {
                    let current = read_counter_tx(&txn, CURRENT_MSG_KEY)?;
                    let planned = self.next_free_msg_id(current)?;
                    let virtual_id = allocate_msg_tx(&txn, planned, &cache_key, expires)?;
                    (virtual_id, true)
                }
            }
        };
        txn.commit().map_err(rb)?;

        if created {
            debug!(msg_id = %msg_id, virtual_id, "idmap 消息 ID 已分配");
        } else {
            debug!(msg_id = %msg_id, virtual_id, "idmap 消息 ID 命中");
        }
        Ok(virtual_id)
    }

    /// 从虚拟消息 ID 反查真实消息 ID（Go `RetrieveMsgID`）。
    ///
    /// # Errors
    ///
    /// 不存在返回 [`StorageError::NotFound`]；TTL 过期返回 [`StorageError::Expired`]
    /// （Go 把两者都映射为 `ErrKeyNotFound`，用 [`StorageError::is_not_found`] 可得到
    /// 同样的单值语义）。
    pub fn retrieve_row_by_cache(&self, vuin: i64) -> Result<String, StorageError> {
        if vuin <= 0 {
            return Err(StorageError::NotFound(format!("虚拟消息 ID={vuin}")));
        }
        let key = vuin.to_string();
        let txn = self.msg_db.begin_read().map_err(rb)?;
        {
            let expires_table = txn.open_table(EXPIRES_AT).map_err(rb)?;
            if let Some(guard) = expires_table.get(key.as_str()).map_err(rb)? {
                let expires = decode_vuin(guard.value())?;
                // Go：exp > 0 && exp < now 才算过期；这里 exp <= now 即过期（零 TTL 立即失效）
                if expires > 0 && expires <= unix_now() {
                    return Err(StorageError::Expired(format!("虚拟消息 ID={vuin}")));
                }
            }
        }
        let reverse = txn.open_table(VIRTUAL_TO_MSG).map_err(rb)?;
        match reverse.get(key.as_str()).map_err(rb)? {
            Some(guard) => Ok(decode_text(guard.value())?),
            None => Err(StorageError::NotFound(format!("虚拟消息 ID={vuin}"))),
        }
    }

    /// 记录"群 × 用户"的最新消息 ID（Go `StoreLatestMsgID`）。
    ///
    /// Go 对空参数静默返回，这里保持一致。TTL 取 [`IdMapOptions::msg_ttl`]；
    /// 与 Go 相同**没有** 1 小时回退——TTL ≤ 0 时过期时间就是 `now`，
    /// 随后按 `expires <= now` 立即过期。
    ///
    /// # Errors
    ///
    /// redb 事务失败时返回 [`StorageError`]。
    pub fn store_latest_msg_id(
        &self,
        group: &str,
        user: &str,
        msg_id: &str,
    ) -> Result<(), StorageError> {
        if group.is_empty() || user.is_empty() || msg_id.is_empty() {
            return Ok(());
        }
        let key = latest_key(group, user);
        let expires = unix_now().saturating_add(ttl_seconds(self.options.msg_ttl));

        let txn = self.msg_db.begin_write().map_err(rb)?;
        {
            let mut table = txn.open_table(LATEST_BY_GROUP_USER).map_err(rb)?;
            table.insert(key.as_str(), msg_id.as_bytes()).map_err(rb)?;
            let expires_bytes = encode_i64(expires);
            let mut expires_table = txn.open_table(LATEST_EXPIRES_AT).map_err(rb)?;
            expires_table
                .insert(key.as_str(), expires_bytes.as_slice())
                .map_err(rb)?;
        }
        txn.commit().map_err(rb)?;
        Ok(())
    }

    /// 取"群 × 用户"的最新未过期消息 ID（Go `GetLatestMsgID`）。
    ///
    /// # Errors
    ///
    /// 从未记录（没有过期时间）返回 [`StorageError::NotFound`]；记录存在但已过期返回
    /// [`StorageError::Expired`]（Go 把两者都映射为 `ErrKeyNotFound`，用
    /// [`StorageError::is_not_found`] 可得到同样的单值语义）。
    pub fn get_latest_msg_id(&self, group: &str, user: &str) -> Result<String, StorageError> {
        let key = latest_key(group, user);
        let txn = self.msg_db.begin_read().map_err(rb)?;
        {
            let expires_table = txn.open_table(LATEST_EXPIRES_AT).map_err(rb)?;
            let expires = match expires_table.get(key.as_str()).map_err(rb)? {
                Some(guard) => decode_vuin(guard.value())?,
                None => 0,
            };
            // Go：exp == 0 视为"没有记录"；exp < now 才算过期
            if expires <= 0 {
                return Err(StorageError::NotFound(format!("最新消息 key={key}")));
            }
            if expires <= unix_now() {
                return Err(StorageError::Expired(format!("最新消息 key={key}")));
            }
        }
        let table = txn.open_table(LATEST_BY_GROUP_USER).map_err(rb)?;
        match table.get(key.as_str()).map_err(rb)? {
            Some(guard) => Ok(decode_text(guard.value())?),
            None => Err(StorageError::NotFound(format!("最新消息 key={key}"))),
        }
    }

    /// 写 vUIN 对应的用户名到内存缓存（Go `StoreUserName`）。
    pub fn store_user_name(&self, vuin: i64, name: &str) {
        let mut cache = self
            .user_names
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if cache.len() >= USER_NAME_SWEEP_THRESHOLD {
            let ttl = self.options.user_name_ttl;
            let now = Instant::now();
            cache.retain(|_, entry| now.duration_since(entry.stored_at) <= ttl);
        }
        cache.insert(
            vuin,
            UserNameEntry {
                name: name.to_owned(),
                stored_at: Instant::now(),
            },
        );
    }

    /// 取 vUIN 对应的用户名，过期或不存在返回 `None`（Go `GetUserName`）。
    #[must_use]
    pub fn get_user_name(&self, vuin: i64) -> Option<String> {
        let mut cache = self
            .user_names
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let ttl = self.options.user_name_ttl;
        if cache
            .get(&vuin)
            .is_none_or(|entry| entry.stored_at.elapsed() > ttl)
        {
            cache.remove(&vuin);
            return None;
        }
        cache.get(&vuin).map(|entry| entry.name.clone())
    }

    /// 写配置（Go `WriteConfig`）：身份库 `config` 表，key `"<section>:<subtype>"`，
    /// value 是原始字符串字节（与 bbolt 的 `b.Put(key, []byte(value))` 逐字节一致）。
    ///
    /// # Errors
    ///
    /// redb 事务失败时返回 [`StorageError`]。
    pub fn write_config(
        &self,
        section: &str,
        subtype: &str,
        value: &str,
    ) -> Result<(), StorageError> {
        let key = config_key(section, subtype);
        let txn = self.identity_db.begin_write().map_err(rb)?;
        {
            let mut table = txn.open_table(CONFIG).map_err(rb)?;
            table.insert(key.as_str(), value.as_bytes()).map_err(rb)?;
        }
        txn.commit().map_err(rb)?;
        Ok(())
    }

    /// 读配置（Go `ReadConfig`）：键不存在返回 [`StorageError::NotFound`]。
    ///
    /// # Errors
    ///
    /// 键不存在、value 不是合法 UTF-8、redb 事务失败时返回 [`StorageError`]。
    pub fn read_config(&self, section: &str, subtype: &str) -> Result<String, StorageError> {
        let key = config_key(section, subtype);
        let txn = self.identity_db.begin_read().map_err(rb)?;
        let table = txn.open_table(CONFIG).map_err(rb)?;
        match table.get(key.as_str()).map_err(rb)? {
            Some(guard) => decode_text(guard.value()),
            None => Err(StorageError::NotFound(format!("config {key}"))),
        }
    }

    /// 删配置（Go `DeleteConfig`）：返回删除前该键是否存在。
    ///
    /// Go 对不存在的键也算成功（bbolt `Delete` 幂等），gRPC 侧因此恒回 `success`。
    ///
    /// # Errors
    ///
    /// redb 事务失败时返回 [`StorageError`]。
    pub fn delete_config(&self, section: &str, subtype: &str) -> Result<bool, StorageError> {
        let key = config_key(section, subtype);
        let txn = self.identity_db.begin_write().map_err(rb)?;
        let removed = {
            let mut table = txn.open_table(CONFIG).map_err(rb)?;
            table.remove(key.as_str()).map_err(rb)?.is_some()
        };
        txn.commit().map_err(rb)?;
        Ok(removed)
    }

    /// 从已提交状态里找下一个未被占用的 vUIN（Go `allocateVuinTx` 的前缀扫描）。
    fn next_free_vuin(&self, current: i64) -> Result<i64, StorageError> {
        let mut candidate = if current <= 0 {
            1
        } else {
            current.saturating_add(1)
        };
        let txn = self.identity_db.begin_read().map_err(rb)?;
        let table = txn.open_table(VUIN_TO_IDENTITY).map_err(rb)?;
        while prefix_exists(&table, candidate)? {
            candidate = candidate
                .checked_add(1)
                .ok_or_else(|| StorageError::Corrupt("vUIN 空间耗尽".to_owned()))?;
        }
        Ok(candidate)
    }

    /// 从已提交状态里找下一个未被占用的虚拟消息 ID（Go `allocateMsgIDTx`）。
    fn next_free_msg_id(&self, current: i64) -> Result<i64, StorageError> {
        let mut candidate = if current <= 0 {
            1
        } else {
            current.saturating_add(1)
        };
        let txn = self.msg_db.begin_read().map_err(rb)?;
        let table = txn.open_table(VIRTUAL_TO_MSG).map_err(rb)?;
        while prefix_exists(&table, candidate)? {
            candidate = candidate
                .checked_add(1)
                .ok_or_else(|| StorageError::Corrupt("虚拟消息 ID 空间耗尽".to_owned()))?;
        }
        Ok(candidate)
    }
}

/// 在写事务里分配 vUIN：推进计数器并写正反两表 + `last_seen`。
fn allocate_vuin_tx(
    txn: &WriteTransaction,
    planned: i64,
    identity: &encoding::NormalizedIdentity,
) -> Result<i64, StorageError> {
    let mut meta = txn.open_table(META).map_err(rb)?;
    let mut forward = txn.open_table(IDENTITY_TO_VUIN).map_err(rb)?;
    let mut reverse = txn.open_table(VUIN_TO_IDENTITY).map_err(rb)?;
    let mut last_seen = txn.open_table(LAST_SEEN).map_err(rb)?;

    let current = match meta.get(CURRENT_VUIN_KEY).map_err(rb)? {
        Some(guard) => decode_vuin(guard.value())?,
        None => 0,
    };
    // planned 已跳过反表中占用中的 vUIN；并发写入方可能已推进 counter，
    // 取两者较大者，保证同一事务内分配到的 vUIN 不重号。
    let vuin = if current >= planned {
        current.checked_add(1)
    } else {
        Some(planned)
    }
    .ok_or_else(|| StorageError::Corrupt("vUIN 空间耗尽".to_owned()))?;

    let counter = encode_i64(vuin);
    meta.insert(CURRENT_VUIN_KEY, counter.as_slice())
        .map_err(rb)?;
    forward
        .insert(identity.key.as_str(), counter.as_slice())
        .map_err(rb)?;
    let reverse_key = vuin_identity_key(vuin, &identity.key);
    reverse
        .insert(reverse_key.as_str(), identity.raw.as_bytes())
        .map_err(rb)?;
    let seen_key = last_seen_key(vuin);
    last_seen
        .insert(seen_key.as_str(), identity.raw.as_bytes())
        .map_err(rb)?;
    Ok(vuin)
}

/// 在写事务里分配虚拟消息 ID：推进计数器并写正反两表 + 过期时间。
fn allocate_msg_tx(
    txn: &WriteTransaction,
    planned: i64,
    msg_id: &str,
    expires: i64,
) -> Result<i64, StorageError> {
    let mut meta = txn.open_table(META).map_err(rb)?;
    let mut forward = txn.open_table(MSG_TO_VIRTUAL).map_err(rb)?;
    let mut reverse = txn.open_table(VIRTUAL_TO_MSG).map_err(rb)?;
    let mut expires_table = txn.open_table(EXPIRES_AT).map_err(rb)?;

    let current = match meta.get(CURRENT_MSG_KEY).map_err(rb)? {
        Some(guard) => decode_vuin(guard.value())?,
        None => 0,
    };
    let virtual_id = if current >= planned {
        current.checked_add(1)
    } else {
        Some(planned)
    }
    .ok_or_else(|| StorageError::Corrupt("虚拟消息 ID 空间耗尽".to_owned()))?;

    let counter = encode_i64(virtual_id);
    meta.insert(CURRENT_MSG_KEY, counter.as_slice())
        .map_err(rb)?;
    forward.insert(msg_id, counter.as_slice()).map_err(rb)?;
    let key = counter_text(virtual_id);
    reverse
        .insert(key.as_str(), msg_id.as_bytes())
        .map_err(rb)?;
    expires_table
        .insert(key.as_str(), encode_i64(expires).as_slice())
        .map_err(rb)?;
    Ok(virtual_id)
}

/// 写 `last_seen`（Go `touchLastSeenTx`）。
fn touch_last_seen_tx(txn: &WriteTransaction, vuin: i64, raw: &str) -> Result<(), StorageError> {
    let mut table = txn.open_table(LAST_SEEN).map_err(rb)?;
    table
        .insert(last_seen_key(vuin).as_str(), raw.as_bytes())
        .map_err(rb)?;
    Ok(())
}

/// 在写事务里读 8 字节大端计数器（Go `readInt64(meta.Get(key))`）。
fn read_counter_tx(txn: &WriteTransaction, key: &str) -> Result<i64, StorageError> {
    let table = txn.open_table(META).map_err(rb)?;
    match table.get(key).map_err(rb)? {
        Some(guard) => decode_vuin(guard.value()),
        None => Ok(0),
    }
}

/// 反表里是否存在 `"<vUIN>:"` 前缀的 key（Go `vuinExistsTx`）。
fn prefix_exists(
    table: &ReadOnlyTable<&'static str, &'static [u8]>,
    vuin: i64,
) -> Result<bool, StorageError> {
    let prefix = vuin_prefix(vuin);
    let mut range = table.range(prefix.as_str()..).map_err(rb)?;
    match range.next() {
        Some(entry) => {
            let (key, _) = entry.map_err(rb)?;
            Ok(key.value().starts_with(prefix.as_str()))
        }
        None => Ok(false),
    }
}

fn is_openid(value: &str) -> bool {
    value.len() == encoding::OPENID_LEN
}

/// Go `lastSeenKey`：`"default:<十进制 vUIN>"`。
fn last_seen_key(vuin: i64) -> String {
    format!("{LAST_SEEN_SCOPE}:{vuin}")
}

/// Go `joinSectionAndKey`：`"<section>:<subtype>"`。
fn config_key(section: &str, subtype: &str) -> String {
    format!("{section}:{subtype}")
}

/// Go `StoreLatestMsgID` 的 key 构造：`"<群>:<用户>"`。
fn latest_key(group: &str, user: &str) -> String {
    format!("{group}:{user}")
}

fn counter_text(value: i64) -> String {
    value.to_string()
}

fn decode_vuin(bytes: &[u8]) -> Result<i64, StorageError> {
    decode_i64(bytes).ok_or_else(|| StorageError::Corrupt("期望 8 字节大端整数".to_owned()))
}

fn decode_text(bytes: &[u8]) -> Result<String, StorageError> {
    String::from_utf8(bytes.to_vec())
        .map_err(|_| StorageError::Corrupt("身份串/消息 ID 不是合法 UTF-8".to_owned()))
}

/// redb 的具体错误统一归入 [`StorageError::Redb`]。
fn rb<E: Into<redb::Error>>(err: E) -> StorageError {
    StorageError::Redb(err.into())
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

fn ttl_seconds(ttl: Duration) -> i64 {
    i64::try_from(ttl.as_secs()).unwrap_or(i64::MAX)
}

/// Go `StoreMsgID` 的过期时间：`now + ttl`，不晚于 `now` 时回退 [`DEFAULT_MSG_TTL`]。
///
/// 覆盖 TTL ≤ 0 与被截断成 0 的亚秒 TTL；`store_latest_msg_id` 不走这里（Go 无回退）。
fn expires_after(now: i64, ttl: Duration) -> i64 {
    let expires = now.saturating_add(ttl_seconds(ttl));
    if expires > now {
        expires
    } else {
        now.saturating_add(ttl_seconds(DEFAULT_MSG_TTL))
    }
}

#[cfg(test)]
mod tests;
