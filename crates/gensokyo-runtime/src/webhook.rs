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
//! # union_webhook 扇出（Go `UnionFanout`，`main.go:1048-1100`）
//!
//! `union_webhook` 非空且不等于 `"0"` 时，每条回调在**本地处理之前**异步原样转发一份到该
//! URL——调用点与 Go 的包装器一致：body 大小检查之后、本地验签之前（Go 里包装器先于
//! handler 运行，所以验签失败/JSON 非法的请求同样会被转发）。
//!
//! - 同一 `POST` + 原样 body + 复制后的请求头，剔除 Go `sensitiveHeaders` 里的六个敏感头
//!   （`authorization` / `cookie` / `set-cookie` / `x-token` / `x-signature` /
//!   `x-signature-256`，大小写不敏感），避免把凭据转发给第三方。
//! - 5s 超时（Go `unionWebhookHTTPClient`）；超时、连接失败、非 2xx 只记日志，**绝不**
//!   影响本地响应（Go 的 `_, _ = client.Do(req)` 同样丢弃结果）。
//! - 与 Go 的差异：`host` / `content-length` / `transfer-encoding` 不转发——Go 的
//!   `http.Transport` 也不从 Header 复制这三个（由请求对象重算），线上结果一致；
//!   非 UTF-8 的 header 值直接跳过（[`HttpRequest`] 的头值是 `String`）。
//! - 传输构建失败（二进制未安装 rustls CryptoProvider）只 warn 一次，转发静默不启动；
//!   生产路径由 `gensokyo-bin` 启动时安装 provider（与 post_url 上报同口径）。
//!
//! # 其他差异
//!
//! - Go 用 5000 条有界队列 + 处理协程「先 ACK 后异步处理」；这里在 handler 里同步转换
//!   并 `publish`（broadcast 不阻塞），没有队列堆积问题，也就不需要 5000 的缓冲。

use std::sync::Once;
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use gensokyo_config::Settings;
use gensokyo_qq::{HttpMethod, HttpRequest, HttpTransport, ReqwestTransport};
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

/// Go `sensitiveHeaders`（`main.go:1036-1043`）：转发时剔除，大小写不敏感。
const SENSITIVE_HEADERS: [&str; 6] = [
    "authorization",
    "cookie",
    "set-cookie",
    "x-token",
    "x-signature",
    "x-signature-256",
];

/// 由 HTTP 客户端按请求对象重算的头，不从转发的 Header 里复制（Go `http.Transport` 同）。
const OVERRIDE_HEADERS: [&str; 3] = ["host", "content-length", "transfer-encoding"];

/// Go `unionWebhookHTTPClient` 的 `Timeout`（`main.go:1045`）。
const UNION_FANOUT_TIMEOUT: Duration = Duration::from_secs(5);

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

    // Go 的包装器在本地 handler 之前转发：位置就是这里（大小检查之后、验签之前）。
    spawn_union_fanout(&state, &headers, &body);

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
            for mut event in events {
                crate::gateway::apply_op_userid_type(&mut event, settings.get_op_userid_type());
                state.downtime().publish(event);
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

/// Go `UnionFanout` 第 3 步（`main.go:1065-1095`）：`tokio::spawn` 里异步转发一份。
///
/// 配置为空 / `"0"` 时零开销返回；本地响应不等它，也不看它的结果。
fn spawn_union_fanout(state: &AppState, headers: &HeaderMap, body: &Bytes) {
    let settings = state.config().snapshot();
    let Some(target) = union_fanout_target(&settings).map(str::to_owned) else {
        return;
    };

    let request = fanout_request(&target, headers, body);
    tokio::spawn(async move {
        let transport = match ReqwestTransport::new() {
            Ok(transport) => transport,
            Err(err) => {
                // 每条回调都 warn 会刷屏（Go 的客户端是包级单例、构建期就成功）。
                static WARNED: Once = Once::new();
                WARNED.call_once(|| {
                    tracing::warn!(error = %err, "union_webhook 转发跳过：HTTP 传输构建失败");
                });
                return;
            }
        };
        forward(&transport, request).await;
    });
}

/// 转发目标：Go 的判定 `uw != "" && uw != "0"`（不做 trim，与 Go 一致）。
fn union_fanout_target(settings: &Settings) -> Option<&str> {
    let target = settings.get_union_webhook();
    if target.is_empty() || target == "0" {
        None
    } else {
        Some(target)
    }
}

/// 构造转发请求：同一个 `POST`、过滤后的请求头、原样 body。
fn fanout_request(target: &str, headers: &HeaderMap, body: &[u8]) -> HttpRequest {
    HttpRequest {
        method: HttpMethod::Post,
        url: target.to_owned(),
        headers: forward_headers(headers),
        body: body.to_vec(),
    }
}

/// 复制请求头并剔除敏感头（Go `main.go:1066-1074` 的 `Clone` + `delete`）。
fn forward_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter(|(name, _)| {
            !is_sensitive_header(name.as_str()) && !OVERRIDE_HEADERS.contains(&name.as_str())
        })
        .filter_map(|(name, value)| {
            Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned()))
        })
        .collect()
}

