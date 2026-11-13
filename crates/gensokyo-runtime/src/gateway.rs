//! QQ 上游事件 WebSocket 网关客户端（Go `botgo/websocket/client` + `botgo/sessions/local`
//! + `botgo/sessions/multi`）。
//!
//! 入站链的源头：主动连 QQ 网关、按 `text_intent` 订阅 intent 位，把 dispatch 事件交给
//! [`crate::inbound::Inbound`] 转成 OneBot 事件后发布到 [`crate::event_hub::EventHub`]。
//!
//! # 协议流程（逐条对齐 botgo）
//!
//! 1. `GET {api_base}/gateway/bot`（`Authorization: QQBot <access_token>`）→
//!    `{url, shards, session_start_limit}`（`botgo/openapi/v2/ws.go`）；HTTP 401 /
//!    `code=40001` 时强制刷新 token 后再试一次。
//! 2. 连 `url`，等 Hello（op 10）取 `heartbeat_interval`（`client.go:318-325`）。
//! 3. 发 Identify（op 2，`d.token = "QQBot <access_token>"`、`intents`、`shard`、
//!    `properties`）或 Resume（op 6，带 `session_id` + `seq`）（`client.go:155-185`）。
//! 4. 按 Hello 的间隔发心跳（op 1，`d` 为最近 seq 或 `null`），op 11 ACK 忽略
//!    （`client.go:127-137`）；首次 identify 之前不发心跳。
//! 5. dispatch（op 0，`s` = seq、`t` = 事件名、`d` = 负载）→ Inbound → EventHub。
//!    首个 `t=READY` 记下 `session_id`（`client.go:328-349`）。
//! 6. op 7 立即重连、op 9 `d=false` 重新 identify / `d=true` resume、4009 可 resume，
//!    其余 close 码按 botgo `IsUnexpectedCloseError` 语义清空会话重新 identify
//!    （`client.go:100-126`、`sessions/local/local.go:125-152`）；连接失败按 1s→60s 指数退避。
//!
//! # 与 Go 的取舍
//!
//! - **API 域**：`botgo/openapi/v2/resource.go:7-8` 已把正式与沙箱统一为 `api.bot.qq.com`，
//!   因此 `sandbox_mode` 不再切换域名（与 botgo 一致）；[`GatewayOptions::api_base`] 只给
//!   测试与自定义网关用。
//! - **分片**：照搬 Go `main.go:375-399` 的两条路。`shard_count == 1` 时把 `shard_num`
//!   当本进程要跑的分片数：`> 1` 就起 N 个连接、每个 Identify `[i, N]`（Go
//!   `multi.NewShardManager`），`== 1` 是 `[0, 1]` 单连接收全量事件；`shard_count > 1`
//!   时只跑配置指定的 `[shard_id, shard_count]`（Go `StartSingle`），其余分片要另开进程。
//!   网关建议的 `shards` 只打日志，不覆盖配置。
//! - **启动节流**：多分片按 `session_start_limit.max_concurrency` 算启动间隔
//!   （Go `manager.CalcInterval`：2s 窗口四舍五入，最少 1s，`0` 当 1），第 i 个分片延迟
//!   `i × 间隔` 再连；单分片不延迟——Go 的 local 管理器也会 sleep 一个间隔，这里保留
//!   #70 的「即连」语义，避免单连接启动凭空多等。
//! - **鉴权失败**：连续 3 次鉴权失败（4004 / op 9 `d=false`）后停止重连并报错，对齐
//!   botgo `maxAuthFailCount`（`client.go:29`）；Go fork 的「整轮重试」未搬。
//! - 不处理 `t=RESUMED`（Inbound 视作未知事件忽略），与 botgo 只特判 READY 一致。

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use gensokyo_config::{ConfigManager, Settings};
use gensokyo_protocol::{Event, Extra, parse_id};
use gensokyo_qq::{HttpRequest, HttpTransport, TokenManager};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use crate::downtime::Downtime;
use crate::inbound::Inbound;
use crate::intent::IntentPlan;
use crate::state::AppState;

/// QQ 开放平台 API 域（Go `botgo/openapi/v2/resource.go:7`；正式与沙箱已统一）。
pub const API_BASE: &str = "https://api.bot.qq.com";
/// 网关接入点路径（Go `gatewayBotURI`）。
pub const GATEWAY_PATH: &str = "/gateway/bot";
/// Hello 未给出合法间隔时的兜底心跳周期。
pub const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
/// 首次连接失败后的等待时间；之后每次失败翻倍。
pub const DEFAULT_INITIAL_BACKOFF: Duration = Duration::from_secs(1);
/// 指数退避上限。
pub const DEFAULT_MAX_BACKOFF: Duration = Duration::from_secs(60);

/// 等 Hello 的超时：连上后网关会立刻推 Hello，10s 还没到说明链路异常。
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// op 9 `d=false` 后重新 identify 前的等待（Discord 语义是 1~5s，取固定值避免引入随机数依赖）。
const INVALID_SESSION_DELAY: Duration = Duration::from_secs(2);
/// 连续鉴权失败上限（botgo `maxAuthFailCount`）。
const MAX_AUTH_FAILURES: u32 = 3;
/// 写任务收尾的等待上限：发送端关闭后它会自然结束，超时只是兜底。
const WRITER_STOP_GRACE: Duration = Duration::from_secs(1);

