//! webhook 集成测试：HTTP POST → ed25519 验签（Go `server/webhook.go`）→ 入站转换 → 事件总线。

mod support;

use std::net::SocketAddr;
use std::time::Duration;

use gensokyo_protocol::Event;
use gensokyo_runtime::{Runtime, RuntimeOptions, WEBHOOK_ACK_BODY};
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

const SECRET: &str = "bot-secret";
const PATH: &str = "/onebot";
const TIMESTAMP: &str = "1700000000";

const SETTINGS: &str = r#"  app_id: 12345
  client_secret: bot-secret
  string_ob11: true
  native_ob11: true
  enable_ws_server: false
  http_address: "127.0.0.1:0"
  webhook_path: "onebot"
"#;

fn group_at_body() -> String {
    json!({
        "id": "EV1",
        "op": 0,
        "s": 1,
        "t": "GROUP_AT_MESSAGE_CREATE",
        "d": {
            "id": "M1",
            "group_id": "G1",
            "content": "hello",
            "timestamp": "1700000000",
            "author": {"id": "U1", "username": "alice"}
        }
    })
    .to_string()
}

/// 照抄 Go `server.InitPrivateKey`：`client_secret` 自我拼接到 ≥32 字节后取前 32 字节作 seed。
fn key_pair(secret: &str) -> Ed25519KeyPair {
    let mut seed = secret.as_bytes().to_vec();
    while seed.len() < 32 {
        seed = seed.iter().chain(seed.iter()).copied().collect();
    }
    seed.truncate(32);
    Ed25519KeyPair::from_seed_unchecked(&seed).expect("派生测试密钥失败")
}

/// `hex.EncodeToString` 的测试版：小写。
fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn from_hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("十六进制"))
        .collect()
}

/// 平台侧签名：`ed25519_sign(timestamp ‖ body)`（Go `validateSignatureBody`）。
fn sign(secret: &str, timestamp: &str, body: &str) -> String {
    let mut message = Vec::new();
    message.extend_from_slice(timestamp.as_bytes());
    message.extend_from_slice(body.as_bytes());
    to_hex(key_pair(secret).sign(&message).as_ref())
}

