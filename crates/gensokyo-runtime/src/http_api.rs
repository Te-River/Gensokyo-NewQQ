//! 正向 HTTP API（Go `httpapi/httpapi.go` 的 `NoRoute` 入口 + `main.go` 的 `hr` 服务）。
//!
//! - `POST` 任意路径，body 是一个 OneBot action 请求 JSON，响应即 [`gensokyo_protocol::ActionResponse`]；
//! - `GET /health` 返回健康检查文本，**不鉴权**（只含服务名，无敏感信息，方便探活）；
//! - `GET /metrics` 返回 Prometheus 文本（`text/plain; version=0.0.4`），**不鉴权**（同 Go
//!   `main.go:550` 挂在 HTTP API listener 上的那份）；
//! - `GET /` 未挂 WebUI 时同样回健康检查文本；挂 WebUI 后 `/` 归前端首页，探活固定走
//!   `/health`（见下文「挂 WebUI 后的优先级」）；
//! - `POST /url` 与 `GET /url/{code}`：短链服务（Go `main.go:566-567`），**不鉴权**——与 Go
//!   一致，只按 `lotus_password` 的 token 规则把关，见 [`short_url_router`]；
//! - `http_access_token` 非空时，`Authorization: Bearer <token>` 或 `?access_token=` 必须匹配，
//!   否则 **403** `{"error":"鉴权失败"}`（Go `httpapi/httpapi.go:23-31` 从不回 401）；
//! - 非 `POST` 的 action 请求回 405。
//!
//! 与 Go 的差异：Go 在 `NoRoute` 里按路径分派 `/send_group_msg` 等专用 handler，并支持 GET
//! 查询参数；M1h-a 只保留「body 即 action JSON」这一条统一入口，专用路由随 M1i 的 action
//! 实现再补。
//!
//! # 挂 WebUI 后的优先级（[`with_webui_router`]，M3 起）
//!
//! WebUI 接管根路径与 `/api`，action 不能再做「兜底回退」，否则任何 `GET` 都会落到 action
//! 的 405。改由 [`action_gate`] 这层中间件先判优先级：
//!
//! 1. `POST` 且路径不在 `/api`、`/api/**` 下 → action（保持「`POST` 任意路径即 action」）；
//! 2. 其余一律交给 WebUI：`GET /` 静态页、`GET /api/*` 查询、`POST /api/login|logout` 登录，
//!    WebUI 自己的静态回退只认 `GET`/`HEAD`，未注册的 `/api` 回 404。
//!
//! 也就是说 `/api` 是 WebUI 的保留前缀：`POST /api/login` 是登录而不是同名 action。
//! 未挂 WebUI（`disable_webui=true`）时仍是 M1h-a 的路由表，`POST /api/**` 照旧走 action。

use std::collections::HashMap;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Form, FromRequest, Multipart, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use gensokyo_storage::hex_lower;
use gensokyo_storage::short_url::{is_malicious, redirect_target, short_url_link, validate_url};
use serde::Deserialize;
use serde_json::json;

use crate::auth::is_authorized;
use crate::dispatcher::dispatch_json;
use crate::state::AppState;

/// action 请求体上限，与 `Bytes` 提取器（axum `DEFAULT_LIMIT`）的 2 MiB 对齐。
///
/// action 门自己要读 body，不再经过提取器，故显式带上同一上限。
const MAX_ACTION_BODY: usize = 2 * 1024 * 1024;

/// 健康检查正文；`/health` 与未挂 WebUI 时的 `/` 共用。
const HEALTH_BODY: &str = "gensokyo-runtime: ok";

/// Prometheus 文本格式的 Content-Type（与 Go `MetricsHandler` 显式设置的一致）。
const METRICS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// 独立健康检查路由：`GET /health` → 200 `text/plain`，不鉴权、不进 action 分派。
///
/// 挂 WebUI 后 `/` 被前端首页占用，探活只能固定走这条路径。WebUI 的静态回退只处理
/// **没有匹配到路由**的请求，所以 `/health` 不会被首页兜底吞掉（与注册顺序无关：
/// axum 先查路由表，全部未命中才进 fallback）。
pub(crate) fn health_router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route("/health", get(health))
}

