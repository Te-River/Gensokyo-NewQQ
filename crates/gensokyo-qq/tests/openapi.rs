//! OpenAPI 客户端：URL / 方法 / 请求头 / 请求体 / 错误映射 / 富媒体上传。

mod support;

use std::sync::Arc;

use gensokyo_qq::{Error, FileUpload, HttpMethod, MessageToCreate, file_type};
use serde_json::{Value, json};
use support::{FakeTransport, ManualClock, ok_json, test_client, token_response};

const START: u64 = 1_700_000_000;

fn setup() -> (
    Arc<FakeTransport>,
    Arc<ManualClock>,
    gensokyo_qq::OpenApiClient<FakeTransport>,
) {
    let transport = Arc::new(FakeTransport::new());
    let clock = Arc::new(ManualClock::new(START));
    let client = test_client(&transport, &clock);
    (transport, clock, client)
}

fn body_of(request: &gensokyo_qq::HttpRequest) -> Value {
    serde_json::from_slice(&request.body).expect("请求体应是 JSON")
}

#[tokio::test]
async fn sends_text_group_message_with_auth_headers() {
    let (transport, _clock, client) = setup();
    transport.push_response(token_response("tok", 7200));
    transport.push_response(ok_json(
        r#"{"id":"m1","timestamp":"2026-10-22T10:00:00+08:00"}"#,
    ));

    let response = client
        .send_group_message("GRP123", &MessageToCreate::text("hello"))
        .await
        .unwrap();
    assert_eq!(response.id, "m1");
    assert_eq!(response.timestamp, "2026-10-22T10:00:00+08:00");

    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    let request = &requests[1];
    assert_eq!(request.method, HttpMethod::Post);
    assert_eq!(
        request.url,
        "https://api.bot.qq.com/v2/groups/GRP123/messages"
    );
    assert_eq!(request.header("Authorization"), Some("QQBot tok"));
    assert_eq!(request.header("X-Union-Appid"), Some("10001"));
    assert_eq!(request.header("Content-Type"), Some("application/json"));
    assert_eq!(body_of(request), json!({"content": "hello"}));
}

