//! QQ 上游 WS 网关集成测试：本地 mock 网关（`tokio-tungstenite`）+ 假 HTTP 传输。
//!
//! 全部本地回环，不触网；每个用例 5s 兜底超时防 flake。

mod support;

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use gensokyo_qq::{
    HttpRequest, HttpResponse, HttpTransport, Result as QqResult, TokenConfig, TokenManager,
};
use gensokyo_runtime::gateway::{GatewayClient, GatewayOptions};
use gensokyo_runtime::{AppState, GatewayHandle, Runtime, RuntimeHandle, RuntimeOptions};
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message as TungMessage;
use tokio_tungstenite::WebSocketStream;

type ServerSocket = WebSocketStream<TcpStream>;

const SETTINGS: &str = r#"  app_id: 12345
  enable_ws_server: false
  http_address: ""
  text_intent: ["GroupATMessageEventHandler"]
"#;

/// 假 HTTP 传输：`/gateway/bot` 回 mock 网关地址，其余（token 端点）回固定 token。
#[derive(Debug)]
struct FakeTransport {
    gateway_url: String,
    max_concurrency: i64,
    requests: Mutex<Vec<HttpRequest>>,
}

impl FakeTransport {
    fn new(gateway_url: String) -> Self {
        Self {
            gateway_url,
            max_concurrency: 1,
            requests: Mutex::new(Vec::new()),
        }
    }

    /// `/gateway/bot` 的 `session_start_limit.max_concurrency`（决定多分片启动间隔）。
    fn with_max_concurrency(mut self, max_concurrency: i64) -> Self {
        self.max_concurrency = max_concurrency;
        self
    }

    fn last_gateway_request(&self) -> HttpRequest {
        self.requests
            .lock()
            .expect("请求记录锁")
            .iter()
            .rev()
            .find(|request| request.url.ends_with("/gateway/bot"))
            .cloned()
            .expect("应请求过 /gateway/bot")
    }
}

impl HttpTransport for FakeTransport {
    fn request(&self, req: HttpRequest) -> impl Future<Output = QqResult<HttpResponse>> + Send {
        self.requests.lock().expect("请求记录锁").push(req.clone());
        let response = if req.url.ends_with("/gateway/bot") {
            HttpResponse::new(200).with_body(
                json!({
                    "url": self.gateway_url,
                    "shards": 1,
                    "session_start_limit": {
                        "total": 1000,
                        "remaining": 999,
                        "reset_after": 0,
                        "max_concurrency": self.max_concurrency,
                    },
                })
                .to_string(),
            )
        } else {
            HttpResponse::new(200).with_body(r#"{"access_token":"tok-70","expires_in":7200}"#)
        };
        std::future::ready(Ok(response))
    }
}

fn client(
    state: &AppState,
    transport: Arc<FakeTransport>,
    options: GatewayOptions,
) -> GatewayClient<FakeTransport> {
    let tokens = Arc::new(TokenManager::new(
        Arc::clone(&transport),
        TokenConfig::new("12345", "s3cret"),
    ));
    GatewayClient::new(transport, tokens, state, options)
}

fn options(api_base: &str) -> GatewayOptions {
    GatewayOptions {
        api_base: Some(api_base.to_owned()),
        initial_backoff: Duration::from_millis(50),
        max_backoff: Duration::from_millis(200),
    }
}

fn hello(interval_ms: i64) -> Value {
    json!({"op": 10, "d": {"heartbeat_interval": interval_ms}})
}

fn ready(session_id: &str) -> Value {
    json!({
        "op": 0,
        "s": 1,
        "t": "READY",
        "d": {
            "version": 1,
            "session_id": session_id,
            "user": {"id": "1", "username": "bot", "bot": true},
            "shard": [0, 1],
        },
    })
}

fn group_message(seq: i64) -> Value {
    json!({
        "op": 0,
        "s": seq,
        "t": "GROUP_AT_MESSAGE_CREATE",
        "id": "EVENT-1",
        "d": {
            "id": "M1",
            "group_id": "G1",
            "content": "hi",
            "timestamp": "1700000000",
            "author": {"id": "U1", "username": "alice", "member_role": "admin"},
        },
    })
}

async fn send(socket: &mut ServerSocket, value: Value) {
    socket
        .send(TungMessage::text(value.to_string()))
        .await
        .expect("mock 网关写帧失败");
}

async fn recv_json(socket: &mut ServerSocket) -> Value {
    loop {
        match socket.next().await {
            Some(Ok(TungMessage::Text(text))) => {
                return serde_json::from_str(text.as_str()).expect("收到非 JSON 帧");
            }
            Some(Ok(_)) => continue,
            other => panic!("mock 网关未收到文本帧: {other:?}"),
        }
    }
}

async fn bind() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("绑定失败");
    let addr = listener.local_addr().expect("读取地址失败");
    (listener, format!("ws://{addr}"))
}

