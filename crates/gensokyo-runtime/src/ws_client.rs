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
//! 4. 收到的 OneBot action 交给 [`crate::dispatcher`] 分发，回执写回同一连接（`echo` 原样
//!    带回，JSON 非法回 `1400`）；dispatch 在独立任务里跑，慢 action 不拖住心跳与事件推送。
//! 5. 连接断开立即重连；**连接失败**按指数退避（`initial_backoff` 起翻倍、
//!    `max_backoff` 封顶），累计失败超过 `reconnect_times` 后放弃该地址。
//!    Go 是固定 5s 间隔重试，这里按需求改成指数退避。
//!
//! token 选择顺序与 Go 一致：URL 查询参数 `access_token` 优先，其次
//! `ws_token[index]`（按下标与 `ws_address` 对齐）。
//!
//! # 与 Go 的差异 / 未做项
//!
//! - 反向 WS 收到的 OneBot action（`send_group_msg` 等）经 [`dispatch_json`] 分发，回执 JSON
//!   写回**同一连接**（`echo` 原样带回）；解析失败回 `retcode=1400` 形状的失败响应。
//!   与 Go 的 `go callapi.CallAPIFromDict(...)` 一致，每个 action 在独立任务里跑，
//!   慢 action 不阻塞事件转发与心跳。
//! - Go 的 `sendFailures` / `processFailedMessages` 已对齐：写失败且 `disable_error_chan=false`
//!   时把原事件 JSON 入队，下一个心跳 tick 重发后**无条件清空**（重发失败的也不再重试）；
//!   `disable_error_chan=true` 时只记日志丢弃。缓冲按地址持有、跨重连保留（同 Go 把
//!   `sendFailures` 挂在 `WebSocketClient` 上）；与 Go 一样**没有容量上限**，长时间断线
//!   会持续占用内存。
//! - 代理：对齐 Go `websocket.Dialer{Proxy: http.ProxyFromEnvironment}`——`ws://` 读
//!   `HTTP_PROXY`、`wss://` 读 `HTTPS_PROXY`（大写优先），`localhost` / 回环 IP 永远直连
//!   （Go `httpproxy` 硬规则），`NO_PROXY` 命中时直连。代理一律走
//!   HTTP `CONNECT` 隧道（`tokio-tungstenite` 0.30 无内置 proxy API），`wss://` 的 TLS 在隧道
//!   上完成。未实现：SOCKS5 代理、`NO_PROXY` 的 CIDR 写法、userinfo 的 percent-decode。
//! - `launch_reconnect_times`（启动阶段单独的重试上限）未区分，统一用 `reconnect_times`。

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures::{SinkExt, StreamExt};
use gensokyo_config::Settings;
use gensokyo_protocol::Event;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, Request, Uri};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, client_async_tls_with_config, connect_async,
};