fn is_sensitive_header(name: &str) -> bool {
    SENSITIVE_HEADERS
        .iter()
        .any(|sensitive| name.eq_ignore_ascii_case(sensitive))
}

/// 发起转发；5s 超时，成败只记日志（Go `_, _ = client.Do(req)`）。
async fn forward<T>(transport: &T, request: HttpRequest)
where
    T: HttpTransport,
{
    match tokio::time::timeout(UNION_FANOUT_TIMEOUT, transport.request(request)).await {
        Ok(Ok(response)) => tracing::debug!(status = response.status, "union_webhook 转发完成"),
        Ok(Err(err)) => tracing::warn!(error = %err, "union_webhook 转发失败"),
        Err(_) => tracing::warn!(timeout = ?UNION_FANOUT_TIMEOUT, "union_webhook 转发超时"),
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use axum::http::{HeaderMap, HeaderValue};
    use gensokyo_config::Settings;
    use gensokyo_qq::{HttpMethod, HttpRequest, HttpResponse, HttpTransport};

    use super::{
        SENSITIVE_HEADERS, fanout_request, forward, forward_headers, is_sensitive_header,
        union_fanout_target,
    };

    /// 记录收到的请求并按 200 空 body 应答的假传输：不碰 TLS，也就不需要 rustls provider。
    #[derive(Debug, Default)]
    struct RecordingTransport {
        requests: Mutex<Vec<HttpRequest>>,
    }

    impl HttpTransport for RecordingTransport {
        async fn request(&self, request: HttpRequest) -> gensokyo_qq::Result<HttpResponse> {
            self.requests.lock().expect("测试互斥锁").push(request);
            Ok(HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: Vec::new(),
            })
        }
    }

    #[test]
    fn detects_sensitive_headers_case_insensitively() {
        for name in [
            "authorization",
            "Cookie",
            "SET-COOKIE",
            "X-Token",
            "x-Signature",
            "X-Signature-256",
        ] {
            assert!(is_sensitive_header(name), "{name} 应被判为敏感头");
        }
        // Go 的清单里没有 x-signature-ed25519：验签头照转。
        assert!(!is_sensitive_header("x-signature-ed25519"));
        assert!(!is_sensitive_header("content-type"));
    }

    #[test]
    fn forwards_ordinary_headers_only() {
        let mut headers = HeaderMap::new();
        for name in SENSITIVE_HEADERS {
            headers.insert(name, HeaderValue::from_static("secret"));
        }
        headers.insert("host", HeaderValue::from_static("127.0.0.1:5700"));
        headers.insert("content-length", HeaderValue::from_static("2"));
        headers.insert("transfer-encoding", HeaderValue::from_static("chunked"));
        headers.insert("content-type", HeaderValue::from_static("application/json"));
        headers.insert("x-signature-ed25519", HeaderValue::from_static("beef"));
        headers.insert("x-custom", HeaderValue::from_static("v"));

        let mut names: Vec<String> = forward_headers(&headers)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["content-type", "x-custom", "x-signature-ed25519"],
            "敏感头与客户端重算的头都应剔除，其余原样保留"
        );
    }

    #[test]
    fn union_webhook_gate_matches_go() {
        let mut settings = Settings::default();
        assert_eq!(union_fanout_target(&settings), None, "空串不转发");
        settings.union_webhook = "0".to_owned();
        assert_eq!(union_fanout_target(&settings), None, "\"0\" 不转发");
        settings.union_webhook = "https://union.example.com/hook".to_owned();
        assert_eq!(
            union_fanout_target(&settings),
            Some("https://union.example.com/hook")
        );
    }

    #[tokio::test]
    async fn forwards_post_with_filtered_headers_and_raw_body() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", HeaderValue::from_static("QQBot token"));
        headers.insert("content-type", HeaderValue::from_static("application/json"));
        headers.insert("content-length", HeaderValue::from_static("2"));

        let request = fanout_request("https://union.example.com/hook", &headers, b"{}");
        let transport = RecordingTransport::default();
        forward(&transport, request).await;

        let sent = transport.requests.lock().expect("测试互斥锁");
        assert_eq!(sent.len(), 1, "应恰好转发一次");
        assert_eq!(sent[0].method, HttpMethod::Post);
        assert_eq!(sent[0].url, "https://union.example.com/hook");
        assert_eq!(
            sent[0].headers,
            [("content-type".to_owned(), "application/json".to_owned())]
        );
        assert_eq!(sent[0].body, b"{}", "body 原样转发");
    }
}
