//! 会话持久化：redb 文件 `cookie.redb`，对应 Go `webui/cookie.go` 的 `cookie.db`（bbolt）。
//!
//! # 文件位置
//!
//! 与 `config.yml` 同目录（[`crate::WebUiState::new`] 从 [`gensokyo_config::ConfigManager::path`]
//! 取父目录）。默认 `config.yml` 是相对路径，父目录为空串，落点就是进程工作目录——与 Go
//! `bbolt.Open(DBName)` 的"工作目录"口径一致；用绝对路径指定配置文件时，会话库随之移动。
//!
//! # 与 Go 的差异
//!
//! - **格式**：Go 是 bbolt 的页格式，本实现是 redb；表名 `sessions` 对应 Go 的
//!   `CookieBucket = "cookies"` 单桶，key 是 cookie token，value 的编码也不同
//!   （Go：8 字节大端 `int64` Unix **秒**；本实现：redb 原生 `u64` Unix **毫秒**）。
//!   毫秒与进程内判据同源，24h TTL 语义不变，只避免秒粒度让 TTL=0 的会话在同一秒内仍可用。
//! - **过期行**：Go 只在 `ValidateCookie` 时判过期，过期行永远留在库里；本实现打开时
//!   （[`SessionDb::load`]）删掉过期行，[`crate::SessionStore::cleanup`] 也会同步删盘。
//! - **打不开时**：Go `log.Fatalf` 直接退出进程；本实现由
//!   [`crate::SessionStore::open_default`] 降级为纯内存会话并 `warn`——磁盘只读不该让整个
//!   机器人在启动阶段挂掉。

use std::collections::HashMap;
use std::path::Path;

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

/// 会话表（Go `CookieBucket` 的单表投影）。
const SESSIONS: TableDefinition<'static, &str, u64> = TableDefinition::new("sessions");

/// 会话库文件名（Go `DBName = "cookie.db"`）。
pub const SESSION_DB_FILE: &str = "cookie.redb";

/// `cookie.redb` 的薄封装：`token → 过期时刻`（Unix 毫秒）。
#[derive(Debug)]
pub struct SessionDb {
    db: Database,
}

impl SessionDb {
    /// 打开（必要时创建）会话库并建表。
    ///
    /// # Errors
    ///
    /// 路径不可写、文件损坏或 redb 打开/建表失败时返回 [`redb::Error`]。
    pub fn open(path: &Path) -> Result<Self, redb::Error> {
        let db = Database::create(path)?;
        let txn = db.begin_write()?;
        txn.open_table(SESSIONS)?;
        txn.commit()?;
        Ok(Self { db })
    }

    /// 读出全部**未过期**会话，并顺手删掉已过期行。
    ///
    /// # Errors
    ///
    /// 读事务或删除事务失败时返回 [`redb::Error`]。
    pub fn load(&self, now_ms: u64) -> Result<HashMap<String, u64>, redb::Error> {
        let mut live = HashMap::new();
        let mut expired = Vec::new();
        {
            let txn = self.db.begin_read()?;
            let table = txn.open_table(SESSIONS)?;
            for entry in table.iter()? {
                let (token, expires_at) = entry?;
                let (token, expires_at) = (token.value().to_owned(), expires_at.value());
                if expires_at > now_ms {
                    live.insert(token, expires_at);
                } else {
                    expired.push(token);
                }
            }
        }
        if !expired.is_empty() {
            self.delete(&expired)?;
        }
        Ok(live)
    }

    /// 写入一条会话（覆盖同 token 的旧值）。
    ///
    /// # Errors
    ///
    /// 写事务或提交失败时返回 [`redb::Error`]。
    pub fn put(&self, token: &str, expires_at_ms: u64) -> Result<(), redb::Error> {
        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(SESSIONS)?;
            table.insert(token, expires_at_ms)?;
        }
        txn.commit()?;
        Ok(())
    }

    /// 删除一条会话（不存在也算成功）。
    ///
    /// # Errors
    ///
    /// 写事务或提交失败时返回 [`redb::Error`]。
    pub fn remove(&self, token: &str) -> Result<(), redb::Error> {
        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(SESSIONS)?;
            table.remove(token)?;
        }
        txn.commit()?;
        Ok(())
    }

    fn delete(&self, tokens: &[String]) -> Result<(), redb::Error> {
        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(SESSIONS)?;
            for token in tokens {
                table.remove(token.as_str())?;
            }
        }
        txn.commit()?;
        Ok(())
    }
}
