//! 登录、会话 cookie 与登录态查询。
//!
//! 凭据来自 `config.yml` 的 `server_user_name` / `server_user_password`（与 Go
//! `webui/api.go:checkCredentials` 同源）。会话默认落 redb（`cookie.redb`，与 `config.yml`
//! 同目录，见 [`crate::session_db`]），重启后浏览器无需重新登录；磁盘不可写时降级为
//! 纯内存会话并 `warn`。
//!
//! # 响应体（与 Go 逐条对照）
//!
//! - `POST /api/login`：成功 200 `{isLoggedIn:true,cookie:"<token>"}` + 24h cookie（Go
//!   `webui/api.go:330-335`；前端 `LoginView.vue` 读 `data.isLoggedIn`，M3 的 `{code:0}`
//!   会被当成登录失败）；失败 401 `{isLoggedIn:false}`；请求体不是合法 JSON / 缺字段 → 400
//!   `{error:...}`（Go `ShouldBindJSON`）。
//! - `GET /api/check-login-status`（Go 的路径）与 `GET /api/checkLoginStatus`（M3 旧名）
//!   共用同一个处理函数：**始终 200**，body `{"isLoggedIn":true}` /
//!   `{"isLoggedIn":false,"error":...}`，与 Go `HandleCheckLoginStatusRequest` 一致——前端
//!   `router/index.ts` 与 `LoginView.vue` 只读 `isLoggedIn`，不看 HTTP 状态码。旧名保留，
//!   避免 M3 起的调用方 404。
//! - `POST /api/logout`：Rust 扩展。Go 没有登出接口，会话只能等 24h 过期或删 `cookie.db`。

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::session_db::{SESSION_DB_FILE, SessionDb};
use crate::state::WebUiState;

/// 会话 cookie 名（与 Go `webui/api.go` 的 `login_cookie` 一致）。
pub const SESSION_COOKIE: &str = "login_cookie";

/// 会话有效期 24h（与 Go `SetCookie(..., 3600*24, ...)` 一致）。
pub const SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// 会话存储：`token → 过期时刻`（Unix 毫秒），可选落 redb。
///
/// 本类型不带后台任务：runtime 按固定周期外调 [`SessionStore::cleanup`]
/// （见 `gensokyo-runtime/src/boot.rs` 的 `spawn_session_cleanup`）；在此之前过期条目只是
/// "校验失败"，仍占内存与磁盘。写路径只有 issue / remove / cleanup。
#[derive(Debug)]
pub struct SessionStore {
    sessions: RwLock<HashMap<String, u64>>,
    ttl: Duration,
    db: Option<SessionDb>,
}

