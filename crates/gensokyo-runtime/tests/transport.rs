//! M1h-a 传输层集成测试：正向 WS + HTTP API。
//!
//! 全部绑定 `127.0.0.1:0`（`RuntimeOptions` 覆盖），不依赖外网、不占用固定端口。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use gensokyo_config::ConfigManager;
use gensokyo_protocol::action::ActionRequest;
use gensokyo_protocol::{Handler, HandlerFuture, Registry};
use gensokyo_runtime::{
    AppState, DEFAULT_SHUTDOWN_TIMEOUT, Runtime, RuntimeHandle, RuntimeOptions,
};
use serde_json::Value;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as TungMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, Request};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

const WS_PLAIN: &str = r#"  port: "5700"
  enable_ws_server: true
  ws_server_path: ws
  ws_server_token: ""
  http_address: ""
"#;

const WS_TOKEN: &str = r#"  port: "5700"
  enable_ws_server: true
  ws_server_path: ws
  ws_server_token: "s3cret"
  http_address: ""
"#;

const WS_SERVER_DIR: &str = r#"  port: "5700"
  enable_ws_server: true
  ws_server_path: ws
  ws_server_token: ""
  http_address: ""
  server_dir: "bot.example.com"
"#;

const WS_BAD_PATH: &str = r#"  port: "5700"
  enable_ws_server: true
  ws_server_path: "ws/{id}"
  ws_server_token: ""
  http_address: ""
"#;

const HTTP_PLAIN: &str = r#"  port: "5700"
  enable_ws_server: false
  http_address: "127.0.0.1:0"
  http_access_token: ""
  app_id: 12345
  custom_bot_name: TestBot
"#;

const HTTP_TOKEN: &str = r#"  port: "5700"
  enable_ws_server: false
  http_address: "127.0.0.1:0"
  http_access_token: "s3cret"
"#;

/// 注入的 panic handler：验证传输层把 panic 隔离成 1500，而不是带走连接或进程。
///
/// 刻意在 `handle` 本体 panic（future 构造阶段），覆盖分发器把调用与轮询一起捕获的路径。
#[derive(Debug)]
struct Panics;

impl Handler for Panics {
    fn name(&self) -> &'static str {
        "panics"
    }

    fn handle<'a>(&'a self, _request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        panic!("injected boom");
    }
}

