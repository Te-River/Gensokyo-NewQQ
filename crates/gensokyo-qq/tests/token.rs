//! TokenManager：缓存、提前刷新、并发、失败回退与脱敏。

mod support;

use std::sync::Arc;

use gensokyo_qq::{
    Clock, Error, HttpMethod, HttpResponse, ReqwestTransport, TokenConfig, TokenManager,
};
use support::{FakeTransport, ManualClock, token_response};

const START: u64 = 1_700_000_000;

fn manager(
    transport: &Arc<FakeTransport>,
    clock: &Arc<ManualClock>,
) -> TokenManager<FakeTransport> {
    TokenManager::with_clock(
        Arc::clone(transport),
        TokenConfig::new("10001", "test-secret"),
        Arc::clone(clock) as Arc<dyn Clock>,
    )
}

#[tokio::test]
async fn fetches_token_once_and_caches() {
    let transport = Arc::new(FakeTransport::new());
    transport.push_response(token_response("tok-1", 7200));
    let clock = Arc::new(ManualClock::new(START));
    let tokens = manager(&transport, &clock);

    assert_eq!(tokens.token().await.unwrap(), "tok-1");
    clock.advance(1000);
    assert_eq!(tokens.token().await.unwrap(), "tok-1");
    assert_eq!(transport.request_count(), 1);

    let request = transport.last_request();
    assert_eq!(request.method, HttpMethod::Post);
    assert_eq!(request.url, "https://api.bot.qq.com/app/getAppAccessToken");
    assert!(
        request.header("Authorization").is_none(),
        "token 接口不需要 Authorization 头"
    );
    let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(body["appId"], "10001");
    assert_eq!(body["clientSecret"], "test-secret");
}

#[tokio::test]
async fn refreshes_only_inside_sixty_second_window() {
    let transport = Arc::new(FakeTransport::new());
    transport.push_response(token_response("tok-1", 3600));
    transport.push_response(token_response("tok-2", 3600));
    let clock = Arc::new(ManualClock::new(START));
    let tokens = manager(&transport, &clock);

    assert_eq!(tokens.token().await.unwrap(), "tok-1");
    // 有效期 3600s：剩余 100s 时仍在缓存窗口内
    clock.advance(3500);
    assert_eq!(tokens.token().await.unwrap(), "tok-1");
    assert_eq!(transport.request_count(), 1);
    // 剩余正好 60s：提前刷新
    clock.advance(40);
    assert_eq!(tokens.token().await.unwrap(), "tok-2");
    assert_eq!(transport.request_count(), 2);
}

