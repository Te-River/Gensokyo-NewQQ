//! 发送队列与限速（Go `messagequeue/`，P10）。
//!
//! Go 侧是两块互不相干的设施：
//! - `messagequeue/ratelimit.go`：全局令牌桶 5qps / burst 10 + `WaitWithTimeout(5s)`；唯一调用点
//!   `handlers/send_group_msg.go:3065`（`postGroupMessageWithRetry` 的每次尝试之前）。
//! - `messagequeue/queue.go`：`Queue` + `StartWorker`——入队后由单 worker 串行取出，每条发送前
//!   睡 `send_delay`（`<= 0` 取 300ms），失败重试最多 3 次、线性退避 `1s × n`。
//!
//! **审计结论（决定本模块哪半是活路径）**：`StartWorker` / `Enqueue` / `GetQueue` 在 Go 全仓
//! **零调用**，`queue.go` 是一套没有启用点的设施；生产发送真正的退避语义在
//! `handlers/retry_policy.go`（`MaxAttempts = 3`、只对投递超时重试、`Backoff(attempt) =
//! attempt × 1s`），由 `postGroupMessageWithRetry` / `postC2CMessageWithRetry` 使用。因此：
//! - [`RateLimiter`]、[`should_retry`]、[`backoff`] 是**活路径**，被
//!   [`crate::deferred::send_body`]（发送唯一收口）直接调用；
//! - [`SendQueue`] 逐条对齐 `queue.go` 的 worker 语义并给全生命周期（[`SendQueue::close`] ↔
//!   Go `Queue.Close`），但**默认不启动**——与 Go 同构，启动与否是独立决定，不是本模块的默认
//!   行为（见 `reports/refactor/17-go-rust-parity.md` 的 B5 条目）。

use std::fmt;
use std::future::Future;
use std::sync::OnceLock;
use std::time::Duration;