// 网关 op 码（botgo `dto/opcode.go`）。
const OP_DISPATCH: i64 = 0;
const OP_HEARTBEAT: i64 = 1;
const OP_IDENTIFY: i64 = 2;
const OP_RESUME: i64 = 6;
const OP_RECONNECT: i64 = 7;
const OP_INVALID_SESSION: i64 = 9;
const OP_HELLO: i64 = 10;
const OP_HEARTBEAT_ACK: i64 = 11;

/// botgo `dto.IntentGuilds`：intent 为 0 时的兜底位（`client.go:170`）。
const INTENT_GUILDS: i64 = 1 << 0;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type Result<T> = std::result::Result<T, GatewayError>;

/// 网关内部错误：只用于日志与重连决策，不进入公开 API。
#[derive(Debug)]
struct GatewayError(String);

impl fmt::Display for GatewayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GatewayError {}

/// 网关客户端参数（生产默认即够用；测试覆盖 `api_base` 与退避时长）。
#[derive(Debug, Clone)]
pub struct GatewayOptions {
    /// `/gateway/bot` 基址覆盖；`None` 用 [`API_BASE`]。
    pub api_base: Option<String>,
    /// 首次连接失败后的退避。
    pub initial_backoff: Duration,
    /// 退避上限。
    pub max_backoff: Duration,
}

impl Default for GatewayOptions {
    fn default() -> Self {
        Self {
            api_base: None,
            initial_backoff: DEFAULT_INITIAL_BACKOFF,
            max_backoff: DEFAULT_MAX_BACKOFF,
        }
    }
}

/// 分片配置：`[id, count]`（botgo `dto.ShardConfig`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Shard {
    id: i64,
    count: i64,
}

/// `/gateway/bot` 响应（botgo `dto.WebsocketAP`）。
#[derive(Debug, Deserialize)]
struct GatewayInfo {
    url: String,
    #[serde(default)]
    shards: i64,
    #[serde(default)]
    session_start_limit: SessionStartLimit,
}

/// 连接频控信息（启动日志 + 多分片启动节流，botgo `dto.SessionStartLimit`）。
#[derive(Debug, Default, Deserialize)]
struct SessionStartLimit {
    #[serde(default)]
    total: i64,
    #[serde(default)]
    remaining: i64,
    /// 每 2s 允许建立的连接数（Go `manager.CalcInterval`）。
    #[serde(default)]
    max_concurrency: i64,
}

/// 网关帧信封（botgo `dto.WSPayload`）。
#[derive(Debug, Deserialize)]
struct Envelope {
    op: i64,
    #[serde(default)]
    t: Option<String>,
    #[serde(default)]
    s: Option<i64>,
    #[serde(default)]
    d: Option<Value>,
}

/// Hello 负载（botgo `dto.WSHelloData`）。
#[derive(Debug, Deserialize)]
struct Hello {
    heartbeat_interval: i64,
}

/// READY 负载里本模块关心的字段（botgo `dto.WSReadyData`）。
#[derive(Debug, Deserialize)]
struct Ready {
    session_id: String,
}

/// 一次连接结束的原因，决定外层如何重连。
#[derive(Debug)]
enum SessionEnd {
    /// 保留 session_id + seq，立即重连并 resume（op 7 / 4009 / 正常关闭）。
    Resume,
    /// 会话作废，清空 session_id + seq，延迟后重新 identify（op 9 d=false / 非预期关闭）。
    ReIdentify,
    /// 无法鉴权（4914/4915）或连续失败达上限：停止网关，不再重连。
    Fatal(String),
    /// 收到关闭信号。
    Stopped,
}

/// 一次连接的收尾状态。
#[derive(Debug)]
struct SessionOut {
    end: SessionEnd,
    session_id: Option<String>,
    seq: Option<i64>,
    ready: bool,
    auth_failure: bool,
}

impl SessionOut {
    fn new(
        end: SessionEnd,
        session_id: Option<String>,
        seq: &AtomicI64,
        ready: bool,
        auth_failure: bool,
    ) -> Self {
        let seq = seq.load(Ordering::Relaxed);
        Self {
            end,
            session_id,
            seq: (seq > 0).then_some(seq),
            ready,
            auth_failure,
        }
    }

    fn stopped() -> Self {
        Self {
            end: SessionEnd::Stopped,
            session_id: None,
            seq: None,
            ready: false,
            auth_failure: false,
        }
    }
}

/// QQ 上游 WS 网关客户端（按 `text_intent` 订阅事件）。
pub struct GatewayClient<T: HttpTransport> {
    transport: Arc<T>,
    tokens: Arc<TokenManager<T>>,
    config: Arc<ConfigManager>,
    downtime: Downtime,
    inbound: Inbound,
    options: GatewayOptions,
    /// 多分片时本实例固定的分片；`None` 表示每次 identify 按配置现算（单分片）。
    shard: Option<Shard>,
    /// 启动序号（多分片节流的倍数）；单分片恒 0。
    index: u32,
}

impl<T: HttpTransport> fmt::Debug for GatewayClient<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GatewayClient")
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl<T: HttpTransport> Clone for GatewayClient<T> {
    fn clone(&self) -> Self {
        Self {
            transport: Arc::clone(&self.transport),
            tokens: Arc::clone(&self.tokens),
            config: Arc::clone(&self.config),
            downtime: self.downtime.clone(),
            inbound: self.inbound.clone(),
            options: self.options.clone(),
            shard: self.shard,
            index: self.index,
        }
    }
}

