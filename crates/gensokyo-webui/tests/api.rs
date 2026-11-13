//! WebUI 管理接口的端到端测试：`Router` 直接吃 `Request`（`tower::ServiceExt::oneshot`），
//! 不监听端口、不访问外网。

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode, header};
use futures::StreamExt as _;
use gensokyo_config::ConfigManager;
use gensokyo_webui::logs::{LogEntry, hub};
use gensokyo_webui::{SESSION_DB_FILE, SESSION_TTL, SessionStore, WebUi, WebUiState};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message as TungMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
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
struct TempDir(std::path::PathBuf);

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

fn write_config(dir: &TempDir) {
    std::fs::write(dir.path().join("config.yml"), CONFIG_YAML).expect("写测试配置失败");
}

fn manager(dir: &TempDir) -> Arc<ConfigManager> {
    write_config(dir);
    let path = dir.path().join("config.yml");
    Arc::new(ConfigManager::new(path).expect("加载测试配置失败"))
}

fn state_with_db(manager: &Arc<ConfigManager>, path: &Path) -> Arc<WebUiState> {
    Arc::new(WebUiState::with_store(
        Arc::clone(manager),
        SessionStore::with_db(SESSION_TTL, path).expect("打开会话库失败"),
    ))
}

struct TestApp {
    _dir: TempDir,
    router: Router,
    state: Arc<WebUiState>,
}

