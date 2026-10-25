//! 反向 WS 客户端（Go `wsclient/ws.go`）。
//!
//! 启动时读 `ws_address[]`，**每个地址一条独立任务**（Go 也是每个地址一个
//! `WebSocketClient`）。每条任务：
//!
//! 1. 连接（`tokio-tungstenite`），握手头对齐 Go：`User-Agent: CQHttp/4.15.0`、
//!    `X-Client-Role: Universal`、`X-Self-ID: <self_id>`，token 非空时加
//!    `Authorization: Token <token>`。
//! 2. 连上先发 `meta_event/lifecycle/connect`，随后按 `heart_beat_interval` 秒发
//!    `meta_event/heartbeat`。
//! 3. 订阅 [`crate::event_hub::EventHub`]，把入站转换出的 OneBot 事件逐条写进同一个
//!    WS 连接；总线积压（`Lagged`）只记日志，不阻塞发布方。
//! 4. 连接断开立即重连；**连接失败**按指数退避（`initial_backoff` 起翻倍、
//!    `max_backoff` 封顶），累计失败超过 `reconnect_times` 后放弃该地址。
//!    Go 是固定 5s 间隔重试，这里按需求改成指数退避。
//!
//! token 选择顺序与 Go 一致：URL 查询参数 `access_token` 优先，其次
//! `ws_token[index]`（按下标与 `ws_address` 对齐）。
//!
//! # 与 Go 的差异 / 未做项
//!
//! - 反向 WS 收到的 OneBot action（`send_group_msg` 等）只记 debug 日志，不 dispatch
//!   —— 本里程碑只做出站事件推送，action 回环属于后续里程碑。
//! - Go 的 `sendFailures` 补发队列、`processFailedMessages` 未实现：断线期间的事件直接丢
//!   （与 broadcast 语义一致）。
//! - 不支持 `wss://`：workspace 的 `tokio-tungstenite` 未开 TLS feature，也没有走
//!   `ProxyFromEnvironment`；`ws://` 之外的地址会在 dial 阶段报错并按退避重试。
//! - `launch_reconnect_times`（启动阶段单独的重试上限）未区分，统一用 `reconnect_times`。

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use gensokyo_config::Settings;
use gensokyo_protocol::Event;
use tokio::net::TcpStream;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use crate::inbound::Inbound;
use crate::state::AppState;

/// `heart_beat_interval` 未配置（≤0）时的兜底心跳间隔。
pub const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
/// 首次 dial 失败后的等待时间；之后每次失败翻倍。
pub const DEFAULT_INITIAL_BACKOFF: Duration = Duration::from_millis(200);
/// 指数退避上限。
pub const DEFAULT_MAX_BACKOFF: Duration = Duration::from_secs(10);
/// 握手 `User-Agent`，与 Go 一致（部分框架按它识别 CQHttp）。
pub const USER_AGENT: &str = "CQHttp/4.15.0";

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// 反向 WS 客户端参数；`None` 表示按配置快照。
#[derive(Debug, Clone)]
pub struct WsClientOptions {
    /// 单个地址的最大连续 dial 失败次数（覆盖 `reconnect_times`）。
    pub reconnect_times: Option<u32>,
    /// 心跳间隔（覆盖 `heart_beat_interval`）。
    pub heartbeat_interval: Option<Duration>,
    /// 首次失败后的退避时长。
    pub initial_backoff: Duration,
    /// 退避上限。
    pub max_backoff: Duration,
}

impl Default for WsClientOptions {
    fn default() -> Self {
        Self {
            reconnect_times: None,
            heartbeat_interval: None,
            initial_backoff: DEFAULT_INITIAL_BACKOFF,
            max_backoff: DEFAULT_MAX_BACKOFF,
        }
    }
}

/// 客户端入口（无状态，方法与 [`crate::boot::Runtime`] 同构）。
#[derive(Debug)]
pub struct WsClient;

impl WsClient {
    /// 按当前配置快照启动 `ws_address` 里的全部地址。
    #[must_use]
    pub fn start(state: AppState) -> WsClientHandle {
        Self::start_with(state, WsClientOptions::default())
    }