use gensokyo_core::{Error, Result};
use tokio::sync::{Mutex, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

/// `send_delay` 缺省 / 兜底值：Go `StartWorker` 的 `delay <= 0 → 300ms`。
pub const DEFAULT_SEND_DELAY: Duration = Duration::from_millis(300);

/// 令牌生成速率（Go `rate.NewLimiter(rate.Limit(5), 10)`，按 QQ 官方约 5qps 的频控口径）。
pub const QPS: f64 = 5.0;

/// 令牌桶容量（突发允许量，Go 同上 10）。
pub const BURST: u32 = 10;

/// 单次取令牌的等待上限（Go `WaitWithTimeout(5 * time.Second)`）。
pub const WAIT_TIMEOUT: Duration = Duration::from_secs(5);

/// 发送最多尝试次数（Go `RetryPolicy{MaxAttempts: 3}`：首试 + 2 次重试）。
pub const MAX_ATTEMPTS: u32 = 3;

/// 线性退避单位（Go `Backoff(attempt) = attempt * time.Second`）。
pub const BACKOFF_STEP: Duration = Duration::from_secs(1);

/// 队列 worker 的最大重试次数（Go `Enqueue` 里的 `task.MaxRetries = 3`：首试 + 3 次重试）。
pub const MAX_QUEUE_RETRIES: u32 = 3;

/// 配置里的 `send_delay`（毫秒，`i64`）换算成等待间隔，`<= 0` 落回 [`DEFAULT_SEND_DELAY`]。
///
/// 逐条对齐 Go `messagequeue/queue.go:98-101`。
#[must_use]
pub fn effective_delay(configured_ms: i64) -> Duration {
    if configured_ms <= 0 {
        return DEFAULT_SEND_DELAY;
    }
    Duration::from_millis(u64::try_from(configured_ms).unwrap_or(u64::MAX))
}

/// 本次失败是否值得重试（Go `RetryPolicy.ShouldRetry`：`attempt + 1 < MaxAttempts &&
/// IsDeliveryTimeout(err)`）。
///
/// 频率限制（`22009`）、事件过期（`400340xx`）等业务错误**不**重试——重试只会再撞一次同样的
/// 闸门，而且这条路径上失败要落 SSM 队列（见 [`crate::deferred`] 的模块文档）。
#[must_use]
pub fn should_retry(err: &Error, attempt: u32) -> bool {
    attempt + 1 < MAX_ATTEMPTS && is_delivery_timeout(err)
}

/// Go `IsDeliveryTimeout`：`context deadline exceeded` 与富媒体上传超时。
///
/// Rust 的传输层没有独立的超时变体（`gensokyo-qq` 把连接 / TLS / 超时 / 读体失败统统归进
/// [`Error::Network`]），所以这里按文案判定，而不是「只要是 Network 就重试」：连接被重置时
/// 重发会造出重复消息，Go 同样不重试这一类。
fn is_delivery_timeout(err: &Error) -> bool {
    let Error::Network(message) = err else {
        return false;
    };
    let text = message.to_lowercase();
    ["timed out", "timeout", "deadline", "超时"]
        .iter()
        .any(|needle| text.contains(needle))
}

/// 第 `attempt` 次重试前的线性退避（Go `Backoff`；`attempt == 0` 不退避）。
#[must_use]
pub fn backoff(attempt: u32) -> Duration {
    BACKOFF_STEP * attempt
}

/// 令牌桶：Go `golang.org/x/time/rate` 的 5qps / burst 10，语义对齐 `WaitWithTimeout`。
///
/// 与 Go 的差别只在排队公平性：`rate.Limiter.Wait` 会按 FIFO 预约未来的令牌，这里用「桶空就
/// 睡到下一个令牌生成」的近似，突发额度与平均速率一致（单 worker 串行发送的整体节奏也一致）。
#[derive(Debug)]
pub struct RateLimiter {
    bucket: Mutex<Bucket>,
    qps: f64,
    burst: u32,
}

/// 桶状态：`tokens` 按经过时间线性回填，上限 `burst`。
#[derive(Debug)]
struct Bucket {
    tokens: f64,
    updated: Instant,
}

impl RateLimiter {
    /// 构造一个限速器；`burst` 至少为 1（否则永远取不到令牌）。
    #[must_use]
    pub fn new(qps: f64, burst: u32) -> Self {
        let burst = burst.max(1);
        Self {
            bucket: Mutex::new(Bucket {
                tokens: f64::from(burst),
                updated: Instant::now(),
            }),
            qps,
            burst,
        }
    }

    /// 生产用全局实例（Go `GetRateLimiter` 的 `sync.Once` 单例）。
    ///
    /// 必须是进程级共享的：每次 `Dispatcher` 现建一个的话，桶会随请求被重置，等于没有限速。
    pub fn shared() -> &'static Self {
        static SHARED: OnceLock<RateLimiter> = OnceLock::new();
        SHARED.get_or_init(|| Self::new(QPS, BURST))
    }

    /// 取一个令牌，最多等 `timeout`；等不到返回 `false`（Go `WaitWithTimeout`）。
    pub async fn wait_with_timeout(&self, timeout: Duration) -> bool {
        tokio::time::timeout(timeout, self.acquire()).await.is_ok()
    }

    /// 阻塞到拿到令牌：桶空时睡到下个令牌生成，再试一次。
    async fn acquire(&self) {
        loop {
            let wait = {
                let mut bucket = self.bucket.lock().await;
                let now = Instant::now();
                let elapsed = now.saturating_duration_since(bucket.updated);
                bucket.updated = now;
                bucket.tokens =
                    (bucket.tokens + elapsed.as_secs_f64() * self.qps).min(f64::from(self.burst));
                if bucket.tokens >= 1.0 {
                    bucket.tokens -= 1.0;
                    return;
                }
                Duration::from_secs_f64((1.0 - bucket.tokens) / self.qps)
            };
            tokio::time::sleep(wait).await;
        }
    }
}

