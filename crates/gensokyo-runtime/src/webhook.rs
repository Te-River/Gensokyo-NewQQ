//! 反向 HTTP 回调（Go `server/webhook.go` 的 `CreateHandleValidationSafe`）。
//!
//! `webhook_path` 非空时，在监听端口上注册一条 `POST` 路由：
//! body 是 QQ 机器人平台的回调信封 `{ "id", "op", "s", "t", "d" }`，先验签、再转成 OneBot
//! 事件发布到 [`crate::event_hub::EventHub`]，并回 `{"op":12}` ACK（Go 同款）。
//!
//! 路由挂载位置：Go 把回调挂在主服务器端口（与正向 WS 同一个 gin router）上；Rust 侧
//! 由 [`crate::boot::Runtime`] 把它 merge 进正向 WS 与 HTTP API 两个 listener 的路由表
//! （哪个存在就挂哪个，两个都存在就都挂），因为 M1h 还没有「主服务器端口」这一个
//! 统一监听。
//!
//! # 鉴权（实读 Go 后的结论）
//!
//! Go 的 `CreateHandleValidationSafe` **每条**请求都校验 `X-Signature-Ed25519`（hex）+
//! `X-Signature-Timestamp`，待签串是 `timestamp ‖ body`；校验失败（含缺 header）回
//! `401 {"error":"Invalid signature"}`。Go 没有开关：`main.go:527` 无条件
//! `server.InitPrivateKey(conf.Settings.ClientSecret)`。这里同口径、始终校验。
//!
//! - 密钥派生照抄 `server.InitPrivateKey`：`seed = client_secret` 反复自我拼接到 ≥32 字节
//!   后取前 32 字节（**不是** sha256(client_secret)），再作为 ed25519 seed。
//! - Go 在启动时派生一次（热重载 `client_secret` 不会换钥）；这里按请求读取配置快照
//!   重新派生，因此热重载后立刻生效。等价于「Go 启动值 == 当前值」时的行为。
//! - `client_secret` 为空时 Go 的拼接循环永远长不到 32 字节（`strings.Repeat("", 2)` 仍是
//!   空串），启动即死循环；这里改为**全部 401** + 一次性 warn：同样「回调不可用」，但不挂死。
//! - `webhook_prefix_ip` 在 Go 里只有 `structs.Settings` 字段，**没有任何代码读它**
//!   （全仓 grep 仅命中 `structs/structs.go:81`），所以这里也不做 IP 白名单——不发明
//!   一套 Go 没有的策略。
//!
//! # op 13（回调地址校验）
//!
//! Go 先验签再按 op 分发，所以 op=13 同样要求合法签名。应答是
//! `{"plain_token": d.plain_token, "signature": hex(sign(d.event_ts ‖ d.plain_token))}`
//! （`webhook.go:110-121`，**event_ts 在前**，与请求验签的 `timestamp ‖ body` 不同）。
//!
//! # 其他差异
//!
//! - Go 用 5000 条有界队列 + 处理协程「先 ACK 后异步处理」；这里在 handler 里同步转换
//!   并 `publish`（broadcast 不阻塞），没有队列堆积问题，也就不需要 5000 的缓冲。
//! - `union_webhook` 扇出（Go `UnionFanout`）未实现。

use std::sync::Once;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde_json::{Value, json};

use crate::inbound::Inbound;
use crate::state::AppState;

/// 与 Go `server.MaxWebhookBodyBytes` 一致：4 MiB。
pub const MAX_BODY_BYTES: usize = 4 << 20;

/// 回调 ACK 体（Go 回 `{"op":12}`）。
pub const ACK_BODY: &str = r#"{"op":12}"#;

/// 签名头。Go 用 `Header.Get`（大小写不敏感），这里同样按小写名查。
const SIGNATURE_HEADER: &str = "x-signature-ed25519";
const TIMESTAMP_HEADER: &str = "x-signature-timestamp";

/// Go `ed25519.SeedSize`。
const SEED_LEN: usize = 32;
/// Go `ed25519.SignatureSize`。
const SIGNATURE_LEN: usize = 64;

/// webhook 路由；`webhook_path` 的判定与归一化在 [`crate::boot::Runtime`]。
pub(crate) fn router(state: AppState, path: &str) -> Router {
    Router::new().route(path, post(handle)).with_state(state)
}