use crate::dispatcher::dispatch_json;
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
/// action 回执写回通道的容量（条）；分发任务在通道满时背压等待。
pub(crate) const RECEIPT_CAPACITY: usize = 64;
/// 代理 `CONNECT` 握手超时，对齐 Go `websocket.Dialer.HandshakeTimeout`。
const PROXY_CONNECT_TIMEOUT: Duration = Duration::from_secs(45);
/// `CONNECT` 应答头的字节上限：对端不回 CRLFCRLF 时不能无限读。
const MAX_CONNECT_RESPONSE: usize = 8 * 1024;

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
        let heartbeat = options
            .heartbeat_interval
            .unwrap_or_else(|| heartbeat_interval(&settings))
            // `tokio::time::interval` 不接受零间隔：调用方传 0 时退化到 1ms。
            .max(Duration::from_millis(1));

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
    // Go 的 `sendFailures` 挂在 `WebSocketClient` 上：缓冲按地址持有，重连不清空。
    let mut failed = FailedMessages::default();

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

                let reason =
                    serve(socket, &state, &inbound, heartbeat, stop, &mut failed).await;
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
    failed: &mut FailedMessages,
) -> StopReason {
    // 每次连接重新订阅：broadcast 不重放历史，写失败的事件由 `failed` 缓冲补发。
    let mut events = state.hub().subscribe();
    let dispatcher = state.dispatcher();
    // action 回执走独立通道：分发在 spawn 的任务里跑，慢 action 不挡住事件与心跳。
    let (outbox, mut receipts) = mpsc::channel::<String>(RECEIPT_CAPACITY);
    let (mut sink, mut source) = socket.split();

    if send_event(
        &mut sink,
        &inbound.lifecycle_connect(),
        failed,
        disable_error_chan(state),
    )
    .await
    {
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
                // 反向 WS 上的 OneBot action：分发结果（含 echo）写回同一连接。
                Some(Ok(Message::Text(text))) => {
                    let dispatcher = dispatcher.clone();
                    let outbox = outbox.clone();
                    tokio::spawn(async move {
                        let response =
                            dispatch_json(dispatcher, text.as_str().as_bytes().to_vec()).await;
                        // 连接已断时接收端已释放：回执丢弃即可（回执没有重发语义）。
                        let _ = outbox.send(response).await;
                    });
                }
                Some(Ok(_)) => {}
            },
            // action 回执：与事件共用同一个 sink、同一个任务串行写，帧不会交错。
            // `outbox` 由本函数持有，通道不会关闭，所以这里只可能等到 `Some`。
            Some(payload) = receipts.recv() => {
                if !write_payload(&mut sink, &payload).await {
                    return StopReason::Disconnected;
                }
            },
            event = events.recv() => match event {
                Ok(event) => {
                    // 事件 JSON 由发布方序列化一次，这里直接复用同一份字节。
                    if send_payload(&mut sink, event.json(), failed, disable_error_chan(state)).await {
                        return StopReason::Disconnected;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "事件总线积压，部分事件已丢弃");
                }
                Err(broadcast::error::RecvError::Closed) => return StopReason::Stopped,
            },
            _ = ticker.tick() => {
                let heartbeat_event = inbound.heartbeat(interval_ms);
                if send_event(&mut sink, &heartbeat_event, failed, disable_error_chan(state)).await {
                    return StopReason::Disconnected;
                }
                // 与 Go 一致：心跳写完补发缓冲，补发后无条件清空。
                failed.flush(&mut sink).await;
            }
        }
    }
}

/// 写一条**已序列化**的事件 JSON。
///
/// 写失败时（[`write_payload`] 报错且 `disable_error_chan=false`）把该 JSON 压入
/// [`FailedMessages`]，返回 `true` 表示连接已不可写、调用方应断开重连。
async fn send_payload<S>(
    sink: &mut S,
    payload: &str,
    failed: &mut FailedMessages,
    disable_error_chan: bool,
) -> bool
where
    S: futures::Sink<Message> + Unpin,
{
    if write_payload(sink, payload).await {
        return false;
    }
    failed.record(payload.to_owned(), disable_error_chan);
    true
}

/// 写一条**本地构造**的事件（connect / 心跳）：序列化后交给 [`send_payload`]。
///
/// 序列化失败只记 error：没有可补发的 payload，也不断开。
async fn send_event<S>(
    sink: &mut S,
    event: &Event,
    failed: &mut FailedMessages,
    disable_error_chan: bool,
) -> bool
where
    S: futures::Sink<Message> + Unpin,
{
    match serde_json::to_string(event) {
        Ok(text) => send_payload(sink, &text, failed, disable_error_chan).await,
        Err(err) => {
            tracing::error!(error = %err, "事件序列化失败，跳过该事件");
            false
        }
    }
}

/// 写一帧文本；`false` 表示底层写失败。
async fn write_payload<S>(sink: &mut S, payload: &str) -> bool
where
    S: futures::Sink<Message> + Unpin,
{
    sink.send(Message::Text(payload.to_owned().into()))
        .await
        .is_ok()
}

/// Go `config.GetDisableErrorChan()`：每次写失败现读快照，热重载即时生效。
fn disable_error_chan(state: &AppState) -> bool {
    state.config().snapshot().get_disable_error_chan()
}

