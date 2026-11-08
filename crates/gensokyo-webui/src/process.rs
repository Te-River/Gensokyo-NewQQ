//! 进程面与日志：Go `webui/api.go:208-228` 的 `/{uin}/process*`、`/api/logs`、`/{uin}/api`。
//!
//! Rust 版**没有**进程守护（一个进程就是一个实例，重启/停止靠外部 supervisor），因此：
//!
//! - `GET /api/{uin}/process/status` 回 Go 的完整形状；`details` 复用
//!   [`SystemStatus`](crate::status::SystemStatus) 的进程投影（pid 是真实进程号，内存/CPU
//!   尚未采集故为 0），`total_logs` / `restarts` 没有对应计数器，如实给 0；
//! - `DELETE /api/{uin}/process` 在 Go 里是 `os.Exit(0)`。这里**不**自杀（会把共享的 runtime
//!   连同未落盘的队列一起掀掉，且没有优雅关闭入口），回 501 并说明；
//! - `GET /api/{uin}/process/logs` 与 `GET /api/logs`：Go `webui/api.go:52-70` 两条路径
//!   **同语义**——带 `Upgrade: websocket` 头就升级成日志流（`mylog.WsHandlerWithDependencies`），
//!   否则走 `getProcessLogs` 回空数组。这里逐条照搬：WS 升级后先补发 [`crate::logs`] 的
//!   环形缓冲快照，再实时推送新日志；非 WS 请求仍回 `[]`（Go 源码里标着"待完善：从 mylog
//!   通道取日志"，至今没实现，前端也不用它）。
//! - `POST /api/{uin}/api?name=...`：Go 的 switch 只有 `default: 400 Invalid API name`，
//!   没有任何真实转发分支；这里逐字照搬（前端也不依赖它）。

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Path, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::json;
use tokio::sync::broadcast;
use tokio::time::MissedTickBehavior;

use crate::auth::{guard_uin, is_authenticated, unauthorized};
use crate::logs::{LogEntry, hub};
use crate::state::WebUiState;
use crate::status::{RunningProcessDetail, SystemStatus};

/// 日志 WS 的心跳间隔；Go `writePump` 的 `heartbeatTicker` 是 10s。
const LOG_HEARTBEAT: Duration = Duration::from_secs(10);

/// `GET /api/{uin}/process/status` 的响应体（Go `HandleProcessStatusRequest` 逐字段）。
#[derive(Debug, Serialize)]
pub struct ProcessInfo {
    /// 进程状态；Go 固定 `running`。
    pub status: &'static str,
    /// 日志条数；Go `HandleProcessStatusRequest` 写死 0（不反射 `logChannel` 长度），
    /// 这里保持同一口径。
    pub total_logs: u64,
    /// 重启次数；没有进程守护，恒为 0。
    pub restarts: u64,
    /// 二维码登录串；Go 固定 `null`（Rust 无扫码登录通道）。
    pub qr_uri: Option<String>,
    /// 进程明细。
    pub details: RunningProcessDetail,
}

impl ProcessInfo {
    /// 当前进程的一次快照。
    #[must_use]
    pub fn current() -> Self {
        Self {
            status: "running",
            total_logs: 0,
            restarts: 0,
            qr_uri: None,
            details: SystemStatus::current().process,
        }
    }
}

/// `GET /api/{uin}/process/status`。
pub(crate) async fn process_status(
    State(state): State<Arc<WebUiState>>,
    Path(uin): Path<u64>,
    headers: HeaderMap,
) -> Response {
    if let Some(reject) = guard_uin(&state, &headers, uin) {
        return reject;
    }
    Json(ProcessInfo::current()).into_response()
}

/// `DELETE /api/{uin}/process`：Go 用 `os.Exit(0)` 结束进程，Rust 没有可交还的关闭入口。
pub(crate) async fn process_stop(
    State(state): State<Arc<WebUiState>>,
    Path(uin): Path<u64>,
    headers: HeaderMap,
) -> Response {
    if let Some(reject) = guard_uin(&state, &headers, uin) {
        return reject;
    }
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({ "error": "process stop is not available: no process supervisor" })),
    )
        .into_response()
}

/// `GET /api/{uin}/process/logs` 与 `GET /api/logs`：Go 两条路径同语义（`webui/api.go:52-70`）。
///
/// - `Upgrade: websocket` → 升级成日志流（见 [`push_logs`]）；
/// - 否则回 `[]`——Go `getProcessLogs` 就是 `c.JSON(200, []interface{}{})`。
///
/// 鉴权照 Go：中间件对 `/api/*` 一律校验 `login_cookie`（只有 `/api/login` 与
/// `/api/check-login-status` 例外），WS 握手同样带 cookie，所以未登录一律 401。
/// `/api/*/process/logs` 也不校验 uin，这里照旧只查登录态。
pub(crate) async fn logs(
    State(state): State<Arc<WebUiState>>,
    headers: HeaderMap,
    request: Request,
) -> Response {
    if !is_authenticated(&state, &headers) {
        return unauthorized();
    }
    if !is_websocket_upgrade(&headers) {
        return Json(Vec::<String>::new()).into_response();
    }
    // 先自己判 `Upgrade` 头（对齐 Go 的分支条件），再让 axum 校验完整的握手要素。
    let (mut parts, _body) = request.into_parts();
    match WebSocketUpgrade::from_request_parts(&mut parts, &state).await {
        Ok(upgrade) => upgrade.on_upgrade(push_logs).into_response(),
        Err(rejection) => rejection.into_response(),
    }
}

/// `Upgrade` 头是否声明 websocket。Go 直接比较 `== "websocket"`，这里放宽大小写与空白。
fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    headers
        .get(header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("websocket"))
}

/// WS 会话：缓冲快照 → 实时日志；每 10s 发一次 Ping（Go `writePump`）。
async fn push_logs(mut socket: WebSocket) {
    let (history, mut rx) = hub().subscribe_with_history();
    for entry in &history {
        if send_log(&mut socket, entry).await.is_err() {
            return;
        }
    }

    let mut heartbeat = tokio::time::interval(LOG_HEARTBEAT);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // `interval` 的首个 tick 立即就绪，先消费掉，第一次心跳落在 10s 后。
    heartbeat.tick().await;

    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                // 对端关闭或读失败即结束会话；其余消息（含 Go 约定的文本 "heartbeat"）
                // 只是续期，不必回应。
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
                Some(Ok(_)) => {}
            },
            entry = rx.recv() => match entry {
                Ok(entry) => {
                    if send_log(&mut socket, &entry).await.is_err() {
                        return;
                    }
                }
                // 慢客户端落后：Go 的广播同样直接丢（`send` 非阻塞），不追补缺口。
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            },
            _ = heartbeat.tick() => {
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    return;
                }
            }
        }
    }
}

/// 推送一条日志；连接已断开时返回 `Err`，由调用方结束会话。
async fn send_log(socket: &mut WebSocket, entry: &LogEntry) -> Result<(), axum::Error> {
    // 三个字段都是 String，序列化不会失败。
    let text = serde_json::to_string(entry).expect("LogEntry 序列化不会失败");
    socket.send(Message::Text(text.into())).await
}

/// `POST /api/{uin}/api?name=...`：Go 的 switch 只有 `default`，任何 name 都回 400。
pub(crate) async fn api_forward(
    State(state): State<Arc<WebUiState>>,
    Path(uin): Path<u64>,
    headers: HeaderMap,
) -> Response {
    if let Some(reject) = guard_uin(&state, &headers, uin) {
        return reject;
    }
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": "Invalid API name" })),
    )
        .into_response()
}