impl SessionStore {
    /// 进程内会话（不落盘）。测试与降级路径用。
    #[must_use]
    pub fn new(ttl: Duration) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            ttl,
            db: None,
        }
    }

    /// 打开 `cookie.redb` 并把未过期会话读回内存。
    ///
    /// # Errors
    ///
    /// 库打不开、读事务或过期行清理失败时返回 [`redb::Error`]。
    pub fn with_db(ttl: Duration, path: impl AsRef<Path>) -> Result<Self, redb::Error> {
        let db = SessionDb::open(path.as_ref())?;
        let sessions = db.load(now_ms())?;
        Ok(Self {
            sessions: RwLock::new(sessions),
            ttl,
            db: Some(db),
        })
    }

    /// 生产装配：会话库放 `config.yml` 同目录的 [`SESSION_DB_FILE`]。
    ///
    /// 打不开时降级为纯内存（只 `warn`）：磁盘只读不该让机器人在启动阶段挂掉。
    #[must_use]
    pub fn open_default(ttl: Duration, config_path: &Path) -> Self {
        let path = config_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(SESSION_DB_FILE);
        match Self::with_db(ttl, &path) {
            Ok(store) => {
                tracing::debug!(
                    path = %path.display(),
                    sessions = store.len(),
                    "WebUI 会话库已加载"
                );
                store
            }
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "WebUI 会话库不可用，降级为纯内存会话"
                );
                Self::new(ttl)
            }
        }
    }

    /// 发放新会话并返回 token。
    ///
    /// # Errors
    ///
    /// 会话库写入失败时返回 [`redb::Error`]：此时不创建内存条目，避免向调用方报告一个
    /// 重启就会消失的会话。
    pub fn issue(&self) -> Result<String, redb::Error> {
        let token = generate_token();
        let expires_at = now_ms().saturating_add(ttl_ms(self.ttl));
        if let Some(db) = &self.db {
            db.put(&token, expires_at)?;
        }
        self.write().insert(token.clone(), expires_at);
        Ok(token)
    }

    /// token 存在且未过期。
    #[must_use]
    pub fn validate(&self, token: &str) -> bool {
        let now = now_ms();
        self.read()
            .get(token)
            .is_some_and(|expires_at| *expires_at > now)
    }

    /// 删除会话（登出）；返回是否确实删掉了一个内存条目。
    ///
    /// 落盘失败只 `warn`：内存条目已失效、当前进程内登出立即生效，但重启后该 token 会
    /// 复活（直到 24h 过期）——失败只进日志，因为登出没有可回滚的语义。
    pub fn remove(&self, token: &str) -> bool {
        if let Some(db) = &self.db
            && let Err(err) = db.remove(token)
        {
            tracing::warn!(error = %err, "WebUI 会话删除落盘失败");
        }
        self.write().remove(token).is_some()
    }

    /// 清掉已过期会话（内存 + 磁盘），返回删除条数。
    pub fn cleanup(&self) -> usize {
        let now = now_ms();
        let mut expired = Vec::new();
        let mut sessions = self.write();
        let before = sessions.len();
        sessions.retain(|token, expires_at| {
            let live = *expires_at > now;
            if !live {
                expired.push(token.clone());
            }
            live
        });
        let removed = before - sessions.len();
        drop(sessions);
        if let Some(db) = &self.db {
            for token in &expired {
                if let Err(err) = db.remove(token) {
                    tracing::warn!(error = %err, "WebUI 过期会话删盘失败");
                }
            }
        }
        removed
    }

    /// 当前会话数（含尚未 cleanup 的过期条目）。
    #[must_use]
    pub fn len(&self) -> usize {
        self.read().len()
    }

    /// 是否没有任何会话（含尚未 cleanup 的过期条目）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, u64>> {
        self.sessions
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<String, u64>> {
        self.sessions
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// 当前 Unix 毫秒；系统时间早于 epoch（不可能）时取 0。
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

fn ttl_ms(ttl: Duration) -> u64 {
    ttl.as_millis() as u64
}

/// 32 位 hex 会话 token。
///
/// 不引入 `rand` / `uuid`：`RandomState` 的两把 SipHash 密钥由 OS 熵在进程内播种，
/// 每次 `new()` 还会递增密钥；两轮把「纳秒时间 + 原子计数 + 轮次」摊成 128 bit，对
/// 落盘后的离线猜测同样要求先拿到进程的密钥，与 Go 的 `uuid.New()` 同级。
fn generate_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos() as u64);
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);

    let mut token = String::with_capacity(32);
    for round in 0..2 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(nanos);
        hasher.write_u64(seq);
        hasher.write_u64(round);
        token.push_str(&format!("{:016x}", hasher.finish()));
    }
    token
}

/// 从 `Cookie` 头取会话 token（多条 cookie 时按名匹配）。
pub(crate) fn cookie_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|raw| raw.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == SESSION_COOKIE)
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
}

/// 请求是否携带有效会话 cookie。
pub(crate) fn is_authenticated(state: &WebUiState, headers: &HeaderMap) -> bool {
    cookie_token(headers).is_some_and(|token| state.sessions().validate(token))
}

/// `Set-Cookie` 头；`max_age` 为 0 即清除。
fn set_cookie(token: &str, max_age: u64) -> HeaderValue {
    let value =
        format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}");
    // token 固定为 32 位 hex，其余是静态属性：构造必然合法。
    HeaderValue::from_str(&value).expect("会话 cookie 字面量非法")
}

/// 登录请求体。
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    /// 配置中的 `server_user_name`。
    pub username: String,
    /// 配置中的 `server_user_password`。
    pub password: String,
}

/// `POST /api/login`：成功下发 24h 会话 cookie 并回 `{isLoggedIn:true,cookie}`，失败 401。
pub(crate) async fn login(
    State(state): State<Arc<WebUiState>>,
    body: Result<Json<LoginRequest>, JsonRejection>,
) -> Response {
    // Go `ShouldBindJSON` 失败（非 JSON、缺字段）回 400 且 body 带解析错误。
    let Ok(Json(body)) = body else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid request body" })),
        )
            .into_response();
    };
    let snapshot = state.config().snapshot();
    let username = snapshot.get_username();
    let password = snapshot.get_password();
    // 凭据未配置（空串）时一律拒绝：Go 的 `username == serverUsername` 会让空/空直接通过。
    let configured = !username.is_empty() && !password.is_empty();
    let mut matched = secret_eq(body.username.as_bytes(), username.as_bytes());
    matched &= secret_eq(body.password.as_bytes(), password.as_bytes());
    matched &= configured;

    if !matched {
        tracing::debug!("WebUI 登录失败");
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "isLoggedIn": false })),
        )
            .into_response();
    }

    let token = match state.sessions().issue() {
        Ok(token) => token,
        Err(err) => {
            // Go 在 `GenerateCookie` 失败时回 500 + `Failed to generate cookie`。
            tracing::error!(error = %err, "WebUI 会话落盘失败，登录中止");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Failed to generate cookie" })),
            )
                .into_response();
        }
    };
    (
        [(
            header::SET_COOKIE,
            set_cookie(&token, SESSION_TTL.as_secs()),
        )],
        // `cookie` 与 Set-Cookie 同值：Go 把 token 回给前端，前端只依赖 `isLoggedIn`。
        Json(json!({ "isLoggedIn": true, "cookie": token })),
    )
        .into_response()
}