/// Prometheus 指标路由：`GET /metrics` → 200 `text/plain; version=0.0.4`，不鉴权。
///
/// 对应 Go `main.go:550` 挂在 HTTP API listener 上的那份（不带 uploadAuth）。Go 还在主服务器
/// `main.go:522` 挂了一份带 uploadAuth 的；Rust 只在 HTTP API 端口提供——主 listener 缺
/// uploadAuth 的等价中间件，公网暴露消息量不合适，等该中间件落地再补。
pub(crate) fn metrics_router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route("/metrics", get(metrics))
}

/// 短链路由（Go `main.go:566-567` 的 `POST /url` / `GET /url/:shortURL`）。
///
/// 挂在主 listener（Go 的 `r`）；主 listener 缺席时由 [`crate::boot`] 落到 HTTP API 端口。
/// 短链库来自 [`AppState::short_url`]（生产由 `Runtime::attach_short_url` 注入）。
///
/// 与 Go 的差异：
/// - Go 在 `url.init()` 里打开 `gensokyo.db`，失败直接 `panic`；这里打不开只 warn，
///   handler 回 **500**（进程照常启动，其余功能不受影响）。
/// - 301 用 `Location` 手写：axum 的 `Redirect::permanent` 是 308，Go 是 301。
pub(crate) fn short_url_router(state: AppState) -> Router {
    Router::new()
        .route("/url", post(create_short_url))
        .route("/url/{code}", get(redirect_short_url))
        .with_state(state)
}

/// `POST /url` 的表单字段（Go `c.PostForm("url")` / `c.PostForm("token")`）。
///
/// 缺字段按空串处理：Go 的 `PostForm` 读不到同样返回 `""`，随后统一走 400。
#[derive(Debug, Default, Deserialize)]
struct ShortUrlForm {
    #[serde(default)]
    url: String,
    #[serde(default)]
    token: String,
}

/// Gin `c.PostForm` 的两种编码：urlencoded 与 `multipart/form-data`。
///
/// axum 的 [`Form`] 只认前者（0.8 的 `Form` 文档明确指向 [`Multipart`]），所以这里按
/// content-type 分派——Go 的 `initFormCache` 两种都解析，`curl -F` 发的就是 multipart。
async fn parse_short_url_form(request: Request) -> Option<ShortUrlForm> {
    let is_multipart = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("multipart/form-data"));
    if !is_multipart {
        return Form::<ShortUrlForm>::from_request(request, &())
            .await
            .ok()
            .map(|Form(form)| form);
    }

    let mut multipart = Multipart::from_request(request, &()).await.ok()?;
    let mut form = ShortUrlForm::default();
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or_default().to_owned();
        let Ok(value) = field.text().await else {
            continue;
        };
        match name.as_str() {
            "url" => form.url = value,
            "token" => form.token = value,
            _ => {}
        }
    }
    Some(form)
}

/// token 校验（Go `isValidToken`，`url/shorturl.go:303-324`）。
///
/// Go 的实现是**自指**的：拿收到的 `token` 当消息、`lotus_password` 当 key 算一次
/// HMAC-SHA256，再把小写十六进制与 `token` 本身比较（`hex(HMAC(pwd, token)) == token`）。
/// 除哈希不动点外没有任何 token 能通过，因此**密码非空时创建短链恒为 400**；Go 客户端
/// 送的是 `HMAC(pwd, 长链)`，与服务端的检查口径从来不一致。
///
/// 这里照抄实际行为而不是注释意图：Rust 与 Go 混跑同一个短链服务时必须给同样的答案。
/// 想放行就把 `lotus_password` 留空（空密码直接返回 `true`，与 Go 一致）。
///
/// HMAC 用 `ring`（已在依赖图里）算、比较走常量时间；短链库只留纯短码函数。
fn is_valid_token(password: &str, token: &str) -> bool {
    if password.is_empty() {
        return true;
    }
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, password.as_bytes());
    let expected = hex_lower(ring::hmac::sign(&key, token.as_bytes()).as_ref());
    constant_time_eq(expected.as_bytes(), token.as_bytes())
}

