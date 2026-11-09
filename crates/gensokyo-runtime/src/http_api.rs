//! 正向 HTTP API（Go `httpapi/httpapi.go` 的 `NoRoute` 入口 + `main.go` 的 `hr` 服务）。
//!
//! - `POST` 任意路径，body 是一个 OneBot action 请求 JSON，响应即 [`gensokyo_protocol::ActionResponse`]；
//! - `GET /health` 返回健康检查文本，**不鉴权**（只含服务名，无敏感信息，方便探活）；
//! - `GET /metrics` 返回 Prometheus 文本（`text/plain; version=0.0.4`），**不鉴权**（同 Go
//!   `main.go:550` 挂在 HTTP API listener 上的那份）；
//! - `GET /` 未挂 WebUI 时同样回健康检查文本；挂 WebUI 后 `/` 归前端首页，探活固定走
//!   `/health`（见下文「挂 WebUI 后的优先级」）；
//! - `http_access_token` 非空时，`Authorization: Bearer <token>` 或 `?access_token=` 必须匹配，
//!   否则 **403** `{"error":"鉴权失败"}`（Go `httpapi/httpapi.go:23-31` 从不回 401）；
//! - 非 `POST` 的 action 请求回 405。
//!
//! 与 Go 的差异：Go 在 `NoRoute` 里按路径分派 `/send_group_msg` 等专用 handler，并支持 GET
//! 查询参数；M1h-a 只保留「body 即 action JSON」这一条统一入口，专用路由随 M1i 的 action
//! 实现再补。
//!
//! # 挂 WebUI 后的优先级（[`with_webui_router`]，M3 起）
//!
//! WebUI 接管根路径与 `/api`，action 不能再做「兜底回退」，否则任何 `GET` 都会落到 action
//! 的 405。改由 [`action_gate`] 这层中间件先判优先级：
//!
//! 1. `POST` 且路径不在 `/api`、`/api/**` 下 → action（保持「`POST` 任意路径即 action」）；
//! 2. 其余一律交给 WebUI：`GET /` 静态页、`GET /api/*` 查询、`POST /api/login|logout` 登录，
//!    WebUI 自己的静态回退只认 `GET`/`HEAD`，未注册的 `/api` 回 404。
//!
//! 也就是说 `/api` 是 WebUI 的保留前缀：`POST /api/login` 是登录而不是同名 action。
//! 未挂 WebUI（`disable_webui=true`）时仍是 M1h-a 的路由表，`POST /api/**` 照旧走 action。

use std::collections::HashMap;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::json;

use crate::auth::is_authorized;
use crate::dispatcher::dispatch_json;
use crate::state::AppState;

/// action 请求体上限，与 `Bytes` 提取器（axum `DEFAULT_LIMIT`）的 2 MiB 对齐。
///
/// action 门自己要读 body，不再经过提取器，故显式带上同一上限。
const MAX_ACTION_BODY: usize = 2 * 1024 * 1024;

/// 健康检查正文；`/health` 与未挂 WebUI 时的 `/` 共用。
const HEALTH_BODY: &str = "gensokyo-runtime: ok";

/// Prometheus 文本格式的 Content-Type（与 Go `MetricsHandler` 显式设置的一致）。
const METRICS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// 独立健康检查路由：`GET /health` → 200 `text/plain`，不鉴权、不进 action 分派。
///
/// 挂 WebUI 后 `/` 被前端首页占用，探活只能固定走这条路径。WebUI 的静态回退只处理
/// **没有匹配到路由**的请求，所以 `/health` 不会被首页兜底吞掉（与注册顺序无关：
/// axum 先查路由表，全部未命中才进 fallback）。
pub(crate) fn health_router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route("/health", get(health))
}

/// Prometheus 指标路由：`GET /metrics` → 200 `text/plain; version=0.0.4`，不鉴权。
///
/// 对应 Go `main.go:550` 挂在 HTTP API listener 上的那份（不带 uploadAuth）。Go 还在主服务器
/// `main.go:522` 挂了一份带 uploadAuth 的；Rust 只在 HTTP API 端口提供——主 listener 缺
/// uploadAuth 的等价中间件，公网暴露消息量不合适，等该中间件落地再补。
pub(crate) fn metrics_router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route("/metrics", get(metrics))
}