/// `GET /api/check-login-status` 与 `GET /api/checkLoginStatus`：始终 200，见模块文档。
pub(crate) async fn check_login_status(
    State(state): State<Arc<WebUiState>>,
    headers: HeaderMap,
) -> Response {
    let Some(token) = cookie_token(&headers) else {
        return login_status(false, Some("Cookie not provided"));
    };
    if state.sessions().validate(token) {
        login_status(true, None)
    } else {
        // Go 区分 "Cookie not found" 与 "Cookie has expired"，前端只读 `isLoggedIn`；
        // 这里合成一条，避免为两个错误串多加一个查询枚举。
        login_status(false, Some("Cookie not found or expired"))
    }
}

/// `POST /api/logout`：删除会话并清除 cookie（未登录也回 `{code:0}`）。
pub(crate) async fn logout(State(state): State<Arc<WebUiState>>, headers: HeaderMap) -> Response {
    if let Some(token) = cookie_token(&headers) {
        state.sessions().remove(token);
    }
    let mut response = success();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, set_cookie("", 0));
    response
}

fn login_status(is_logged_in: bool, error: Option<&str>) -> Response {
    let body = error.map_or_else(
        || json!({ "isLoggedIn": is_logged_in }),
        |error| json!({ "isLoggedIn": is_logged_in, "error": error }),
    );
    Json(body).into_response()
}

fn success() -> Response {
    Json(json!({ "code": 0 })).into_response()
}

pub(crate) fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({ "code": 1 }))).into_response()
}

/// 账号维度路由（`/{uin}/config`、`/{uin}/process*`、`/{uin}/api`）的前置条件：
/// 已登录 + `{uin}` 等于配置里的 `app_id`。
///
/// Go 拿 `GetAppIDStr()` 拼路径做字符串比较，其它 uin 落不到任何分支 → 404。
pub(crate) fn guard_uin(state: &WebUiState, headers: &HeaderMap, uin: u64) -> Option<Response> {
    if !is_authenticated(state, headers) {
        return Some(unauthorized());
    }
    if state.config().snapshot().get_app_id() != uin {
        return Some(crate::assets::not_found());
    }
    None
}

/// 常数时间比较：长度不同也走满较长者，避免 `==` 提前返回泄露密码前缀。
fn secret_eq(left: &[u8], right: &[u8]) -> bool {
    let len = left.len().max(right.len());
    let mut diff = left.len() ^ right.len();
    let mut index = 0;
    while index < len {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        diff |= usize::from(left_byte ^ right_byte);
        index += 1;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers_with_cookie(value: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, HeaderValue::from_static(value));
        headers
    }

    #[test]
    fn cookie_token_picks_named_cookie() {
        let headers = headers_with_cookie("theme=dark; login_cookie=abc123; other=1");
        assert_eq!(cookie_token(&headers), Some("abc123"));
    }

    #[test]
    fn missing_empty_or_other_cookies_are_ignored() {
        assert_eq!(cookie_token(&HeaderMap::new()), None);
        assert_eq!(cookie_token(&headers_with_cookie("theme=dark")), None);
        assert_eq!(cookie_token(&headers_with_cookie("login_cookie=")), None);
        assert_eq!(cookie_token(&headers_with_cookie("login_cookie")), None);
    }

    #[test]
    fn secret_eq_rejects_length_and_content_mismatch() {
        assert!(secret_eq(b"hunter2", b"hunter2"));
        assert!(!secret_eq(b"hunter2", b"hunter3"));
        assert!(!secret_eq(b"hunter", b"hunter2"));
        assert!(!secret_eq(b"", b"x"));
        assert!(secret_eq(b"", b""));
    }

    #[test]
    fn issued_tokens_are_32_hex_and_unique() {
        let store = SessionStore::new(SESSION_TTL);
        let first = store.issue().expect("内存会话发放不应失败");
        let second = store.issue().expect("内存会话发放不应失败");
        assert_eq!(first.len(), 32);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second);
        assert!(store.validate(&first));
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn zero_ttl_sessions_expire_and_cleanup_drops_them() {
        let store = SessionStore::new(Duration::ZERO);
        let token = store.issue().expect("内存会话发放不应失败");
        assert!(!store.validate(&token), "TTL=0 的会话必须立即过期");
        assert_eq!(store.cleanup(), 1);
        assert!(store.is_empty());
    }

    #[test]
    fn remove_reports_whether_the_session_existed() {
        let store = SessionStore::new(SESSION_TTL);
        let token = store.issue().expect("内存会话发放不应失败");
        assert!(store.remove(&token));
        assert!(!store.remove(&token));
        assert!(!store.validate(&token));
    }
}
