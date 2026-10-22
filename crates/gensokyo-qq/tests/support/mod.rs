//! 集成测试共用件：记录请求的 FakeTransport 与手动时钟。
#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use gensokyo_qq::{
    Clock, HttpRequest, HttpResponse, HttpTransport, OpenApiClient, Result, TokenConfig,
    TokenManager,
};

/// 记录每个请求、按队列返回预设响应的传输实现。
#[derive(Debug, Default)]
pub struct FakeTransport {
    requests: Mutex<Vec<HttpRequest>>,
    responses: Mutex<VecDeque<HttpResponse>>,
}

impl FakeTransport {
    /// 空队列。
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一个预设响应。
    pub fn push_response(&self, response: HttpResponse) {
        self.responses
            .lock()
            .expect("responses 锁中毒")
            .push_back(response);
    }

    /// 已记录的请求（按发生顺序）。
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.requests.lock().expect("requests 锁中毒").clone()
    }

    /// 请求数量。
    pub fn request_count(&self) -> usize {
        self.requests.lock().expect("requests 锁中毒").len()
    }

    /// 最后一个请求。
    pub fn last_request(&self) -> HttpRequest {
        self.requests().pop().expect("预期至少发生一次请求")
    }
}

impl HttpTransport for FakeTransport {
    fn request(
        &self,
        req: HttpRequest,
    ) -> impl std::future::Future<Output = Result<HttpResponse>> + Send {
        self.requests.lock().expect("requests 锁中毒").push(req);
        let response = self
            .responses
            .lock()
            .expect("responses 锁中毒")
            .pop_front()
            .unwrap_or_else(|| {
                HttpResponse::new(599)
                    .with_body("{\"message\":\"fake 无预设响应\"}".as_bytes().to_vec())
            });
        async move { Ok(response) }
    }
}

/// 手动推进的秒级时钟。
#[derive(Debug, Default)]
pub struct ManualClock {
    now: AtomicU64,
}

impl ManualClock {
    /// 指定起始时间。
    pub fn new(now: u64) -> Self {
        Self {
            now: AtomicU64::new(now),
        }
    }

    /// 前进指定秒数。
    pub fn advance(&self, seconds: u64) {
        self.now.fetch_add(seconds, Ordering::Relaxed);
    }
}

impl Clock for ManualClock {
    fn now_unix(&self) -> u64 {
        self.now.load(Ordering::Relaxed)
    }
}

/// token 接口成功响应（扁平结构）。
pub fn token_response(token: &str, expires_in: u64) -> HttpResponse {
    let body = format!(r#"{{"access_token":"{token}","expires_in":{expires_in}}}"#);
    HttpResponse::new(200).with_body(body.into_bytes())
}

/// 任意 JSON 成功响应。
pub fn ok_json(body: &str) -> HttpResponse {
    HttpResponse::new(200).with_body(body.as_bytes().to_vec())
}

/// 构造一个已注入 fake 传输与手动时钟的客户端。
pub fn test_client(
    transport: &Arc<FakeTransport>,
    clock: &Arc<ManualClock>,
) -> OpenApiClient<FakeTransport> {
    let tokens = Arc::new(TokenManager::with_clock(
        Arc::clone(transport),
        TokenConfig::new("10001", "test-secret"),
        Arc::clone(clock) as Arc<dyn Clock>,
    ));
    OpenApiClient::new(Arc::clone(transport), tokens).with_base_url("https://api.bot.qq.com")
}