/// 正向 HTTP API 路由；`http_address` 的判定在 [`crate::boot::Runtime`]。
pub(crate) fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(health).post(action))
        .merge(health_router())
        .merge(metrics_router())
        .fallback(action)
        .with_state(state)
}

/// 把 WebUI 路由挂到 HTTP API 端口：`POST`（非 `/api`）仍走 action，其余进 WebUI。
///
/// 中间件包住路由与回退（axum 的 `Router::layer` 同时作用于两者），所以门能拦住本该由
/// WebUI 静态回退处理的 `POST`——那是 action 的唯一去处。后 merge 进来的 webhook 路由不会
/// 被这层包住，`POST webhook_path` 仍是回调（见 [`crate::boot`]）。
pub(crate) fn with_webui_router(webui: Router, state: AppState) -> Router {
    webui
        .merge(health_router())
        .merge(metrics_router())
        .layer(middleware::from_fn_with_state(state, action_gate))
}

async fn health() -> &'static str {
    HEALTH_BODY
}

/// Prometheus 文本；指标集合与数值格式见 [`gensokyo_core::telemetry::metrics`]。
async fn metrics() -> Response {
    (
        [(header::CONTENT_TYPE, METRICS_CONTENT_TYPE)],
        gensokyo_core::telemetry::metrics::render_prometheus_text(),
    )
        .into_response()
}

async fn action(
    State(state): State<AppState>,
    method: Method,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if method != Method::POST {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            axum::Json(json!({ "error": "只接受 POST" })),
        )
            .into_response();
    }

    dispatch_action(&state, &params, &headers, body).await
}

/// action 的统一处理：鉴权 → dispatch → JSON 响应。
///
/// `fallback`（未挂 WebUI）与 [`action_gate`]（挂了 WebUI）共用这一条实现，避免两套鉴权
/// 与错误码。
async fn dispatch_action(
    state: &AppState,
    params: &HashMap<String, String>,
    headers: &HeaderMap,
    body: Bytes,
) -> Response {
    let expected_token = state.config().snapshot().get_access_token().to_owned();
    if !is_authorized(
        &expected_token,
        headers,
        params.get("access_token").map(String::as_str),
    ) {
        tracing::debug!("HTTP API 鉴权失败");
        return auth_failed();
    }

    let response = dispatch_json(state.dispatcher(), body.to_vec()).await;
    ([(header::CONTENT_TYPE, "application/json")], response).into_response()
}

/// Go `httpapi/httpapi.go:23-31`：缺 token 与 token 错都是 403，从不回 401。
fn auth_failed() -> Response {
    (
        StatusCode::FORBIDDEN,
        axum::Json(json!({ "error": "鉴权失败" })),
    )
        .into_response()
}

/// 优先级判据：`POST` 且不落在 WebUI 的 `/api` 前缀下才是 action。
async fn action_gate(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if request.method() != Method::POST || is_webui_path(request.uri().path()) {
        return next.run(request).await;
    }

    let (parts, body) = request.into_parts();
    // 查询参数仍要参与鉴权：`POST /get_status?access_token=...` 与未挂 WebUI 时行为一致。
    let params = Query::<HashMap<String, String>>::try_from_uri(&parts.uri)
        .map(|Query(params)| params)
        .unwrap_or_default();
    let body = match axum::body::to_bytes(body, MAX_ACTION_BODY).await {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::debug!(error = %err, "HTTP API 读取请求体失败");
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({ "error": "读取请求体失败" })),
            )
                .into_response();
        }
    };

    dispatch_action(&state, &params, &parts.headers, body).await
}

/// `/api` 与 `/api/**` 归 WebUI（登录、登出、配置查询）。
fn is_webui_path(path: &str) -> bool {
    path == "/api" || path.starts_with("/api/")
}

#[cfg(test)]
mod tests {
    use super::is_webui_path;

    #[test]
    fn api_prefix_belongs_to_webui_only_as_a_path_segment() {
        assert!(is_webui_path("/api"));
        assert!(is_webui_path("/api/login"));
        assert!(is_webui_path("/api/config"));
        // 只是以 /api 开头的路径仍是 action 路径（`POST /apixxx` 不是 WebUI）。
        assert!(!is_webui_path("/apixxx"));
        assert!(!is_webui_path("/get_status"));
        assert!(!is_webui_path("/"));
    }
}