impl TestApp {
    /// `ttl = None` 用生产的 24h；`Some(Duration::ZERO)` 用来直接走过期路径。
    ///
    /// 固定内存存储：生产路径（`WebUiState::new` 把 `cookie.redb` 放配置同目录）由
    /// 两个持久化用例覆盖，这里避免把会话库写进 crate 目录。
    fn new(ttl: Option<Duration>) -> Self {
        let dir = TempDir::new();
        let manager = manager(&dir);
        let state = Arc::new(WebUiState::with_ttl(manager, ttl.unwrap_or(SESSION_TTL)));
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
    json_request("POST", path, body, cookie)
}

fn patch_json(path: &str, body: &str, cookie: &str) -> Request<Body> {
    json_request("PATCH", path, body, Some(cookie))
}

fn json_request(method: &str, path: &str, body: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .uri(path)
        .method(method)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder
        .body(Body::from(body.to_owned()))
        .expect("构造 JSON 请求失败")
}

fn delete_with_cookie(path: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .uri(path)
        .method("DELETE")
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .expect("构造 DELETE 请求失败")
}

fn set_cookie_raw(headers: &HeaderMap) -> &str {
    headers
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("缺少 Set-Cookie")
}

fn json_body(body: &str) -> Value {
    serde_json::from_str(body).expect("响应必须是 JSON")
}

/// 登录并返回可直接放进 `Cookie` 头的 `login_cookie=<token>`。
async fn login_cookie_on(router: &Router) -> String {
    let (status, headers, body) = send(
        router,
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

async fn login_cookie(app: &TestApp) -> String {
    login_cookie_on(&app.router).await
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
    // Go `HandleLoginRequest:332-335`：前端读 `data.isLoggedIn`，`{code:0}` 会被当成登录失败。
    let value = json_body(&body);
    assert_eq!(value["isLoggedIn"], true, "登录响应: {body}");
    let cookie = value["cookie"].as_str().expect("登录响应应带 cookie");

    let raw = set_cookie_raw(&headers);
    let (name_value, attributes) = raw.split_once(';').expect("Set-Cookie 缺少属性");
    let (name, token) = name_value.split_once('=').expect("Set-Cookie 缺少等号");
    assert_eq!(name, "login_cookie");
    assert_eq!(token, cookie, "响应体的 cookie 必须与 Set-Cookie 同值");
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
        assert_eq!(json_body(&payload)["isLoggedIn"], false, "body={body}");
        assert!(
            headers.get(header::SET_COOKIE).is_none(),
            "登录失败不得下发 cookie"
        );
    }
    assert!(app.state.sessions().is_empty());
}

/// 请求体不是合法 JSON：Go `ShouldBindJSON` 回 400 + `{error}`。
#[tokio::test]
async fn login_rejects_malformed_body_with_400() {
    let app = TestApp::new(None);
    let (status, _, payload) = send(&app.router, post_json("/api/login", "not-json")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={payload}");
    assert!(json_body(&payload)["error"].is_string(), "body={payload}");
    assert!(app.state.sessions().is_empty());
}

#[tokio::test]
async fn cookie_status_and_logout() {
    let app = TestApp::new(None);

    // Go 口径：未登录也是 200 + `isLoggedIn:false`，前端只读这个布尔值。
    let (status, _, payload) = send(&app.router, get("/api/checkLoginStatus")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&payload)["isLoggedIn"], false);

    let cookie = login_cookie(&app).await;
    let (status, _, payload) = send(
        &app.router,
        get_with_cookie("/api/checkLoginStatus", &cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&payload)["isLoggedIn"], true);

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

    let (status, _, payload) = send(
        &app.router,
        get_with_cookie("/api/checkLoginStatus", &cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "旧 token 失效后仍是 200");
    assert_eq!(json_body(&payload)["isLoggedIn"], false);
    assert!(app.state.sessions().is_empty());
}

/// Go 的 `/api/check-login-status` 与 M3 的 `/api/checkLoginStatus` 必须同语义同状态码。
#[tokio::test]
async fn both_login_status_paths_are_200_and_agree() {
    let app = TestApp::new(None);
    let paths = ["/api/check-login-status", "/api/checkLoginStatus"];

    for path in paths {
        let (status, _, payload) = send(&app.router, get(path)).await;
        assert_eq!(status, StatusCode::OK, "GET {path} 未登录也应 200");
        let value = json_body(&payload);
        assert_eq!(value["isLoggedIn"], false, "GET {path}");
        assert!(
            value["error"].is_string(),
            "GET {path} 未登录应带 error 说明，实际 {payload}"
        );
    }

    let cookie = login_cookie(&app).await;
    for path in paths {
        let (status, _, payload) = send(&app.router, get_with_cookie(path, &cookie)).await;
        assert_eq!(status, StatusCode::OK, "GET {path}");
        let value = json_body(&payload);
        assert_eq!(value["isLoggedIn"], true, "GET {path}");
        assert!(
            value.get("error").is_none(),
            "GET {path} 已登录不应带 error，实际 {payload}"
        );
    }
}

/// `/api/status` 的字段与 Go `handleSysInfo` 逐字段对齐（前端 `SystemStatus` 直接消费）。
#[tokio::test]
async fn status_mirrors_go_system_info_shape() {
    let app = TestApp::new(None);
    let (status, _, _) = send(&app.router, get("/api/status")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "未登录不得读系统状态");

    let cookie = login_cookie(&app).await;
    let (status, _, body) = send(&app.router, get_with_cookie("/api/status", &cookie)).await;
    assert_eq!(status, StatusCode::OK);
    let value = json_body(&body);

    for key in ["cpu_percent", "memory", "disk", "boot_time", "process"] {
        assert!(!value[key].is_null(), "缺少字段 {key}: {body}");
    }
    for key in ["total", "available", "used", "percent"] {
        assert!(!value["memory"][key].is_null(), "缺少 memory.{key}: {body}");
    }
    for key in ["total", "free", "percent"] {
        assert!(!value["disk"][key].is_null(), "缺少 disk.{key}: {body}");
    }
    for key in ["pid", "status", "memory_used", "cpu_percent", "start_time"] {
        assert!(
            !value["process"][key].is_null(),
            "缺少 process.{key}: {body}"
        );
    }
    assert_eq!(
        value["process"]["pid"].as_u64(),
        Some(u64::from(std::process::id())),
        "process.pid 应是当前进程"
    );
    assert_eq!(value["process"]["status"], "running");
    assert!(value["cpu_percent"].is_number());
    assert!(value["boot_time"].is_number());
}

/// `/api/accounts`：Go 只回配置里的那一个 AppID。
#[tokio::test]
async fn accounts_lists_the_configured_app_id() {
    let app = TestApp::new(None);
    let (status, _, _) = send(&app.router, get("/api/accounts")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "未登录不得读账号列表");

    let cookie = login_cookie(&app).await;
    let (status, _, body) = send(&app.router, get_with_cookie("/api/accounts", &cookie)).await;
    assert_eq!(status, StatusCode::OK);
    let value = json_body(&body);
    let list = value.as_array().expect("账号列表应是数组");
    assert_eq!(list.len(), 1, "只配置了一个 AppID: {body}");
    assert_eq!(list[0]["uin"], 102_001_234_u64);
    assert_eq!(list[0]["predefined"], false);
    assert_eq!(list[0]["process_created"], true);
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

/// 账号配置路由的 uin 段（Go 用 `appIDStr` 拼路径，换一个值就落不到任何分支）。
const UIN: u64 = 102_001_234;

fn account_path(suffix: &str) -> String {
    format!("/api/{UIN}/{suffix}")
}

/// `GET /api/{uin}/config`：Go `AccountConfigReadHandler` 回 `config.yml` 原文。
#[tokio::test]
async fn account_config_read_returns_file_content() {
    let app = TestApp::new(None);
    let path = account_path("config");

    let (status, _, _) = send(&app.router, get(&path)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "未登录不得读 config.yml");

    let cookie = login_cookie(&app).await;
    let (status, _, body) = send(&app.router, get_with_cookie(&path, &cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json_body(&body)["content"],
        CONFIG_YAML,
        "GET 应回配置文件原文"
    );

    // 其它 uin 不是 Go 拼出来的路径 → 404（不能读别人的配置）
    let (status, _, _) = send(&app.router, get_with_cookie("/api/1/config", &cookie)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// `PATCH /api/{uin}/config`：写盘 + 热重载，热字段立刻进快照。
#[tokio::test]
async fn account_config_patch_writes_file_and_applies_hot_fields() {
    let app = TestApp::new(None);
    let cookie = login_cookie(&app).await;
    let path = account_path("config");
    let content = CONFIG_YAML.replace("arch_mode: shadow", "arch_mode: legacy");

    let (status, _, body) = send(
        &app.router,
        patch_json(&path, &json!({ "content": content }).to_string(), &cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let value = json_body(&body);
    assert_eq!(value["message"], "Config updated successfully");
    assert_eq!(
        value["content"], content,
        "回包要带回原文：前端保存后把它写回编辑器"
    );

    assert_eq!(
        std::fs::read_to_string(app.state.config().path()).expect("读回配置失败"),
        content
    );
    assert_eq!(
        app.state.config().snapshot().get_arch_mode(),
        "legacy",
        "热字段应立刻生效"
    );
}

/// 重启项（Go `restartRequiredFields`）只落盘不生效——与 Go"改完要重启"同口径。
#[tokio::test]
async fn account_config_patch_defers_restart_fields() {
    let app = TestApp::new(None);
    let cookie = login_cookie(&app).await;
    let path = account_path("config");
    let content = CONFIG_YAML.replace(r#"port: "5700""#, r#"port: "5701""#);

    let (status, _, body) = send(
        &app.router,
        patch_json(&path, &json!({ "content": content }).to_string(), &cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        app.state.config().snapshot().port,
        "5700",
        "重启项不进入快照"
    );
    assert!(
        std::fs::read_to_string(app.state.config().path())
            .expect("读回配置失败")
            .contains(r#"port: "5701""#),
        "重启项仍要落盘"
    );
}

/// 坏 YAML 挡在写盘之前：Go 是写完直接重启，坏文件会让进程起不来。
#[tokio::test]
async fn account_config_patch_rejects_broken_yaml_without_touching_disk() {
    let app = TestApp::new(None);
    let cookie = login_cookie(&app).await;
    let path = account_path("config");

    let (status, _, body) = send(
        &app.router,
        patch_json(&path, r#"{"content":"settings: [unclosed"}"#, &cookie),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert!(json_body(&body)["error"].is_string(), "body={body}");
    assert_eq!(
        std::fs::read_to_string(app.state.config().path()).expect("读回配置失败"),
        CONFIG_YAML,
        "被拒绝的写入不得改动文件"
    );
}

/// `DELETE /api/{uin}/config`：删档后按首启模板重建（Go `DeleteConfig`）。
#[tokio::test]
async fn account_config_delete_rebuilds_template() {
    let app = TestApp::new(None);
    let cookie = login_cookie(&app).await;
    let path = account_path("config");

    let (status, _, body) = send(&app.router, delete_with_cookie(&path, &cookie)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "body={body}");

    let regenerated = std::fs::read_to_string(app.state.config().path()).expect("模板应被重建");
    assert!(
        regenerated.starts_with("version: 1\nsettings:\n"),
        "应是首启模板: {regenerated}"
    );
    assert!(!regenerated.contains("102001234"), "旧配置字段不应残留");

    // uin 取自旧快照（模板差异都是重启项，未热生效），重建后仍可读
    let (status, _, body) = send(&app.router, get_with_cookie(&path, &cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&body)["content"], regenerated);
}

/// `GET /api/{uin}/process/status`：Go `HandleProcessStatusRequest` 的完整形状。
#[tokio::test]
async fn process_status_mirrors_go_shape() {
    let app = TestApp::new(None);
    let path = account_path("process/status");

    let (status, _, _) = send(&app.router, get(&path)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "未登录不得读进程状态");

    let cookie = login_cookie(&app).await;
    let (status, _, body) = send(&app.router, get_with_cookie(&path, &cookie)).await;
    assert_eq!(status, StatusCode::OK);
    let value = json_body(&body);
    assert_eq!(value["status"], "running");
    assert_eq!(value["total_logs"], 0, "没有日志缓冲: {body}");
    assert_eq!(value["restarts"], 0, "没有进程守护: {body}");
    assert!(value["qr_uri"].is_null(), "Rust 无扫码登录: {body}");
    for key in ["pid", "status", "memory_used", "cpu_percent", "start_time"] {
        assert!(
            !value["details"][key].is_null(),
            "缺少 details.{key}: {body}"
        );
    }
    assert_eq!(
        value["details"]["pid"].as_u64(),
        Some(u64::from(std::process::id())),
        "details.pid 应是当前进程"
    );

    let (status, _, _) = send(
        &app.router,
        get_with_cookie("/api/1/process/status", &cookie),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "未知 uin 应 404");
}

/// `DELETE /api/{uin}/process`：Go 是 `os.Exit(0)`，Rust 没有守护进程，如实回 501。
#[tokio::test]
async fn process_stop_reports_unavailable_instead_of_exiting() {
    let app = TestApp::new(None);
    let cookie = login_cookie(&app).await;
    let path = account_path("process");

    let (status, _, body) = send(&app.router, delete_with_cookie(&path, &cookie)).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "body={body}");
    assert!(json_body(&body)["error"].is_string(), "body={body}");

    // 进程还活着：紧接着的状态查询仍然成功
    let (status, _, _) = send(
        &app.router,
        get_with_cookie(&account_path("process/status"), &cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

/// `GET /api/{uin}/process/logs` 与 `GET /api/logs`：Go `getProcessLogs` 回 `[]`。
#[tokio::test]
async fn log_history_is_an_empty_list() {
    let app = TestApp::new(None);
    let paths = ["/api/logs".to_owned(), account_path("process/logs")];

    for path in &paths {
        let (status, _, _) = send(&app.router, get(path)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "GET {path} 未登录应 401");
    }

    let cookie = login_cookie(&app).await;
    for path in &paths {
        let (status, _, body) = send(&app.router, get_with_cookie(path, &cookie)).await;
        assert_eq!(status, StatusCode::OK, "GET {path}");
        assert!(
            json_body(&body)
                .as_array()
                .expect("日志历史上限应是数组")
                .is_empty(),
            "GET {path} 应回空数组，实际 {body}"
        );
    }
}

// ---- 日志 WS（P16）：Go `mylog.WsHandlerWithDependencies` ----

/// 明文 `ws://` 客户端类型。
type WsClient =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// 起 loopback listener 并返回 `ws://` 主机段；WS 握手需要真实 TCP，`oneshot` 做不到。
async fn serve_loopback(router: &Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("绑定 loopback 端口失败");
    let addr = listener.local_addr().expect("读取监听地址失败");
    let router = router.clone();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("127.0.0.1:{}", addr.port()), handle)
}

/// 带 cookie 的 WS 握手（浏览器 WS 也自动带 cookie，Go 的中间件正是靠它鉴权）。
async fn connect_logs(
    addr: &str,
    path: &str,
    cookie: Option<&str>,
) -> Result<WsClient, tokio_tungstenite::tungstenite::Error> {
    let mut request = format!("ws://{addr}{path}")
        .into_client_request()
        .expect("构造 WS 请求失败");
    if let Some(cookie) = cookie {
        request.headers_mut().insert(
            "cookie",
            HeaderValue::from_str(cookie).expect("cookie 头非法"),
        );
    }
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(socket, _response)| socket)
}

/// 唯一标记：并行的其它日志用例也会往同一个进程级中枢投递，靠标记筛出自己的那条。
fn log_marker(tag: &str) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!(
        "p16-{tag}-{}-{nanos}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// 读到包含 `marker` 的那条日志（跳过期间其它用例的日志与心跳帧）。
async fn read_log_containing(socket: &mut WsClient, marker: &str) -> Value {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .unwrap_or_else(|_| panic!("等待日志 {marker} 超时"))
            .expect("WS 在收到日志前被关闭")
            .expect("WS 读取失败");
        if let TungMessage::Text(text) = message {
            let value: Value = serde_json::from_str(text.as_str()).expect("日志必须是 JSON");
            if value["message"]
                .as_str()
                .is_some_and(|message| message.contains(marker))
            {
                return value;
            }
        }
    }
}

fn push_log(level: &str, message: &str) {
    hub().push(LogEntry {
        time: "2026/NOV/08 PM 03:04:05".to_owned(),
        level: level.to_owned(),
        message: message.to_owned(),
    });
}

/// 日志 WS：连接先补发缓冲快照、再推实时；两条路由同语义；JSON 字段照 Go `EnhancedLogEntry`。
#[tokio::test]
async fn logs_ws_replays_history_then_pushes_live_entries() {
    let app = TestApp::new(None);
    let cookie = login_cookie(&app).await;
    let history_marker = log_marker("history");
    push_log("INFO", &history_marker);

    let (addr, server) = serve_loopback(&app.router).await;
    let mut socket = connect_logs(&addr, "/api/logs", Some(&cookie))
        .await
        .expect("已登录应能升级 WS");

    let entry = read_log_containing(&mut socket, &history_marker).await;
    let mut keys: Vec<&str> = entry
        .as_object()
        .expect("日志应是 JSON 对象")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["level", "message", "time"],
        "字段照 Go EnhancedLogEntry"
    );
    assert_eq!(entry["time"], "2026/NOV/08 PM 03:04:05");
    assert_eq!(entry["level"], "INFO");
    assert_eq!(entry["message"], history_marker);

    // 收到快照说明会话已订阅，此时再投递一定能收到实时推送。
    let live_marker = log_marker("live");
    push_log("WARN", &live_marker);
    let live = read_log_containing(&mut socket, &live_marker).await;
    assert_eq!(live["level"], "WARN");

    // `/api/{uin}/process/logs` 与 `/api/logs` 同语义（Go `webui/api.go:52-70`）。
    let mut second = connect_logs(&addr, &account_path("process/logs"), Some(&cookie))
        .await
        .expect("账号路径同样应能升级 WS");
    let replayed = read_log_containing(&mut second, &history_marker).await;
    assert_eq!(replayed["message"], history_marker);

    server.abort();
}

/// 未登录的 WS 握手必须被拒（Go 中间件对 `/api/*` 一律校验 cookie）。
#[tokio::test]
async fn logs_ws_requires_login_cookie() {
    let app = TestApp::new(None);
    let (addr, server) = serve_loopback(&app.router).await;

    let error = connect_logs(&addr, "/api/logs", None)
        .await
        .expect_err("未登录必须拒绝升级");
    match error {
        tokio_tungstenite::tungstenite::Error::Http(response) => {
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        other => panic!("期望 HTTP 401，实际 {other:?}"),
    }

    server.abort();
}

/// `POST /api/{uin}/api?name=...`：Go 的 switch 只有 `default: 400 Invalid API name`。
#[tokio::test]
async fn account_api_forward_rejects_every_name() {
    let app = TestApp::new(None);
    let path = account_path("api");
    let uri = format!("{path}?name=get_status");

    let (status, _, _) = send(&app.router, post_json(&uri, "{}")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "未登录不得转发");

    let cookie = login_cookie(&app).await;
    let (status, _, body) = send(
        &app.router,
        post_json_with_cookie(&uri, "{}", Some(&cookie)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert_eq!(json_body(&body)["error"], "Invalid API name");
}

#[tokio::test]
async fn expired_session_is_rejected_and_cleaned_up() {
    let app = TestApp::new(Some(Duration::ZERO));
    let cookie = login_cookie(&app).await;
    let (status, _, payload) = send(
        &app.router,
        get_with_cookie("/api/checkLoginStatus", &cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json_body(&payload)["isLoggedIn"],
        false,
        "TTL=0 必须立即失效"
    );
    assert_eq!(app.state.sessions().cleanup(), 1);
    assert!(app.state.sessions().is_empty());
}

/// 会话落 `cookie.redb`：重建 `WebUiState`（模拟进程重启）后登录态仍在。
#[tokio::test]
async fn session_survives_restart_via_cookie_redb() {
    let dir = TempDir::new();
    let manager = manager(&dir);
    let db = dir.path().join(SESSION_DB_FILE);

    let cookie = {
        let state = state_with_db(&manager, &db);
        let router = WebUi::router(Arc::clone(&state));
        let cookie = login_cookie_on(&router).await;
        assert!(db.exists(), "登录后应生成 {}", SESSION_DB_FILE);
        cookie
    };

    // 用同一个库重新装配 = 进程重启。
    let state = state_with_db(&manager, &db);
    assert_eq!(state.sessions().len(), 1, "重启后会话应从 cookie.redb 恢复");
    let router = WebUi::router(Arc::clone(&state));
    let (status, _, payload) =
        send(&router, get_with_cookie("/api/check-login-status", &cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json_body(&payload)["isLoggedIn"],
        true,
        "重启后登录态应有效"
    );
}

/// 过期会话既不能复活，也不能留在 `cookie.redb` 里。
#[tokio::test]
async fn expired_sessions_are_dropped_from_cookie_redb() {
    let dir = TempDir::new();
    let manager = manager(&dir);
    let db = dir.path().join(SESSION_DB_FILE);

    // 阶段一：cleanup 清掉内存与磁盘，重开不得复活。
    let cleaned = {
        let store = SessionStore::with_db(Duration::ZERO, &db).expect("打开会话库失败");
        let state = Arc::new(WebUiState::with_store(Arc::clone(&manager), store));
        let router = WebUi::router(Arc::clone(&state));
        let cookie = login_cookie_on(&router).await;
        assert_eq!(state.sessions().cleanup(), 1);
        cookie
    };
    {
        let state = state_with_db(&manager, &db);
        assert!(state.sessions().is_empty(), "cleanup 后不得留下过期会话");
        let router = WebUi::router(Arc::clone(&state));
        let (_, _, payload) = send(
            &router,
            get_with_cookie("/api/check-login-status", &cleaned),
        )
        .await;
        assert_eq!(json_body(&payload)["isLoggedIn"], false);
    }

    // 阶段二：不调 cleanup 直接重启，打开会话库时也要丢掉过期行。
    let stale = {
        let store = SessionStore::with_db(Duration::ZERO, &db).expect("打开会话库失败");
        let state = Arc::new(WebUiState::with_store(Arc::clone(&manager), store));
        let router = WebUi::router(Arc::clone(&state));
        login_cookie_on(&router).await
    };
    let state = state_with_db(&manager, &db);
    assert!(
        state.sessions().is_empty(),
        "打开会话库时应把过期行删掉，残留 {stale}"
    );
}
