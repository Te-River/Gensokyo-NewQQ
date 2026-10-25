//! 反向 HTTP 回调（Go `server/webhook.go` 的 `CreateHandleValidationSafe`）。
//!
//! `webhook_path` 非空时，在监听端口上注册一条 `POST` 路由：
//! body 是 QQ 机器人平台的回调信封 `{ "id", "op", "s", "t", "d" }`，转成 OneBot 事件
//! 后发布到 [`crate::event_hub::EventHub`]，并回 `{"op":12}` ACK（Go 同款）。
//!
//! 路由挂载位置：Go 把回调挂在主服务器端口（与正向 WS 同一个 gin router）上；Rust 侧
//! 由 [`crate::boot::Runtime`] 把它 merge 进正向 WS 与 HTTP API 两个 listener 的路由表
//! （哪个存在就挂哪个，两个都存在就都挂），因为 M1h 还没有「主服务器端口」这一个
//! 统一监听。
//!
//! # 鉴权（实读 Go 后的结论）
//!
//! - Go 的 `CreateHandleValidationSafe` **每条**请求都校验 `X-Signature-Ed25519` +
//!   `X-Signature-Timestamp`，公钥来自 `client_secret` 派生的 ed25519 密钥对
//!   （`server.InitPrivateKey`）；缺签名直接 401。
//! - `webhook_prefix_ip` 在 Go 里只有 `structs.Settings` 字段，**没有任何代码读它**
//!   （全仓 grep 仅命中 `structs/structs.go:81`），所以这里也不做 IP 白名单——不发明
//!   一套 Go 没有的策略。
//!
//! 因此**签名校验在本里程碑未实现**（需要 ed25519，workspace 依赖里没有）。当前行为：
//! 配置了 `client_secret` 时启动/首个回调会打一条 `warn` 提示该缺口，请求仍按 ACK 处理。
//! 生产暴露前必须补上，切勿把该端口放到不可信网络。
//!
//! # 其他差异
//!
//! - Go 用 5000 条有界队列 + 处理协程「先 ACK 后异步处理」；这里在 handler 里同步转换
//!   并 `publish`（broadcast 不阻塞），没有队列堆积问题，也就不需要 5000 的缓冲。
//! - `op = 13`（平台回调地址校验）需要 ed25519 签名应答，当前回 501 并记 warn；Go 回
//!   `{plain_token, signature}`。
//! - `union_webhook` 扇出（Go `UnionFanout`）未实现。

use std::sync::Once;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde_json::{Value, json};

use crate::inbound::Inbound;
use crate::state::AppState;

/// 与 Go `server.MaxWebhookBodyBytes` 一致：4 MiB。
pub const MAX_BODY_BYTES: usize = 4 << 20;

/// 回调 ACK 体（Go 回 `{"op":12}`）。
pub const ACK_BODY: &str = r#"{"op":12}"#;

/// webhook 路由；`webhook_path` 的判定与归一化在 [`crate::boot::Runtime`]。
pub(crate) fn router(state: AppState, path: &str) -> Router {
    Router::new().route(path, post(handle)).with_state(state)
}

async fn handle(State(state): State<AppState>, body: Bytes) -> Response {
    if body.len() > MAX_BODY_BYTES {
        tracing::warn!(bytes = body.len(), "webhook 请求体超过上限");
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            axum::Json(json!({ "error": "请求体过大" })),
        )
            .into_response();
    }

    let Ok(envelope) = serde_json::from_slice::<Value>(&body) else {
        tracing::warn!("webhook 请求体不是合法 JSON");
        return bad_request("事件 JSON 非法");
    };

    if envelope.get("op").and_then(Value::as_i64) == Some(13) {
        warn_signature_gap();
        tracing::warn!("收到 op=13 的回调地址校验请求，ed25519 签名应答未实现");
        return (
            StatusCode::NOT_IMPLEMENTED,
            axum::Json(json!({ "error": "ed25519 回调校验未实现" })),
        )
            .into_response();
    }

    let settings = state.config().snapshot();
    if !settings.get_client_secret().trim().is_empty() {
        warn_signature_gap();
    }

    let inbound = Inbound::from_state(&state);
    match inbound.convert_json(&body) {
        Ok(events) => {
            let published = events.len();
            for event in events {
                state.hub().publish(event);
            }
            tracing::debug!(published, "webhook 事件已发布");
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                ACK_BODY,
            )
                .into_response()
        }
        Err(err) => {
            tracing::warn!(error = %err, "webhook JSON 解析失败");
            bad_request("事件 JSON 非法")
        }
    }
}

fn bad_request(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(json!({ "error": message })),
    )
        .into_response()
}

/// 缺口只提示一次，避免每条回调刷一条 warn。
fn warn_signature_gap() {
    static WARNED: Once = Once::new();
    WARNED.call_once(|| {
        tracing::warn!(
            "webhook 的 X-Signature-Ed25519 校验尚未实现（M1h-b 无 ed25519 依赖），请勿把回调端口暴露到不可信网络"
        );
    });
}
