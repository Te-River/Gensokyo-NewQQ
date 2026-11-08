//! post_url 事件上报（Go `Processor/Processor.go:401-488`）。
//!
//! Go 在 `BroadcastMessageToAll` 末尾发现 `post_url` 非全空时执行
//! `go PostMessageToUrls(message)`：把刚发给 WS 客户端的同一条 OneBot 事件 JSON 并发
//! POST 到列表里的每个 URL，**不重试、不看状态码**，失败只记日志。
//!
//! Rust 侧没有 `BroadcastMessageToAll` 这条同步路径：入站转换只往
//! [`crate::event_hub::EventHub`] 发布，反向 / 正向 WS 各自订阅。因此这里同样订阅总线，
//! 逐事件上报，语义对齐 Go：
//!
//! | 维度 | Go | 这里 |
//! |------|----|------|
//! | 触发 | 每条 `BroadcastMessageToAll` | 每条 hub 事件（发布方不等待） |
//! | 并发 | 每条事件一个 goroutine，URL 之间并发 | 每条事件一个任务（`JoinSet`），URL 之间 `join_all` 并发 |
//! | body | `ConvertMapToJSONString(message)`，即发给 WS 的同一份事件 JSON | `serde_json::to_string(event)` |
//! | 请求头 | `Content-Type: application/json` + `X-Self-ID: uin 或 app_id` | 同左 |
//! | 鉴权 | 无（`GetPostSecret` 在 Go 里没有任何调用点） | 无 |
//! | 失败 | 记日志后放弃，不重试 | 同左（`warn`） |
//! | 配置 | 每条事件现读 `config.GetPostUrl()`，热重载即时生效 | 同左 |
//!
//! # 与 Go 的差异
//!
//! - URL 列表**逐项 trim 并丢弃空串**：Go 只在「全部为空」时跳过（`allEmpty`），混着空串时
//!   会对空 URL 发一次必然失败的请求并打一条错误日志；模板默认值正是 `post_url: [""]`。
//! - 单请求超时取 [`gensokyo_qq::ReqwestTransport`] 的 10s（`DEFAULT_REQUEST_TIMEOUT`），
//!   Go 是 `http.Client{Timeout: 30 * time.Second}`。
//! - 请求带 `User-Agent: gensokyo-qq/<ver>`（Go 是 `Go-http-client/1.1`）；两者都跟随重定向。
//! - 关闭时在途上报直接取消：Go 的 goroutine 随进程退出，同样没有「等上报完」的语义。
//! - 总线积压（`Lagged`）时落后的条数直接丢弃，与反向 WS 一致。
//! - Go 在 `disable_error_chan=true`（性能模式）时群消息走 `BroadcastMessageToAllFAF`，
//!   那条路径**没有** post_url 上报；这里是总线订阅，性能模式下同样上报（保留功能而非照抄
//!   该分支的遗漏）。默认 `disable_error_chan=false` 时两者一致。
//!
//! 配置为空时任务照常订阅总线（**空转**）：Go 的判定发生在每条事件上，配置热重载新增 URL
//! 后下一条事件就会开始上报，这里保持同一口径。

use std::sync::Arc;
use std::time::Duration;

use gensokyo_config::Settings;
use gensokyo_protocol::Event;
use gensokyo_qq::{HttpRequest, HttpTransport, ReqwestTransport};
use tokio::sync::{broadcast, watch};
use tokio::task::{JoinHandle, JoinSet};

use crate::state::AppState;

/// POST 体的内容类型（Go `sendPostRequest`）。
const CONTENT_TYPE: &str = "application/json";

/// 上报入口（无状态，方法与 [`crate::boot::Runtime`] 同构）。
#[derive(Debug)]
pub struct PostUrlReporter;

impl PostUrlReporter {
    /// 用 `gensokyo-qq` 的 [`ReqwestTransport`] 启动上报。
    ///
    /// 传输构建失败（二进制未安装 rustls CryptoProvider）时只 `warn`：上报不启动，
    /// 事件照旧走 WS，不影响其他链路。生产路径由 `gensokyo-bin` 启动时安装 provider。
    #[must_use]
    pub fn start(state: AppState) -> PostUrlReporterHandle {
        match ReqwestTransport::new() {
            Ok(transport) => Self::start_with(state, transport),
            Err(err) => {
                tracing::warn!(error = %err, "post_url 事件上报未启动：HTTP 传输构建失败");
                PostUrlReporterHandle::idle()
            }
        }
    }

    /// 同 [`PostUrlReporter::start`]，由调用方注入 HTTP 传输（测试与嵌入场景）。
    #[must_use]
    pub fn start_with<T>(state: AppState, transport: T) -> PostUrlReporterHandle
    where
        T: HttpTransport + 'static,
    {
        let urls = targets(&state.config().snapshot()).0.len();
        if urls == 0 {
            tracing::info!("post_url 为空，事件上报空转（配置热重载后自动生效）");
        } else {
            tracing::info!(urls, "post_url 事件上报已启动");
        }

        let (stop, stop_rx) = watch::channel(false);
        let task = tokio::spawn(run(state, Arc::new(transport), stop_rx));
        PostUrlReporterHandle {
            stop: Some(stop),
            task: Some(task),
        }
    }
}