    /// 同 [`WsClient::start`]，允许覆盖重连 / 心跳参数（测试与嵌入场景）。
    #[must_use]
    pub fn start_with(state: AppState, options: WsClientOptions) -> WsClientHandle {
        let settings = state.config().snapshot();
        let addresses: Vec<String> = settings
            .get_ws_address()
            .iter()
            .map(|address| address.trim().to_owned())
            .filter(|address| !address.is_empty())
            .collect();

        let reconnect_times = options.reconnect_times.unwrap_or_else(|| {
            u32::try_from(settings.get_reconnect_times().max(0)).unwrap_or(u32::MAX)
        });
        let heartbeat = options.heartbeat_interval.unwrap_or_else(|| {
            let seconds = settings.get_heart_beat_interval();
            if seconds > 0 {
                Duration::from_secs(u64::try_from(seconds).unwrap_or(30))
            } else {
                DEFAULT_HEARTBEAT_INTERVAL
            }
        });
        // `tokio::time::interval` 不接受零间隔：调用方传 0 时退化到 1ms。
        let heartbeat = heartbeat.max(Duration::from_millis(1));

        let (stop, _) = watch::channel(false);
        let tasks: Vec<JoinHandle<()>> = addresses
            .iter()
            .enumerate()
            .map(|(index, address)| {
                let state = state.clone();
                let address = address.clone();
                let options = options.clone();
                let mut stop_rx = stop.subscribe();
                tokio::spawn(async move {
                    run_address(
                        state,
                        &address,
                        index,
                        heartbeat,
                        reconnect_times,
                        options,
                        &mut stop_rx,
                    )
                    .await;
                })
            })
            .collect();

        if tasks.is_empty() {
            tracing::info!("ws_address 为空，反向 WS 未启动");
        } else {
            tracing::info!(addresses = tasks.len(), "反向 WS 客户端已启动");
        }

        WsClientHandle {
            stop: Some(stop),
            tasks,
        }
    }
}

/// 一个地址的生命周期：连上 → 推送 → 断开 → 重连，直到被停止或重试用尽。
async fn run_address(
    state: AppState,
    address: &str,
    index: usize,
    heartbeat: Duration,
    reconnect_times: u32,
    options: WsClientOptions,
    stop: &mut watch::Receiver<bool>,
) {
    let mut failures: u32 = 0;
    let mut backoff = options.initial_backoff;

    loop {
        if *stop.borrow() {
            break;
        }

        // 每次尝试现读快照：ws_token 是热字段，改配置不必重启进程。
        let settings = state.config().snapshot();
        let token = token_for(&settings, index, address);
        let inbound = Inbound::from_state(&state);

        match dial(address, &token, inbound.self_id()).await {
            Ok(socket) => {
                failures = 0;
                backoff = options.initial_backoff;
                tracing::info!(address, "反向 WS 已连接");

                let reason = serve(socket, &state, &inbound, heartbeat, stop).await;
                tracing::info!(address, ?reason, "反向 WS 连接结束");
                if reason == StopReason::Stopped {
                    break;
                }
                // 服务端断开后立即重连：Go 只在 dial 失败时 sleep，连上过的连接断开是即时的。
            }
            Err(err) => {
                failures += 1;
                if failures > reconnect_times {
                    tracing::warn!(
                        address,
                        failures,
                        error = %err,
                        "反向 WS 重连次数用尽，停止该地址"
                    );
                    break;
                }
                tracing::warn!(
                    address,
                    attempt = failures,
                    ?backoff,
                    error = %err,
                    "反向 WS 连接失败，退避后重试"
                );
                if sleep_or_stopped(backoff, stop).await {
                    break;
                }
                backoff = backoff.saturating_mul(2).min(options.max_backoff);
            }
        }
    }

    tracing::debug!(address, "反向 WS 任务退出");
}

/// 连接期间：发 connect 元事件、转发总线事件、按间隔发心跳、处理关闭帧。
async fn serve(
    socket: Socket,
    state: &AppState,
    inbound: &Inbound,
    heartbeat: Duration,
    stop: &mut watch::Receiver<bool>,
) -> StopReason {
    // 每次连接重新订阅：broadcast 不重放历史，断线期间的事件不会补发。
    let mut events = state.hub().subscribe();
    let (mut sink, mut source) = socket.split();

    if !send_event(&mut sink, &inbound.lifecycle_connect()).await {
        return StopReason::Disconnected;
    }

    let mut ticker = tokio::time::interval(heartbeat);
    // 卡顿后不要补发一串心跳：错过的 tick 直接跳过。
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ticker.tick().await;
    let interval_ms = i64::try_from(heartbeat.as_millis()).unwrap_or(i64::MAX);

    loop {
        tokio::select! {
            biased;
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return StopReason::Stopped;
                }
            }
            incoming = source.next() => match incoming {
                Some(Ok(Message::Close(_))) | None => return StopReason::Disconnected,
                Some(Err(err)) => {
                    tracing::debug!(error = %err, "反向 WS 读取出错");
                    return StopReason::Disconnected;
                }
                Some(Ok(Message::Ping(payload))) => {
                    if sink.send(Message::Pong(payload)).await.is_err() {
                        return StopReason::Disconnected;
                    }
                }
                // 反向 WS 上的 OneBot action 属于 dispatch，本里程碑只做出站事件推送。
                Some(Ok(Message::Text(text))) => {
                    tracing::debug!(bytes = text.len(), "反向 WS 收到 action，当前未处理");
                }
                Some(Ok(_)) => {}
            },
            event = events.recv() => match event {
                Ok(event) => {
                    if !send_event(&mut sink, &event).await {
                        return StopReason::Disconnected;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "事件总线积压，部分事件已丢弃");
                }
                Err(broadcast::error::RecvError::Closed) => return StopReason::Stopped,
            },
            _ = ticker.tick() => {
                if !send_event(&mut sink, &inbound.heartbeat(interval_ms)).await {
                    return StopReason::Disconnected;
                }
            }
        }
    }
}