/// `SendQueue` worker 的可调参数；[`QueueOptions::default`] 即 Go 口径。
#[derive(Debug, Clone, Copy)]
pub struct QueueOptions {
    /// 每条发送前的间隔（Go `StartWorker` 的 `delay`）。
    pub delay: Duration,
    /// 线性退避单位（Go `time.Sleep(time.Second * RetryCount)` 的 1s）。
    pub backoff_step: Duration,
    /// 最大重试次数（Go `task.MaxRetries`）。
    pub max_retries: u32,
}

impl Default for QueueOptions {
    fn default() -> Self {
        Self {
            delay: DEFAULT_SEND_DELAY,
            backoff_step: BACKOFF_STEP,
            max_retries: MAX_QUEUE_RETRIES,
        }
    }
}

/// 队列里的一条任务：发送载荷 + 结果回执。
struct Job<T> {
    task: T,
    reply: oneshot::Sender<Result<String>>,
}

/// 串行发送队列：Go `messagequeue.Queue` + `StartWorker` 的 Rust 版。
///
/// 语义逐条对齐 `queue.go`：单 worker、按入队顺序一条一条发、每条前睡 `delay`、失败最多重试
/// `max_retries` 次（线性退避 `backoff_step × n`）后记录日志并丢弃、[`SendQueue::close`] 后
/// 不再接收新任务且 worker 退出。句柄可克隆，克隆共享同一条队列。
///
/// 与 Go 的两点差异（都写在 `queue.go` 的注释旁边）：
/// - Go 重试是把任务**重新入队**（排到队尾）；这里原地重试。单 worker 且队列空时两者等价，
///   队列非空时 Go 会把重试顺延到队尾，这里会更快重试。
/// - Go 重试耗尽时静默丢弃；这里记一条 `warn`（排障需要，行为不变量不变）。
pub struct SendQueue<T> {
    tx: mpsc::UnboundedSender<Job<T>>,
    closed: watch::Sender<bool>,
}

impl<T> Clone for SendQueue<T> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            closed: self.closed.clone(),
        }
    }
}

impl<T> fmt::Debug for SendQueue<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SendQueue").finish_non_exhaustive()
    }
}

impl<T: Clone + Send + 'static> SendQueue<T> {
    /// 启动一个 worker 并返回队列句柄与 worker 任务句柄（后者等价 Go `Queue.Wait` 的等待点）。
    ///
    /// `process` 是真正的发送动作：返回 `Ok(message_id)` 即成功，`Err` 走重试。与 Go 一样，
    /// 队列本身不判断错误类型——重试口径由调用方在 `process` 里决定（活路径用
    /// [`should_retry`]，只重试投递超时）。
    pub fn spawn<F, Fut>(options: QueueOptions, process: F) -> (Self, JoinHandle<()>)
    where
        F: Fn(T) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String>> + Send + 'static,
    {
        let (tx, mut rx) = mpsc::unbounded_channel::<Job<T>>();
        let (closed, mut close_rx) = watch::channel(false);
        let worker = tokio::spawn(async move {
            loop {
                let job = tokio::select! {
                    job = rx.recv() => job,
                    // 显式 close 或全部句柄被丢弃（watch 发送端没了）都收摊
                    _ = close_rx.changed() => None,
                };
                let Some(job) = job else {
                    return;
                };
                tokio::time::sleep(options.delay).await;
                run_job(&process, job, options).await;
            }
        });
        (Self { tx, closed }, worker)
    }

    /// 入队一条任务；队列已关闭时返回 `None`（Go `Enqueue` 在 `closed` 时静默丢弃）。
    ///
    /// 返回的接收端在 worker 处理完这条任务后拿到最终结果——同一任务的重试在 worker 内部完成，
    /// 调用方只会收到一次终局。
    #[must_use]
    pub fn submit(&self, task: T) -> Option<oneshot::Receiver<Result<String>>> {
        if *self.closed.borrow() {
            return None;
        }
        let (reply, receiver) = oneshot::channel();
        self.tx.send(Job { task, reply }).ok()?;
        Some(receiver)
    }

    /// 关闭队列：不再接新任务，worker 在下一次取任务前退出（已在发的那条不会被中断）。
    ///
    /// 对齐 Go `Queue.Close`——它也只是置 `closed` 并唤醒 worker。
    pub fn close(&self) {
        let _ = self.closed.send(true);
    }
}