impl<T: HttpTransport + 'static> GatewayClient<T> {
    /// 从 [`AppState`] 取配置、事件发布器（含总线）与入站转换器。
    #[must_use]
    pub fn new(
        transport: Arc<T>,
        tokens: Arc<TokenManager<T>>,
        state: &AppState,
        options: GatewayOptions,
    ) -> Self {
        Self {
            transport,
            tokens,
            config: state.config_arc(),
            downtime: Downtime::from_state(state),
            inbound: Inbound::from_state(state),
            options,
            shard: None,
            index: 0,
        }
    }

    /// 起一个后台任务跑网关；返回关闭句柄。
    ///
    /// 单分片一个任务；多分片（`shard_count == 1` 且 `shard_num > 1`）每个分片一个任务，
    /// 句柄统一关闭（Go `multi.NewShardManager().StartAllShards`）。
    #[must_use]
    pub fn start(self) -> GatewayHandle {
        let settings = self.config.snapshot();
        let shard = resolve_shard(&settings);
        // Go `main.go:375-399`：只有 `shard_count == 1` 才由本进程把 `shard_num` 展开成
        // N 个分片；`shard_count > 1` 是 `StartSingle`，只连 `[shard_id, shard_count]`。
        let local = if settings.get_shard_count() > 1 {
            1
        } else {
            shard.count
        };
        if local <= 1 {
            let (stop, task) = self.spawn_task();
            return GatewayHandle {
                stops: vec![stop],
                tasks: vec![task],
            };
        }

        tracing::info!(
            shards = local,
            "按 shard_num 启动多分片网关，每个分片一个连接"
        );
        let capacity = usize::try_from(local).unwrap_or(0);
        let mut handle = GatewayHandle {
            stops: Vec::with_capacity(capacity),
            tasks: Vec::with_capacity(capacity),
        };
        for index in 0..local {
            let shard = Shard {
                id: index,
                count: local,
            };
            let ordinal = u32::try_from(index).unwrap_or(u32::MAX);
            let (stop, task) = self.shard_client(shard, ordinal).spawn_task();
            handle.stops.push(stop);
            handle.tasks.push(task);
        }
        handle
    }

    /// 派生一个固定分片与启动序号的实例（多分片用）。
    fn shard_client(&self, shard: Shard, index: u32) -> Self {
        Self {
            shard: Some(shard),
            index,
            ..self.clone()
        }
    }

    /// 起一个后台任务跑网关；返回 (关闭信号, 任务句柄)。
    fn spawn_task(self) -> (oneshot::Sender<()>, JoinHandle<()>) {
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(async move { self.run(stopped).await });
        (stop, task)
    }

    /// 外层重连循环：取接入点 → 连接 → 按 [`SessionEnd`] 决定 resume / 重新 identify。
    async fn run(self, mut stop: oneshot::Receiver<()>) {
        let api_base = self.api_base();
        let mut backoff = self.options.initial_backoff;
        let mut gateway_url: Option<String> = None;
        let mut session_id: Option<String> = None;
        let mut last_seq: Option<i64> = None;
        let mut auth_failures = 0_u32;
        // 多分片的首次连接延迟：间隔要等 `/gateway/bot` 给出 max_concurrency 才算得出来。
        let mut start_delay: Option<Duration> = None;

        loop {
            if gateway_url.is_none() {
                let fetched = tokio::select! {
                    _ = &mut stop => return,
                    fetched = self.fetch_gateway(&api_base) => fetched,
                };
                match fetched {
                    Ok(info) => {
                        tracing::info!(
                            url = info.url.as_str(),
                            suggested_shards = info.shards,
                            sessions_total = info.session_start_limit.total,
                            sessions_remaining = info.session_start_limit.remaining,
                            "已获取 QQ 网关接入点"
                        );
                        self.log_shard(info.shards);
                        start_delay = Some(
                            calc_start_interval(info.session_start_limit.max_concurrency)
                                .saturating_mul(self.index),
                        );
                        gateway_url = Some(info.url);
                    }
                    Err(err) => {
                        tracing::warn!(%err, ?backoff, "获取 QQ 网关接入点失败，退避后重试");
                        if !sleep_or_stop(&mut stop, backoff).await {
                            return;
                        }
                        backoff = next_backoff(backoff, self.options.max_backoff);
                        continue;
                    }
                }
            }
            let Some(url) = gateway_url.clone() else {
                continue;
            };

            // Go `multi.StartAllShards` 按 `CalcInterval` 的间隔逐个拉起分片。
            if let Some(delay) = start_delay.take()
                && !delay.is_zero()
                && !sleep_or_stop(&mut stop, delay).await
            {
                return;
            }

            match self
                .session(&url, session_id.clone(), last_seq, &mut stop)
                .await
            {
                Err(err) => {
                    tracing::warn!(%err, ?backoff, "QQ 网关连接中断，退避后重连");
                    if !sleep_or_stop(&mut stop, backoff).await {
                        return;
                    }
                    backoff = next_backoff(backoff, self.options.max_backoff);
                }
                Ok(out) => {
                    session_id = out.session_id;
                    last_seq = out.seq;
                    if out.ready {
                        backoff = self.options.initial_backoff;
                        auth_failures = 0;
                    }
                    if out.auth_failure {
                        auth_failures += 1;
                        if auth_failures >= MAX_AUTH_FAILURES {
                            tracing::error!(
                                failures = auth_failures,
                                "QQ 网关鉴权连续失败，停止重连；请检查 app_id / client_secret 与机器人状态"
                            );
                            return;
                        }
                    }
                    match out.end {
                        SessionEnd::Stopped => return,
                        SessionEnd::Resume => {
                            tracing::info!("QQ 网关要求重连，将携带 session_id + seq 走 resume");
                        }
                        SessionEnd::ReIdentify => {
                            tracing::warn!("QQ 网关会话失效，清空会话后重新 identify");
                            session_id = None;
                            last_seq = None;
                            if !sleep_or_stop(&mut stop, INVALID_SESSION_DELAY).await {
                                return;
                            }
                        }
                        SessionEnd::Fatal(reason) => {
                            tracing::error!(reason = reason.as_str(), "QQ 网关无法鉴权，停止重连");
                            return;
                        }
                    }
                }
            }
        }
    }

    /// 一次完整连接：Hello → Identify/Resume → 心跳 + 读循环。
    async fn session(
        &self,
        url: &str,
        resume_id: Option<String>,
        resume_seq: Option<i64>,
        stop: &mut oneshot::Receiver<()>,
    ) -> Result<SessionOut> {
        let (socket, _response) = tokio::select! {
            _ = &mut *stop => return Ok(SessionOut::stopped()),
            result = connect_async(url) => result
                .map_err(|err| GatewayError(format!("连接 QQ 网关 {url} 失败: {err}")))?,
        };
        let (sink, mut stream) = socket.split();

        // 1. Hello：网关连上后的首帧，间隔由它决定。
        let hello = tokio::select! {
            _ = &mut *stop => return Ok(SessionOut::stopped()),
            result = tokio::time::timeout(HELLO_TIMEOUT, next_text(&mut stream)) => result,
        };
        let hello = match hello {
            Err(_) => return Err(GatewayError("等待 QQ 网关 Hello 超时".to_owned())),
            Ok(None) => return Err(GatewayError("QQ 网关在 Hello 前关闭了连接".to_owned())),
            Ok(Some(Err(err))) => return Err(err),
            Ok(Some(Ok(text))) => text,
        };
        let envelope: Envelope = serde_json::from_str(&hello)
            .map_err(|err| GatewayError(format!("QQ 网关 Hello 不是合法 JSON: {err}")))?;
        if envelope.op != OP_HELLO {
            return Err(GatewayError(format!(
                "QQ 网关首帧不是 Hello（op={}）",
                envelope.op
            )));
        }
        let hello: Hello = serde_json::from_value(envelope.d.unwrap_or(Value::Null))
            .map_err(|err| GatewayError(format!("QQ 网关 Hello 负载非法: {err}")))?;
        let interval = heartbeat_interval(hello.heartbeat_interval);

        // 2. Identify / Resume 的 token：每次都现取（TokenManager 自带缓存与提前刷新）。
        let token = tokio::select! {
            _ = &mut *stop => return Ok(SessionOut::stopped()),
            token = self.tokens.token() => token
                .map_err(|err| GatewayError(format!("获取 access_token 失败: {err}")))?,
        };

        let seq = Arc::new(AtomicI64::new(resume_seq.unwrap_or(0)));
        let (out, out_rx) = mpsc::unbounded_channel::<Message>();
        let writer = tokio::spawn(write_loop(sink, out_rx));

        let (payload, resuming) = match &resume_id {
            Some(id) => (
                json!({
                    "op": OP_RESUME,
                    "d": {
                        "token": format!("QQBot {token}"),
                        "session_id": id,
                        "seq": resume_seq.unwrap_or(0),
                    },
                }),
                true,
            ),
            None => {
                let shard = self.resolved_shard();
                (
                    json!({
                        "op": OP_IDENTIFY,
                        "d": {
                            "token": format!("QQBot {token}"),
                            "intents": self.intents(),
                            "shard": [shard.id, shard.count],
                            "properties": {
                                "$os": std::env::consts::OS,
                                "$browser": "gensokyo",
                                "$device": "gensokyo",
                            },
                        },
                    }),
                    false,
                )
            }
        };
        tracing::info!(
            resuming,
            interval_ms = interval.as_millis(),
            "QQ 网关已连接，发送鉴权帧"
        );
        if out.send(Message::text(payload.to_string())).is_err() {
            writer.abort();
            return Err(GatewayError("QQ 网关写通道已关闭".to_owned()));
        }

        // 3. 心跳：identify / resume 发出后才启动（与 Go 的 ticker 时序一致）。
        let heartbeat = spawn_heartbeat(interval, Arc::clone(&seq), out.clone());
        let result = self
            .read_loop(&mut stream, &out, &seq, stop, resume_id)
            .await;

        heartbeat.abort();
        let _ = heartbeat.await;
        drop(out);
        let _ = tokio::time::timeout(WRITER_STOP_GRACE, writer).await;
        result
    }

    /// 读循环：dispatch → 入站转换；op 7/9 与 close 码决定本次连接如何收尾。
    async fn read_loop(
        &self,
        stream: &mut SplitStream<Socket>,
        out: &mpsc::UnboundedSender<Message>,
        seq: &AtomicI64,
        stop: &mut oneshot::Receiver<()>,
        mut session_id: Option<String>,
    ) -> Result<SessionOut> {
        let mut ready = false;
        loop {
            let frame = tokio::select! {
                _ = &mut *stop => {
                    return Ok(SessionOut::new(
                        SessionEnd::Stopped,
                        session_id,
                        seq,
                        ready,
                        false,
                    ));
                }
                frame = stream.next() => frame,
            };
            let Some(frame) = frame else {
                // 对端无 close 帧直接断开：保留会话，走 resume（botgo 对非 CloseError 同样保留）。
                return Ok(SessionOut::new(
                    SessionEnd::Resume,
                    session_id,
                    seq,
                    ready,
                    false,
                ));
            };
            let message =
                frame.map_err(|err| GatewayError(format!("读取 QQ 网关帧失败: {err}")))?;
            match message {
                Message::Text(text) => {
                    let envelope: Envelope = match serde_json::from_str(&text) {
                        Ok(envelope) => envelope,
                        Err(err) => {
                            tracing::warn!(%err, "QQ 网关帧不是合法 JSON，已忽略");
                            continue;
                        }
                    };
                    if envelope.op == OP_DISPATCH
                        && let Some(s) = envelope.s
                    {
                        seq.store(s, Ordering::Relaxed);
                    }
                    match envelope.op {
                        OP_DISPATCH => {
                            let Some(kind) = envelope.t.as_deref() else {
                                tracing::debug!("dispatch 缺少 t 字段，已忽略");
                                continue;
                            };
                            if kind == "READY" {
                                session_id = ready_session(envelope.d.as_ref());
                                ready = true;
                                tracing::info!(
                                    session_id = session_id.as_deref().unwrap_or(""),
                                    "QQ 网关 READY"
                                );
                                continue;
                            }
                            self.publish_dispatch(&text, kind);
                        }
                        OP_HEARTBEAT_ACK => {}
                        OP_RECONNECT => {
                            return Ok(SessionOut::new(
                                SessionEnd::Resume,
                                session_id,
                                seq,
                                ready,
                                false,
                            ));
                        }
                        OP_INVALID_SESSION => {
                            let resumable = envelope
                                .d
                                .as_ref()
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            let end = if resumable {
                                SessionEnd::Resume
                            } else {
                                SessionEnd::ReIdentify
                            };
                            return Ok(SessionOut::new(end, session_id, seq, ready, !resumable));
                        }
                        OP_HELLO => tracing::warn!("QQ 网关重复发送 Hello，已忽略"),
                        other => tracing::debug!(op = other, "忽略未知网关 op"),
                    }
                }
                Message::Ping(payload) => {
                    // split 之后 tungstenite 不会自动回 pong，必须显式回。
                    let _ = out.send(Message::Pong(payload));
                }
                Message::Close(frame) => {
                    return self.close_outcome(frame, session_id, seq, ready).await;
                }
                Message::Pong(_) | Message::Binary(_) => {}
                _ => {}
            }
        }
    }

    /// close 帧 → 重连语义（botgo `client.go:100-126`）。
    async fn close_outcome(
        &self,
        frame: Option<CloseFrame>,
        session_id: Option<String>,
        seq: &AtomicI64,
        ready: bool,
    ) -> Result<SessionOut> {
        let code = frame.as_ref().map(|frame| u16::from(frame.code));
        match code {
            Some(4004) => {
                // access_token 过期：刷新后重新 identify（botgo `UpAccessToken`）。
                self.refresh_token("websocket 4004").await;
                Ok(SessionOut::new(
                    SessionEnd::ReIdentify,
                    session_id,
                    seq,
                    ready,
                    true,
                ))
            }
            Some(4914 | 4915) => Ok(SessionOut::new(
                SessionEnd::Fatal(format!(
                    "QQ 网关拒绝鉴权（close {}）：机器人可能被封禁/下架，或不是群聊机器人",
                    code.unwrap_or_default()
                )),
                session_id,
                seq,
                ready,
                true,
            )),
            Some(4009 | 1000 | 1001) | None => Ok(SessionOut::new(
                SessionEnd::Resume,
                session_id,
                seq,
                ready,
                false,
            )),
            Some(other) => {
                // botgo `IsUnexpectedCloseError`：非 4009 的异常关闭不能 resume。
                tracing::warn!(code = other, "QQ 网关异常关闭，清空会话后重新 identify");
                Ok(SessionOut::new(
                    SessionEnd::ReIdentify,
                    session_id,
                    seq,
                    ready,
                    false,
                ))
            }
        }
    }

    /// dispatch → [`Inbound::convert_json`] → [`EventHub`]。
    fn publish_dispatch(&self, raw: &str, kind: &str) {
        let mode = self.config.snapshot().get_op_userid_type().to_owned();
        match self.inbound.convert_json(raw.as_bytes()) {
            Ok(events) => {
                let published = events.len();
                for mut event in events {
                    apply_op_userid_type(&mut event, &mode);
                    self.downtime.publish(event);
                }
                tracing::debug!(kind, published, "QQ 网关事件已发布");
            }
            Err(err) => tracing::warn!(%err, kind, "QQ 网关事件解析失败，已忽略"),
        }
    }

    /// `GET /gateway/bot`；401 / `code=40001` 时刷新 token 重试一次。
    async fn fetch_gateway(&self, api_base: &str) -> Result<GatewayInfo> {
        let url = format!("{}{GATEWAY_PATH}", api_base.trim_end_matches('/'));
        let mut refreshed = false;
        loop {
            let token = self
                .tokens
                .token()
                .await
                .map_err(|err| GatewayError(format!("获取 access_token 失败: {err}")))?;
            let request = HttpRequest::get(&url)
                .with_header("Authorization", format!("QQBot {token}"))
                .with_header("Accept", "application/json");
            let response = self
                .transport
                .request(request)
                .await
                .map_err(|err| GatewayError(format!("请求 {url} 失败: {err}")))?;
            if (200..300).contains(&response.status) {
                return serde_json::from_slice(&response.body)
                    .map_err(|err| GatewayError(format!("{url} 响应不是合法 JSON: {err}")));
            }
            if !refreshed && is_auth_failure(response.status, &response.body) {
                refreshed = true;
                self.tokens
                    .force_refresh()
                    .await
                    .map_err(|err| GatewayError(format!("刷新 access_token 失败: {err}")))?;
                continue;
            }
            return Err(GatewayError(format!(
                "{url} 返回 HTTP {}：{}",
                response.status,
                truncate(&String::from_utf8_lossy(&response.body))
            )));
        }
    }

    /// 强制刷新 token（只影响下一次取用）。
    async fn refresh_token(&self, reason: &str) {
        match self.tokens.force_refresh().await {
            Ok(_) => tracing::info!(reason, "已强制刷新 access_token"),
            Err(err) => tracing::warn!(%err, reason, "强制刷新 access_token 失败"),
        }
    }

    /// intent 位：`text_intent` → botgo 位；为 0 时按 botgo 兜底 `IntentGuilds`。
    fn intents(&self) -> i64 {
        let bits = IntentPlan::from_settings(&self.config.snapshot()).bits();
        if bits == 0 {
            tracing::warn!("intent 位为 0（text_intent 为空），按 botgo 兜底订阅 IntentGuilds");
            INTENT_GUILDS
        } else {
            bits
        }
    }

    fn api_base(&self) -> String {
        self.options
            .api_base
            .as_deref()
            .unwrap_or(API_BASE)
            .to_owned()
    }

    /// 本实例的分片：多分片时固定，否则每次按当前配置快照现算（热重载即时生效）。
    fn resolved_shard(&self) -> Shard {
        self.shard
            .unwrap_or_else(|| resolve_shard(&self.config.snapshot()))
    }

    /// 启动时分片取舍的日志（Go `main.go:319-324` 打印网关建议）。
    fn log_shard(&self, suggested: i64) {
        let shard = self.resolved_shard();
        tracing::info!(
            shard_id = shard.id,
            shard_count = shard.count,
            suggested,
            "QQ 网关注册分片"
        );
        if self.shard.is_some() {
            return;
        }
        if shard.count > 1 {
            tracing::warn!(
                shard_id = shard.id,
                shard_count = shard.count,
                "多分片模式下本进程只连接一个分片，其余分片的事件需要另开进程"
            );
        } else if suggested > 1 {
            tracing::info!(
                suggested,
                "网关建议多分片，但按 shard_count/shard_num 配置以单分片（全量事件）连接"
            );
        }
    }
}

