//! 正向 WS 服务端（Go `server/wsserver.go`）。
//!
//! 与 Go 的差异（均为 M1h-a 有意收敛）：
//! - 连接建立后**不**发送 `meta_event/lifecycle/connect` 元事件——那属于 QQ 事件转换
//!   （M1h-b）；先发一条非响应帧会让 `get_status` 的首帧断言变复杂。
//! - 不维护 `WsServerClients` 共享列表：每个连接一个 axum 任务，断开即随任务结束清理
//!   （Go 的列表只服务于主动推送，M1h-a 还没有推送路径）。
//!
//! 鉴权与 `Origin` 校验都在升级前完成：鉴权失败回 401、`Origin` 非法回 403，握手都不会
//! 升级为 WebSocket。`Origin` 规则对齐 Go 的 `upgrader.CheckOrigin`：没有 `Origin` 头时
//! 放行（非浏览器客户端），否则只允许本地回环与 `server_dir` 白名单；**该校验与 token
//! 是否配置无关，token 为空时同样生效**。

use std::collections::HashMap;

use axum::Json;
use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use gensokyo_core::{Error, Result};
use serde_json::json;

use crate::auth::{is_authorized, unauthorized};
use crate::dispatcher::{Dispatcher, dispatch_json};
use crate::state::AppState;

/// 正向 WS 路由；`enable_ws_server` 的判定与 `path` 的校验都在 [`crate::boot::Runtime`]。
pub(crate) fn router(state: AppState, path: &str) -> Router {
    Router::new().route(path, get(ws_handler)).with_state(state)
}

/// `ws_server_path` → axum 路由路径。
///
/// Go 里空串与 `nil` 都注册到根路径（`main.go` 的 `r.GET("")` 分支），这里统一成 `/`；
/// 其余值补前导 `/`、去尾随 `/`，避免 `/ws` 与 `/ws/` 注册成两条路由。
///
/// # Errors
/// 含 axum 保留字符（`{` `}` `*`）、空段（`//`）、空白或非字母数字的段时返回
/// [`Error::Config`]：这些值会让 `Router::route` 在启动时 panic，提前在这里变成可诊断的
/// 启动错误。
pub(crate) fn route_path(raw: &str) -> Result<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw == "nil" {
        return Ok("/".to_owned());
    }
    let trimmed = raw.trim_matches('/');
    if trimmed.is_empty() {
        return Ok("/".to_owned());
    }
    if !trimmed.split('/').all(is_literal_segment) {
        return Err(Error::Config(format!(
            "ws_server_path 非法: {raw:?}（每段只允许字母数字与 - _ . ~ % ）"
        )));
    }
    Ok(format!("/{trimmed}"))
}

/// 路由段是否是可以按字面匹配的一段路径。
///
/// `{param}` / `*wildcard` 是 matchit 的占位符语法，配置里的字面路径不该被当成占位符，
/// 因此直接判为非法而不是交给 `Router::route` 去 panic。
fn is_literal_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '~' | '%'))
}

/// 对齐 Go `server/wsserver.go` 的 `upgrader.CheckOrigin`。
///
/// Go 的条件里 `contains(origin, server_dir)` 已经覆盖了后面两个 `http(s)://` 前缀分支，
/// 这里保留同样三条判断只是为了与 Go 逐条对齐，方便日后现场核对。
fn origin_allowed(origin: &str, server_dir: &str) -> bool {
    if origin.is_empty() {
        return true;
    }
    if origin.starts_with("http://127.0.0.1") || origin.starts_with("http://localhost") {
        return true;
    }
    let server_dir = server_dir.trim();
    !server_dir.is_empty()
        && (origin.contains(server_dir)
            || origin.starts_with(&format!("https://{server_dir}"))
            || origin.starts_with(&format!("http://{server_dir}")))
}

