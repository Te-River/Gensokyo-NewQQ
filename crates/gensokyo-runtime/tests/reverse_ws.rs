//! 反向 WS 客户端集成测试：假服务端、断线重连、握手头与 token。
//!
//! 全部本地回环，无外网。

mod support;

use std::time::{Duration, Instant};

use futures::StreamExt;
use gensokyo_core::Ob11Id;
use gensokyo_protocol::event::{MessageEvent, MessageType};
use gensokyo_protocol::{Event, MessageContent};
use gensokyo_runtime::ws_client::USER_AGENT;
use gensokyo_runtime::{WsClient, WsClientOptions};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message as TungMessage;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

fn settings_with(address: &str, extra: &str) -> String {
    format!(
        "  app_id: 12345\n  string_ob11: true\n  enable_ws_server: false\n  http_address: \"\"\n  ws_address: [\"{address}\"]\n  heart_beat_interval: 1\n  reconnect_times: 3\n{extra}"
    )
}

fn hub_event() -> Event {
    Event::Message(MessageEvent {
        time: 1_700_000_000,
        self_id: Ob11Id::num(12345),
        message_type: MessageType::Group,
        sub_type: Some("normal".to_owned()),
        message_id: Ob11Id::str("M1"),
        user_id: Ob11Id::str("U1"),
        group_id: Some(Ob11Id::str("G1")),
        message: MessageContent::Text("hub".to_owned()),
        raw_message: "hub".to_owned(),
        font: Some(0),
        sender: None,
        anonymous: None,
        message_seq: None,
        to_me: Some(true),
        extra: Default::default(),
    })
}

#[tokio::test]
async fn reverse_ws_receives_hub_events_and_heartbeats() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("绑定失败");
    let addr = listener.local_addr().expect("读取地址失败");
    let (ready_tx, ready_rx) = oneshot::channel();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept 失败");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("服务端握手失败");
        let mut seen = Vec::new();
        let mut ready = Some(ready_tx);
        while let Some(message) = socket.next().await {
            let Ok(TungMessage::Text(text)) = message else {
                break;
            };
            seen.push(serde_json::from_str::<Value>(&text).expect("收到非 JSON 帧"));
            if let Some(tx) = ready.take() {
                let _ = tx.send(());
            }
            if seen.len() == 3 {
                break;
            }
        }
        seen
    });

    let (state, _dir) = support::state(&settings_with(&format!("ws://{addr}/onebot/v11/ws"), ""));
    let client = WsClient::start(state.clone());

    timeout(Duration::from_secs(5), ready_rx)
        .await
        .expect("等待首帧超时")
        .expect("服务端未收到首帧");

    state.hub().publish(hub_event());

    let seen = timeout(Duration::from_secs(5), server)
        .await
        .expect("等待三帧超时")
        .expect("服务端任务 panic");

    assert_eq!(
        seen[0]["post_type"], "meta_event",
        "首帧应为 connect 元事件"
    );
    assert_eq!(seen[0]["meta_event_type"], "lifecycle");
    assert_eq!(seen[0]["sub_type"], "connect");
    assert_eq!(seen[0]["self_id"], 12345);
    assert!(
        seen.iter()
            .any(|frame| frame["post_type"] == "message" && frame["raw_message"] == "hub"),
        "hub 发布的事件应被写进连接：{seen:?}"
    );
    assert!(
        seen.iter()
            .any(|frame| frame["meta_event_type"] == "heartbeat"),
        "应按 heart_beat_interval=1s 发心跳：{seen:?}"
    );

    client.shutdown(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn reverse_ws_reconnects_within_two_seconds_after_server_disconnect() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("绑定失败");
    let addr = listener.local_addr().expect("读取地址失败");

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("首个 accept 失败");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("首次握手失败");
        let _connect = socket.next().await;
        socket.close(None).await.expect("发送关闭帧失败");
        drop(socket);

        let started = Instant::now();
        let (stream, _) = timeout(Duration::from_secs(5), listener.accept())
            .await
            .expect("等待重连超时")
            .expect("重连 accept 失败");
        let elapsed = started.elapsed();
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("二次握手失败");
        let first = socket.next().await;
        (elapsed, matches!(first, Some(Ok(TungMessage::Text(_)))))
    });

    let (state, _dir) = support::state(&settings_with(&format!("ws://{addr}/ws"), ""));
    let client = WsClient::start_with(
        state,
        WsClientOptions {
            heartbeat_interval: Some(Duration::from_secs(30)),
            ..WsClientOptions::default()
        },
    );

    let (elapsed, got_frame) = timeout(Duration::from_secs(10), server)
        .await
        .expect("等待重连结果超时")
        .expect("服务端任务 panic");

    assert!(got_frame, "重连后应立刻收到 connect 帧");
    assert!(
        elapsed < Duration::from_secs(2),
        "重连耗时 {elapsed:?} 应小于 2s"
    );

    client.shutdown(Duration::from_secs(2)).await;
}

#[tokio::test]
#[allow(clippy::result_large_err)] // tungstenite 回调固定返回 Result<Response, ErrorResponse>
async fn handshake_uses_go_headers_and_url_token_wins_over_config() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("绑定失败");
    let addr = listener.local_addr().expect("读取地址失败");
    let (headers_tx, headers_rx) = oneshot::channel();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept 失败");
        let callback = move |request: &Request, response: Response| {
            let read = |name: &str| {
                request
                    .headers()
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_owned()
            };
            let _ = headers_tx.send((
                read("authorization"),
                read("x-self-id"),
                read("user-agent"),
                read("x-client-role"),
            ));
            Ok(response)
        };
        let _socket = timeout(
            Duration::from_secs(5),
            tokio_tungstenite::accept_hdr_async(stream, callback),
        )
        .await
        .expect("握手超时")
        .expect("握手失败");
    });

    let (state, _dir) = support::state(&settings_with(
        &format!("ws://{addr}/ws?access_token=url-token"),
        "  ws_token: [\"config-token\"]\n",
    ));
    let client = WsClient::start_with(
        state,
        WsClientOptions {
            heartbeat_interval: Some(Duration::from_secs(30)),
            ..WsClientOptions::default()
        },
    );

    let (authorization, self_id, user_agent, role) = timeout(Duration::from_secs(5), headers_rx)
        .await
        .expect("等待握手头超时")
        .expect("服务端未拿到握手头");

    assert_eq!(
        authorization, "Token url-token",
        "URL access_token 应覆盖 ws_token"
    );
    assert_eq!(self_id, "12345");
    assert_eq!(user_agent, USER_AGENT);
    assert_eq!(role, "Universal");

    client.shutdown(Duration::from_secs(2)).await;
    let _ = server.await;
}