/// 常量时间字节比较（长度不同直接 false，长度是公开的 64）。
///
/// `ring::constant_time::verify_slices_are_equal` 已被上游标为 deprecated（不再是稳定 API），
/// 工作区也没有 `subtle`，所以这里就地 XOR 折叠。
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// `POST /url`：校验 → 生成短码 → `{"shortURL": ...}`（Go `CreateShortURLHandler`）。
///
/// 校验顺序与 Go 一致：空串 → `is_malicious` → `validate_url` → token；任何一步不过都是
/// 400 `{"error":"Invalid URL or token"}`（Go 的原文，前端/脚本按它判错）。
async fn create_short_url(State(state): State<AppState>, request: Request) -> Response {
    // 读不到表单（缺 content-type、body 不是表单）时按空表单处理，与 Go 读不到字段等价。
    let form = parse_short_url_form(request).await.unwrap_or_default();
    let long_url = decode_base64_if_needed(&form.url);

    if long_url.is_empty() || is_malicious(&long_url) {
        return invalid_short_url();
    }
    if let Err(rejection) = validate_url(&long_url) {
        tracing::debug!(%rejection, "短链入参被拒绝");
        return invalid_short_url();
    }
    let password = state.config().snapshot().get_lotus_password().to_owned();
    if !is_valid_token(&password, &form.token) {
        tracing::debug!("短链 token 校验失败（Go 的 is_valid_token 自指规则，见 short_url 文档）");
        return invalid_short_url();
    }

    let Some(store) = state.short_url() else {
        tracing::warn!("短链库不可用，POST /url 无法写库");
        return short_url_store_unavailable();
    };
    match store.create(&long_url) {
        Ok(code) => {
            let server_dir = state.config().snapshot().get_server_dir().to_owned();
            (
                StatusCode::OK,
                axum::Json(json!({ "shortURL": short_url_link(&server_dir, &code) })),
            )
                .into_response()
        }
        Err(err) => {
            tracing::warn!(error = %err, "短链写库失败");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(json!({ "error": "短链生成失败" })),
            )
                .into_response()
        }
    }
}

/// `GET /url/{code}`：查不到 404，命中 301 + `Location`（Go `RedirectFromShortURLHandler`）。
async fn redirect_short_url(State(state): State<AppState>, Path(code): Path<String>) -> Response {
    let Some(store) = state.short_url() else {
        tracing::warn!("短链库不可用，GET /url 无法查询");
        return short_url_store_unavailable();
    };
    match store.resolve(&code) {
        Ok(Some(long_url)) => match HeaderValue::from_str(&redirect_target(&long_url)) {
            Ok(location) => (
                [(header::LOCATION, location)],
                StatusCode::MOVED_PERMANENTLY,
            )
                .into_response(),
            Err(err) => {
                // Go 会把原始串直接写进头；Rust 的 HeaderValue 只收可见 ASCII，非 ASCII
                // 目标（极少见，且创建时已被 validate_url 收窄）降级为 500 而不是丢头。
                tracing::warn!(error = %err, "短链跳转目标不是合法 header 值");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(json!({ "error": "跳转目标非法" })),
                )
                    .into_response()
            }
        },
        Ok(None) => (
            StatusCode::NOT_FOUND,
            axum::Json(json!({ "error": "URL not found" })),
        )
            .into_response(),
        Err(err) => {
            tracing::warn!(error = %err, "短链查询失败");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(json!({ "error": "短链查询失败" })),
            )
                .into_response()
        }
    }
}

/// Go 的 400 文案（`CreateShortURLHandler`）——前端/脚本按它判错，别翻译。
fn invalid_short_url() -> Response {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(json!({ "error": "Invalid URL or token" })),
    )
        .into_response()
}

/// 短链库打不开时的 500（Go 在 `url.init()` 里 `panic`，这里降级为单个接口不可用）。
fn short_url_store_unavailable() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        axum::Json(json!({ "error": "短链库不可用" })),
    )
        .into_response()
}

/// Go `decodeBase64IfNeeded`（`url/shorturl.go:123-131`）：`len % 4 == 0` 且能按标准
/// base64 解码时用解码结果，否则原样返回。
///
/// Go 的 `string(decoded)` 不校验 UTF-8；Rust 的 `String` 必须合法，解出乱码时保留原串。
fn decode_base64_if_needed(input: &str) -> String {
    if !input.len().is_multiple_of(4) {
        return input.to_owned();
    }
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, input)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_else(|| input.to_owned())
}

/// 正向 HTTP API 路由；`http_address` 的判定在 [`crate::boot::Runtime`]。
pub(crate) fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(health).post(action))
        .merge(health_router())
        .merge(metrics_router())
        .fallback(action)
        .with_state(state)
}