/// Go `Processor.applyOpUserIDType`（`Processor/Processor.go:245-270`）：按 `op_userid_type`
/// 把上报里的 `user_id` / `group_id` 换成 `real_user_id` / `real_group_id`。
///
/// - `vuin`：原样上报虚拟 ID；
/// - `ruin`：仅当真实 ID 以 `ruin-` 开头（Go 先 `strings.ToLower`）才替换；
/// - 其它（默认 `openid`）：真实 ID 非空就替换。
///
/// Go 在每次广播前对事件 map 调用；Rust 的事件是类型化的，所以在两个发布点（网关 dispatch、
/// webhook）发布前改写同一份事件，语义等价。只动 `user_id` / `group_id`，不动
/// `operator_id` / `target_id`。
pub(crate) fn apply_op_userid_type(event: &mut Event, mode: &str) {
    let mode = mode.trim();
    if mode == "vuin" {
        return;
    }
    let ruin_only = mode == "ruin";

    // Go `replace(field, realField)`：真实 ID 非空且满足 ruin 前缀才替换。
    fn real(extra: &Extra, key: &str, ruin_only: bool) -> Option<gensokyo_core::Ob11Id> {
        let raw = extra.get(key)?.as_str()?;
        if raw.is_empty() || (ruin_only && !raw.to_lowercase().starts_with("ruin-")) {
            return None;
        }
        Some(parse_id(raw))
    }

    match event {
        Event::Message(message) | Event::MessageSent(message) => {
            if let Some(user) = real(&message.extra, "real_user_id", ruin_only) {
                message.user_id = user;
                // `sender.user_id` 跟随 `user_id`（Go 的 `sender["user_id"] = user_id`）；
                // 未发生替换时 sender 保持原样，避免把 string_ob11 下恒为 0 的 sender 改掉。
                if let Some(sender) = message.sender.as_mut() {
                    sender.user_id = Some(message.user_id.clone());
                }
            }
            if let Some(group) = real(&message.extra, "real_group_id", ruin_only) {
                message.group_id = Some(group);
            }
        }
        Event::Notice(notice) => {
            if let Some(user) = real(&notice.extra, "real_user_id", ruin_only) {
                notice.user_id = Some(user);
            }
            if let Some(group) = real(&notice.extra, "real_group_id", ruin_only) {
                notice.group_id = Some(group);
            }
        }
        Event::Request(request) => {
            if let Some(user) = real(&request.extra, "real_user_id", ruin_only) {
                request.user_id = Some(user);
            }
            if let Some(group) = real(&request.extra, "real_group_id", ruin_only) {
                request.group_id = Some(group);
            }
        }
        Event::MetaEvent(_) | Event::Unknown => {}
    }
}

