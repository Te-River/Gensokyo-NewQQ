//! M3 WebUI 挂载：静态页、`/api` 登录，以及「`POST` 任意路径仍是 action」的优先级。
//!
//! 优先级（实现见 `http_api` / `boot` 模块文档）：
//!
//! 1. `POST` 且路径不在 `/api`、`/api/**` 下 → OneBot action；
//! 2. 其余 → WebUI：`GET /` 静态页、`GET /api/*` 查询、`POST /api/login|logout`。
//!
//! `disable_webui=true` 时路由表回到 M1h-a：`GET /` 是健康检查，`POST` 任意路径（含
//! `/api/**`）都是 action。

mod support;

use std::time::Duration;

use gensokyo_runtime::{Runtime, RuntimeHandle, RuntimeOptions};
use serde_json::Value;

/// WebUI 开启（`disable_webui: false`）：`/api` 归 WebUI，其余 `POST` 归 action。
const WEBUI_ON: &str = r#"  port: "5700"
  enable_ws_server: false
  http_address: "127.0.0.1:0"
  http_access_token: ""
  disable_webui: false
  server_user_name: admin
  server_user_password: webui-pass
"#;

/// WebUI 关闭（`disable_webui: true`）：保持 M1h-a 的健康检查 + 全路径 action。
const WEBUI_OFF: &str = r#"  port: "5700"
  enable_ws_server: false
  http_address: "127.0.0.1:0"
  http_access_token: ""
  disable_webui: true
  server_user_name: admin
  server_user_password: webui-pass
"#;

async fn start(settings_yaml: &str, options: RuntimeOptions) -> (RuntimeHandle, tempfile::TempDir) {
    let (state, dir) = support::state(settings_yaml);
    let handle = Runtime::start_with(state, options)
        .await
        .expect("启动运行时失败");
    (handle, dir)
}

async fn stop(handle: RuntimeHandle) {
    tokio::time::timeout(Duration::from_secs(10), handle.shutdown())
        .await
        .expect("shutdown 超时");
}

fn parse_json(response: &str) -> Value {
    serde_json::from_str(support::response_body(response)).expect("响应应为 JSON")
}

/// 响应头（转小写，便于比较 Content-Type 这类字段名大小写不敏感的头）。
fn headers_lower(response: &str) -> String {
    response
        .split_once("\r\n\r\n")
        .map_or(String::new(), |(head, _)| head.to_ascii_lowercase())
}

