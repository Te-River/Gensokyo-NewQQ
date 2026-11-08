//! `post_url` 事件上报集成测试（Go `Processor/Processor.go:401-488`）。
//!
//! 上报走 [`gensokyo_qq::HttpTransport`] seam（生产实现是 `ReqwestTransport`），这里用一个
//! **按 `HttpRequest` 发原始 HTTP/1.1 的测试传输**打到本地 mock server：既能断言 Go 口径的
//! 请求头 / body，又不需要 TLS provider（测试进程没装 rustls provider，`ReqwestTransport`
//! 建不起来——与 `gateway` 用例的降级口径一致）。

mod support;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use gensokyo_protocol::event::post_type;
use gensokyo_qq::{Error as QqError, HttpRequest, HttpResponse, HttpTransport};
use gensokyo_runtime::{AppState, PostUrlReporter, Runtime, RuntimeOptions};
use serde_json::Value;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const APP_ID: &str = "102000001";

/// mock server 收到的一条请求。
#[derive(Debug, Clone)]
struct Captured {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Captured {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

type Sink = Arc<Mutex<Vec<Captured>>>;

/// 起一个接收 `POST` 的 mock server；收到的请求按顺序堆进返回值。
async fn mock_server() -> (SocketAddr, Sink, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("绑定 mock server 失败");
    let addr = listener.local_addr().expect("读取 mock server 地址失败");
    let sink: Sink = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&sink);
    let task = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let recorder = Arc::clone(&recorder);
            tokio::spawn(async move {
                if let Some(captured) = read_request(&mut stream).await {
                    recorder.lock().await.push(captured);
                }
                let _ = stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (addr, sink, task)
}

/// 读一条完整的 HTTP/1.1 请求（按 `Content-Length` 收 body）。
async fn read_request(stream: &mut TcpStream) -> Option<Captured> {
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..read]);
        let Some(head_end) = raw.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&raw[..head_end]).into_owned();
        let length = content_length(&head);
        if raw.len() >= head_end + 4 + length {
            break;
        }
    }
    parse_request(&raw)
}

fn content_length(head: &str) -> usize {
    head.split("\r\n")
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse().ok())
        .unwrap_or(0)
}

fn parse_request(raw: &[u8]) -> Option<Captured> {
    let head_end = raw.windows(4).position(|window| window == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&raw[..head_end]).into_owned();
    let body = String::from_utf8_lossy(&raw[head_end + 4..]).into_owned();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next()?.split_whitespace();
    let method = request_line.next()?.to_owned();
    let path = request_line.next()?.to_owned();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
        .collect();
    Some(Captured {
        method,
        path,
        headers,
        body,
    })
}

/// 测试传输：把 `HttpRequest` 原样写成 `http://` 原始 HTTP/1.1 请求。
#[derive(Debug)]
struct TcpTransport;

impl HttpTransport for TcpTransport {
    async fn request(&self, req: HttpRequest) -> gensokyo_qq::Result<HttpResponse> {
        let rest = req
            .url
            .strip_prefix("http://")
            .ok_or_else(|| QqError::Transport(format!("测试传输只支持 http://：{}", req.url)))?;
        let (authority, path) = match rest.split_once('/') {
            Some((authority, path)) => (authority, format!("/{path}")),
            None => (rest, "/".to_owned()),
        };

        let mut head = format!("{} {path} HTTP/1.1\r\nHost: {authority}\r\n", req.method);
        for (name, value) in &req.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            req.body.len()
        ));

        let mut stream = TcpStream::connect(authority)
            .await
            .map_err(|err| QqError::Transport(err.to_string()))?;
        stream
            .write_all(head.as_bytes())
            .await
            .map_err(|err| QqError::Transport(err.to_string()))?;
        stream
            .write_all(&req.body)
            .await
            .map_err(|err| QqError::Transport(err.to_string()))?;

        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .await
            .map_err(|err| QqError::Transport(err.to_string()))?;
        let status = String::from_utf8_lossy(&raw)
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        Ok(HttpResponse::new(status))
    }
}

/// 写一份带 `post_url` 的配置并构造状态。
fn state_with(urls: &[&str]) -> (AppState, TempDir) {
    let list = urls
        .iter()
        .map(|url| format!("\"{url}\""))
        .collect::<Vec<_>>()
        .join(", ");
    support::state(&format!(
        "  app_id: {APP_ID}\n  post_url: [{list}]\n  enable_ws_server: false\n  http_address: \"\"\n"
    ))
}