/// 上报任务：订阅总线，每条事件独立上报（Go `go PostMessageToUrls(message)`）。
async fn run<T>(state: AppState, transport: Arc<T>, mut stop: watch::Receiver<bool>)
where
    T: HttpTransport + 'static,
{
    let mut events = state.hub().subscribe();
    // 在途上报任务：每条事件一个，`join_next` 顺手回收已完成的。
    let mut in_flight: JoinSet<()> = JoinSet::new();

    loop {
        tokio::select! {
            biased;
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break;
                }
            }
            Some(_) = in_flight.join_next(), if !in_flight.is_empty() => {}
            event = events.recv() => match event {
                Ok(event) => {
                    // 与 Go 一致：每条事件现读配置，`post_url` 热重载即时生效。
                    let (urls, self_id) = targets(&state.config().snapshot());
                    if urls.is_empty() {
                        continue;
                    }
                    let transport = Arc::clone(&transport);
                    in_flight.spawn(async move {
                        post_event(transport.as_ref(), &event, &urls, &self_id).await;
                    });
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "事件总线积压，post_url 部分事件已丢弃");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
        }
    }

    // 有界停：在途上报不再等待，直接取消（Go 的 goroutine 随进程退出，没有等待语义）。
    in_flight.abort_all();
    while in_flight.join_next().await.is_some() {}
    tracing::debug!("post_url 上报任务退出");
}

/// 一条事件的上报目标：URL 列表（trim + 去空）+ `X-Self-ID`。
///
/// `X-Self-ID` 的口径与 Go 一致：`use_uin=true` 用 `uin`，否则用 `app_id`。
fn targets(settings: &Settings) -> (Vec<String>, String) {
    let urls: Vec<String> = settings
        .get_post_url()
        .iter()
        .map(|url| url.trim().to_owned())
        .filter(|url| !url.is_empty())
        .collect();
    let self_id = if settings.get_use_uin() {
        settings.get_uin().to_string()
    } else {
        settings.get_app_id().to_string()
    };
    (urls, self_id)
}

/// 把一条事件并发 POST 到全部 URL，等全部完成（Go `WaitGroup`）。
///
/// 单个 URL 失败只记日志：Go 的 `sendPostRequest` 同样不向调用方传播错误。
async fn post_event<T>(transport: &T, event: &Event, urls: &[String], self_id: &str)
where
    T: HttpTransport,
{
    let payload = match serde_json::to_string(event) {
        Ok(payload) => payload,
        Err(err) => {
            tracing::error!(error = %err, "事件序列化失败，post_url 上报跳过");
            return;
        }
    };

    let posts = urls
        .iter()
        .map(|url| post_one(transport, url, &payload, self_id));
    futures::future::join_all(posts).await;
}

/// 单个 URL 的 POST（Go `sendPostRequest`）。
async fn post_one<T>(transport: &T, url: &str, payload: &str, self_id: &str)
where
    T: HttpTransport,
{
    let request = HttpRequest::post(url)
        .with_header("Content-Type", CONTENT_TYPE)
        .with_header("X-Self-ID", self_id)
        .with_body(payload.as_bytes().to_vec());

    match transport.request(request).await {
        // Go 不看状态码：拿到响应就记成功。
        Ok(response) => {
            tracing::debug!(url, status = response.status, "post_url 上报完成");
        }
        Err(err) => tracing::warn!(url, error = %err, "post_url 上报失败"),
    }
}

/// 上报句柄：关闭入口 + 任务。
#[derive(Debug)]
pub struct PostUrlReporterHandle {
    stop: Option<watch::Sender<bool>>,
    task: Option<JoinHandle<()>>,
}

impl PostUrlReporterHandle {
    /// 未启动（传输构建失败）时的空句柄。
    fn idle() -> Self {
        Self {
            stop: None,
            task: None,
        }
    }

    /// 上报任务是否还在跑。
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.task
            .as_ref()
            .is_some_and(|task| !task.is_finished())
    }

    /// 立即中止上报任务（不等在途 POST）。
    pub fn abort(&self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }

    /// 优雅停止：先发关停信号，再等任务退出，超时则强制中止。
    pub async fn shutdown(mut self, limit: Duration) {
        self.signal_stop();

        let Some(task) = self.task.take() else {
            return;
        };
        let abort = task.abort_handle();
        match tokio::time::timeout(limit, task).await {
            Ok(Ok(())) => tracing::debug!("post_url 上报任务已退出"),
            Ok(Err(err)) => tracing::warn!(error = %err, "post_url 上报任务异常退出"),
            Err(_) => {
                abort.abort();
                tracing::warn!("post_url 上报优雅关闭超时，已强制中止");
            }
        }
    }

    fn signal_stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(true);
        }
    }
}

impl Drop for PostUrlReporterHandle {
    fn drop(&mut self) {
        self.signal_stop();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