/// `heart_beat_interval` 秒 → 心跳间隔；非正数（未配置）用 [`DEFAULT_HEARTBEAT_INTERVAL`]。
///
/// 反向 WS 与正向 WS 共用这一份配置口径。
pub(crate) fn heartbeat_interval(settings: &Settings) -> Duration {
    let seconds = settings.get_heart_beat_interval();
    if seconds > 0 {
        Duration::from_secs(u64::try_from(seconds).unwrap_or(30))
    } else {
        DEFAULT_HEARTBEAT_INTERVAL
    }
}

/// 写失败缓冲（Go `WebSocketClient.sendFailures` + `processFailedMessages`）。
///
/// Go 侧没有 key、TTL、容量上限，这里照抄：长时间断线会持续占用内存。
#[derive(Debug, Default)]
struct FailedMessages {
    entries: Vec<String>,
}

impl FailedMessages {
    /// 按写失败入队；`disable_error_chan=true` 时只记日志丢弃（Go 同款开关）。
    fn record(&mut self, payload: String, disable_error_chan: bool) {
        if disable_error_chan {
            tracing::warn!(
                bytes = payload.len(),
                "写失败且 disable_error_chan 已开启，丢弃该事件"
            );
            return;
        }
        self.entries.push(payload);
        tracing::warn!(pending = self.entries.len(), "写失败，事件进入补发缓冲");
    }

    /// 重发缓冲并**无条件清空**（对齐 Go `processFailedMessages`）：
    /// 重发失败的也不再重试，避免无限堆积。
    async fn flush<S>(&mut self, sink: &mut S)
    where
        S: futures::Sink<Message> + Unpin,
    {
        if self.entries.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.entries);
        let total = pending.len();
        let mut failed = 0usize;
        for payload in pending {
            if !write_payload(sink, &payload).await {
                failed += 1;
            }
        }
        if failed == 0 {
            tracing::info!(count = total, "补发缓冲事件完成");
        } else {
            tracing::warn!(count = total, failed, "补发缓冲事件部分失败，已清空不再重试");
        }
    }
}

/// 建连；token 非空时带 `Authorization: Token <token>`（Go 同款）。
///
/// 代理按环境变量选（Go `websocket.Dialer{Proxy: http.ProxyFromEnvironment}`）：命中时先向
/// 代理发 HTTP `CONNECT` 建隧道，WS 握手（`wss://` 的 TLS）在隧道里完成。
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

    let proxy = proxy_for(&ProxyEnv::from_env(), request.uri())
        .map_err(|err| format!("代理配置无效（{address}）: {err}"))?;
    match proxy {
        Some(proxy) => connect_tunneled(request, &proxy)
            .await
            .map_err(|err| format!("经代理 {} 连接 {address} 失败: {err}", proxy.address)),
        None => {
            let (socket, _response) = connect_async(request)
                .await
                .map_err(|err| format!("连接 {address} 失败: {err}"))?;
            Ok(socket)
        }
    }
}

/// 代理环境变量（Go `http.ProxyFromEnvironment` 的输入；只认这三个，无 `ALL_PROXY`）。
#[derive(Debug, Default, Clone)]
struct ProxyEnv {
    http: Option<String>,
    https: Option<String>,
    no_proxy: Option<String>,
}

impl ProxyEnv {
    /// 现读环境变量：大写优先（Go `getEnvAny` 同序），空值视为未设置。
    fn from_env() -> Self {
        Self {
            http: env_var("HTTP_PROXY", "http_proxy"),
            https: env_var("HTTPS_PROXY", "https_proxy"),
            no_proxy: env_var("NO_PROXY", "no_proxy"),
        }
    }
}

fn env_var(upper: &str, lower: &str) -> Option<String> {
    [upper, lower]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|v| !v.trim().is_empty()))
}

/// 代理终点：`address` 直接交给 `TcpStream::connect`（IPv6 保留方括号）。
#[derive(Debug, Clone)]
struct ProxyEndpoint {
    address: String,
    /// userinfo 的 Base64（`Proxy-Authorization: Basic ...`）；无 userinfo 为 `None`。
    authorization: Option<String>,
}

