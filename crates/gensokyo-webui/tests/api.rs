//! WebUI 管理接口的端到端测试：`Router` 直接吃 `Request`（`tower::ServiceExt::oneshot`），
//! 不监听端口、不访问外网。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use gensokyo_config::ConfigManager;
use gensokyo_webui::{WebUi, WebUiState};
use serde_json::Value;
use tower::ServiceExt;

const USERNAME: &str = "admin";
const PASSWORD: &str = "s3cret-pass";
const TOKEN_SECRET: &str = "super-secret-token";
const CLIENT_SECRET: &str = "super-secret-client";

const CONFIG_YAML: &str = r#"version: 1
settings:
  app_id: 102001234
  port: "5700"
  enable_ws_server: true
  ws_server_path: /ws
  arch_mode: shadow
  disable_webui: false
  oss_type: 4
  server_user_name: admin
  server_user_password: s3cret-pass
  token: super-secret-token
  client_secret: super-secret-client
"#;

/// 测试用临时目录，`Drop` 时删除（不引入 `tempfile` 之外的额外依赖）。
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let path = std::env::temp_dir().join(format!(
            "gensokyo-webui-{}-{nanos}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("创建临时目录失败");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct TestApp {
    _dir: TempDir,
    router: Router,
    state: Arc<WebUiState>,
}

impl TestApp {
    /// `ttl = None` 用生产的 24h；`Some(Duration::ZERO)` 用来直接走过期路径。
    fn new(ttl: Option<Duration>) -> Self {
        let dir = TempDir::new();
        let path = dir.path().join("config.yml");
        std::fs::write(&path, CONFIG_YAML).expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载测试配置失败"));
        let state = Arc::new(match ttl {
            Some(ttl) => WebUiState::with_ttl(manager, ttl),
            None => WebUiState::new(manager),
        });
        let router = WebUi::router(Arc::clone(&state));
        Self {
            _dir: dir,
            router,
            state,
        }
    }
}

async fn send(router: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, String) {
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("路由不应拒绝请求");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("读取响应体失败");
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn get(path: &str) -> Request<Body> {
    Request::builder()
        .uri(path)
        .body(Body::empty())
        .expect("构造 GET 请求失败")
}

fn get_with_cookie(path: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .uri(path)
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .expect("构造带 cookie 的 GET 请求失败")
}

fn post_json(path: &str, body: &str) -> Request<Body> {
    post_json_with_cookie(path, body, None)
}

fn post_json_with_cookie(path: &str, body: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .uri(path)
        .method("POST")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder
        .body(Body::from(body.to_owned()))
        .expect("构造 POST 请求失败")
}

fn set_cookie_raw(headers: &HeaderMap) -> &str {
    headers
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("缺少 Set-Cookie")
}

/// 登录并返回可直接放进 `Cookie` 头的 `login_cookie=<token>`。
async fn login_cookie(app: &TestApp) -> String {
    let (status, headers, body) = send(
        &app.router,
        post_json(
            "/api/login",
            &format!(r#"{{"username":"{USERNAME}","password":"{PASSWORD}"}}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "登录应成功，body={body}");
    set_cookie_raw(&headers)
        .split(';')
        .next()
        .expect("Set-Cookie 格式非法")
        .trim()
        .to_owned()
}

#[tokio::test]
async fn index_and_spa_routes_return_html() {
    let app = TestApp::new(None);
    for path in ["/", "/dashboard"] {
        let (status, headers, body) = send(&app.router, get(path)).await;
        assert_eq!(status, StatusCode::OK, "GET {path}");
        let content_type = headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert!(
            content_type.starts_with("text/html"),
            "GET {path} 的 Content-Type 应为 HTML，实际 {content_type}"
        );
        assert!(!body.is_empty(), "GET {path} 响应体不能为空");
    }
}

#[tokio::test]
async fn missing_asset_and_unknown_api_are_404() {
    let app = TestApp::new(None);
    for path in ["/js/definitely-missing.js", "/api/does-not-exist"] {
        let (status, headers, _) = send(&app.router, get(path)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "GET {path}");
        let content_type = headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert!(
            !content_type.starts_with("text/html"),
            "{path} 不得回退 index.html"
        );
    }
}

#[tokio::test]
async fn login_sets_hardened_session_cookie() {
    let app = TestApp::new(None);
    let (status, headers, body) = send(
        &app.router,
        post_json(
            "/api/login",
            &format!(r#"{{"username":"{USERNAME}","password":"{PASSWORD}"}}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, r#"{"code":0}"#);

    let raw = set_cookie_raw(&headers);
    let (name_value, attributes) = raw.split_once(';').expect("Set-Cookie 缺少属性");
    let (name, token) = name_value.split_once('=').expect("Set-Cookie 缺少等号");
    assert_eq!(name, "login_cookie");
    assert_eq!(token.len(), 32, "token 应为 32 位 hex");
    assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
    for attribute in ["Path=/", "HttpOnly", "SameSite=Lax", "Max-Age=86400"] {
        assert!(
            attributes.contains(attribute),
            "缺少属性 {attribute}: {raw}"
        );
    }
}

#[tokio::test]
async fn login_rejects_wrong_or_empty_credentials() {
    let app = TestApp::new(None);
    for body in [
        r#"{"username":"admin","password":"wrong"}"#,
        r#"{"username":"","password":""}"#,
        r#"{"username":"admin","password":""}"#,
        r#"{"username":"nobody","password":"s3cret-pass"}"#,
    ] {
        let (status, headers, payload) = send(&app.router, post_json("/api/login", body)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "body={body}");
        assert_eq!(payload, r#"{"code":1}"#);
        assert!(
            headers.get(header::SET_COOKIE).is_none(),
            "登录失败不得下发 cookie"
        );
    }
    assert!(app.state.sessions().is_empty());
}

#[tokio::test]
async fn cookie_status_and_logout() {
    let app = TestApp::new(None);

    let (status, _, payload) = send(&app.router, get("/api/checkLoginStatus")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(payload, r#"{"code":1}"#);

    let cookie = login_cookie(&app).await;
    let (status, _, payload) = send(
        &app.router,
        get_with_cookie("/api/checkLoginStatus", &cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload, r#"{"code":0}"#);

    let (status, headers, payload) = send(
        &app.router,
        post_json_with_cookie("/api/logout", "{}", Some(&cookie)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload, r#"{"code":0}"#);
    assert!(
        set_cookie_raw(&headers).contains("Max-Age=0"),
        "logout 必须清除 cookie"
    );

    let (status, _, _) = send(
        &app.router,
        get_with_cookie("/api/checkLoginStatus", &cookie),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "旧 token 必须失效");
    assert!(app.state.sessions().is_empty());
}

#[tokio::test]
async fn config_requires_login_and_hides_secrets() {
    let app = TestApp::new(None);

    let (status, _, _) = send(&app.router, get("/api/config")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "未登录不得读配置");

    let cookie = login_cookie(&app).await;
    let (status, _, body) = send(&app.router, get_with_cookie("/api/config", &cookie)).await;
    assert_eq!(status, StatusCode::OK);
    let value: Value = serde_json::from_str(&body).expect("配置响应必须是 JSON");
    assert_eq!(value["app_id"], 102_001_234_u64);
    assert_eq!(value["port"], "5700");
    assert_eq!(value["enable_ws_server"], true);
    assert_eq!(value["ws_server_path"], "/ws");
    assert_eq!(value["arch_mode"], "shadow");
    assert_eq!(value["disable_webui"], false);
    assert_eq!(value["oss_type"], 4);

    for secret in [TOKEN_SECRET, CLIENT_SECRET, PASSWORD] {
        assert!(!body.contains(secret), "响应体泄露了 {secret}");
    }
    for key in [r#""token""#, r#""client_secret""#, r#""password""#] {
        assert!(!body.contains(key), "{key} 不应出现在响应体中");
    }
}

#[tokio::test]
async fn expired_session_is_rejected_and_cleaned_up() {
    let app = TestApp::new(Some(Duration::ZERO));
    let cookie = login_cookie(&app).await;
    let (status, _, _) = send(
        &app.router,
        get_with_cookie("/api/checkLoginStatus", &cookie),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "TTL=0 的会话必须立即失效");
    assert_eq!(app.state.sessions().cleanup(), 1);
    assert!(app.state.sessions().is_empty());
}