/// worker 处理一条任务：发送 → 失败重试 `max_retries` 次（线性退避）→ 交回终局。
async fn run_job<T, F, Fut>(process: &F, job: Job<T>, options: QueueOptions)
where
    T: Clone + Send + 'static,
    F: Fn(T) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<String>> + Send + 'static,
{
    let mut attempt: u32 = 0;
    loop {
        match process(job.task.clone()).await {
            Ok(message_id) => {
                let _ = job.reply.send(Ok(message_id));
                return;
            }
            Err(err) => {
                if attempt >= options.max_retries {
                    tracing::warn!(
                        retries = attempt,
                        error = %err,
                        "消息队列：重试耗尽，丢弃该条"
                    );
                    let _ = job.reply.send(Err(err));
                    return;
                }
                attempt += 1;
                tracing::warn!(attempt, error = %err, "消息队列：发送失败，进入第 {attempt} 次重试");
                tokio::time::sleep(options.backoff_step * attempt).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    #[tokio::test]
    async fn rate_limiter_lets_the_burst_through_then_paces() {
        // 10qps / 桶深 1：第一个令牌立即可用，第二个要等约 100ms
        let limiter = RateLimiter::new(10.0, 1);
        let start = Instant::now();
        assert!(limiter.wait_with_timeout(WAIT_TIMEOUT).await);
        let first = start.elapsed();
        assert!(limiter.wait_with_timeout(WAIT_TIMEOUT).await);
        let second = start.elapsed() - first;
        assert!(
            first < Duration::from_millis(50),
            "首个令牌应即时：{first:?}"
        );
        assert!(
            second >= Duration::from_millis(60),
            "第二个令牌应节流：{second:?}"
        );
    }

    #[tokio::test]
    async fn rate_limiter_burst_is_allowed_at_once() {
        // 桶深 3：前三次不该等
        let limiter = RateLimiter::new(1.0, 3);
        let start = Instant::now();
        for _ in 0..3 {
            assert!(limiter.wait_with_timeout(WAIT_TIMEOUT).await);
        }
        assert!(
            start.elapsed() < Duration::from_millis(50),
            "burst 内应即时通过"
        );
    }

    #[tokio::test]
    async fn rate_limiter_reports_timeout_when_bucket_is_empty() {
        // 1qps / 桶深 1：令牌用掉后要等约 1s，20ms 的超时必然失败
        let limiter = RateLimiter::new(1.0, 1);
        assert!(limiter.wait_with_timeout(WAIT_TIMEOUT).await);
        assert!(!limiter.wait_with_timeout(Duration::from_millis(20)).await);
    }

    #[tokio::test]
    async fn queue_sleeps_the_configured_delay_before_each_send() {
        let sends = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&sends);
        let (queue, worker) = SendQueue::spawn(
            QueueOptions {
                delay: Duration::from_millis(40),
                backoff_step: Duration::ZERO,
                max_retries: 0,
            },
            move |task: u32| {
                counter.fetch_add(1, Ordering::SeqCst);
                async move { Ok(task.to_string()) }
            },
        );
        let start = Instant::now();
        let first = queue.submit(1).expect("应可入队");
        let second = queue.submit(2).expect("应可入队");
        let first = first.await.expect("回执未丢");
        let second = second.await.expect("回执未丢");
        assert_eq!(first.ok().as_deref(), Some("1"));
        assert_eq!(second.ok().as_deref(), Some("2"));
        // 两条各睡一个 delay，串行执行
        assert!(start.elapsed() >= Duration::from_millis(70), "间隔未生效");
        assert_eq!(sends.load(Ordering::SeqCst), 2);
        queue.close();
        worker.await.expect("worker 应正常退出");
    }

    #[tokio::test]
    async fn queue_retries_three_times_then_drops_the_task() {
        let attempts = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&attempts);
        let (queue, worker) = SendQueue::spawn(
            QueueOptions {
                delay: Duration::ZERO,
                backoff_step: Duration::ZERO,
                max_retries: 3,
            },
            move |_task: u32| {
                counter.fetch_add(1, Ordering::SeqCst);
                async { Err(Error::Network("timed out".to_owned())) }
            },
        );
        let outcome = queue.submit(7).expect("应可入队").await.expect("回执未丢");
        assert!(matches!(outcome, Err(Error::Network(_))));
        // 首试 + 3 次重试
        assert_eq!(attempts.load(Ordering::SeqCst), MAX_QUEUE_RETRIES + 1);
        queue.close();
        worker.await.expect("worker 应正常退出");
    }

    #[tokio::test]
    async fn queue_stops_after_a_successful_retry() {
        let attempts = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&attempts);
        let (queue, worker) = SendQueue::spawn(
            QueueOptions {
                delay: Duration::ZERO,
                backoff_step: Duration::ZERO,
                max_retries: 3,
            },
            move |task: u32| {
                let nth = counter.fetch_add(1, Ordering::SeqCst) + 1;
                async move {
                    if nth < 3 {
                        Err(Error::Network("timed out".to_owned()))
                    } else {
                        Ok(task.to_string())
                    }
                }
            },
        );
        let outcome = queue.submit(9).expect("应可入队").await.expect("回执未丢");
        assert_eq!(outcome.ok().as_deref(), Some("9"));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        queue.close();
        worker.await.expect("worker 应正常退出");
    }

    #[tokio::test]
    async fn close_stops_the_worker_and_rejects_new_tasks() {
        let (queue, worker) = SendQueue::spawn(QueueOptions::default(), |task: u32| async move {
            Ok(task.to_string())
        });
        queue.close();
        assert!(queue.submit(1).is_none(), "关闭后不应再接收任务");
        worker.await.expect("worker 应正常退出");
    }

    #[tokio::test]
    async fn worker_exits_when_every_handle_is_dropped() {
        let (queue, worker) = SendQueue::spawn(QueueOptions::default(), |task: u32| async move {
            Ok(task.to_string())
        });
        drop(queue);
        // 没有 close 也应随句柄释放一起收摊（否则进程里会留下空转任务）
        tokio::time::timeout(Duration::from_secs(1), worker)
            .await
            .expect("worker 应在句柄释放后退出")
            .expect("worker 应正常退出");
    }

    #[test]
    fn send_delay_zero_or_negative_falls_back_to_300ms() {
        assert_eq!(effective_delay(0), Duration::from_millis(300));
        assert_eq!(effective_delay(-5), Duration::from_millis(300));
        assert_eq!(effective_delay(80), Duration::from_millis(80));
    }

    #[test]
    fn retry_policy_matches_go() {
        let timeout = Error::Network("operation timed out".to_owned());
        assert!(should_retry(&timeout, 0));
        assert!(should_retry(&timeout, 1));
        // 第 3 次尝试后不再重试
        assert!(!should_retry(&timeout, 2));
        // 频控 / 业务错误不重试：22009 要走 SSM，不能靠重试硬闯
        let rate_limited = Error::Upstream {
            code: 22009,
            message: "msgseq 去重".to_owned(),
        };
        assert!(!should_retry(&rate_limited, 0));
        // 非超时的网络错误同样不重试（重发会造出重复消息）
        assert!(!should_retry(
            &Error::Network("connection reset".to_owned()),
            0
        ));
    }

    #[test]
    fn backoff_is_linear_in_seconds() {
        assert_eq!(backoff(0), Duration::ZERO);
        assert_eq!(backoff(1), Duration::from_secs(1));
        assert_eq!(backoff(2), Duration::from_secs(2));
    }
}
