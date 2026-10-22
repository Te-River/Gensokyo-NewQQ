//! 日志与指标门面（Go `mylog` 的替代）。
//!
//! M0 只提供门面：`tracing` 订阅器（文件 sink / JSON / 彩色输出）的选型与落地放在 M1，
//! 避免在依赖准入评审前锁死订阅器实现。

pub mod metrics;

/// 记录一条启动日志。字段式输出，禁止字符串拼接（design 附录 B）。
pub fn log_startup(crate_name: &str) {
    tracing::info!(crate_name, "gensokyo 启动");
}

/// 当前 UTC 时间戳（毫秒）。
#[must_use]
pub fn unix_millis() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}