async fn ws_handler(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let settings = state.config().snapshot();
    if !is_authorized(
        settings.get_ws_server_token(),
        &headers,
        params.get("access_token").map(String::as_str),
    ) {
        let has_token =
            params.contains_key("access_token") || headers.contains_key("authorization");
        tracing::warn!(has_token, "正向 WS 鉴权失败，拒绝升级");
        return unauthorized();
    }

    // 非法字节的 Origin 不可能命中白名单（Go 侧同样匹配不到前缀），按拒绝处理。
    let origin_header = headers.get(header::ORIGIN);
    let origin_ok = match origin_header {
        None => true,
        Some(value) => value
            .to_str()
            .is_ok_and(|origin| origin_allowed(origin, settings.get_server_dir())),
    };
    if !origin_ok {
        tracing::warn!(origin = ?origin_header, "正向 WS Origin 校验失败，拒绝升级");
        return forbidden();
    }

    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// 403：`Origin` 不在白名单，握手不升级（对齐 Go 侧 gorilla 的拒绝行为）。
fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({ "error": "Origin 校验失败" })),
    )
        .into_response()
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    tracing::debug!("正向 WS 客户端已连接");
    let dispatcher: Dispatcher = state.dispatcher();

    loop {
        match socket.recv().await {
            Some(Ok(Message::Text(text))) => {
                let response =
                    dispatch_json(dispatcher.clone(), text.as_str().as_bytes().to_vec()).await;
                if socket.send(Message::Text(response.into())).await.is_err() {
                    tracing::debug!("正向 WS 回写失败，连接视为断开");
                    break;
                }
            }
            // OneBot V11 只用文本帧；二进制帧忽略，避免误把媒体载荷当 action。
            Some(Ok(Message::Binary(_))) => {
                tracing::debug!("忽略非文本帧");
            }
            Some(Ok(Message::Close(reason))) => {
                tracing::debug!(reason = ?reason, "客户端发送关闭帧");
                break;
            }
            // Ping / Pong 由 axum 自动应答。
            Some(Ok(_)) => {}
            Some(Err(err)) => {
                tracing::debug!(error = %err, "WS 读取出错");
                break;
            }
            None => break,
        }
    }
    tracing::debug!("正向 WS 客户端已断开");
}

#[cfg(test)]
mod tests {
    use super::{origin_allowed, route_path};

    #[test]
    fn empty_and_nil_paths_become_root() {
        assert_eq!(route_path("").expect("空路径合法"), "/");
        assert_eq!(route_path("   ").expect("空白路径合法"), "/");
        assert_eq!(route_path("nil").expect("nil 合法"), "/");
        assert_eq!(route_path("/").expect("根路径合法"), "/");
    }

    #[test]
    fn path_gets_leading_slash_and_loses_trailing_slash() {
        assert_eq!(route_path("ws").expect("合法"), "/ws");
        assert_eq!(route_path("/ws").expect("合法"), "/ws");
        assert_eq!(route_path("ws/").expect("合法"), "/ws");
        assert_eq!(route_path("/onebot/v11/").expect("合法"), "/onebot/v11");
    }

    #[test]
    fn placeholder_and_malformed_paths_are_rejected() {
        // 这些值会让 axum 的 `Router::route` 在启动时 panic，必须在启动期就变成 Err。
        assert!(route_path("ws/{id}").is_err());
        assert!(route_path("ws/*rest").is_err());
        assert!(route_path("/ws//x").is_err());
        assert!(route_path("ws x").is_err());
    }

    #[test]
    fn missing_or_loopback_origin_is_allowed() {
        assert!(origin_allowed("", ""));
        assert!(origin_allowed("http://127.0.0.1", ""));
        assert!(origin_allowed("http://127.0.0.1:5173", ""));
        assert!(origin_allowed("http://localhost:8080", ""));
    }

    #[test]
    fn foreign_origin_is_rejected_without_server_dir() {
        assert!(!origin_allowed("http://evil.example", ""));
        assert!(!origin_allowed("https://localhost.evil.example", ""));
    }

    #[test]
    fn server_dir_whitelists_extra_origins() {
        assert!(origin_allowed("https://bot.example.com", "bot.example.com"));
        assert!(origin_allowed(
            "http://bot.example.com:8443",
            "bot.example.com"
        ));
        assert!(!origin_allowed(
            "https://other.example.com",
            "bot.example.com"
        ));
        // 空白 server_dir 视为未配置，不放行任何外部 Origin。
        assert!(!origin_allowed("https://bot.example.com", "   "));
    }
}