async fn handle(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    if body.len() > MAX_BODY_BYTES {
        tracing::warn!(bytes = body.len(), "webhook 请求体超过上限");
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            axum::Json(json!({ "error": "请求体过大" })),
        )
            .into_response();
    }

    // 与 Go 同序：先验签，再解析 body（`webhook.go:94` 早于 `webhook.go:102`）。
    let settings = state.config().snapshot();
    let Some(keys) = derive_key_pair(settings.get_client_secret()) else {
        warn_missing_secret();
        return unauthorized();
    };
    if let Err(reason) = verify_signature(&keys, &headers, &body) {
        tracing::warn!(reason, "webhook 签名校验失败");
        return unauthorized();
    }

    let Ok(envelope) = serde_json::from_slice::<Value>(&body) else {
        tracing::warn!("webhook 请求体不是合法 JSON");
        return bad_request("事件 JSON 非法");
    };

    if envelope.get("op").and_then(Value::as_i64) == Some(13) {
        return (StatusCode::OK, axum::Json(validation_reply(&keys, &envelope))).into_response();
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

/// 照抄 Go `server.InitPrivateKey`：`client_secret` 反复自我拼接到 ≥32 字节、取前 32 字节
/// 作为 ed25519 seed（不是 sha256）。
fn derive_key_pair(secret: &str) -> Option<Ed25519KeyPair> {
    let mut seed = secret.as_bytes().to_vec();
    while seed.len() < SEED_LEN {
        // Go 的 `strings.Repeat(seed, 2)` 对空串无效（永远 0 字节 → 死循环）：这里放弃派生。
        if seed.is_empty() {
            return None;
        }
        seed = seed.iter().chain(seed.iter()).copied().collect();
    }
    seed.truncate(SEED_LEN);
    Ed25519KeyPair::from_seed_unchecked(&seed).ok()
}

/// Go `validateSignatureBody`（`webhook.go:163-196`）的同口径实现。
fn verify_signature(
    keys: &Ed25519KeyPair,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(), &'static str> {
    let signature = header_str(headers, SIGNATURE_HEADER).ok_or("missing X-Signature-Ed25519 header")?;
    let signature = decode_hex(signature).ok_or("invalid hex encoding in signature")?;
    if signature.len() != SIGNATURE_LEN || (signature[63] & 224) != 0 {
        return Err("invalid signature size or format");
    }

    let timestamp = header_str(headers, TIMESTAMP_HEADER).ok_or("missing X-Signature-Timestamp header")?;

    // 待签串：timestamp ‖ body（`webhook.go:186-188`）。
    let mut message = Vec::with_capacity(timestamp.len() + body.len());
    message.extend_from_slice(timestamp.as_bytes());
    message.extend_from_slice(body);

    UnparsedPublicKey::new(&ED25519, keys.public_key().as_ref())
        .verify(&message, &signature)
        .map_err(|_| "signature verification failed")
}

/// op=13 应答：对 `event_ts ‖ plain_token` 签名（`webhook.go:110-121`）。
fn validation_reply(keys: &Ed25519KeyPair, envelope: &Value) -> Value {
    let plain_token = d_field(envelope, "plain_token");
    let event_ts = d_field(envelope, "event_ts");

    let mut message = Vec::with_capacity(event_ts.len() + plain_token.len());
    message.extend_from_slice(event_ts.as_bytes());
    message.extend_from_slice(plain_token.as_bytes());

    json!({
        "plain_token": plain_token,
        "signature": encode_hex(keys.sign(&message).as_ref()),
    })
}

fn d_field<'a>(envelope: &'a Value, name: &str) -> &'a str {
    envelope
        .get("d")
        .and_then(|d| d.get(name))
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}

/// 与 Go 一致的 401 形状（`webhook.go:96`）。
fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(json!({ "error": "Invalid signature" })),
    )
        .into_response()
}

/// `client_secret` 为空只提示一次，避免每条回调刷一条 warn。
fn warn_missing_secret() {
    static WARNED: Once = Once::new();
    WARNED.call_once(|| {
        tracing::warn!("webhook 已启用但 client_secret 为空，无法验签，所有回调将返回 401");
    });
}

fn bad_request(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(json!({ "error": message })),
    )
        .into_response()
}

/// `hex.DecodeString`：奇数长度或非法字符一律失败。
fn decode_hex(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut index = 0;
    while index < bytes.len() {
        out.push((hex_digit(bytes[index])? << 4) | hex_digit(bytes[index + 1])?);
        index += 2;
    }
    Some(out)
}

/// `hex.EncodeToString`：小写。
fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