async fn start_client(
    state: &AppState,
    url: &str,
    options: GatewayOptions,
) -> (GatewayHandle, Arc<FakeTransport>) {
    let transport = Arc::new(FakeTransport::new(url.to_owned()));
    let handle = client(state, Arc::clone(&transport), options).start();
    (handle, transport)
}

#[tokio::test]
async fn identify_ready_and_dispatch_reach_hub() {
    let (listener, url) = bind().await;
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept 失败");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("mock 网关握手失败");
        send(&mut socket, hello(30_000)).await;
        let identify = recv_json(&mut socket).await;
        send(&mut socket, ready("S-1")).await;
        send(&mut socket, group_message(2)).await;
        identify
    });

    let (state, _dir) = support::state(SETTINGS);
    let mut events = state.hub().subscribe();
    let (handle, transport) = start_client(&state, &url, options("http://mock")).await;

    let event = timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("等待 hub 事件超时")
        .expect("hub 已关闭");
    let value: serde_json::Value = serde_json::from_str(event.json()).expect("事件序列化失败");
    assert_eq!(value["post_type"], "message");
    assert_eq!(value["message_type"], "group");
    assert_eq!(value["group_id"], "G1");
    assert_eq!(value["raw_message"], "hi");

    let identify = timeout(Duration::from_secs(5), server)
        .await
        .expect("mock 网关超时")
        .expect("mock 网关 panic");
    assert_eq!(identify["op"], 2, "应发 Identify");
    let token = identify["d"]["token"].as_str().expect("token 不是字符串");
    assert!(token.starts_with("QQBot "), "token 需要 QQBot 前缀: {token}");
    assert_eq!(identify["d"]["intents"], 1_i64 << 25, "GroupATMessageEventHandler");
    assert_eq!(identify["d"]["shard"], json!([0, 1]), "默认单分片 [0,1]");
    assert_eq!(identify["d"]["properties"]["$os"], std::env::consts::OS);

    let request = transport.last_gateway_request();
    assert_eq!(
        request.header("Authorization"),
        Some("QQBot tok-70"),
        "网关接入点请求需要 Authorization: QQBot <token>"
    );

    handle.shutdown(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn heartbeat_follows_hello_interval_and_carries_last_seq() {
    let (listener, url) = bind().await;
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept 失败");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("mock 网关握手失败");
        send(&mut socket, hello(150)).await;
        let _identify = recv_json(&mut socket).await;
        send(&mut socket, ready("S-2")).await;
        send(&mut socket, group_message(7)).await;
        let first = recv_json(&mut socket).await;
        let second = recv_json(&mut socket).await;
        (first, second)
    });

    let (state, _dir) = support::state(SETTINGS);
    let (handle, _transport) = start_client(&state, &url, options("http://mock")).await;

    let (first, second) = timeout(Duration::from_secs(5), server)
        .await
        .expect("mock 网关超时")
        .expect("mock 网关 panic");
    for heartbeat in [first, second] {
        assert_eq!(heartbeat["op"], 1, "应按 Hello 间隔发心跳");
        assert_eq!(heartbeat["d"], 7, "心跳应带最近一次 dispatch 的 seq");
    }

    handle.shutdown(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn op7_reconnects_with_resume_payload() {
    let (listener, url) = bind().await;
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("第一次 accept");
        let mut first = tokio_tungstenite::accept_async(stream)
            .await
            .expect("第一次握手");
        send(&mut first, hello(30_000)).await;
        let _identify = recv_json(&mut first).await;
        send(&mut first, ready("S-7")).await;
        send(&mut first, group_message(5)).await;
        send(&mut first, json!({"op": 7})).await;

        let (stream, _) = listener.accept().await.expect("第二次 accept");
        let mut second = tokio_tungstenite::accept_async(stream)
            .await
            .expect("第二次握手");
        send(&mut second, hello(30_000)).await;
        recv_json(&mut second).await
    });

    let (state, _dir) = support::state(SETTINGS);
    let (handle, _transport) = start_client(&state, &url, options("http://mock")).await;

    let resume = timeout(Duration::from_secs(5), server)
        .await
        .expect("mock 网关超时")
        .expect("mock 网关 panic");
    assert_eq!(resume["op"], 6, "op 7 后应发 Resume");
    assert_eq!(resume["d"]["session_id"], "S-7");
    assert_eq!(resume["d"]["seq"], 5);
    assert!(
        resume["d"]["token"]
            .as_str()
            .is_some_and(|token| token.starts_with("QQBot "))
    );

    handle.shutdown(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn op9_without_resume_reidentifies() {
    let (listener, url) = bind().await;
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("第一次 accept");
        let mut first = tokio_tungstenite::accept_async(stream)
            .await
            .expect("第一次握手");
        send(&mut first, hello(30_000)).await;
        let _identify = recv_json(&mut first).await;
        send(&mut first, ready("S-9")).await;
        send(&mut first, json!({"op": 9, "d": false})).await;

        let (stream, _) = listener.accept().await.expect("第二次 accept");
        let mut second = tokio_tungstenite::accept_async(stream)
            .await
            .expect("第二次握手");
        send(&mut second, hello(30_000)).await;
        recv_json(&mut second).await
    });

    let (state, _dir) = support::state(SETTINGS);
    let (handle, _transport) = start_client(&state, &url, options("http://mock")).await;

    let frame = timeout(Duration::from_secs(6), server)
        .await
        .expect("mock 网关超时")
        .expect("mock 网关 panic");
    assert_eq!(frame["op"], 2, "op 9 d=false 后必须重新 Identify，不能 Resume");

    handle.shutdown(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn failed_connection_is_retried_with_backoff() {
    let (listener, url) = bind().await;
    let server = tokio::spawn(async move {
        // 第一次：接受 TCP 后立刻断开，客户端 connect_async 失败 → 退避重试。
        let (stream, _) = listener.accept().await.expect("第一次 accept");
        drop(stream);

        let (stream, _) = listener.accept().await.expect("第二次 accept");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("第二次握手");
        send(&mut socket, hello(30_000)).await;
        recv_json(&mut socket).await
    });

    let (state, _dir) = support::state(SETTINGS);
    let (handle, _transport) = start_client(&state, &url, options("http://mock")).await;

    let identify = timeout(Duration::from_secs(5), server)
        .await
        .expect("重连超时")
        .expect("mock 网关 panic");
    assert_eq!(identify["op"], 2, "第二次连接应完成 Identify");

    handle.shutdown(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn shutdown_stops_an_established_connection() {
    let (listener, url) = bind().await;
    let (identified_tx, identified_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept 失败");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("mock 网关握手失败");
        send(&mut socket, hello(30_000)).await;
        let _identify = recv_json(&mut socket).await;
        send(&mut socket, ready("S-STOP")).await;
        let _ = identified_tx.send(());
        // 保持连接直到客户端关闭。
        while socket.next().await.is_some() {}
    });

    let (state, _dir) = support::state(SETTINGS);
    let (handle, _transport) = start_client(&state, &url, options("http://mock")).await;
    timeout(Duration::from_secs(5), identified_rx)
        .await
        .expect("等待 Identify 超时")
        .expect("mock 网关提前退出");

    timeout(Duration::from_secs(5), handle.shutdown(Duration::from_secs(2)))
        .await
        .expect("关闭信号未能让网关退出");
    let _ = server.await;
}

#[tokio::test]
async fn runtime_skips_gateway_when_disabled_or_without_credentials() {
    // 1) 凭据齐备但 gateway=false：显式关闭优先。
    let (state, dir) = support::state(
        "  app_id: 102000001\n  client_secret: s3cret\n  enable_ws_server: false\n  http_address: \"\"\n",
    );
    let state = support::with_temp_stores(state, &dir);
    let handle = Runtime::start_with(
        state,
        RuntimeOptions {
            gateway: false,
            config_watch: false,
            ..RuntimeOptions::default()
        },
    )
    .await
    .expect("启动运行时失败");
    assert!(!handle.gateway_started(), "gateway=false 不应启动网关");
    stop(handle).await;

    // 2) 缺 client_secret + 默认开启：warn 后跳过，不阻断启动。
    let (state, dir) = support::state(
        "  app_id: 102000001\n  enable_ws_server: false\n  http_address: \"\"\n",
    );
    let state = support::with_temp_stores(state, &dir);
    let handle = Runtime::start_with(
        state,
        RuntimeOptions {
            config_watch: false,
            ..RuntimeOptions::default()
        },
    )
    .await
    .expect("启动运行时失败");
    assert!(!handle.gateway_started(), "缺凭据不应启动网关");
    stop(handle).await;
}

const MULTI_SETTINGS: &str = r#"  app_id: 12345
  shard_num: 2
  enable_ws_server: false
  http_address: ""
  text_intent: ["GroupATMessageEventHandler"]
"#;

#[tokio::test]
async fn multi_shard_identifies_each_connection_with_its_shard() {
    let (listener, url) = bind().await;
    let server = tokio::spawn(async move {
        let mut sockets = Vec::new();
        let mut identifies = Vec::new();
        for _ in 0..2 {
            let (stream, _) = listener.accept().await.expect("accept 失败");
            let mut socket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("mock 网关握手失败");
            send(&mut socket, hello(30_000)).await;
            identifies.push(recv_json(&mut socket).await);
            send(&mut socket, ready("S-MULTI")).await;
            sockets.push(socket);
        }
        // 连接留在 socket 里返回，测试断言完成前不主动断开（否则客户端会重连干扰断言）。
        (identifies, sockets)
    });

    let (state, _dir) = support::state(MULTI_SETTINGS);
    let transport = Arc::new(FakeTransport::new(url).with_max_concurrency(8));
    let handle = client(&state, transport, options("http://mock")).start();

    let (identifies, _sockets) = timeout(Duration::from_secs(10), server)
        .await
        .expect("两个分片未在时限内连上")
        .expect("mock 网关 panic");
    assert_eq!(identifies.len(), 2, "shard_num=2 应开两条连接");
    let mut shards: Vec<(i64, i64)> = identifies
        .iter()
        .map(|identify| {
            assert_eq!(identify["op"], 2, "每个分片都要发 Identify");
            let shard = identify["d"]["shard"].as_array().expect("shard 不是数组");
            (
                shard[0].as_i64().expect("shard id 不是整数"),
                shard[1].as_i64().expect("shard 数不是整数"),
            )
        })
        .collect();
    shards.sort_unstable();
    assert_eq!(shards, vec![(0, 2), (1, 2)], "各分片 Identify 的 shard=[id,count]");

    handle.shutdown(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn multi_shard_shutdown_closes_every_connection() {
    let (listener, url) = bind().await;
    let (connected_tx, connected_rx) = oneshot::channel();
    let (closed_tx, closed_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut sockets = Vec::new();
        for _ in 0..2 {
            let (stream, _) = listener.accept().await.expect("accept 失败");
            let mut socket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("mock 网关握手失败");
            send(&mut socket, hello(30_000)).await;
            let _identify = recv_json(&mut socket).await;
            send(&mut socket, ready("S-DOWN")).await;
            sockets.push(socket);
        }
        let _ = connected_tx.send(());

        let mut drains = Vec::new();
        for mut socket in sockets {
            drains.push(tokio::spawn(async move {
                while socket.next().await.is_some() {}
            }));
        }
        for drain in drains {
            let _ = drain.await;
        }
        let _ = closed_tx.send(());
    });

    let (state, _dir) = support::state(MULTI_SETTINGS);
    let transport = Arc::new(FakeTransport::new(url).with_max_concurrency(8));
    let handle = client(&state, transport, options("http://mock")).start();
    timeout(Duration::from_secs(10), connected_rx)
        .await
        .expect("两个分片未在时限内连上")
        .expect("mock 网关提前退出");

    timeout(Duration::from_secs(5), handle.shutdown(Duration::from_secs(3)))
        .await
        .expect("关闭信号未能让多分片网关退出");
    timeout(Duration::from_secs(5), closed_rx)
        .await
        .expect("关闭后两条连接都应有 close 帧")
        .expect("mock 网关提前退出");
    let _ = server.await;
}

async fn stop(handle: RuntimeHandle) {
    timeout(Duration::from_secs(10), handle.shutdown())
        .await
        .expect("shutdown 超时");
}
