//! 登录、会话 cookie 与登录态查询。
//!
//! 凭据来自 `config.yml` 的 `server_user_name` / `server_user_password`（与 Go
//! `webui/api.go:checkCredentials` 同源）。会话只存进程内存，重启后所有浏览器都要重新登录；
//! 持久化到 `cookie.db` 属 M3b。
//!
//! 与 Go 的差异（响应体）：M3 按管理台约定的 `{code:0}` / `{code:1}` 返回，不沿用 Go 的
//! `{isLoggedIn}`；cookie 名保持 `login_cookie`，现有前端无需改动。Go 的
//! `checkLoginStatus` 在未登录时回 200 + `isLoggedIn:false`，这里统一回 401，便于前端用
//! HTTP 状态判断。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::state::WebUiState;

/// 会话 cookie 名（与 Go `webui/api.go` 的 `login_cookie` 一致）。
pub const SESSION_COOKIE: &str = "login_cookie";

/// 会话有效期 24h（与 Go `SetCookie(..., 3600*24, ...)` 一致）。
pub const SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// 进程内会话存储：`token → 过期时刻`。
///
/// 没有后台清理任务：调用方需要定期调 [`SessionStore::cleanup`]（M3b 的持久化任务一并接入），
/// 在此之前过期条目只是"校验失败"，仍占内存。写路径只有 issue / remove / cleanup。
#[derive(Debug)]
pub struct SessionStore {
    sessions: RwLock<HashMap<String, Instant>>,
    ttl: Duration,
}

impl SessionStore {
    /// 以指定 TTL 建空存储（生产走 [`WebUiState::new`]，固定 24h）。
    #[must_use]
    pub fn new(ttl: Duration) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            ttl,
        }
    }

    /// 发放新会话并返回 token。
    pub fn issue(&self) -> String {
        let token = generate_token();
        let expires_at = Instant::now() + self.ttl;
        self.write().insert(token.clone(), expires_at);
        token
    }

    /// token 存在且未过期。
    #[must_use]
    pub fn validate(&self, token: &str) -> bool {
        let now = Instant::now();
        self.read()
            .get(token)
            .is_some_and(|expires_at| *expires_at > now)
    }

    /// 删除会话（登出）；返回是否确实删掉了一个会话。
    pub fn remove(&self, token: &str) -> bool {
        self.write().remove(token).is_some()
    }

    /// 清掉已过期会话，返回删除条数。
    pub fn cleanup(&self) -> usize {
        let now = Instant::now();
        let mut sessions = self.write();
        let before = sessions.len();
        sessions.retain(|_, expires_at| *expires_at > now);
        before - sessions.len()
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

    fn read(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, Instant>> {
        self.sessions
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<String, Instant>> {
        self.sessions
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// 32 位 hex 会话 token。
///
/// 不引入 `rand` / `uuid`：`RandomState` 的键由 OS 熵在进程内播种，两轮 SipHash 把
/// 「纳秒时间 + 原子计数 + 轮次」摊成 128 bit，足以抵抗会话猜测。M3b 把 token 落盘
/// （`cookie.db`）时再换 CSPRNG——离开本进程的凭据应有明确的随机源保证。
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

/// `POST /api/login`：成功下发 24h 会话 cookie，失败 401。
pub(crate) async fn login(
    State(state): State<Arc<WebUiState>>,
    Json(body): Json<LoginRequest>,
) -> Response {
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
        return unauthorized();
    }

    let token = state.sessions().issue();
    (
        [(
            header::SET_COOKIE,
            set_cookie(&token, SESSION_TTL.as_secs()),
        )],
        success(),
    )
        .into_response()
}

/// `GET /api/checkLoginStatus`：cookie 有效 `{code:0}`，否则 401。
pub(crate) async fn check_login_status(
    State(state): State<Arc<WebUiState>>,
    headers: HeaderMap,
) -> Response {
    if is_authenticated(&state, &headers) {
        success()
    } else {
        unauthorized()
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

fn success() -> Response {
    Json(json!({ "code": 0 })).into_response()
}

pub(crate) fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({ "code": 1 }))).into_response()
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
        let first = store.issue();
        let second = store.issue();
        assert_eq!(first.len(), 32);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second);
        assert!(store.validate(&first));
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn zero_ttl_sessions_expire_and_cleanup_drops_them() {
        let store = SessionStore::new(Duration::ZERO);
        let token = store.issue();
        assert!(!store.validate(&token), "TTL=0 的会话必须立即过期");
        assert_eq!(store.cleanup(), 1);
        assert!(store.is_empty());
    }

    #[test]
    fn remove_reports_whether_the_session_existed() {
        let store = SessionStore::new(SESSION_TTL);
        let token = store.issue();
        assert!(store.remove(&token));
        assert!(!store.remove(&token));
        assert!(!store.validate(&token));
    }
}