#[tokio::test]
async fn webui_serves_index_and_login_rejects_empty_credentials() {
    let (handle, _dir) = start(WEBUI_ON, RuntimeOptions::default()).await;
    let addr = handle.http_addr().expect("HTTP 应已监听");

    let (status, response) = support::get(addr, "/").await;
    assert_eq!(status, 200, "GET / 应返回 200: {response}");
    assert!(
        support::response_body(&response).contains("WebUI"),
        "首页应是 WebUI 兜底页: {response}"
    );

    let (status, response) =
        support::post_json(addr, "/api/login", r#"{"username":"","password":""}"#).await;
    assert_eq!(status, 401, "空凭据应回 401: {response}");

    let (status, response) = support::post_json(
        addr,
        "/api/login",
        r#"{"username":"admin","password":"webui-pass"}"#,
    )
    .await;
    assert_eq!(status, 200, "正确凭据应登录成功: {response}");
    assert_eq!(parse_json(&response)["code"], 0, "登录响应: {response}");

    // 未注册的 `/api` 不能落回 SPA（否则前端会把 404 当正常页面）。
    let (status, response) = support::get(addr, "/api/unknown").await;
    assert_eq!(status, 404, "未知 API 应 404: {response}");

    stop(handle).await;
}

/// WebUI 把 `/` 换成前端首页后，`/health` 仍是可探活的纯文本，且不要求 token。
#[tokio::test]
async fn health_stays_text_when_webui_takes_root() {
    let settings = WEBUI_ON.replace(
        "  http_access_token: \"\"",
        "  http_access_token: \"probe-token\"",
    );
    let (handle, _dir) = start(&settings, RuntimeOptions::default()).await;
    let addr = handle.http_addr().expect("HTTP 应已监听");

    let (status, response) = support::get(addr, "/").await;
    assert_eq!(status, 200, "GET / 应返回 200: {response}");
    assert!(
        headers_lower(&response).contains("content-type: text/html"),
        "GET / 应是 HTML: {response}"
    );

    // 配了 http_access_token 也不拦 `/health`：探活不应需要凭据。
    let (status, response) = support::get(addr, "/health").await;
    assert_eq!(status, 200, "GET /health 不应鉴权: {response}");
    assert!(
        headers_lower(&response).contains("content-type: text/plain"),
        "/health 应回 text/plain: {response}"
    );
    let body = support::response_body(&response);
    assert_eq!(body, "gensokyo-runtime: ok", "健康检查正文: {body}");
    assert!(!body.contains("WebUI"), "不应落到首页: {body}");

    stop(handle).await;
}

#[tokio::test]
async fn post_outside_api_still_dispatches_actions() {
    let (handle, _dir) = start(WEBUI_ON, RuntimeOptions::default()).await;
    let addr = handle.http_addr().expect("HTTP 应已监听");

    let (status, response) = support::post_json(
        addr,
        "/get_version_info",
        r#"{"action":"get_version_info","echo":"m3"}"#,
    )
    .await;
    assert_eq!(status, 200, "POST 任意路径仍应进 action: {response}");
    let value = parse_json(&response);
    assert_eq!(value["retcode"], 0, "响应: {response}");
    assert_eq!(value["echo"], "m3");

    // 只有整个 `/api` 段才算 WebUI 前缀：`/apixxx` 仍是 action 路径。
    let (status, response) = support::post_json(addr, "/apixxx", r#"{"action":"nope"}"#).await;
    assert_eq!(status, 200, "响应: {response}");
    assert_eq!(parse_json(&response)["retcode"], 1404, "响应: {response}");

    stop(handle).await;
}

#[tokio::test]
async fn disable_webui_restores_action_semantics() {
    let (handle, _dir) = start(WEBUI_OFF, RuntimeOptions::default()).await;
    let addr = handle.http_addr().expect("HTTP 应已监听");

    let (status, response) = support::get(addr, "/").await;
    assert_eq!(status, 200, "未挂 WebUI 时 GET / 仍是健康检查: {response}");
    let body = support::response_body(&response);
    assert!(
        body.contains("gensokyo-runtime: ok"),
        "健康检查正文: {body}"
    );
    assert!(!body.contains("WebUI"), "不应返回 WebUI 页面: {body}");

    // 独立健康路由在未挂 WebUI 时同样可用。
    let (status, response) = support::get(addr, "/health").await;
    assert_eq!(status, 200, "GET /health 应返回 200: {response}");
    assert_eq!(support::response_body(&response), "gensokyo-runtime: ok");

    // `/api/login` 不再是 WebUI 路由：同名路径落到 action（未知 action 回 1404）。
    let (status, response) = support::post_json(
        addr,
        "/api/login",
        r#"{"action":"nope","username":"admin","password":"webui-pass"}"#,
    )
    .await;
    assert_eq!(status, 200, "响应: {response}");
    let value = parse_json(&response);
    assert_eq!(value["retcode"], 1404, "响应: {response}");
    assert_ne!(value["code"], 0, "不应返回 WebUI 登录成功体: {response}");

    stop(handle).await;
}

#[tokio::test]
async fn webui_is_also_mounted_on_forward_ws_port() {
    // Go 把 WebUI 挂主服务器端口；Rust 的正向 WS 独立监听，同样借出 WebUI 的 GET 面。
    // `ws_server_path` 指到 `/ws`，根路径才归 WebUI（空路径时 WS 自己占着 `/`）。
    let settings = WEBUI_ON
        .replace("enable_ws_server: false", "enable_ws_server: true")
        .replace("http_address: \"127.0.0.1:0\"", "http_address: \"\"")
        .replace(
            "  disable_webui: false",
            "  disable_webui: false\n  ws_server_path: ws",
        );
    let (handle, _dir) = start(
        &settings,
        RuntimeOptions {
            ws_bind: Some("127.0.0.1:0".to_owned()),
            http_bind: None,
        },
    )
    .await;
    let addr = handle.ws_addr().expect("正向 WS 应已监听");

    let (status, response) = support::get(addr, "/").await;
    assert_eq!(status, 200, "WS 端口也应提供 WebUI 首页: {response}");
    assert!(
        support::response_body(&response).contains("WebUI"),
        "首页应是 WebUI 兜底页: {response}"
    );

    // WS 端口同样挂了 WebUI：`/health` 不能被首页兜底吞掉。
    let (status, response) = support::get(addr, "/health").await;
    assert_eq!(status, 200, "WS 端口也应提供 /health: {response}");
    assert_eq!(support::response_body(&response), "gensokyo-runtime: ok");

    stop(handle).await;
}