/// 把 WebUI 路由挂到 HTTP API 端口：`POST`（非 `/api`）仍走 action，其余进 WebUI。
///
/// 中间件包住路由与回退（axum 的 `Router::layer` 同时作用于两者），所以门能拦住本该由
/// WebUI 静态回退处理的 `POST`——那是 action 的唯一去处。后 merge 进来的 webhook 路由不会
/// 被这层包住，`POST webhook_path` 仍是回调（见 [`crate::boot`]）。
pub(crate) fn with_webui_router(webui: Router, state: AppState) -> Router {
    webui
        .merge(health_router())
        .merge(metrics_router())
        .layer(middleware::from_fn_with_state(state, action_gate))
}

async fn health() -> &'static str {
    HEALTH_BODY
}

/// Prometheus 文本；指标集合与数值格式见 [`gensokyo_core::telemetry::metrics`]。
async fn metrics() -> Response {
    (
        [(header::CONTENT_TYPE, METRICS_CONTENT_TYPE)],
        gensokyo_core::telemetry::metrics::render_prometheus_text(),
    )
        .into_response()
}

async fn action(
    State(state): State<AppState>,
    method: Method,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if method != Method::POST {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            axum::Json(json!({ "error": "只接受 POST" })),
        )
            .into_response();
    }

    dispatch_action(&state, &params, &headers, body).await
}

/// action 的统一处理：鉴权 → dispatch → JSON 响应。
///
/// `fallback`（未挂 WebUI）与 [`action_gate`]（挂了 WebUI）共用这一条实现，避免两套鉴权
/// 与错误码。
async fn dispatch_action(
    state: &AppState,
    params: &HashMap<String, String>,
    headers: &HeaderMap,
    body: Bytes,
) -> Response {
    let expected_token = state.config().snapshot().get_access_token().to_owned();
    if !is_authorized(
        &expected_token,
        headers,
        params.get("access_token").map(String::as_str),
    ) {
        tracing::debug!("HTTP API 鉴权失败");
        return auth_failed();
    }

    let response = dispatch_json(state.dispatcher(), body.to_vec()).await;
    ([(header::CONTENT_TYPE, "application/json")], response).into_response()
}

/// Go `httpapi/httpapi.go:23-31`：缺 token 与 token 错都是 403，从不回 401。
fn auth_failed() -> Response {
    (
        StatusCode::FORBIDDEN,
        axum::Json(json!({ "error": "鉴权失败" })),
    )
        .into_response()
}

/// 优先级判据：`POST` 且不落在 WebUI 的 `/api` 前缀下才是 action。
async fn action_gate(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if request.method() != Method::POST || is_webui_path(request.uri().path()) {
        return next.run(request).await;
    }

    let (parts, body) = request.into_parts();
    // 查询参数仍要参与鉴权：`POST /get_status?access_token=...` 与未挂 WebUI 时行为一致。
    let params = Query::<HashMap<String, String>>::try_from_uri(&parts.uri)
        .map(|Query(params)| params)
        .unwrap_or_default();
    let body = match axum::body::to_bytes(body, MAX_ACTION_BODY).await {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::debug!(error = %err, "HTTP API 读取请求体失败");
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({ "error": "读取请求体失败" })),
            )
                .into_response();
        }
    };

    dispatch_action(&state, &params, &parts.headers, body).await
}