async fn start_with_handlers(
    settings_yaml: &str,
    options: RuntimeOptions,
    handlers: Vec<(&'static str, Arc<dyn Handler>)>,
) -> (RuntimeHandle, TempDir) {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = dir.path().join("config.yml");
    std::fs::write(&path, format!("version: 1\nsettings:\n{settings_yaml}"))
        .expect("写测试配置失败");
    let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
    let mut registry = Registry::new();
    gensokyo_runtime::builtin::register_all(&mut registry, Arc::clone(&manager));
    for (action, handler) in handlers {
        assert!(
            registry.register(action, handler).is_none(),
            "测试 handler {action} 不应覆盖已注册的实现"
        );
    }
    let state = AppState::new(manager, Arc::new(registry));
    let handle = Runtime::start_with(state, options)
        .await
        .expect("启动运行时失败");
    (handle, dir)
}

async fn start(settings_yaml: &str, options: RuntimeOptions) -> (RuntimeHandle, TempDir) {
    start_with_handlers(settings_yaml, options, Vec::new()).await
}

fn ws_options() -> RuntimeOptions {
    RuntimeOptions {
        ws_bind: Some("127.0.0.1:0".to_owned()),
        http_bind: None,
    }
}

async fn stop(handle: RuntimeHandle) {
    // 比库内默认 5s 宽：外部超时只用来防止测试整体挂死。
    tokio::time::timeout(Duration::from_secs(10), handle.shutdown())
        .await
        .expect("shutdown 超时");
}

async fn read_json(client: &mut Client) -> Value {
    loop {
        match client.next().await {
            Some(Ok(TungMessage::Text(text))) => {
                return serde_json::from_str(text.as_str()).expect("响应不是 JSON");
            }
            Some(Ok(_)) => {}
            Some(Err(err)) => panic!("读取 WS 失败: {err}"),
            None => panic!("连接在收到响应前关闭"),
        }
    }
}

fn assert_unauthorized(err: tokio_tungstenite::tungstenite::Error) {
    match err {
        tokio_tungstenite::tungstenite::Error::Http(response) => {
            assert_eq!(response.status(), 401, "期望握手返回 401");
        }
        other => panic!("期望 HTTP 401，实际: {other:?}"),
    }
}

fn assert_forbidden(err: tokio_tungstenite::tungstenite::Error) {
    match err {
        tokio_tungstenite::tungstenite::Error::Http(response) => {
            assert_eq!(response.status(), 403, "期望握手返回 403（Origin 被拒）");
        }
        other => panic!("期望 HTTP 403，实际: {other:?}"),
    }
}

fn ws_request(addr: SocketAddr) -> Request<()> {
    format!("ws://{addr}/ws")
        .into_client_request()
        .expect("构造握手请求失败")
}

fn with_origin(mut request: Request<()>, origin: &str) -> Request<()> {
    request
        .headers_mut()
        .insert("origin", HeaderValue::from_str(origin).expect("Origin 头"));
    request
}

async fn connect_with_origin(addr: SocketAddr, origin: &str) -> Client {
    connect_async(with_origin(ws_request(addr), origin))
        .await
        .unwrap_or_else(|err| panic!("Origin={origin} 的握手应成功，实际: {err:?}"))
        .0
}

async fn connect_origin_err(
    addr: SocketAddr,
    origin: &str,
) -> tokio_tungstenite::tungstenite::Error {
    connect_async(with_origin(ws_request(addr), origin))
        .await
        .expect_err("非法 Origin 应被拒")
}

/// 端口不再监听（listener 已释放）。
///
/// 不能只靠 `TcpStream::connect` 的返回值：Windows 上向刚关闭的监听端口发起连接可能收不到
/// RST，要等 SYN 重传（~2s）才报错。因此每次尝试套一个短超时——回环上活着的 listener 是
/// 微秒级完成握手的，300ms 无人应答就等价于「没有 listener」。
async fn assert_port_closed(addr: SocketAddr) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match tokio::time::timeout(Duration::from_millis(300), TcpStream::connect(addr)).await {
            Ok(Ok(_)) if Instant::now() >= deadline => {
                panic!("shutdown 之后 {addr} 仍在监听");
            }
            Ok(Ok(_)) => tokio::time::sleep(Duration::from_millis(20)).await,
            Ok(Err(_)) | Err(_) => return,
        }
    }
}

