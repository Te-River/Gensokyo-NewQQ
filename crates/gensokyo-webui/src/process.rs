//! 进程面与日志：Go `webui/api.go:208-228` 的 `/{uin}/process*`、`/api/logs`、`/{uin}/api`。
//!
//! Rust 版**没有**进程守护（一个进程就是一个实例，重启/停止靠外部 supervisor），因此：
//!
//! - `GET /api/{uin}/process/status` 回 Go 的完整形状；`details` 复用
//!   [`SystemStatus`](crate::status::SystemStatus) 的进程投影（pid 是真实进程号，内存/CPU
//!   尚未采集故为 0），`total_logs` / `restarts` 没有对应计数器，如实给 0；
//! - `DELETE /api/{uin}/process` 在 Go 里是 `os.Exit(0)`。这里**不**自杀（会把共享的 runtime
//!   连同未落盘的队列一起掀掉，且没有优雅关闭入口），回 501 并说明；
//! - `GET /api/{uin}/process/logs` 与 `GET /api/logs` 回 `[]`——Go 的 `getProcessLogs` 本身
//!   就是 `c.JSON(200, []interface{}{})`（源码里标着"待完善：从 mylog 通道取日志"）。
//!   当前 Rust 树里也没有日志环形缓冲/广播，补历史与 WebSocket 推送要先在 bin 的 tracing
//!   subscriber 上挂一层采集，那属于 `gensokyo-bin` 的改动，不在本 crate 可改范围内。
//! - `POST /api/{uin}/api?name=...`：Go 的 switch 只有 `default: 400 Invalid API name`，
//!   没有任何真实转发分支；这里逐字照搬（前端也不依赖它）。

use std::sync::Arc;

use axum::Json;
use axum::extract::Path;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::json;

use crate::auth::{guard_uin, is_authenticated, unauthorized};
use crate::state::WebUiState;
use crate::status::{RunningProcessDetail, SystemStatus};

/// `GET /api/{uin}/process/status` 的响应体（Go `HandleProcessStatusRequest` 逐字段）。
#[derive(Debug, Serialize)]
pub struct ProcessInfo {
    /// 进程状态；Go 固定 `running`。
    pub status: &'static str,
    /// 日志条数；没有日志环形缓冲，恒为 0。
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

/// `GET /api/{uin}/process/logs` 与 `GET /api/logs`：Go `getProcessLogs` 回空数组。
///
/// 两条路径共用一个处理函数：Go 的中间件对 `/api/*/process/logs` 也不校验 uin，这里照旧。
pub(crate) async fn logs(State(state): State<Arc<WebUiState>>, headers: HeaderMap) -> Response {
    if !is_authenticated(&state, &headers) {
        return unauthorized();
    }
    Json(Vec::<String>::new()).into_response()
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