/// `/api` 与 `/api/**` 归 WebUI（登录、登出、配置查询）。
fn is_webui_path(path: &str) -> bool {
    path == "/api" || path.starts_with("/api/")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::extract::Request;
    use axum::http::StatusCode;
    use gensokyo_config::ConfigManager;
    use gensokyo_storage::ShortUrlStore;
    use gensokyo_webui::{WebUi, WebUiState};
    use serde_json::Value;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use super::{hex_lower, is_valid_token, is_webui_path, short_url_router};
    use crate::state::AppState;

    #[test]
    fn api_prefix_belongs_to_webui_only_as_a_path_segment() {
        assert!(is_webui_path("/api"));
        assert!(is_webui_path("/api/login"));
        assert!(is_webui_path("/api/config"));
        // 只是以 /api 开头的路径仍是 action 路径（`POST /apixxx` 不是 WebUI）。
        assert!(!is_webui_path("/apixxx"));
        assert!(!is_webui_path("/get_status"));
        assert!(!is_webui_path("/"));
    }

    /// `server_dir` 决定短链前缀；`yaml_extra` 追加 `settings` 直接子项（如 `lotus_password`）。
    fn config(yaml_extra: &str) -> (TempDir, Arc<ConfigManager>) {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(
            &path,
            format!("settings:\n  server_dir: 1.2.3.4:5700\n{yaml_extra}"),
        )
        .expect("写配置");
        let manager = Arc::new(ConfigManager::new(&path).expect("解析配置"));
        (dir, manager)
    }

    fn short_app(config: &Arc<ConfigManager>, dir: &TempDir) -> Router {
        let store = ShortUrlStore::open(dir.path().join("short-urls.redb")).expect("打开短链库");
        short_url_router(
            AppState::with_builtin_handlers(Arc::clone(config)).with_short_url(Arc::new(store)),
        )
    }

    fn post_form(uri: &str, body: &str) -> Request {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body.to_owned()))
            .expect("构造表单请求")
    }

    fn get_request(uri: &str) -> Request {
        Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .expect("构造 GET 请求")
    }

    async fn json_body(response: axum::response::Response) -> Value {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("读响应体");
        serde_json::from_slice(&bytes).expect("响应不是 JSON")
    }

    #[tokio::test]
    async fn post_returns_short_link_and_get_redirects_with_301() {
        let (dir, config) = config("");
        let app = short_app(&config, &dir);

        let response = app
            .clone()
            .oneshot(post_form("/url", "url=https%3A%2F%2Fexample.com%2Fa"))
            .await
            .expect("创建短链");
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        let link = body["shortURL"].as_str().expect("shortURL 字段").to_owned();
        assert!(
            link.starts_with("https://1.2.3.4:5700/url/"),
            "短链前缀应与 Go 一致: {link}"
        );
        let code = link.rsplit('/').next().expect("短码").to_owned();
        assert_eq!(code.len(), 6, "短码长度与 Go 一致: {code}");

        let response = app
            .oneshot(get_request(&format!("/url/{code}")))
            .await
            .expect("跳转");
        assert_eq!(response.status(), StatusCode::MOVED_PERMANENTLY);
        assert_eq!(
            response.headers()["location"],
            "https://example.com/a",
            "301 必须带 Location"
        );
    }

    #[tokio::test]
    async fn invalid_malicious_and_internal_targets_are_bad_request() {
        let (dir, config) = config("");
        let app = short_app(&config, &dir);

        // 空串 / 缺协议 / XSS 协议 / HTML 标签 / 回环 / 私网 / 缺 url 字段：一律 400。
        for body in [
            "url=",
            "url=example.com",
            "url=javascript%3Aalert(1)",
            "url=https%3A%2F%2Fexample.com%2F%3Cscript%3E",
            "url=http%3A%2F%2Flocalhost%2F",
            "url=http%3A%2F%2F192.168.1.1%2F",
            "token=abc",
        ] {
            let response = app
                .clone()
                .oneshot(post_form("/url", body))
                .await
                .expect("请求");
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
            let json = json_body(response).await;
            assert_eq!(json["error"], "Invalid URL or token", "{body}");
        }

        // 没有 content-type / 空 body：Go 的 PostForm 读不到字段，同样 400。
        let empty = Request::builder()
            .method("POST")
            .uri("/url")
            .body(Body::empty())
            .expect("构造空请求");
        assert_eq!(
            app.oneshot(empty).await.expect("请求").status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn unknown_code_is_not_found() {
        let (dir, config) = config("");
        let app = short_app(&config, &dir);

        let response = app
            .oneshot(get_request("/url/zzzzzz"))
            .await
            .expect("查询未知短码");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(json_body(response).await["error"], "URL not found");
    }

    #[tokio::test]
    async fn base64_and_multipart_inputs_are_accepted_like_gin() {
        let (dir, config) = config("");
        let app = short_app(&config, &dir);

        // Go `decodeBase64IfNeeded`：长度是 4 的倍数且能解码时用解码结果。
        let encoded = "aHR0cHM6Ly9leGFtcGxlLmNvbS9iNjQ="; // https://example.com/b64
        let response = app
            .clone()
            .oneshot(post_form("/url", &format!("url={encoded}")))
            .await
            .expect("base64 创建");
        assert_eq!(response.status(), StatusCode::OK);
        let code = json_body(response).await["shortURL"]
            .as_str()
            .expect("shortURL")
            .rsplit('/')
            .next()
            .expect("短码")
            .to_owned();
        let response = app
            .clone()
            .oneshot(get_request(&format!("/url/{code}")))
            .await
            .expect("跳转");
        assert_eq!(
            response.headers()["location"],
            "https://example.com/b64",
            "base64 入参应按解码后的长链落库"
        );

        // Gin 的 PostForm 同样收 multipart/form-data。
        let boundary = "gensokyo-short-url";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"url\"\r\n\r\n\
             https://example.com/form\r\n--{boundary}--\r\n"
        );
        let request = Request::builder()
            .method("POST")
            .uri("/url")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .expect("构造 multipart 请求");
        assert_eq!(
            app.oneshot(request).await.expect("multipart").status(),
            StatusCode::OK
        );
    }

    #[test]
    fn token_check_reproduces_the_go_self_referential_rule() {
        // 空密码：任何 token 都算通过（Go 直接返回 true）。
        assert!(is_valid_token("", ""));
        assert!(is_valid_token("", "whatever"));

        // 非空密码：Go 比较的是 hex(HMAC(pwd, token)) == token，正常 token 恒不通过。
        let password = "lotus-secret";
        assert!(!is_valid_token(password, ""));
        assert!(!is_valid_token(password, "abc123"));
        // Go 客户端会送 HMAC(pwd, 长链)，服务端仍然拒绝——这就是自指 bug 的可见后果。
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, password.as_bytes());
        let client_token = hex_lower(ring::hmac::sign(&key, b"https://example.com/a").as_ref());
        assert!(!is_valid_token(password, &client_token));

        // 比较口径没写反：实现算的是 hex(HMAC(pwd, token))，标准 HMAC-SHA256 固定 64 字符，
        // 不会与对自身的摘要相等，所以密码非空时不存在能通过的 token。
        assert_eq!(client_token.len(), 64);
        assert_ne!(
            hex_lower(ring::hmac::sign(&key, client_token.as_bytes()).as_ref()),
            client_token
        );
    }

    #[tokio::test]
    async fn non_empty_lotus_password_rejects_every_token() {
        let (dir, config) = config("  lotus_password: lotus-secret\n");
        let app = short_app(&config, &dir);

        for body in [
            "url=https%3A%2F%2Fexample.com%2Fa",
            "url=https%3A%2F%2Fexample.com%2Fa&token=abc123",
        ] {
            let response = app
                .clone()
                .oneshot(post_form("/url", body))
                .await
                .expect("请求");
            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "Go 的 is_valid_token 自指比较使密码非空时恒拒绝: {body}"
            );
            assert_eq!(json_body(response).await["error"], "Invalid URL or token");
        }
    }

    #[tokio::test]
    async fn short_url_routes_win_over_the_action_fallback() {
        let (dir, config) = config("");
        let app = crate::http_api::router(AppState::with_builtin_handlers(Arc::clone(&config)))
            .merge(short_app(&config, &dir));

        // 未合并时 `GET /url/x` 会落到 action 回退（405）；合并后必须先命中短链路由。
        let response = app
            .clone()
            .oneshot(get_request("/url/abcdef"))
            .await
            .expect("跳转");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let response = app
            .oneshot(post_form("/url", "url=https%3A%2F%2Fexample.com%2Fmerged"))
            .await
            .expect("创建");
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// 生产装配（`boot.rs::http_app`）：WebUI 路由先包上 `action_gate`，`/url` 再 merge 进来。
    /// 门只作用于被它包住的路由，短链因此既不被 action 吞掉、也不落 WebUI 的 SPA 回退。
    #[tokio::test]
    async fn url_routes_merged_after_the_action_gate_are_not_wrapped_by_it() {
        let (dir, config) = config("");
        let state = AppState::with_builtin_handlers(Arc::clone(&config));
        let webui = WebUi::router(Arc::new(WebUiState::new(Arc::clone(&config))));
        let app = crate::http_api::with_webui_router(webui, state).merge(short_app(&config, &dir));

        // `action_gate` 把「POST 且非 /api」转成 action，无 `access_token` 会回 403；
        // 200 证明 `POST /url` 在门之外。
        let response = app
            .clone()
            .oneshot(post_form("/url", "url=https%3A%2F%2Fexample.com%2Fa"))
            .await
            .expect("创建");
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "POST /url 不得被 action 门拦下"
        );

        // 未知短码必须在短链路由里回 404，而不是落 WebUI 静态回退（那是 200 + 首页）。
        let response = app
            .oneshot(get_request("/url/zzzzzz"))
            .await
            .expect("查询未知短码");
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "GET /url/{{code}} 必须命中短链路由，而不是 WebUI 的 SPA 回退"
        );
    }
}