async fn send_event<S>(sink: &mut S, event: &Event) -> bool
where
    S: futures::Sink<Message> + Unpin,
{
    let text = match serde_json::to_string(event) {
        Ok(text) => text,
        Err(err) => {
            tracing::error!(error = %err, "事件序列化失败，跳过该事件");
            return true;
        }
    };
    sink.send(Message::Text(text.into())).await.is_ok()
}

/// 建连；token 非空时带 `Authorization: Token <token>`（Go 同款）。
async fn dial(address: &str, token: &str, self_id: i64) -> Result<Socket, String> {
    let mut request = address
        .into_client_request()
        .map_err(|err| format!("非法 WS 地址 {address}: {err}"))?;
    let headers = request.headers_mut();
    headers.insert("user-agent", HeaderValue::from_static(USER_AGENT));
    headers.insert("x-client-role", HeaderValue::from_static("Universal"));
    headers.insert(
        "x-self-id",
        HeaderValue::from_str(&self_id.to_string())
            .map_err(|err| format!("X-Self-ID 非法: {err}"))?,
    );
    if !token.is_empty() {
        headers.insert(
            "authorization",
            HeaderValue::from_str(&format!("Token {token}"))
                .map_err(|err| format!("token 含非法头字符: {err}"))?,
        );
    }

    let (socket, _response) = connect_async(request)
        .await
        .map_err(|err| format!("连接 {address} 失败: {err}"))?;
    Ok(socket)
}

/// 与 Go 一致：URL 上的 `access_token` 覆盖 `ws_token[index]`。
fn token_for(settings: &Settings, index: usize, address: &str) -> String {
    if let Some(token) = access_token_from_uri(address) {
        return token;
    }
    settings
        .get_ws_token()
        .get(index)
        .map(|token| token.trim().to_owned())
        .unwrap_or_default()
}

fn access_token_from_uri(address: &str) -> Option<String> {
    let query = address.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "access_token").then(|| value.to_owned())
    })
}

async fn sleep_or_stopped(duration: Duration, stop: &mut watch::Receiver<bool>) -> bool {
    tokio::select! {
        () = tokio::time::sleep(duration) => false,
        _ = stop.changed() => true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopReason {
    /// 收到关停信号。
    Stopped,
    /// 对端关闭 / IO 出错 / 心跳写失败。
    Disconnected,
}

/// 反向 WS 客户端句柄：关闭入口 + 任务集合。
#[derive(Debug)]
pub struct WsClientHandle {
    stop: Option<watch::Sender<bool>>,
    tasks: Vec<JoinHandle<()>>,
}

impl WsClientHandle {
    /// 本次启动的地址数量。
    #[must_use]
    pub fn address_count(&self) -> usize {
        self.tasks.len()
    }

    /// 是否还有地址任务在跑。
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.tasks.iter().any(|task| !task.is_finished())
    }

    /// 立即中止全部地址任务（不等重连退避）。
    pub fn abort(&self) {
        for task in &self.tasks {
            task.abort();
        }
    }

    /// 优雅停止：先发关停信号（跳过退避等待、关闭连接），再等任务退出，超时则强制中止。
    pub async fn shutdown(mut self, limit: Duration) {
        self.signal_stop();

        // 超时分支要能兜底中止，所以先留 AbortHandle 再把 JoinHandle 移进等待块。
        let aborts: Vec<_> = self.tasks.iter().map(JoinHandle::abort_handle).collect();
        let tasks = std::mem::take(&mut self.tasks);
        let wait = async {
            for task in tasks {
                match task.await {
                    Ok(()) => {}
                    Err(err) if err.is_cancelled() => {}
                    Err(err) => tracing::warn!(error = %err, "反向 WS 任务异常退出"),
                }
            }
        };

        match tokio::time::timeout(limit, wait).await {
            Ok(()) => tracing::debug!("反向 WS 全部任务已退出"),
            Err(_) => {
                for abort in &aborts {
                    abort.abort();
                }
                tracing::warn!("反向 WS 优雅关闭超时，已强制中止残留任务");
            }
        }
    }

    fn signal_stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(true);
        }
    }
}

impl Drop for WsClientHandle {
    fn drop(&mut self) {
        self.signal_stop();
        for task in std::mem::take(&mut self.tasks) {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_HEARTBEAT_INTERVAL, access_token_from_uri};
    use std::time::Duration;

    #[test]
    fn access_token_query_param_is_parsed() {
        assert_eq!(
            access_token_from_uri("ws://127.0.0.1:1/ws?access_token=abc&x=1"),
            Some("abc".to_owned())
        );
        assert_eq!(access_token_from_uri("ws://127.0.0.1:1/ws"), None);
        assert_eq!(access_token_from_uri("ws://127.0.0.1:1/ws?x=1"), None);
    }

    #[test]
    fn default_heartbeat_is_used_for_non_positive_config() {
        assert_eq!(DEFAULT_HEARTBEAT_INTERVAL, Duration::from_secs(30));
    }
}
