//! webhook 集成测试：HTTP POST → 入站转换 → 事件总线。

mod support;

use std::time::Duration;

use gensokyo_protocol::Event;
use gensokyo_runtime::{Runtime, RuntimeOptions, WEBHOOK_ACK_BODY};
use serde_json::json;
use tokio::time::timeout;

const SETTINGS: &str = r#"  app_id: 12345
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

#[tokio::test]
async fn webhook_post_publishes_onebot_event() {
    let (state, _dir) = support::state(SETTINGS);
    let mut events = state.hub().subscribe();
    let handle = Runtime::start(state).await.expect("启动运行时失败");
    let addr = handle.http_addr().expect("HTTP API 未启动");

    let (status, response) = support::post_json(addr, "/onebot", &group_at_body()).await;
    assert_eq!(status, 200, "webhook 应回 200：{response}");
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
async fn webhook_rejects_invalid_json_and_ignores_unknown_events() {
    let (state, _dir) = support::state(SETTINGS);
    let mut events = state.hub().subscribe();
    let handle = Runtime::start(state).await.expect("启动运行时失败");
    let addr = handle.http_addr().expect("HTTP API 未启动");

    let (status, _response) = support::post_json(addr, "/onebot", "not json").await;
    assert_eq!(status, 400, "非法 JSON 应回 400");

    let (status, response) =
        support::post_json(addr, "/onebot", r#"{"t":"GUILD_CREATE","d":{}}"#).await;
    assert_eq!(status, 200, "未知事件仍应 ACK");
    assert_eq!(support::response_body(&response), WEBHOOK_ACK_BODY);

    // 两次请求都不该发布任何事件。
    assert!(
        timeout(Duration::from_millis(300), events.recv())
            .await
            .is_err(),
        "非法 / 未知事件不应进入总线"
    );

    handle.shutdown_with_timeout(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn webhook_is_also_mounted_on_forward_ws_port() {
    // Go 把回调挂在主服务器端口（与正向 WS 同一 router）：这里验证 merge 到 WS listener。
    let settings = SETTINGS
        .replace("enable_ws_server: false", "enable_ws_server: true")
        .replace("http_address: \"127.0.0.1:0\"", "http_address: \"\"");
    let (state, _dir) = support::state(&settings);
    let mut events = state.hub().subscribe();
    let handle = Runtime::start_with(
        state,
        RuntimeOptions {
            ws_bind: Some("127.0.0.1:0".to_owned()),
            http_bind: None,
        },
    )
    .await
    .expect("启动运行时失败");
    let addr = handle.ws_addr().expect("正向 WS 未启动");

    let (status, response) = support::post_json(addr, "/onebot", &group_at_body()).await;
    assert_eq!(status, 200, "WS 端口上的 webhook 应回 200：{response}");
    assert_eq!(support::response_body(&response), WEBHOOK_ACK_BODY);

    let event = timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("订阅者未收到事件")
        .expect("事件总线已关闭");
    assert_eq!(event.post_type(), "message");

    handle.shutdown_with_timeout(Duration::from_secs(2)).await;
}