#[tokio::test]
async fn refresh_failure_returns_error_without_panic() {
    let transport = Arc::new(FakeTransport::new());
    transport.push_response(token_response("tok-1", 3600));
    transport.push_response(
        HttpResponse::new(500).with_body(br#"{"message":"upstream boom"}"#.to_vec()),
    );
    let clock = Arc::new(ManualClock::new(START));
    let tokens = manager(&transport, &clock);

    assert_eq!(tokens.token().await.unwrap(), "tok-1");
    clock.advance(4000);

    let error = tokens.token().await.expect_err("刷新失败必须返回错误");
    let api = error.api().expect("应是上游 ApiError");
    assert_eq!(api.http_status, 500);
    assert_eq!(api.code, 500, "无业务码时 code 退回 HTTP 状态码");
    assert_eq!(api.message, "upstream boom");
}

#[tokio::test]
async fn accepts_data_envelope_and_string_expires_in() {
    let transport = Arc::new(FakeTransport::new());
    transport.push_response(support::ok_json(
        r#"{"code":0,"message":"ok","data":{"access_token":"tok-env","expires_in":"7200"}}"#,
    ));
    let clock = Arc::new(ManualClock::new(START));
    let tokens = manager(&transport, &clock);

    assert_eq!(tokens.token().await.unwrap(), "tok-env");
}

#[tokio::test]
async fn rejects_business_error_envelope() {
    let transport = Arc::new(FakeTransport::new());
    transport.push_response(support::ok_json(
        r#"{"code":10001,"message":"invalid clientSecret"}"#,
    ));
    let clock = Arc::new(ManualClock::new(START));
    let tokens = manager(&transport, &clock);

    let error = tokens.token().await.expect_err("业务码非 0 必须是错误");
    let api = error.api().expect("应是 ApiError");
    assert_eq!(api.code, 10001);
    assert_eq!(api.http_status, 200);
    assert_eq!(api.message, "invalid clientSecret");
}

#[tokio::test]
async fn rejects_response_without_token() {
    let transport = Arc::new(FakeTransport::new());
    transport.push_response(support::ok_json(r#"{"code":0,"message":"ok"}"#));
    let clock = Arc::new(ManualClock::new(START));
    let tokens = manager(&transport, &clock);

    let error = tokens.token().await.expect_err("没有 token 必须报错");
    assert!(matches!(error, Error::InvalidResponse(_)), "{error:?}");
}

#[tokio::test]
async fn force_refresh_bypasses_cache_and_invalidate_clears_it() {
    let transport = Arc::new(FakeTransport::new());
    transport.push_response(token_response("tok-1", 7200));
    transport.push_response(token_response("tok-2", 7200));
    transport.push_response(token_response("tok-3", 7200));
    let clock = Arc::new(ManualClock::new(START));
    let tokens = manager(&transport, &clock);

    assert_eq!(tokens.token().await.unwrap(), "tok-1");
    assert_eq!(tokens.force_refresh().await.unwrap(), "tok-2");
    assert_eq!(transport.request_count(), 2);

    tokens.invalidate().await;
    assert_eq!(tokens.token().await.unwrap(), "tok-3");
    assert_eq!(transport.request_count(), 3);
}

#[tokio::test]
async fn debug_output_never_leaks_secret_or_token() {
    let transport = Arc::new(FakeTransport::new());
    transport.push_response(token_response("super-secret-token", 7200));
    let clock = Arc::new(ManualClock::new(START));
    let config = TokenConfig::new("10001", "super-secret").with_sandbox(true);
    let tokens = TokenManager::with_clock(
        Arc::clone(&transport),
        config.clone(),
        Arc::clone(&clock) as Arc<dyn Clock>,
    );
    assert_eq!(tokens.token().await.unwrap(), "super-secret-token");

    let config_debug = format!("{config:?}");
    let manager_debug = format!("{tokens:?}");
    assert!(!config_debug.contains("super-secret"), "{config_debug}");
    assert!(
        !manager_debug.contains("super-secret-token"),
        "{manager_debug}"
    );
    assert!(config_debug.contains("<redacted>"), "{config_debug}");
    assert!(clock.now_unix() >= START);
    assert!(config.sandbox());
}

#[tokio::test]
async fn concurrent_calls_fetch_token_once() {
    let transport = Arc::new(FakeTransport::new());
    transport.push_response(token_response("tok-1", 7200));
    let clock = Arc::new(ManualClock::new(START));
    let tokens = manager(&transport, &clock);

    // 四个 future 并发轮询：互斥锁串行化后只有第一个真正打 token 接口。
    let (a, b, c, d) = tokio::join!(
        tokens.token(),
        tokens.token(),
        tokens.token(),
        tokens.token()
    );
    for token in [a, b, c, d] {
        assert_eq!(token.unwrap(), "tok-1");
    }
    assert_eq!(transport.request_count(), 1, "并发获取只应 fetch 一次");
}

#[tokio::test]
async fn refresh_failure_falls_back_to_unexpired_token() {
    let transport = Arc::new(FakeTransport::new());
    transport.push_response(token_response("tok-1", 3600));
    transport.push_response(
        HttpResponse::new(500).with_body(br#"{"message":"upstream boom"}"#.to_vec()),
    );
    transport.push_response(
        HttpResponse::new(500).with_body(br#"{"message":"upstream boom"}"#.to_vec()),
    );
    let clock = Arc::new(ManualClock::new(START));
    let tokens = manager(&transport, &clock);

    assert_eq!(tokens.token().await.unwrap(), "tok-1");
    // 剩余 10s：已在提前刷新窗口内，但旧 token 尚未过期 → 刷新失败回退旧 token。
    clock.advance(3590);
    assert_eq!(tokens.token().await.unwrap(), "tok-1");
    assert_eq!(transport.request_count(), 2);

    // 旧 token 真正过期后再刷新失败，必须返回错误。
    clock.advance(20);
    let error = tokens
        .token()
        .await
        .expect_err("旧 token 已过期时刷新失败必须报错");
    assert_eq!(error.api().expect("应是上游 ApiError").http_status, 500);
    assert_eq!(transport.request_count(), 3);
}

#[tokio::test]
async fn custom_token_url_uses_get_without_body() {
    let transport = Arc::new(FakeTransport::new());
    transport.push_response(token_response("tok-dev", 7200));
    let clock = Arc::new(ManualClock::new(START));
    let config = TokenConfig::new("10001", "test-secret")
        .with_token_url("https://dev.example.com/access_token");
    let tokens = TokenManager::with_clock(
        Arc::clone(&transport),
        config.clone(),
        Arc::clone(&clock) as Arc<dyn Clock>,
    );

    assert_eq!(tokens.token().await.unwrap(), "tok-dev");
    assert_eq!(
        config.token_url(),
        Some("https://dev.example.com/access_token")
    );
    let request = transport.last_request();
    assert_eq!(request.method, HttpMethod::Get);
    assert_eq!(request.url, "https://dev.example.com/access_token");
    assert!(request.body.is_empty(), "自定义 token 端点不带请求体");
    assert!(request.header("Authorization").is_none());
}

#[tokio::test]
async fn blank_token_url_falls_back_to_official_endpoint() {
    let transport = Arc::new(FakeTransport::new());
    transport.push_response(token_response("tok-1", 7200));
    let clock = Arc::new(ManualClock::new(START));
    let config = TokenConfig::new("10001", "test-secret").with_token_url("   ");
    let tokens = TokenManager::with_clock(
        Arc::clone(&transport),
        config.clone(),
        Arc::clone(&clock) as Arc<dyn Clock>,
    );

    assert_eq!(tokens.token().await.unwrap(), "tok-1");
    assert!(config.token_url().is_none(), "空白 URL 等价于未配置");
    assert_eq!(
        config.endpoint(),
        "https://api.bot.qq.com/app/getAppAccessToken"
    );
    let request = transport.last_request();
    assert_eq!(request.method, HttpMethod::Post);
    assert_eq!(request.url, "https://api.bot.qq.com/app/getAppAccessToken");
    assert!(!request.body.is_empty());
}

#[test]
fn reqwest_transport_without_provider_returns_tls_error() {
    // 测试进程从不安装 rustls CryptoProvider，`new()` 必须落到 Error::Tls 而不是 panic。
    let error = ReqwestTransport::new().expect_err("未安装 provider 时必须返回错误");
    assert!(matches!(error, Error::Tls(_)), "{error:?}");
}