impl ProxyEndpoint {
    /// 解析 `http://[user:pass@]host[:port]`；缺省 scheme 为 `http`、缺省端口为 8080。
    ///
    /// 与 Go 一致：非 `http` 协议（SOCKS5）直接报错，不静默降级成直连。
    fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        let (scheme, rest) = raw.split_once("://").unwrap_or(("http", raw));
        if !scheme.eq_ignore_ascii_case("http") {
            return Err(format!(
                "不支持的代理协议 {scheme}（仅支持 http:// CONNECT 代理）"
            ));
        }
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        if !path.is_empty() {
            return Err(format!("代理 URL 不应带路径: {raw}"));
        }
        let (userinfo, authority) = authority.rsplit_once('@').unwrap_or(("", authority));
        let (host, port) = split_host_port(authority)?;
        // Go 的 Basic 凭据是 `user:password`；只给用户名时补上冒号。
        let authorization = (!userinfo.is_empty()).then(|| {
            let credentials = if userinfo.contains(':') {
                userinfo.to_owned()
            } else {
                format!("{userinfo}:")
            };
            BASE64.encode(credentials)
        });
        Ok(Self {
            address: format!("{host}:{port}"),
            authorization,
        })
    }
}

/// 拆 `host[:port]`（IPv6 用 `[...]`），缺省端口 8080（Go `httpproxy` 同款）。
fn split_host_port(authority: &str) -> Result<(String, u16), String> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, tail) = rest
            .split_once(']')
            .ok_or_else(|| format!("代理 URL 的 IPv6 地址非法: {authority}"))?;
        let port = match tail.strip_prefix(':') {
            Some(port) => port
                .parse()
                .map_err(|_| format!("代理端口非法: {port}"))?,
            None => 8080,
        };
        return Ok((format!("[{host}]"), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => Ok((
            host.to_owned(),
            port.parse().map_err(|_| format!("代理端口非法: {port}"))?,
        )),
        None => Ok((authority.to_owned(), 8080)),
    }
}

/// 按 URL scheme 选代理：`ws` → `HTTP_PROXY`、`wss` → `HTTPS_PROXY`（Go `httpproxy` 把
/// `ws`/`wss` 归一成 `http`/`https` 后再查表），`NO_PROXY` 命中时直连。
fn proxy_for(env: &ProxyEnv, uri: &Uri) -> Result<Option<ProxyEndpoint>, String> {
    let (raw, default_port) = match uri.scheme_str() {
        Some("ws") => (env.http.as_deref(), 80),
        Some("wss") => (env.https.as_deref(), 443),
        // 其他 scheme 交给 `connect_async` 报错，不去猜代理。
        _ => (None, 0),
    };
    let Some(raw) = raw else { return Ok(None) };
    let host = uri
        .host()
        .ok_or_else(|| format!("WS 地址缺少主机名: {uri}"))?;
    // Go `httpproxy` 的硬规则：localhost / 回环 IP 永远直连，与 `NO_PROXY` 无关。
    if is_loopback(host) {
        tracing::debug!(host, "目标为回环地址，反向 WS 直连");
        return Ok(None);
    }
    if bypass_no_proxy(
        env.no_proxy.as_deref().unwrap_or_default(),
        host,
        uri.port_u16().unwrap_or(default_port),
    ) {
        tracing::debug!(host, "NO_PROXY 命中，反向 WS 直连");
        return Ok(None);
    }
    ProxyEndpoint::parse(raw).map(Some)
}

