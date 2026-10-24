//! 正向 HTTP API（Go `httpapi/httpapi.go` 的 `NoRoute` 入口 + `main.go` 的 `hr` 服务）。
//!
//! - `POST` 任意路径，body 是一个 OneBot action 请求 JSON，响应即 [`gensokyo_protocol::ActionResponse`]；
//! - `GET /` 返回健康检查文本，**不鉴权**（只含服务名，无敏感信息，方便探活）；
//! - `http_access_token` 非空时，`Authorization: Bearer <token>` 或 `?access_token=` 必须匹配，
//!   失败回 401；
//! - 非 `POST` 的 action 请求回 405。
//!
//! 与 Go 的差异：Go 在 `NoRoute` 里按路径分派 `/send_group_msg` 等专用 handler，并支持 GET
//! 查询参数；M1h-a 只保留「body 即 action JSON」这一条统一入口，专用路由随 M1i 的 action
//! 实现再补。

use std::collections::HashMap;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::json;

use crate::auth::{is_authorized, unauthorized};
use crate::dispatcher::dispatch_json;
use crate::state::AppState;

/// 正向 HTTP API 路由；`http_address` 的判定在 [`crate::boot::Runtime`]。
pub(crate) fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(health))
        .fallback(action)
        .with_state(state)
}

async fn health() -> &'static str {
    "gensokyo-runtime: ok"
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

    let expected_token = state.config().snapshot().get_access_token().to_owned();
    if !is_authorized(
        &expected_token,
        &headers,
        params.get("access_token").map(String::as_str),
    ) {
        tracing::debug!("HTTP API 鉴权失败");
        return unauthorized();
    }

    let response = dispatch_json(state.dispatcher(), body.to_vec()).await;
    ([(header::CONTENT_TYPE, "application/json")], response).into_response()
}