/// 等上报任务订阅总线（`start_with` 的订阅发生在 spawn 之后）。
async fn wait_subscribed(state: &AppState, expected: usize) {
    for _ in 0..500 {
        if state.hub().receiver_count() >= expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("上报任务未订阅事件总线");
}

/// 等 mock server 收够 `count` 条请求。
async fn wait_requests(sink: &Sink, count: usize) -> Vec<Captured> {
    for _ in 0..500 {
        {
            let captured = sink.lock().await;
            if captured.len() >= count {
                return captured.clone();
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let captured = sink.lock().await;
    panic!("mock server 未收到 {count} 条请求，实际 {}", captured.len());
}

#[tokio::test]
async fn publishes_event_json_with_go_headers() {
    let (addr, sink, server) = mock_server().await;
    let (state, _dir) = state_with(&[&format!("http://{addr}/cb")]);
    let reporter = PostUrlReporter::start_with(state.clone(), TcpTransport);
    wait_subscribed(&state, 1).await;

    let event = support::hub_event();
    let expected = serde_json::to_value(&event).expect("序列化事件失败");
    state.hub().publish(event);

    let captured = wait_requests(&sink, 1).await;
    assert_eq!(captured[0].method, "POST");
    assert_eq!(captured[0].path, "/cb");
    assert_eq!(captured[0].header("content-type"), Some("application/json"));
    assert_eq!(captured[0].header("x-self-id"), Some(APP_ID));
    let body: Value = serde_json::from_str(&captured[0].body).expect("body 不是 JSON");
    assert_eq!(body, expected, "上报 body 必须与发给 WS 的事件 JSON 一致");

    reporter.shutdown(Duration::from_secs(2)).await;
    server.abort();
}

#[tokio::test]
async fn every_configured_url_receives_the_event() {
    let (first, first_sink, first_server) = mock_server().await;
    let (second, second_sink, second_server) = mock_server().await;
    let (state, _dir) = state_with(&[&format!("http://{first}/a"), &format!("http://{second}/b")]);
    let reporter = PostUrlReporter::start_with(state.clone(), TcpTransport);
    wait_subscribed(&state, 1).await;

    state.hub().publish(support::hub_event());

    let first_requests = wait_requests(&first_sink, 1).await;
    let second_requests = wait_requests(&second_sink, 1).await;
    assert_eq!(first_requests[0].path, "/a");
    assert_eq!(second_requests[0].path, "/b");

    reporter.shutdown(Duration::from_secs(2)).await;
    first_server.abort();
    second_server.abort();
}

#[tokio::test]
async fn empty_config_stays_idle() {
    let (_addr, sink, server) = mock_server().await;
    let (state, _dir) = state_with(&[""]);
    let reporter = PostUrlReporter::start_with(state.clone(), TcpTransport);
    wait_subscribed(&state, 1).await;
    assert!(reporter.is_running(), "空配置应空转（热重载后生效）");

    state.hub().publish(support::hub_event());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(sink.lock().await.is_empty(), "post_url 为空时不应发出请求");

    reporter.shutdown(Duration::from_secs(2)).await;
    server.abort();
}

#[tokio::test]
async fn failing_url_does_not_affect_other_urls_or_subscribers() {
    let (addr, sink, server) = mock_server().await;
    // 127.0.0.1:1 没有监听：连接必然失败，对应 Go 的「错误只记日志」分支。
    let (state, _dir) = state_with(&[&format!("http://{addr}/ok"), "http://127.0.0.1:1/down"]);
    let mut direct = state.hub().subscribe();
    let reporter = PostUrlReporter::start_with(state.clone(), TcpTransport);
    wait_subscribed(&state, 2).await;

    state.hub().publish(support::hub_event());

    assert_eq!(wait_requests(&sink, 1).await[0].path, "/ok");
    let received = timeout(Duration::from_secs(5), direct.recv())
        .await
        .expect("其它订阅者仍应收到事件")
        .expect("事件总线不应关闭");
    assert_eq!(received.post_type(), post_type::MESSAGE);

    reporter.shutdown(Duration::from_secs(2)).await;
    server.abort();
}

#[tokio::test]
async fn shutdown_stops_reporting() {
    let (addr, sink, server) = mock_server().await;
    let (state, _dir) = state_with(&[&format!("http://{addr}/cb")]);
    let reporter = PostUrlReporter::start_with(state.clone(), TcpTransport);
    wait_subscribed(&state, 1).await;

    state.hub().publish(support::hub_event());
    wait_requests(&sink, 1).await;

    reporter.shutdown(Duration::from_secs(2)).await;
    assert_eq!(state.hub().receiver_count(), 0, "关闭后订阅应已释放");

    state.hub().publish(support::hub_event());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(sink.lock().await.len(), 1, "关闭后不应再有上报");
    server.abort();
}

#[tokio::test]
async fn runtime_can_disable_post_url_reporting() {
    let (state, dir) = support::state(
        "  app_id: 102000001\n  post_url: [\"http://127.0.0.1:1/cb\"]\n  enable_ws_server: false\n  http_address: \"\"\n",
    );
    let state = support::with_temp_idmap(state, &dir);
    let handle = Runtime::start_with(
        state,
        RuntimeOptions {
            post_url: false,
            gateway: false,
            config_watch: false,
            ..RuntimeOptions::default()
        },
    )
    .await
    .expect("启动运行时失败");
    assert!(!handle.post_url_started(), "post_url=false 不应启动上报");

    timeout(
        Duration::from_secs(10),
        handle.shutdown_with_timeout(Duration::from_secs(2)),
    )
    .await
    .expect("shutdown 超时");
}