#[tokio::test]
async fn serializes_markdown_message() {
    let (transport, _clock, client) = setup();
    transport.push_response(token_response("tok", 7200));
    transport.push_response(ok_json(r#"{"id":"m2","timestamp":""}"#));

    client
        .send_c2c_message("USER1", &MessageToCreate::markdown("# 标题"))
        .await
        .unwrap();

    let requests = transport.requests();
    let request = &requests[1];
    assert_eq!(
        request.url,
        "https://api.bot.qq.com/v2/users/USER1/messages"
    );
    assert_eq!(
        body_of(request),
        json!({"msg_type": 2, "markdown": {"content": "# 标题"}})
    );
}

#[tokio::test]
async fn serializes_msg_id_and_seq_without_event_id() {
    let (transport, _clock, client) = setup();
    transport.push_response(token_response("tok", 7200));
    transport.push_response(ok_json(r#"{"id":"m3","timestamp":""}"#));

    client
        .send_group_message(
            "G",
            &MessageToCreate::text("reply")
                .with_msg_id("mid-1")
                .with_msg_seq(2),
        )
        .await
        .unwrap();

    let body = body_of(&transport.requests()[1]);
    assert_eq!(body["msg_id"], "mid-1");
    assert_eq!(body["msg_seq"], 2);
    assert!(body.get("event_id").is_none());
    assert!(body.get("is_wakeup").is_none());
}

#[tokio::test]
async fn rejects_conflicting_reply_modes_before_sending() {
    let (transport, _clock, client) = setup();
    let message = MessageToCreate::text("x")
        .with_msg_id("mid")
        .with_event_id("event");

    let error = client.send_group_message("G", &message).await.unwrap_err();
    assert!(
        matches!(error, Error::InvalidMessage(_)),
        "expected InvalidMessage, got {error:?}"
    );
    assert_eq!(transport.request_count(), 0, "校验失败不应发出任何请求");
}

#[tokio::test]
async fn url_encodes_openid_segments() {
    let (transport, _clock, client) = setup();
    transport.push_response(token_response("tok", 7200));
    transport.push_response(ok_json(r#"{"id":"m","timestamp":""}"#));

    client
        .send_group_message("g/1 2", &MessageToCreate::text("x"))
        .await
        .unwrap();

    assert_eq!(
        transport.requests()[1].url,
        "https://api.bot.qq.com/v2/groups/g%2F1%202/messages"
    );
}

#[tokio::test]
async fn maps_401_to_api_error_and_invalidates_token() {
    let (transport, _clock, client) = setup();
    transport.push_response(token_response("tok-1", 7200));
    transport.push_response(
        gensokyo_qq::HttpResponse::new(401)
            .with_header("X-Tps-trace-ID", "trace-1")
            .with_body(br#"{"code":11244,"message":"token invalid"}"#.to_vec()),
    );

    let error = client
        .send_group_message("G", &MessageToCreate::text("x"))
        .await
        .unwrap_err();
    let api = error.api().expect("应是 ApiError");
    assert_eq!(api.http_status, 401);
    assert_eq!(api.code, 11244);
    assert_eq!(api.message, "token invalid");
    assert_eq!(api.trace_id.as_deref(), Some("trace-1"));
    assert!(api.is_auth());

    // 401 已丢弃 token 缓存：下一次调用会先重新取 token。
    transport.push_response(token_response("tok-2", 7200));
    transport.push_response(ok_json(r#"{"id":"m","timestamp":""}"#));
    client
        .send_group_message("G", &MessageToCreate::text("x"))
        .await
        .unwrap();
    let requests = transport.requests();
    assert_eq!(requests.len(), 4);
    assert_eq!(
        requests[2].url,
        "https://api.bot.qq.com/app/getAppAccessToken"
    );
    assert_eq!(requests[3].header("Authorization"), Some("QQBot tok-2"));
}

#[tokio::test]
async fn maps_non_json_5xx_body_to_api_error() {
    let (transport, _clock, client) = setup();
    transport.push_response(token_response("tok", 7200));
    transport
        .push_response(gensokyo_qq::HttpResponse::new(503).with_body(b"upstream boom".to_vec()));

    let error = client
        .send_group_message("G", &MessageToCreate::text("x"))
        .await
        .unwrap_err();
    let api = error.api().expect("应是 ApiError");
    assert_eq!(api.http_status, 503);
    assert_eq!(api.code, 503);
    assert_eq!(api.message, "upstream boom");
    assert!(!api.is_auth());
}

#[tokio::test]
async fn maps_string_business_code_in_4xx() {
    let (transport, _clock, client) = setup();
    transport.push_response(token_response("tok", 7200));
    transport.push_response(
        gensokyo_qq::HttpResponse::new(400)
            .with_body(br#"{"code":"40034001","message":"invalid message"}"#.to_vec()),
    );

    let error = client
        .send_group_message("G", &MessageToCreate::text("x"))
        .await
        .unwrap_err();
    let api = error.api().expect("应是 ApiError");
    assert_eq!(api.http_status, 400);
    assert_eq!(api.code, 40_034_001);
    assert_eq!(api.message, "invalid message");
}

#[tokio::test]
async fn uploads_group_file_as_json() {
    let (transport, _clock, client) = setup();
    transport.push_response(token_response("tok", 7200));
    transport.push_response(ok_json(
        r#"{"file_uuid":"uuid","file_info":"fi","ttl":3600}"#,
    ));

    let file = FileUpload::from_url(file_type::IMAGE, "https://example.com/a.png");
    let response = client.upload_group_file("G", &file).await.unwrap();
    assert_eq!(response.file_uuid, "uuid");
    assert_eq!(response.file_info, "fi");
    assert_eq!(response.ttl, 3600);

    let request = &transport.requests()[1];
    assert_eq!(request.url, "https://api.bot.qq.com/v2/groups/G/files");
    assert_eq!(
        body_of(request),
        json!({"file_type": 1, "url": "https://example.com/a.png"})
    );
}

#[tokio::test]
async fn uploads_c2c_base64_file_with_name() {
    let (transport, _clock, client) = setup();
    transport.push_response(token_response("tok", 7200));
    transport.push_response(ok_json(
        r#"{"file_uuid":"u2","file_info":"fi2","ttl":7200}"#,
    ));

    let file = FileUpload::from_base64(file_type::FILE, "aGVsbG8=", "hello.txt");
    client.upload_c2c_file("U", &file).await.unwrap();

    let request = &transport.requests()[1];
    assert_eq!(request.url, "https://api.bot.qq.com/v2/users/U/files");
    assert_eq!(
        body_of(request),
        json!({
            "file_type": 4,
            "file_data": "aGVsbG8=",
            "file_name": "hello.txt",
        })
    );
}

#[tokio::test]
async fn rejects_file_with_both_url_and_data() {
    let (transport, _clock, client) = setup();
    let mut file = FileUpload::from_url(file_type::IMAGE, "https://example.com/a.png");
    file.file_data = Some("aGVsbG8=".to_owned());

    let error = client.upload_group_file("G", &file).await.unwrap_err();
    assert!(matches!(error, Error::InvalidMessage(_)), "{error:?}");
    assert_eq!(transport.request_count(), 0);
}