async fn http_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: &str,
    authorization: Option<&str>,
) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).await.expect("连接 HTTP 失败");
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(value) = authorization {
        request.push_str(&format!("Authorization: {value}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream
        .write_all(request.as_bytes())
        .await
        .expect("写请求失败");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("读响应失败");
    let text = String::from_utf8(response).expect("响应不是 UTF-8");
    let (head, body) = text.split_once("\r\n\r\n").expect("响应缺少头部分隔");
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .expect("状态码解析失败");
    (status, body.to_owned())
}

#[tokio::test]
async fn ws_get_status_roundtrip() {
    let (handle, _dir) = start(WS_PLAIN, ws_options()).await;
    let addr = handle.ws_addr().expect("WS 应已监听");
    assert!(
        handle.http_addr().is_none(),
        "http_address 为空时不应启动 HTTP API"
    );

    let (mut client, _) = connect_async(format!("ws://{addr}/ws"))
        .await
        .expect("WS 连接失败");
    client
        .send(TungMessage::Text(
            r#"{"action":"get_status","echo":"probe"}"#.into(),
        ))
        .await
        .expect("发送失败");

    let value = read_json(&mut client).await;
    assert_eq!(value["status"], "ok");
    assert_eq!(value["retcode"], 0);
    assert_eq!(value["data"]["online"], true);
    assert_eq!(value["data"]["good"], true);
    assert_eq!(value["echo"], "probe");

    drop(client);
    stop(handle).await;
}

#[tokio::test]
async fn ws_rejects_bad_tokens_and_answers_invalid_json_with_1400() {
    let (handle, _dir) = start(WS_TOKEN, ws_options()).await;
    let addr = handle.ws_addr().expect("WS 应已监听");

    let err = connect_async(format!("ws://{addr}/ws"))
        .await
        .expect_err("缺少 token 应被拒");
    assert_unauthorized(err);
    let err = connect_async(format!("ws://{addr}/ws?access_token=wrong"))
        .await
        .expect_err("错误 token 应被拒");
    assert_unauthorized(err);

    let (mut query_client, _) = connect_async(format!("ws://{addr}/ws?access_token=s3cret"))
        .await
        .expect("query token 应通过");

    let mut request = format!("ws://{addr}/ws")
        .into_client_request()
        .expect("构造握手请求失败");
    request
        .headers_mut()
        .insert("authorization", HeaderValue::from_static("Bearer s3cret"));
    let (mut header_client, _) = connect_async(request).await.expect("Bearer token 应通过");

    query_client
        .send(TungMessage::Text("not-json".into()))
        .await
        .expect("发送失败");
    let value = read_json(&mut query_client).await;
    assert_eq!(value["status"], "failed");
    assert_eq!(value["retcode"], 1400);

    header_client
        .send(TungMessage::Text(r#"{"action":"get_status"}"#.into()))
        .await
        .expect("发送失败");
    let value = read_json(&mut header_client).await;
    assert_eq!(value["status"], "ok");
    assert_eq!(value["retcode"], 0);

    drop((query_client, header_client));
    stop(handle).await;
}

#[tokio::test]
async fn ws_origin_allows_loopback_and_rejects_foreign_hosts() {
    // token 为空时 Origin 校验同样生效。
    let (handle, _dir) = start(WS_PLAIN, ws_options()).await;
    let addr = handle.ws_addr().expect("WS 应已监听");

    let mut loopback = connect_with_origin(addr, "http://127.0.0.1:5173").await;
    loopback
        .send(TungMessage::Text(r#"{"action":"get_status"}"#.into()))
        .await
        .expect("发送失败");
    assert_eq!(read_json(&mut loopback).await["retcode"], 0);

    let mut localhost = connect_with_origin(addr, "http://localhost:8080").await;
    localhost
        .send(TungMessage::Text(r#"{"action":"get_status"}"#.into()))
        .await
        .expect("发送失败");
    assert_eq!(read_json(&mut localhost).await["retcode"], 0);

    assert_forbidden(connect_origin_err(addr, "http://evil.example").await);
    assert_forbidden(connect_origin_err(addr, "https://127.0.0.1.evil.example").await);

    drop((loopback, localhost));
    stop(handle).await;
}

#[tokio::test]
async fn ws_origin_whitelists_server_dir() {
    let (handle, _dir) = start(WS_SERVER_DIR, ws_options()).await;
    let addr = handle.ws_addr().expect("WS 应已监听");

    let mut client = connect_with_origin(addr, "https://bot.example.com").await;
    client
        .send(TungMessage::Text(r#"{"action":"get_status"}"#.into()))
        .await
        .expect("发送失败");
    assert_eq!(read_json(&mut client).await["retcode"], 0);

    assert_forbidden(connect_origin_err(addr, "https://other.example.com").await);

    drop(client);
    stop(handle).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ws_clients_are_concurrent_and_get_their_own_echo() {
    let (handle, _dir) = start(WS_PLAIN, ws_options()).await;
    let addr = handle.ws_addr().expect("WS 应已监听");

    let mut tasks = Vec::new();
    for index in 0..4u8 {
        tasks.push(tokio::spawn(async move {
            let (mut client, _) = connect_async(format!("ws://{addr}/ws"))
                .await
                .expect("WS 连接失败");
            let payload = format!(r#"{{"action":"get_status","echo":"c{index}"}}"#);
            client
                .send(TungMessage::Text(payload.into()))
                .await
                .expect("发送失败");
            let value = read_json(&mut client).await;
            assert_eq!(value["retcode"], 0);
            assert_eq!(value["echo"], format!("c{index}"));
        }));
    }
    for task in tasks {
        task.await.expect("客户端任务失败");
    }

    stop(handle).await;
}

#[tokio::test]
async fn ws_unknown_action_is_1404_and_handler_panic_is_1500() {
    let (handle, _dir) = start_with_handlers(
        WS_PLAIN,
        ws_options(),
        vec![("panics", Arc::new(Panics) as Arc<dyn Handler>)],
    )
    .await;
    let addr = handle.ws_addr().expect("WS 应已监听");

    let (mut client, _) = connect_async(format!("ws://{addr}/ws"))
        .await
        .expect("WS 连接失败");

    client
        .send(TungMessage::Text(r#"{"action":"nope","echo":"u1"}"#.into()))
        .await
        .expect("发送失败");
    let value = read_json(&mut client).await;
    assert_eq!(value["status"], "failed");
    assert_eq!(value["retcode"], 1404);
    assert_eq!(value["echo"], "u1");

    client
        .send(TungMessage::Text(
            r#"{"action":"panics","echo":"p1"}"#.into(),
        ))
        .await
        .expect("发送失败");
    let value = read_json(&mut client).await;
    assert_eq!(value["status"], "failed");
    assert_eq!(value["retcode"], 1500);
    assert_eq!(value["echo"], "p1");
    assert_eq!(
        value["message"], "handler panic",
        "panic 全文不应回给客户端"
    );

    // panic 被隔离：同一连接仍然可用。
    client
        .send(TungMessage::Text(r#"{"action":"get_status"}"#.into()))
        .await
        .expect("发送失败");
    assert_eq!(read_json(&mut client).await["retcode"], 0);

    drop(client);
    stop(handle).await;
}

#[tokio::test]
async fn invalid_ws_server_path_fails_startup() {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = dir.path().join("config.yml");
    std::fs::write(&path, format!("version: 1\nsettings:\n{WS_BAD_PATH}")).expect("写测试配置失败");
    let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
    let state = AppState::with_builtin_handlers(manager);

    let err = Runtime::start_with(state, ws_options())
        .await
        .expect_err("非法 ws_server_path 应在启动时报错");
    assert!(
        err.to_string().contains("ws_server_path"),
        "错误信息应指明 ws_server_path: {err}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_with_live_ws_connection_returns_within_timeout() {
    let (handle, _dir) = start(WS_PLAIN, ws_options()).await;
    let addr = handle.ws_addr().expect("WS 应已监听");

    let (mut client, _) = connect_async(format!("ws://{addr}/ws"))
        .await
        .expect("WS 连接失败");
    client
        .send(TungMessage::Text(
            r#"{"action":"get_status","echo":"live"}"#.into(),
        ))
        .await
        .expect("发送失败");
    assert_eq!(read_json(&mut client).await["retcode"], 0);

    // 连接保持打开（客户端不主动关闭）：shutdown 最多花掉默认时限，不能无限挂住。
    let started = Instant::now();
    tokio::time::timeout(
        DEFAULT_SHUTDOWN_TIMEOUT + Duration::from_secs(3),
        handle.shutdown(),
    )
    .await
    .expect("带活连接的 shutdown 应在超时内返回");
    let elapsed = started.elapsed();
    assert!(
        elapsed < DEFAULT_SHUTDOWN_TIMEOUT + Duration::from_secs(2),
        "shutdown 应在默认时限内返回，实际耗时 {elapsed:?}"
    );

    assert_port_closed(addr).await;
    drop(client);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_timeout_forces_abort_and_releases_port() {
    let (handle, _dir) = start(WS_PLAIN, ws_options()).await;
    let addr = handle.ws_addr().expect("WS 应已监听");

    let (mut client, _) = connect_async(format!("ws://{addr}/ws"))
        .await
        .expect("WS 连接失败");
    client
        .send(TungMessage::Text(r#"{"action":"get_status"}"#.into()))
        .await
        .expect("发送失败");
    assert_eq!(read_json(&mut client).await["retcode"], 0);

    // 零时限：必定走「超时 → AbortHandle 强制中止」分支，端口必须随之释放。
    handle.shutdown_with_timeout(Duration::ZERO).await;

    assert_port_closed(addr).await;
    drop(client);
}

#[tokio::test]
async fn http_health_unknown_action_and_version_info() {
    let (handle, _dir) = start(HTTP_PLAIN, RuntimeOptions::default()).await;
    let addr = handle.http_addr().expect("HTTP 应已监听");
    assert!(
        handle.ws_addr().is_none(),
        "enable_ws_server=false 时不应启动 WS"
    );

    let (status, body) = http_request(addr, "GET", "/", "", None).await;
    assert_eq!(status, 200);
    assert!(body.contains("gensokyo"), "健康检查文本: {body}");

    let (status, body) = http_request(
        addr,
        "POST",
        "/whatever",
        r#"{"action":"nope","echo":"u1"}"#,
        None,
    )
    .await;
    assert_eq!(status, 200);
    let value: Value = serde_json::from_str(&body).expect("响应应为 JSON");
    assert_eq!(value["status"], "failed");
    assert_eq!(value["retcode"], 1404);
    assert_eq!(value["echo"], "u1");

    let (status, body) = http_request(addr, "POST", "/get_status", "not-json", None).await;
    assert_eq!(status, 200);
    let value: Value = serde_json::from_str(&body).expect("响应应为 JSON");
    assert_eq!(value["status"], "failed");
    assert_eq!(value["retcode"], 1400, "非 JSON body 应回 1400");

    let (status, body) = http_request(
        addr,
        "POST",
        "/get_version_info",
        r#"{"action":"get_version_info"}"#,
        None,
    )
    .await;
    assert_eq!(status, 200);
    let value: Value = serde_json::from_str(&body).expect("响应应为 JSON");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["retcode"], 0);
    assert_eq!(value["data"]["app_name"], "gensokyo");
    assert_eq!(value["data"]["app_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["data"]["protocol_version"], "v11");

    let (status, _) = http_request(addr, "GET", "/get_status", "", None).await;
    assert_eq!(status, 405, "非 POST 的 action 路径应回 405");

    stop(handle).await;
}

#[tokio::test]
async fn http_requires_token_when_configured() {
    let (handle, _dir) = start(HTTP_TOKEN, RuntimeOptions::default()).await;
    let addr = handle.http_addr().expect("HTTP 应已监听");
    let payload = r#"{"action":"get_status"}"#;

    let (status, _) = http_request(addr, "POST", "/get_status", payload, None).await;
    assert_eq!(status, 401, "缺少 token 应回 401");
    let (status, _) =
        http_request(addr, "POST", "/get_status", payload, Some("Bearer wrong")).await;
    assert_eq!(status, 401, "错误 token 应回 401");

    let (status, body) =
        http_request(addr, "POST", "/get_status", payload, Some("Bearer s3cret")).await;
    assert_eq!(status, 200);
    let value: Value = serde_json::from_str(&body).expect("响应应为 JSON");
    assert_eq!(value["retcode"], 0);

    let (status, _) = http_request(
        addr,
        "POST",
        "/get_status?access_token=s3cret",
        payload,
        None,
    )
    .await;
    assert_eq!(status, 200, "query token 应通过");

    let (status, _) = http_request(addr, "GET", "/", "", None).await;
    assert_eq!(status, 200, "健康检查不强制 token");

    stop(handle).await;
}
