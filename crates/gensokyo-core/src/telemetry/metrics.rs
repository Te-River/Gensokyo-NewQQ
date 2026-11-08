//! 轻量指标注册表（原子计数）与 Prometheus 文本导出。
//!
//! 指标名 / help / 类型 / 数值格式逐字对齐 Go `main.go:1141-1177` 的 `MetricsHandler`：
//!
//! | 指标 | 类型 | 来源（Go） |
//! |------|------|------------|
//! | `gensokyo_uptime_seconds` | gauge | 读时计算 `time.Since(mylog.StartTime)` |
//! | `gensokyo_messages_received_total` | counter | `mylog.MetricMsgReceived`（`botstats.RecordMessageReceived`） |
//! | `gensokyo_messages_sent_total` | counter | `mylog.MetricMsgSent`（`botstats.RecordMessageSent`） |
//! | `gensokyo_errors_total` | counter | `mylog.MetricErrorCount`——**全仓零调用**，Go 侧恒为 0 |
//! | `gensokyo_slow_events_total` | counter | `mylog.MetricSlowEvents`（`runWithTimer` 超时，`main.go:726`） |
//! | `gensokyo_memory_allocated_bytes` | gauge | 读时计算 `runtime.ReadMemStats().Alloc` |
//!
//! 端点（`GET /metrics`，`text/plain; version=0.0.4`）在 `gensokyo-runtime::http_api`，
//! 与 Go 挂在 HTTP API listener 上的那份一样不鉴权。

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// 单调递增计数器。
#[derive(Debug, Default)]
pub struct Counter(AtomicU64);

impl Counter {
    /// `const` 构造，便于在 `static` 中声明。
    #[must_use]
    pub const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    /// 计数 +1。
    pub fn inc(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    /// 计数 +n。
    pub fn add(&self, n: u64) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }

    /// 读取当前值。
    #[must_use]
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// 收消息计数（Go `mylog.MetricMsgReceived`）：入站消息事件处理成功后 +1。
pub static MESSAGES_RECEIVED: Counter = Counter::new();

/// 发消息计数（Go `mylog.MetricMsgSent`）：一条 deferred 发送计划执行成功后 +1。
pub static MESSAGES_SENT: Counter = Counter::new();

/// 日志错误计数（Go `mylog.MetricErrorCount`）。
///
/// Go 侧只有定义、没有任何递增点，`/metrics` 上恒为 0；这里保留同名指标是为了指标集合一致。
pub static ERRORS: Counter = Counter::new();

/// 慢事件计数（Go `mylog.MetricSlowEvents`）：处理耗时超过阈值的入站事件 +1。
pub static SLOW_EVENTS: Counter = Counter::new();

/// uptime 起点。Go 在包初始化时记 `mylog.StartTime`；这里由 [`mark_start`] 在运行时装配
/// 处锚定，未锚定时退回首次读取的时刻。
static START: OnceLock<Instant> = OnceLock::new();

/// 锚定 uptime 起点（幂等，重复调用无副作用）。
pub fn mark_start() {
    START.get_or_init(Instant::now);
}

/// 进程已运行秒数（uptime 指标）。
#[must_use]
pub fn uptime_seconds() -> f64 {
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// Prometheus 文本格式（`version=0.0.4`）的完整响应体。
///
/// 行序、help、类型与数值格式与 Go 输出一致（uptime 两位小数）。
#[must_use]
pub fn render_prometheus_text() -> String {
    let uptime = format!("{:.2}", uptime_seconds());
    let mut out = format!(
        "# HELP gensokyo_uptime_seconds Uptime of the bot in seconds.\n\
         # TYPE gensokyo_uptime_seconds gauge\n\
         gensokyo_uptime_seconds {uptime}\n\
         # HELP gensokyo_messages_received_total Total number of received messages.\n\
         # TYPE gensokyo_messages_received_total counter\n\
         gensokyo_messages_received_total {}\n\
         # HELP gensokyo_messages_sent_total Total number of sent messages.\n\
         # TYPE gensokyo_messages_sent_total counter\n\
         gensokyo_messages_sent_total {}\n\
         # HELP gensokyo_errors_total Total number of log errors.\n\
         # TYPE gensokyo_errors_total counter\n\
         gensokyo_errors_total {}\n\
         # HELP gensokyo_slow_events_total Total number of slow processing events.\n\
         # TYPE gensokyo_slow_events_total counter\n\
         gensokyo_slow_events_total {}\n\
         # HELP gensokyo_memory_allocated_bytes Memory currently allocated in bytes.\n\
         # TYPE gensokyo_memory_allocated_bytes gauge\n",
        MESSAGES_RECEIVED.get(),
        MESSAGES_SENT.get(),
        ERRORS.get(),
        SLOW_EVENTS.get(),
    );
    // 取不到内存口径的平台上只输出 HELP/TYPE（Prometheus 允许空样本的指标族），不编造 0。
    if let Some(bytes) = memory_allocated_bytes() {
        out.push_str(&format!("gensokyo_memory_allocated_bytes {bytes}\n"));
    }
    out
}

/// 进程当前内存占用（字节）。
///
/// Go 读 `runtime.ReadMemStats().Alloc`（堆上存活对象）。Rust 没有零依赖的等价堆统计：
/// Linux 上读 `/proc/self/status` 的 `VmRSS`（常驻内存，含堆以外的映射，量级可比但不可
/// 逐值对齐）；其余平台没有零依赖 API，返回 `None`。
#[cfg(target_os = "linux")]
fn memory_allocated_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(kib.saturating_mul(1024))
}

/// 进程当前内存占用（字节）：见 Linux 实现的说明，非 Linux 平台没有零依赖来源。
#[cfg(not(target_os = "linux"))]
fn memory_allocated_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 把渲染结果里某个指标的最后一条样本值取出来。
    fn sample(text: &str, name: &str) -> Option<u64> {
        text.lines()
            .filter_map(|line| line.strip_prefix(name)?.strip_prefix(' '))
            .next_back()
            .and_then(|value| value.trim().parse().ok())
    }