/// 关闭句柄：`shutdown` 有界等待，`abort` 立即中止，`Drop` 兜底中止。
///
/// 多分片时持有一组任务，关闭/中止对每个分片都生效。
#[derive(Debug)]
pub struct GatewayHandle {
    stops: Vec<oneshot::Sender<()>>,
    tasks: Vec<JoinHandle<()>>,
}

impl GatewayHandle {
    /// 网关任务是否已全部结束（正常退出或出错退出）。
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.tasks.iter().all(JoinHandle::is_finished)
    }

    /// 发关闭信号并等待任务退出；超时则强制中止。
    pub async fn shutdown(mut self, limit: Duration) {
        for stop in self.stops.drain(..) {
            let _ = stop.send(());
        }
        let wait = async {
            for task in &mut self.tasks {
                let _ = task.await;
            }
        };
        let timed_out = tokio::time::timeout(limit, wait).await.is_err();
        if timed_out {
            for task in &self.tasks {
                task.abort();
            }
            tracing::warn!(?limit, "QQ 网关客户端未在时限内退出，已强制中止");
        }
    }

    /// 立即中止（不等关闭信号）。
    pub fn abort(&self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl Drop for GatewayHandle {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// 分片启动间隔（Go `manager.CalcInterval`：`max_concurrency` 个连接分摊 2s 窗口，
/// 四舍五入到秒、最少 1s；`0` 按 1 处理）。
fn calc_start_interval(max_concurrency: i64) -> Duration {
    let concurrency = if max_concurrency > 0 {
        max_concurrency
    } else {
        1
    };
    let seconds = 2.0_f64 / concurrency as f64;
    Duration::from_secs((seconds.round() as u64).max(1))
}

/// 分片配置：以配置为准，网关建议的 `shards` 只打日志（Go `main.go:375-399`）。
fn resolve_shard(settings: &Settings) -> Shard {
    let configured = settings.get_shard_count();
    if configured > 1 {
        let count = configured;
        Shard {
            id: settings.get_shard_id().clamp(0, count - 1),
            count,
        }
    } else {
        // `shard_count == 1` 时 Go 把网关建议覆盖成 `shard_num`（默认 1 → 单连接收全量）。
        Shard {
            id: 0,
            count: settings.get_shard_num().max(1),
        }
    }
}

/// Hello 的 `heartbeat_interval`（毫秒）→ 周期；非法值回退默认值。
fn heartbeat_interval(millis: i64) -> Duration {
    if millis > 0 {
        Duration::from_millis(u64::try_from(millis).unwrap_or(30_000))
    } else {
        DEFAULT_HEARTBEAT_INTERVAL
    }
}

/// READY 的 `session_id`；缺失时返回 `None`（下一次连接会重新 identify）。
fn ready_session(data: Option<&Value>) -> Option<String> {
    data.and_then(|data| serde_json::from_value::<Ready>(data.clone()).ok())
        .map(|ready| ready.session_id)
}

/// 指数退避：翻倍直到上限。
fn next_backoff(current: Duration, max: Duration) -> Duration {
    (current * 2).min(max)
}

/// 退避等待；收到关闭信号返回 `false`。
async fn sleep_or_stop(stop: &mut oneshot::Receiver<()>, delay: Duration) -> bool {
    tokio::select! {
        _ = &mut *stop => false,
        () = tokio::time::sleep(delay) => true,
    }
}

/// 读一帧文本（跳过非文本帧）；连接关闭或出错时返回原因。
async fn next_text(stream: &mut SplitStream<Socket>) -> Option<Result<String>> {
    loop {
        match stream.next().await? {
            Ok(Message::Text(text)) => return Some(Ok(text.to_string())),
            Ok(Message::Close(frame)) => {
                let reason = frame.map_or_else(
                    || "QQ 网关在握手前关闭了连接".to_owned(),
                    |frame| {
                        format!(
                            "QQ 网关在握手前关闭了连接（close {}）",
                            u16::from(frame.code)
                        )
                    },
                );
                return Some(Err(GatewayError(reason)));
            }
            Ok(_) => continue,
            Err(err) => {
                return Some(Err(GatewayError(format!("读取 QQ 网关帧失败: {err}"))));
            }
        }
    }
}

/// 写任务：把通道里的帧顺序写进 socket；写失败即结束本次连接。
async fn write_loop(
    mut sink: SplitSink<Socket, Message>,
    mut rx: mpsc::UnboundedReceiver<Message>,
) {
    while let Some(message) = rx.recv().await {
        if let Err(err) = sink.send(message).await {
            tracing::warn!(%err, "QQ 网关写入失败，结束本次连接");
            return;
        }
    }
}

/// 心跳任务：按 Hello 的间隔发 op 1，`d` 为最近 seq（没有则 `null`）。
fn spawn_heartbeat(
    interval: Duration,
    seq: Arc<AtomicI64>,
    out: mpsc::UnboundedSender<Message>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        // 首个 tick 在间隔之后：identify 之后立刻发心跳与 Go 的 Ticker.Reset 语义一致。
        let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let seq = seq.load(Ordering::Relaxed);
            let payload = if seq > 0 {
                json!({ "op": OP_HEARTBEAT, "d": seq })
            } else {
                json!({ "op": OP_HEARTBEAT, "d": null })
            };
            if out.send(Message::text(payload.to_string())).is_err() {
                return;
            }
        }
    })
}