/// 带（可缺省的）签名头的裸 HTTP `POST`；`None` 表示不带该 header。
async fn post_raw(
    addr: SocketAddr,
    signature: Option<&str>,
    timestamp: Option<&str>,
    body: &str,
) -> (u16, String) {
    let mut headers = String::new();
    if let Some(signature) = signature {
        headers.push_str(&format!("X-Signature-Ed25519: {signature}\r\n"));
    }
    if let Some(timestamp) = timestamp {
        headers.push_str(&format!("X-Signature-Timestamp: {timestamp}\r\n"));
    }

    let mut stream = TcpStream::connect(addr).await.expect("连接测试服务失败");
    let request = format!(
        "POST {PATH} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("写请求失败");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.expect("读响应失败");
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_default();
    (status, text)
}

/// 正常路径：按固定时间戳签好名再发。
async fn post_signed(addr: SocketAddr, body: &str) -> (u16, String) {
    let signature = sign(SECRET, TIMESTAMP, body);
    post_raw(addr, Some(&signature), Some(TIMESTAMP), body).await
}

#[tokio::test]
async fn webhook_post_publishes_onebot_event() {
    let (state, dir) = support::state(SETTINGS);
    let state = support::with_temp_stores(state, &dir);
    let mut events = state.hub().subscribe();
    let handle = Runtime::start(state).await.expect("启动运行时失败");
    let addr = handle.http_addr().expect("HTTP API 未启动");

    let (status, response) = post_signed(addr, &group_at_body()).await;
    assert_eq!(status, 200, "签名正确的 webhook 应回 200：{response}");
    assert_eq!(support::response_body(&response), WEBHOOK_ACK_BODY);

    let event = timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("订阅者未收到事件")
        .expect("事件总线已关闭");
    match event.as_ref() {
        Event::Message(message) => {
            assert_eq!(message.raw_message, "hello");
            assert_eq!(message.text(), "hello");
            assert_eq!(
                message.group_id.as_ref().map(ToString::to_string),
                Some("G1".to_owned())
            );
            assert_eq!(message.user_id.to_string(), "U1");
        }
        other => panic!("应为 message 事件，实际 {other:?}"),
    }

    handle.shutdown_with_timeout(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn webhook_rejects_bad_signatures() {
    let (state, dir) = support::state(SETTINGS);
    let state = support::with_temp_stores(state, &dir);
    let mut events = state.hub().subscribe();
    let handle = Runtime::start(state).await.expect("启动运行时失败");
    let addr = handle.http_addr().expect("HTTP API 未启动");

    let body = group_at_body();
    let signature = sign(SECRET, TIMESTAMP, &body);
    let flipped = format!(
        "{}{}",
        if signature.starts_with('0') { "1" } else { "0" },
        &signature[1..]
    );

    let cases: [(&str, Option<&str>, Option<&str>, String); 7] = [
        (
            "篡改 body",
            Some(signature.as_str()),
            Some(TIMESTAMP),
            body.replace("hello", "hell0"),
        ),
        (
            "篡改时间戳",
            Some(signature.as_str()),
            Some("1700000001"),
            body.clone(),
        ),
        (
            "篡改签名",
            Some(flipped.as_str()),
            Some(TIMESTAMP),
            body.clone(),
        ),
        ("签名非十六进制", Some("zzzz"), Some(TIMESTAMP), body.clone()),
        ("签名长度不足", Some("deadbeef"), Some(TIMESTAMP), body.clone()),
        ("缺签名 header", None, Some(TIMESTAMP), body.clone()),
        ("缺时间戳 header", Some(signature.as_str()), None, body.clone()),
    ];

    for (name, candidate, timestamp, payload) in cases {
        let (status, response) = post_raw(addr, candidate, timestamp, &payload).await;
        assert_eq!(status, 401, "{name} 应回 401：{response}");
        assert!(
            support::response_body(&response).contains("Invalid signature"),
            "{name} 的响应体应照 Go 回 Invalid signature：{response}"
        );
    }

    assert!(
        timeout(Duration::from_millis(300), events.recv())
            .await
            .is_err(),
        "验签失败的请求不应进入总线"
    );

    handle.shutdown_with_timeout(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn webhook_verifies_signature_before_parsing_body() {
    let (state, dir) = support::state(SETTINGS);
    let state = support::with_temp_stores(state, &dir);
    let mut events = state.hub().subscribe();
    let handle = Runtime::start(state).await.expect("启动运行时失败");
    let addr = handle.http_addr().expect("HTTP API 未启动");

    // Go 在验签之后才 json.Unmarshal：无签名的坏 JSON 是 401，不是 400。
    let (status, _response) = post_raw(addr, None, None, "not json").await;
    assert_eq!(status, 401, "验签先于 JSON 解析");

    let (status, _response) = post_signed(addr, "not json").await;
    assert_eq!(status, 400, "签名正确但 JSON 非法应回 400");

    // 未知事件仍应 ACK，且不发布任何事件。
    let (status, response) = post_signed(addr, r#"{"t":"GUILD_CREATE","d":{}}"#).await;
    assert_eq!(status, 200, "未知事件仍应 ACK：{response}");
    assert_eq!(support::response_body(&response), WEBHOOK_ACK_BODY);

    assert!(
        timeout(Duration::from_millis(300), events.recv())
            .await
            .is_err(),
        "非法 / 未知事件不应进入总线"
    );

    handle.shutdown_with_timeout(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn webhook_answers_op13_callback_validation() {
    let (state, dir) = support::state(SETTINGS);
    let state = support::with_temp_stores(state, &dir);
    let mut events = state.hub().subscribe();
    let handle = Runtime::start(state).await.expect("启动运行时失败");
    let addr = handle.http_addr().expect("HTTP API 未启动");

    let body = json!({
        "op": 13,
        "d": {"plain_token": "Arq0D5A61EgUu4OxUvOp", "event_ts": "1725442341"}
    })
    .to_string();

    // op=13 也要求合法签名（Go 在按 op 分发前就验签）。
    let (status, _response) = post_raw(addr, None, None, &body).await;
    assert_eq!(status, 401, "op=13 同样要求签名");

    let (status, response) = post_signed(addr, &body).await;
    assert_eq!(status, 200, "op=13 应回 200：{response}");
    let reply: Value =
        serde_json::from_str(support::response_body(&response)).expect("op=13 应答应为 JSON");
    assert_eq!(reply["plain_token"], "Arq0D5A61EgUu4OxUvOp");

    // 应答签名是 ed25519_sign(event_ts ‖ plain_token)（event_ts 在前）。
    let signature = reply["signature"].as_str().expect("缺少 signature");
    let mut message = Vec::new();
    message.extend_from_slice(b"1725442341");
    message.extend_from_slice(b"Arq0D5A61EgUu4OxUvOp");
    UnparsedPublicKey::new(&ED25519, key_pair(SECRET).public_key().as_ref())
        .verify(&message, &from_hex(signature))
        .expect("op=13 应答签名应能用派生公钥验过");

    // 校验请求本身不是事件，不该进总线。
    assert!(
        timeout(Duration::from_millis(300), events.recv())
            .await
            .is_err(),
        "op=13 不应发布事件"
    );

    handle.shutdown_with_timeout(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn webhook_is_also_mounted_on_forward_ws_port() {
    // Go 把回调挂在主服务器端口（与正向 WS 同一 router）：这里验证 merge 到 WS listener。
    let settings = SETTINGS
        .replace("enable_ws_server: false", "enable_ws_server: true")
        .replace("http_address: \"127.0.0.1:0\"", "http_address: \"\"");
    let (state, dir) = support::state(&settings);
    let state = support::with_temp_stores(state, &dir);
    let mut events = state.hub().subscribe();
    let handle = Runtime::start_with(
        state,
        RuntimeOptions {
            ws_bind: Some("127.0.0.1:0".to_owned()),
            http_bind: None,
            config_watch: false,
            gateway: false,
            post_url: false,
        },
    )
    .await
    .expect("启动运行时失败");
    let addr = handle.ws_addr().expect("正向 WS 未启动");

    let (status, response) = post_signed(addr, &group_at_body()).await;
    assert_eq!(status, 200, "WS 端口上的 webhook 应回 200：{response}");
    assert_eq!(support::response_body(&response), WEBHOOK_ACK_BODY);

    let event = timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("订阅者未收到事件")
        .expect("事件总线已关闭");
    assert_eq!(event.post_type(), "message");

    handle.shutdown_with_timeout(Duration::from_secs(2)).await;
}