    #[test]
    fn rendered_text_carries_every_go_metric() {
        let text = render_prometheus_text();
        for name in [
            "gensokyo_uptime_seconds",
            "gensokyo_messages_received_total",
            "gensokyo_messages_sent_total",
            "gensokyo_errors_total",
            "gensokyo_slow_events_total",
            "gensokyo_memory_allocated_bytes",
        ] {
            assert!(text.contains(&format!("# HELP {name} ")), "{name} 缺 HELP");
            assert!(text.contains(&format!("# TYPE {name} ")), "{name} 缺 TYPE");
        }
        // 除了内存（平台相关）以外，每个指标都必须有样本行。
        for name in [
            "gensokyo_uptime_seconds",
            "gensokyo_messages_received_total",
            "gensokyo_messages_sent_total",
            "gensokyo_errors_total",
            "gensokyo_slow_events_total",
        ] {
            assert!(text.contains(&format!("\n{name} ")), "{name} 缺样本行");
        }
    }

    #[test]
    fn counter_growth_shows_up_in_the_rendered_text() {
        let before = MESSAGES_SENT.get();
        MESSAGES_SENT.add(3);
        let text = render_prometheus_text();
        assert_eq!(
            sample(&text, "gensokyo_messages_sent_total"),
            Some(before + 3),
            "渲染值应跟随计数器\n{text}"
        );
    }

    #[test]
    fn uptime_starts_at_mark_and_is_a_gauge() {
        mark_start();
        let text = render_prometheus_text();
        let line = text
            .lines()
            .find(|line| line.starts_with("gensokyo_uptime_seconds "))
            .expect("uptime 样本行");
        let value: f64 = line
            .split_whitespace()
            .nth(1)
            .expect("样本值")
            .parse()
            .expect("uptime 应是浮点");
        assert!(value >= 0.0);
        assert!(text.contains("# TYPE gensokyo_uptime_seconds gauge"));
    }
}
