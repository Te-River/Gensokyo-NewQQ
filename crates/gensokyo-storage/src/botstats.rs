//! botstats：当日收发消息计数（Go `botstats/botstats.go` 的等价物）。
//!
//! Go 用 bbolt 单 bucket `stats`，三个键：
//!
//! | 键 | 值（Go） | 含义 |
//! |----|----------|------|
//! | `messageReceived` | 十进制 ASCII | 当日收到的消息数 |
//! | `messageSent` | 十进制 ASCII | 当日发出的消息数 |
//! | `lastMessageTime` | RFC3339 字符串 | 最后一次统计写入的时刻 |
//!
//! 统计维度只有「当日收发总数」两个标量，**没有**按群 / 用户 / 日分桶；跨天时 Go 在
//! `recordStats` 里把两个计数器清回 0（判据是 `lastMessageTime` 的日期前缀不是今天）。
//!
//! # 与 Go 的差异（明确列出）
//!
//! - **文件**：`botstats.db`（bbolt）→ `bot-stats.redb`（redb），页格式不同；表名与键名
//!   逐字一致，计数器 value 仍是十进制 ASCII，JSONL 之外无需重新编码。
//! - **`lastMessageTime`**：Go 存本地时区的 RFC3339 字符串，这里存**十进制 Unix 秒**——
//!   Go `GetStats` 返回的也是 Unix 秒，读接口语义不变，但跨天清零按 **UTC 日界**而非本地
//!   日界（东八区最多相差 8 小时）。
//! - **损坏 value**：Go `getInt` 用 `strconv.Atoi` 并忽略错误（解析失败当 0）；这里返回
//!   [`StorageError::Corrupt`]，不静默把统计清零。
//! - **未初始化**：Go 用包级 `db` 指针，nil 时 `recordStats` 只打日志；这里没有全局状态，
//!   「没装配统计库」由 `Option<Arc<BotStatsStore>>` 表达（见 `gensokyo-runtime::AppState`）。
//! - **读接口**：Go `GetStats` 额外返回 `error`（db 未初始化）；这里由调用方先判 `Option`。

use std::path::Path;

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use crate::StorageError;
use crate::idmap::{rb, unix_now};

/// 默认库文件名（Go `botstats.db`；扩展名随 redb 改）。
pub const DEFAULT_BOTSTATS_PATH: &str = "bot-stats.redb";

/// 唯一的表，对应 Go 的 bucket `stats`。
const STATS: TableDefinition<'static, &'static str, &'static [u8]> = TableDefinition::new("stats");

/// Go `messageReceivedKey`。
const MESSAGE_RECEIVED_KEY: &str = "messageReceived";
/// Go `messageSentKey`。
const MESSAGE_SENT_KEY: &str = "messageSent";
/// Go `lastMessageTimeKey`。
const LAST_MESSAGE_TIME_KEY: &str = "lastMessageTime";

/// 一天的秒数：跨天判据按 Unix 秒整除取日序号。
const SECONDS_PER_DAY: i64 = 86_400;

/// 当日统计快照（Go `GetStats` 的三个返回值）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BotStats {
    /// 当日收到（`messageReceived`）。
    pub message_received: i64,
    /// 当日发出（`messageSent`）。
    pub message_sent: i64,
    /// 最后一次统计写入时刻（Unix 秒；从未写入过为 0）。
    pub last_message_time: i64,
}

/// 收发消息统计库。
#[derive(Debug)]
pub struct BotStatsStore {
    db: Database,
}