/// Go `httpproxy` 的本机绕行判定：`localhost`（不分大小写）或回环 IP。
fn is_loopback(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']);
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// `NO_PROXY` 匹配（Go `httpproxy` 的子集）：`*` 全绕行；条目支持 `host`、`.suffix`、
/// `host:port`。未实现 CIDR / IP 段写法。
fn bypass_no_proxy(no_proxy: &str, host: &str, port: u16) -> bool {
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    no_proxy
        .split([',', ' '])
        .filter(|entry| !entry.is_empty())
        .any(|entry| {
            if entry == "*" {
                return true;
            }
            // 末段是纯数字才算端口，避免把裸 IPv6 的 `::1` 当端口拆开。
            let (entry_host, entry_port) = match entry.rsplit_once(':') {
                Some((entry_host, entry_port))
                    if !entry_port.is_empty()
                        && entry_port.bytes().all(|byte| byte.is_ascii_digit()) =>
                {
                    (entry_host, entry_port.parse::<u16>().ok())
                }
                _ => (entry, None),
            };
            if entry_port.is_some_and(|entry_port| entry_port != port) {
                return false;
            }
            let entry_host = entry_host
                .trim_start_matches('.')
                .trim_matches(['[', ']'])
                .to_ascii_lowercase();
            !entry_host.is_empty()
                && (host == entry_host || host.ends_with(&format!(".{entry_host}")))
        })
}

/// `CONNECT` 的目标（`host:port`，端口沿用 tungstenite 的 ws=80 / wss=443 约定）。
fn target_authority(uri: &Uri) -> Result<String, String> {
    let authority = uri
        .authority()
        .ok_or_else(|| format!("WS 地址缺少主机: {uri}"))?;
    if authority.port_u16().is_some() {
        return Ok(authority.as_str().to_owned());
    }
    let default = match uri.scheme_str() {
        Some("ws") => 80,
        Some("wss") => 443,
        other => return Err(format!("不支持的 WS scheme: {other:?}")),
    };
    Ok(format!("{authority}:{default}"))
}

/// 经 HTTP 代理建 WS：`CONNECT` 建隧道后把裸 TCP 交给 tungstenite 做 WS 握手；
/// `wss://` 由 `client_async_tls_with_config` 的缺省 connector（webpki 根）在隧道上叠 TLS。
async fn connect_tunneled(request: Request<()>, proxy: &ProxyEndpoint) -> Result<Socket, String> {
    let target = target_authority(request.uri())?;
    let mut stream = tokio::time::timeout(PROXY_CONNECT_TIMEOUT, TcpStream::connect(&proxy.address))
        .await
        .map_err(|_| format!("连接代理 {} 超时", proxy.address))?
        .map_err(|err| format!("连接代理 {} 失败: {err}", proxy.address))?;

    let mut head = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    if let Some(authorization) = &proxy.authorization {
        head.push_str(&format!("Proxy-Authorization: Basic {authorization}\r\n"));
    }
    head.push_str("\r\n");
    tokio::time::timeout(PROXY_CONNECT_TIMEOUT, stream.write_all(head.as_bytes()))
        .await
        .map_err(|_| "写 CONNECT 请求超时".to_owned())?
        .map_err(|err| format!("写 CONNECT 请求失败: {err}"))?;

    let status = tokio::time::timeout(PROXY_CONNECT_TIMEOUT, read_connect_status(&mut stream))
        .await
        .map_err(|_| "读 CONNECT 应答超时".to_owned())??;
    if status != 200 {
        return Err(format!("代理对 CONNECT {target} 返回 {status}"));
    }

    let (socket, _response) = client_async_tls_with_config(request, stream, None, None)
        .await
        .map_err(|err| format!("代理隧道上 WS 握手失败: {err}"))?;
    Ok(socket)
}

/// 读 `CONNECT` 应答头，返回状态码。逐字节读到 CRLFCRLF：代理不会在 WS 请求前主动发数据，
/// 这样不会把隧道里的后续字节读进本函数的缓冲。
async fn read_connect_status(stream: &mut TcpStream) -> Result<u16, String> {
    let mut head = Vec::with_capacity(128);
    let mut byte = [0u8; 1];
    loop {
        let read = stream
            .read(&mut byte)
            .await
            .map_err(|err| format!("读 CONNECT 应答失败: {err}"))?;
        if read == 0 {
            return Err("代理在 CONNECT 应答前关闭连接".to_owned());
        }
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
        if head.len() > MAX_CONNECT_RESPONSE {
            return Err(format!("CONNECT 应答头超过 {MAX_CONNECT_RESPONSE} 字节"));
        }
    }
    let text = std::str::from_utf8(&head).map_err(|_| "CONNECT 应答不是合法 UTF-8".to_owned())?;
    text.split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| {
            format!(
                "CONNECT 应答缺少状态码: {}",
                text.lines().next().unwrap_or_default()
            )
        })
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
    use super::{
        DEFAULT_HEARTBEAT_INTERVAL, FailedMessages, ProxyEnv, ProxyEndpoint, access_token_from_uri,
        connect_tunneled, proxy_for, send_event,
    };
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::Duration;

    use futures::Sink;
    use gensokyo_core::Ob11Id;
    use gensokyo_protocol::Event;
    use gensokyo_protocol::event::MetaEvent;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

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

    /// 可注入写失败的假连接：`fail=true` 时每次写都返回错误，否则记录写入的文本帧。
    #[derive(Debug, Default)]
    struct FakeSink {
        fail: bool,
        sent: Vec<String>,
    }

    impl Sink<Message> for FakeSink {
        type Error = std::io::Error;

        fn poll_ready(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn start_send(mut self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
            if self.fail {
                return Err(std::io::Error::other("注入的写失败"));
            }
            let Message::Text(text) = item else {
                panic!("只应写入文本帧");
            };
            self.sent.push(text.to_string());
            Ok(())
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    fn heartbeat_event() -> Event {
        Event::MetaEvent(MetaEvent {
            time: 1_700_000_000,
            self_id: Ob11Id::num(12345),
            meta_event_type: "heartbeat".to_owned(),
            sub_type: None,
            interval: Some(30_000),
            status: None,
            extra: Default::default(),
        })
    }

    #[tokio::test]
    async fn write_failure_is_buffered_and_resent_after_heartbeat() {
        let event = heartbeat_event();
        let expected = serde_json::to_string(&event).expect("序列化失败");
        let mut sink = FakeSink {
            fail: true,
            ..FakeSink::default()
        };
        let mut failed = FailedMessages::default();

        assert!(send_event(&mut sink, &event, &mut failed, false).await);
        assert_eq!(failed.entries, vec![expected.clone()], "写失败应压入缓冲");

        // 下一个心跳 tick：连接恢复后补发并清空。
        sink.fail = false;
        failed.flush(&mut sink).await;
        assert_eq!(sink.sent, vec![expected], "心跳 tick 应补发缓冲事件");
        assert!(failed.entries.is_empty(), "补发后应清空缓冲");
    }

    #[tokio::test]
    async fn write_failure_is_dropped_when_disable_error_chan() {
        let mut sink = FakeSink {
            fail: true,
            ..FakeSink::default()
        };
        let mut failed = FailedMessages::default();

        assert!(send_event(&mut sink, &heartbeat_event(), &mut failed, true).await);
        assert!(failed.entries.is_empty(), "disable_error_chan 时不进缓冲");

        failed.flush(&mut sink).await;
        assert!(sink.sent.is_empty(), "没有缓冲就没有补发");
    }

    #[tokio::test]
    async fn flush_clears_buffer_even_when_resend_fails() {
        let mut failed = FailedMessages::default();
        failed.record("first".to_owned(), false);
        failed.record("second".to_owned(), false);
        let mut sink = FakeSink {
            fail: true,
            ..FakeSink::default()
        };

        failed.flush(&mut sink).await;
        assert!(failed.entries.is_empty(), "重发失败也要无条件清空（对齐 Go）");
    }

    #[tokio::test]
    async fn successful_write_does_not_buffer() {
        let mut sink = FakeSink::default();
        let mut failed = FailedMessages::default();

        assert!(!send_event(&mut sink, &heartbeat_event(), &mut failed, false).await);
        assert!(failed.entries.is_empty(), "成功路径不应缓冲");
        assert_eq!(sink.sent.len(), 1);
    }

    #[test]
    fn proxy_is_selected_by_scheme_and_no_proxy() {
        let env = ProxyEnv {
            http: Some("http://127.0.0.1:7890".to_owned()),
            https: Some("http://user:pw@127.0.0.1:7891".to_owned()),
            no_proxy: Some("localhost, .example.com, 10.0.0.1:9000".to_owned()),
        };
        let endpoint = |url: &str| {
            proxy_for(&env, &url.parse().expect("URI 解析失败"))
                .expect("代理配置非法")
                .map(|endpoint| endpoint.address)
        };

        assert_eq!(
            endpoint("ws://bot.example.org/ws").as_deref(),
            Some("127.0.0.1:7890")
        );
        assert_eq!(
            endpoint("wss://bot.example.org/ws").as_deref(),
            Some("127.0.0.1:7891")
        );
        // NO_PROXY：精确主机 / 后缀 / host:port
        assert_eq!(endpoint("ws://localhost:9000/ws"), None);
        assert_eq!(endpoint("wss://a.example.com/ws"), None);
        assert_eq!(endpoint("ws://10.0.0.1:9000/ws"), None);
        // 端口不匹配时不绕行
        assert!(endpoint("ws://10.0.0.1:9001/ws").is_some());

        // 未配置代理 → 直连
        assert!(
            proxy_for(&ProxyEnv::default(), &"wss://a.b/ws".parse().expect("URI"))
                .expect("未配置代理不应报错")
                .is_none()
        );
        // 非 http 代理（SOCKS5）→ 报错，不静默直连
        let socks = ProxyEnv {
            https: Some("socks5://127.0.0.1:1080".to_owned()),
            ..ProxyEnv::default()
        };
        assert!(proxy_for(&socks, &"wss://a.b/ws".parse().expect("URI")).is_err());
    }

    /// Go `http.ProxyFromEnvironment` 对 loopback 永远直连：即使配了代理且 `NO_PROXY` 未覆盖。
    #[test]
    fn loopback_targets_bypass_proxy_without_no_proxy() {
        let env = ProxyEnv {
            http: Some("http://127.0.0.1:7890".to_owned()),
            https: Some("http://127.0.0.1:7890".to_owned()),
            no_proxy: None,
        };
        let endpoint =
            |url: &str| proxy_for(&env, &url.parse().expect("URI 解析失败")).expect("代理配置非法");

        assert!(endpoint("ws://localhost:8081/ws").is_none());
        assert!(endpoint("wss://LOCALHOST/ws").is_none());
        assert!(endpoint("ws://127.0.0.1:8081/ws").is_none());
        assert!(endpoint("ws://[::1]:8081/ws").is_none());
        assert!(endpoint("wss://bot.example.org/ws").is_some());
    }

    /// 本地 mock 代理：断言 CONNECT 请求行、回 200，再在同一连接上完成 WS 握手，
    /// 证明隧道与后续握手确实共用同一条流。
    #[tokio::test]
    async fn connect_tunneled_sends_connect_and_handshakes_over_the_tunnel() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定 mock 代理失败");
        let proxy_address = listener.local_addr().expect("mock 代理地址");
        let proxy = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                stream.read_exact(&mut byte).await.expect("读 CONNECT");
                head.push(byte[0]);
                if head.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let head = String::from_utf8(head).expect("CONNECT 头不是 UTF-8");
            assert!(
                head.starts_with("CONNECT 127.0.0.1:9 HTTP/1.1\r\n"),
                "CONNECT 请求行: {head}"
            );
            stream
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .await
                .expect("写 200");
            tokio_tungstenite::accept_async(stream)
                .await
                .expect("隧道上的 WS 握手失败");
        });

        let request = "ws://127.0.0.1:9/ws"
            .into_client_request()
            .expect("构造请求");
        let endpoint =
            ProxyEndpoint::parse(&format!("http://{proxy_address}")).expect("解析代理 URL");
        let socket = connect_tunneled(request, &endpoint)
            .await
            .expect("隧道建连失败");
        drop(socket);
        proxy.await.expect("mock 代理任务 panic");
    }
}