/// 401 或 `code=40001` 视为 token 失效。
fn is_auth_failure(status: u16, body: &[u8]) -> bool {
    if status == 401 {
        return true;
    }
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.get("code").and_then(Value::as_i64))
        .is_some_and(|code| code == 40001)
}

/// 错误信息里的响应体截断，避免把整页 HTML 写进日志。
fn truncate(text: &str) -> String {
    const LIMIT: usize = 512;
    if text.len() <= LIMIT {
        return text.to_owned();
    }
    let mut end = LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::{
        GatewayInfo, calc_start_interval, heartbeat_interval, is_auth_failure, resolve_shard,
    };
    use gensokyo_protocol::Event;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn hello_interval_falls_back_when_missing_or_zero() {
        assert_eq!(
            heartbeat_interval(0),
            super::DEFAULT_HEARTBEAT_INTERVAL,
            "0 回退默认 30s"
        );
        assert_eq!(heartbeat_interval(-1), super::DEFAULT_HEARTBEAT_INTERVAL);
        assert_eq!(heartbeat_interval(30_000), Duration::from_secs(30));
    }

    #[test]
    fn gateway_info_tolerates_missing_optional_fields() {
        let info: GatewayInfo =
            serde_json::from_str(r#"{"url":"wss://example"}"#).expect("解析失败");
        assert_eq!(info.shards, 0);
        assert_eq!(info.session_start_limit.total, 0);
    }

    #[test]
    fn auth_failure_detects_http_and_business_code() {
        assert!(is_auth_failure(401, b""));
        assert!(is_auth_failure(
            400,
            br#"{"code":40001,"message":"invalid token"}"#
        ));
        assert!(!is_auth_failure(400, br#"{"code":11244}"#));
        assert!(!is_auth_failure(400, b"not json"));
    }

    #[test]
    fn shard_prefers_config_and_defaults_to_single() {
        let mut settings = gensokyo_config::Settings::default();
        assert_eq!(resolve_shard(&settings), super::Shard { id: 0, count: 1 });

        settings.shard_count = 4;
        settings.shard_id = 2;
        assert_eq!(resolve_shard(&settings), super::Shard { id: 2, count: 4 });

        settings.shard_count = 1;
        settings.shard_num = 3;
        assert_eq!(
            resolve_shard(&settings),
            super::Shard { id: 0, count: 3 },
            "shard_count=1 时按 shard_num 覆盖（Go main.go:377）"
        );
    }

    #[test]
    fn shard_start_interval_matches_go_calc_interval() {
        assert_eq!(
            calc_start_interval(0),
            Duration::from_secs(2),
            "0 按 1 处理"
        );
        assert_eq!(calc_start_interval(1), Duration::from_secs(2));
        assert_eq!(calc_start_interval(2), Duration::from_secs(1));
        assert_eq!(
            calc_start_interval(3),
            Duration::from_secs(1),
            "round(2/3)=1"
        );
        assert_eq!(calc_start_interval(8), Duration::from_secs(1), "最少 1s");
    }

    fn group_message_event(real_user_id: &str) -> Event {
        serde_json::from_value(json!({
            "post_type": "message",
            "time": 1,
            "self_id": 100,
            "message_type": "group",
            "message_id": 9,
            "user_id": 200,
            "group_id": 300,
            "message": "hi",
            "raw_message": "hi",
            "sender": { "user_id": 200, "nickname": "n" },
            "real_user_id": real_user_id,
            "real_group_id": "GROUP-REAL"
        }))
        .expect("构造消息事件")
    }

    #[test]
    fn op_userid_type_rewrites_ids_in_three_modes() {
        // vuin：开关关闭，原样上报虚拟 ID
        let mut event = group_message_event("OPENID");
        super::apply_op_userid_type(&mut event, "vuin");
        let value = serde_json::to_value(&event).expect("序列化");
        assert_eq!(value["user_id"], 200);
        assert_eq!(value["group_id"], 300);
        assert_eq!(value["sender"]["user_id"], 200);

        // openid（默认）：真实 ID 直接替换，sender.user_id 跟随
        let mut event = group_message_event("OPENID");
        super::apply_op_userid_type(&mut event, "openid");
        let value = serde_json::to_value(&event).expect("序列化");
        assert_eq!(value["user_id"], "OPENID");
        assert_eq!(value["group_id"], "GROUP-REAL");
        assert_eq!(value["sender"]["user_id"], "OPENID");

        // ruin：只认 ruin- 前缀（这里 real_user_id 不带前缀，保持虚拟 ID）
        let mut event = group_message_event("OPENID");
        super::apply_op_userid_type(&mut event, "ruin");
        let value = serde_json::to_value(&event).expect("序列化");
        assert_eq!(value["user_id"], 200, "非 ruin- 前缀不替换");
        assert_eq!(value["group_id"], 300, "group 前缀同样判定");

        let mut event = group_message_event("RUIN-ABC");
        super::apply_op_userid_type(&mut event, "ruin");
        let value = serde_json::to_value(&event).expect("序列化");
        assert_eq!(value["user_id"], "RUIN-ABC", "前缀大小写不敏感");
    }

    #[test]
    fn op_userid_type_leaves_native_events_untouched() {
        let mut event: Event = serde_json::from_value(json!({
            "post_type": "meta_event",
            "time": 1,
            "self_id": 100,
            "meta_event_type": "lifecycle",
            "sub_type": "connect",
            "real_user_id": "OPENID"
        }))
        .expect("构造元事件");
        let before = serde_json::to_value(&event).expect("序列化");
        super::apply_op_userid_type(&mut event, "openid");
        assert_eq!(serde_json::to_value(&event).expect("序列化"), before);
    }
}