impl BotStatsStore {
    /// 打开（必要时创建）统计库并建表。
    ///
    /// # Errors
    ///
    /// 路径不可写、或 redb 打开 / 建表失败时返回 [`StorageError::Redb`]。
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let db = Database::create(path).map_err(rb)?;
        let store = Self { db };
        let txn = store.db.begin_write().map_err(rb)?;
        {
            // 先建表，后续读事务才不需要处理「表不存在」。
            txn.open_table(STATS).map_err(rb)?;
        }
        txn.commit().map_err(rb)?;
        Ok(store)
    }

    /// 记一条收到的消息（Go `RecordMessageReceived` 的落盘部分）。
    ///
    /// # Errors
    ///
    /// 事务失败或库里已有损坏 value 时返回 [`StorageError`]。
    pub fn record_received(&self) -> Result<(), StorageError> {
        self.record(1, 0)
    }

    /// 记一条发出的消息（Go `RecordMessageSent` 的落盘部分）。
    ///
    /// # Errors
    ///
    /// 同 [`BotStatsStore::record_received`]。
    pub fn record_sent(&self) -> Result<(), StorageError> {
        self.record(0, 1)
    }

    /// 读当日快照（Go `GetStats`）。
    ///
    /// # Errors
    ///
    /// 读事务失败或 value 不是十进制整数时返回 [`StorageError`]。
    pub fn get_stats(&self) -> Result<BotStats, StorageError> {
        let txn = self.db.begin_read().map_err(rb)?;
        let table = txn.open_table(STATS).map_err(rb)?;
        Ok(BotStats {
            message_received: read_int(&table, MESSAGE_RECEIVED_KEY)?,
            message_sent: read_int(&table, MESSAGE_SENT_KEY)?,
            last_message_time: read_int(&table, LAST_MESSAGE_TIME_KEY)?,
        })
    }

    /// 写入一次增量，并维护 `lastMessageTime`。
    ///
    /// 与 Go `recordStats` 一致：`lastMessageTime` 不是今天时先把两个计数器清 0，本次增量
    /// 只落在新的一天上（库是「当日口径」，历史不会累积）。从未写过（值缺失）不清零。
    fn record(&self, received: i64, sent: i64) -> Result<(), StorageError> {
        let now = unix_now();
        let txn = self.db.begin_write().map_err(rb)?;
        {
            let mut table = txn.open_table(STATS).map_err(rb)?;
            let last = read_int(&table, LAST_MESSAGE_TIME_KEY)?;
            let crossed_day = last != 0 && last / SECONDS_PER_DAY != now / SECONDS_PER_DAY;
            let base_received = if crossed_day {
                0
            } else {
                read_int(&table, MESSAGE_RECEIVED_KEY)?
            };
            let base_sent = if crossed_day {
                0
            } else {
                read_int(&table, MESSAGE_SENT_KEY)?
            };
            let received = base_received.saturating_add(received).to_string();
            let sent = base_sent.saturating_add(sent).to_string();
            let now = now.to_string();
            table
                .insert(MESSAGE_RECEIVED_KEY, received.as_bytes())
                .map_err(rb)?;
            table
                .insert(MESSAGE_SENT_KEY, sent.as_bytes())
                .map_err(rb)?;
            table
                .insert(LAST_MESSAGE_TIME_KEY, now.as_bytes())
                .map_err(rb)?;
        }
        txn.commit().map_err(rb)?;
        Ok(())
    }
}

/// 读一个十进制整数 value；缺失按 Go `getInt` 的语义当 0。
fn read_int(
    table: &impl ReadableTable<&'static str, &'static [u8]>,
    key: &str,
) -> Result<i64, StorageError> {
    let Some(value) = table.get(key).map_err(rb)? else {
        return Ok(0);
    };
    let text = std::str::from_utf8(value.value())
        .map_err(|err| StorageError::Corrupt(format!("{key} 不是 UTF-8: {err}")))?;
    text.parse::<i64>()
        .map_err(|err| StorageError::Corrupt(format!("{key} 不是十进制整数: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn store() -> (BotStatsStore, TempDir) {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join(DEFAULT_BOTSTATS_PATH);
        let store = BotStatsStore::open(&path).expect("打开 botstats");
        (store, dir)
    }

    #[test]
    fn counts_round_trip_and_survive_reopen() {
        let (store, dir) = store();
        store.record_received().expect("记收");
        store.record_received().expect("记收");
        store.record_sent().expect("记发");
        let stats = store.get_stats().expect("读统计");
        assert_eq!((stats.message_received, stats.message_sent), (2, 1));
        assert!(stats.last_message_time > 0, "写入后应记下时刻");
        drop(store);

        // 重开同一文件：当日计数还在（Go 的 bbolt 同样持久化）。
        let reopened = BotStatsStore::open(dir.path().join(DEFAULT_BOTSTATS_PATH)).expect("重开");
        let stats = reopened.get_stats().expect("读统计");
        assert_eq!((stats.message_received, stats.message_sent), (2, 1));
    }

    #[test]
    fn counters_reset_when_last_write_was_another_day() {
        let (store, _dir) = store();
        let yesterday = unix_now() - SECONDS_PER_DAY;
        // 直接写入「昨天」的库状态：Go 靠同一个判据（日期前缀不同）清零。
        let txn = store.db.begin_write().expect("写事务");
        {
            let mut table = txn.open_table(STATS).expect("开表");
            table
                .insert(MESSAGE_RECEIVED_KEY, b"7".as_slice())
                .expect("写收");
            table
                .insert(MESSAGE_SENT_KEY, b"9".as_slice())
                .expect("写发");
            table
                .insert(LAST_MESSAGE_TIME_KEY, yesterday.to_string().as_bytes())
                .expect("写时间");
        }
        txn.commit().expect("提交");

        store.record_sent().expect("记发");
        let stats = store.get_stats().expect("读统计");
        assert_eq!(
            (stats.message_received, stats.message_sent),
            (0, 1),
            "跨天应清零后只记本次增量"
        );
    }

    #[test]
    fn corrupt_counter_is_reported_not_silently_zeroed() {
        let (store, _dir) = store();
        let txn = store.db.begin_write().expect("写事务");
        {
            let mut table = txn.open_table(STATS).expect("开表");
            table
                .insert(MESSAGE_RECEIVED_KEY, b"abc".as_slice())
                .expect("写坏值");
        }
        txn.commit().expect("提交");
        assert!(matches!(store.get_stats(), Err(StorageError::Corrupt(_))));
    }
}
