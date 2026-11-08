//! 日志中枢：环形缓冲 + 广播（Go `mylog/mylog.go:426-618` 的进程级通道）。
//!
//! Go 侧是包级 `logChannel`（容量 1000、非阻塞投递）+ `WsHandler` 广播给所有 WebUI 连接
//! （消息体 `EnhancedLogEntry{time,level,message}`）。这里拆成两端：
//!
//! - **生产**：`gensokyo-bin` 的 tracing 订阅器把每条已过滤事件推进 [`hub`]；
//! - **消费**：WebUI 的 `/api/logs` 与 `/api/{uin}/process/logs` 在 WS 升级后先补发
//!   缓冲快照，再逐条推送新事件。
//!
//! 与 Go 的差异：Go 没有历史补发（连上之前发生的日志收不到），缓冲快照是 Rust 的扩展；
//! JSON 字段与时间格式照 Go 逐字对齐（见 [`LogEntry`]）。

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use serde::Serialize;
use tokio::sync::broadcast;

/// 环形缓冲容量（条）。Go 的 `logChannel` 容量是 1000 且不留历史，这里保留最近 500 条供补发。
pub const LOG_BUFFER_CAPACITY: usize = 500;

/// 一条 WebUI 日志。JSON 字段照 Go `mylog.EnhancedLogEntry`（`time` / `level` / `message`）。
#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    /// 时间串。格式同 Go：`strings.ToUpper(time.Now().Format("2006/Jan/02 PM 03:04:05"))`，
    /// 即 `2026/NOV/08 PM 03:04:05`（12 小时制、本地时区）。
    pub time: String,
    /// 级别，大写：`TRACE` / `DEBUG` / `INFO` / `WARN` / `ERROR`。
    ///
    /// Go 的 `emitLog` 只有 `INFO` / `WARN` / `ERROR` / `FATAL` 四种；Rust 侧按 tracing 级别
    /// 输出，多出 `TRACE` / `DEBUG`，`FATAL` 没有对应物（进程直接退出）。
    pub level: String,
    /// 正文。Go 是单条格式化消息；Rust 把结构化字段按 ` key=value` 追加在消息之后，
    /// 与 stderr 打印的内容一致。
    pub message: String,
}

/// 进程级日志中枢：环形缓冲 + `tokio::sync::broadcast`。
#[derive(Debug)]
pub struct LogHub {
    buffer: Mutex<VecDeque<LogEntry>>,
    capacity: usize,
    tx: broadcast::Sender<LogEntry>,
}

impl LogHub {
    /// 新中枢；`capacity` 为 0 时按 1 处理（broadcast 不接受 0 容量）。
    ///
    /// 缓冲与广播用同一个容量：慢客户端落后超过窗口就丢缺口（Go `emitLog` 的投递同样非阻塞丢弃）。
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let (tx, _rx) = broadcast::channel(capacity);
        Self {
            buffer: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
            tx,
        }
    }

    /// 记录一条日志：入缓冲（超容量丢最旧）并广播给所有订阅者。
    ///
    /// 入缓冲与广播在同一把锁内完成，[`LogHub::subscribe_with_history`] 因此只能排在
    /// 某次 `push` 的全程之前或之后——订阅者不会既在快照里看到某条、又从广播里再收一次。
    pub fn push(&self, entry: LogEntry) {
        let mut buffer = self.lock();
        if buffer.len() >= self.capacity {
            buffer.pop_front();
        }
        buffer.push_back(entry.clone());
        // 没有订阅者（还没有 WebUI 连上）时返回 Err，属正常情况。
        let _ = self.tx.send(entry);
    }

    /// 订阅实时日志并返回当前缓冲快照（旧到新）。先订阅后取快照，见 [`LogHub::push`]。
    #[must_use]
    pub fn subscribe_with_history(&self) -> (Vec<LogEntry>, broadcast::Receiver<LogEntry>) {
        let buffer = self.lock();
        let rx = self.tx.subscribe();
        (buffer.iter().cloned().collect(), rx)
    }

    fn lock(&self) -> MutexGuard<'_, VecDeque<LogEntry>> {
        // Go 的 channel 没有"锁中毒"概念：持锁线程 panic 后继续用缓冲，不把 WebUI 一起拖死。
        self.buffer.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// 进程级单例：采集层写它，WebUI 读它（等价 Go 的包级 `logChannel`）。
#[must_use]
pub fn hub() -> &'static LogHub {
    static HUB: OnceLock<LogHub> = OnceLock::new();
    HUB.get_or_init(|| LogHub::new(LOG_BUFFER_CAPACITY))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{LogEntry, LogHub};

    fn entry(message: &str) -> LogEntry {
        LogEntry {
            time: "2026/NOV/08 PM 03:04:05".to_owned(),
            level: "INFO".to_owned(),
            message: message.to_owned(),
        }
    }

    #[test]
    fn buffer_keeps_only_the_newest_entries() {
        let hub = LogHub::new(2);
        for message in ["first", "second", "third"] {
            hub.push(entry(message));
        }

        let (history, _rx) = hub.subscribe_with_history();
        let messages: Vec<&str> = history.iter().map(|e| e.message.as_str()).collect();
        assert_eq!(messages, ["second", "third"], "超容量应丢最旧的一条");
    }

    #[test]
    fn zero_capacity_is_clamped_to_one() {
        let hub = LogHub::new(0);
        hub.push(entry("first"));
        hub.push(entry("second"));

        let (history, _rx) = hub.subscribe_with_history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].message, "second");
    }

    #[test]
    fn entry_json_matches_go_enhanced_log_entry() {
        let value = serde_json::to_value(entry("hello")).expect("序列化不会失败");
        assert_eq!(
            value,
            json!({"time": "2026/NOV/08 PM 03:04:05", "level": "INFO", "message": "hello"}),
            "字段必须与 Go EnhancedLogEntry 逐字一致"
        );
    }
}
