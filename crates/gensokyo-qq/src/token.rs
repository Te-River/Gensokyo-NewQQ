//! access_token 获取与刷新（对齐 `botgo/token/authtoken.go`）。
//!
//! 与 botgo 的差异：botgo 用一个后台 goroutine 定时刷新；本 crate 不创建 runtime、不启后台
//! 任务，而是在每次 [`TokenManager::token`] 调用时检查缓存，距过期不足
//! [`REFRESH_AHEAD_SECS`] 秒就重新获取。并发调用由异步互斥锁串行化，避免打爆 token 接口。
//! 刷新窗口内拉取失败时，只要旧 token 尚未真正过期就继续使用（记录 `warn`），否则返回错误。
//!
//! token 端点默认 [`TOKEN_ENDPOINT`]，可用 [`TokenConfig::with_token_url`] 覆盖为自定义
//! 地址（等价 Go `config.yml` 的 `develop_access_token_dir`，`GET` 拉取、无请求体）。

use std::fmt;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::warn;

use crate::error::{ApiError, Error, Result};
use crate::http::{HttpRequest, HttpTransport};

/// token 端点（`botgo/token/authtoken.go` 的 `getAccessTokenURL`）。
pub const TOKEN_ENDPOINT: &str = "https://api.bot.qq.com/app/getAppAccessToken";
/// 提前刷新窗口：剩余有效期 ≤ 60 秒即重新获取。
pub const REFRESH_AHEAD_SECS: u64 = 60;
/// 上游未返回 `expires_in` 时的兜底有效期（与 botgo 一致）。
pub const DEFAULT_EXPIRES_IN_SECS: u64 = 7200;

/// 单调递增的秒级时钟抽象；测试可注入手动时钟。
pub trait Clock: Send + Sync + fmt::Debug {
    /// 当前 Unix 秒。
    fn now_unix(&self) -> u64;
}

/// 系统时钟。
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs())
    }
}

/// access_token 申请配置。
#[derive(Clone)]
pub struct TokenConfig {
    app_id: String,
    client_secret: String,
    sandbox: bool,
    token_url: Option<String>,
}

impl TokenConfig {
    /// 创建配置。
    #[must_use]
    pub fn new(app_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
        Self {
            app_id: app_id.into(),
            client_secret: client_secret.into(),
            sandbox: false,
            token_url: None,
        }
    }

    /// 设置沙箱开关。
    ///
    /// 官方 `api-call-guide` 已把正式与沙箱域名统一为 `api.bot.qq.com`
    /// （见 `botgo/openapi/v2/resource.go`），因此该开关当前不改变 token 端点；保留它是为了
    /// 对齐 botgo `Setup(token, sandbox)` 的构造语义。
    #[must_use]
    pub fn with_sandbox(mut self, sandbox: bool) -> Self {
        self.sandbox = sandbox;
        self
    }

    /// AppID。
    #[must_use]
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// 是否沙箱。
    #[must_use]
    pub fn sandbox(&self) -> bool {
        self.sandbox
    }

    /// 覆盖 token 端点（对齐 Go `config.yml` 的 `develop_access_token_dir`）。
    ///
    /// 非空时改为 `GET <url>` 拉取 token（不带请求体），响应体解析规则与官方端点完全一致；
    /// 空串 / 全空白等价于未设置，回退 [`TOKEN_ENDPOINT`]（对齐 Go 模板注释
    /// "默认留空 请留空忽略"）。
    #[must_use]
    pub fn with_token_url(mut self, url: impl Into<String>) -> Self {
        let url = url.into();
        let trimmed = url.trim();
        self.token_url = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        };
        self
    }

    /// 自定义 token 端点；`None` 表示使用官方端点。
    #[must_use]
    pub fn token_url(&self) -> Option<&str> {
        self.token_url.as_deref()
    }

    /// token 端点：自定义端点优先，否则 [`TOKEN_ENDPOINT`]。
    #[must_use]
    pub fn endpoint(&self) -> &str {
        self.token_url.as_deref().unwrap_or(TOKEN_ENDPOINT)
    }
}

impl fmt::Debug for TokenConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // client_secret 永不进入日志 / panic 信息。
        f.debug_struct("TokenConfig")
            .field("app_id", &self.app_id)
            .field("client_secret", &"<redacted>")
            .field("sandbox", &self.sandbox)
            .field("token_url", &self.token_url)
            .finish()
    }
}

/// 缓存的 access_token。
struct CachedToken {
    access_token: String,
    /// 上游给出的绝对过期时间（Unix 秒）。
    expires_at: u64,
}

impl CachedToken {
    /// 剩余有效期 ≤ 60s 即视为需要刷新。
    fn needs_refresh(&self, now: u64) -> bool {
        now >= self.expires_at.saturating_sub(REFRESH_AHEAD_SECS)
    }
}

impl fmt::Debug for CachedToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CachedToken")
            .field("access_token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// 线程安全的 access_token 缓存与刷新器。
pub struct TokenManager<T: HttpTransport> {
    config: TokenConfig,
    transport: Arc<T>,
    clock: Arc<dyn Clock>,
    cache: Mutex<Option<CachedToken>>,
}

impl<T: HttpTransport> fmt::Debug for TokenManager<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenManager")
            .field("config", &self.config)
            .field("clock", &self.clock)
            .finish_non_exhaustive()
    }
}

impl<T: HttpTransport> TokenManager<T> {
    /// 使用系统时钟创建。
    #[must_use]
    pub fn new(transport: Arc<T>, config: TokenConfig) -> Self {
        Self::with_clock(transport, config, Arc::new(SystemClock))
    }

    /// 注入自定义时钟（测试用）。
    #[must_use]
    pub fn with_clock(transport: Arc<T>, config: TokenConfig, clock: Arc<dyn Clock>) -> Self {
        Self {
            config,
            transport,
            clock,
            cache: Mutex::new(None),
        }
    }

    /// 配置。
    #[must_use]
    pub fn config(&self) -> &TokenConfig {
        &self.config
    }

    /// 取一个可用的 access_token；缓存过期或距过期不足 60 秒时自动刷新。
    ///
    /// 刷新失败时：旧 token 若尚未真正过期（`now < expires_at`）则继续返回它并记录 `warn`，
    /// 让提前刷新窗口内的瞬时故障不至于打断出站；旧 token 已过期或不存在时返回错误。
    /// 两条路径都不会 panic，也都不会丢弃旧缓存。
    pub async fn token(&self) -> Result<String> {
        let now = self.clock.now_unix();
        let mut cached = self.cache.lock().await;

        if let Some(token) = cached.as_ref()
            && !token.needs_refresh(now)
        {
            return Ok(token.access_token.clone());
        }

        let fetched = match self.fetch(now).await {
            Ok(fetched) => fetched,
            Err(error) => {
                if let Some(unexpired) = cached.as_ref().filter(|token| now < token.expires_at) {
                    warn!(
                        expires_at = unexpired.expires_at,
                        %error,
                        "access_token 刷新失败，回退未过期旧 token"
                    );
                    return Ok(unexpired.access_token.clone());
                }
                return Err(error);
            }
        };
        let token = fetched.access_token.clone();
        *cached = Some(fetched);
        Ok(token)
    }

    /// 忽略缓存强制刷新（例如收到 401 后主动换 token）。
    pub async fn force_refresh(&self) -> Result<String> {
        let now = self.clock.now_unix();
        let mut cached = self.cache.lock().await;
        let fetched = self.fetch(now).await?;
        let token = fetched.access_token.clone();
        *cached = Some(fetched);
        Ok(token)
    }

    /// 丢弃缓存；下一次 [`TokenManager::token`] 会重新获取。
    pub async fn invalidate(&self) {
        *self.cache.lock().await = None;
    }

    /// 请求 token 端点并解析响应。
    async fn fetch(&self, now: u64) -> Result<CachedToken> {
        let request = if self.config.token_url().is_some() {
            // 自定义端点（Go `develop_access_token_dir`）：GET，无请求体。
            HttpRequest::get(self.config.endpoint()).with_header("Accept", "application/json")
        } else {
            let body = serde_json::to_vec(&TokenRequest {
                app_id: &self.config.app_id,
                client_secret: &self.config.client_secret,
            })?;
            HttpRequest::post(self.config.endpoint())
                .with_header("Content-Type", "application/json")
                .with_header("Accept", "application/json")
                .with_body(body)
        };

        let response = self.transport.request(request).await?;
        if !(200..300).contains(&response.status) {
            return Err(ApiError::from_response(
                response.status,
                &response.body,
                response.header("X-Tps-trace-ID"),
            )
            .into());
        }

        let parsed: TokenResponse = serde_json::from_slice(&response.body)
            .map_err(|err| Error::InvalidResponse(format!("解析 access_token 响应失败: {err}")))?;

        if let Some(code) = parsed.code.as_ref().and_then(parse_code)
            && code != 0
        {
            return Err(ApiError {
                http_status: response.status,
                code,
                message: parsed
                    .message
                    .filter(|message| !message.is_empty())
                    .unwrap_or_else(|| "token 接口返回业务错误".to_owned()),
                trace_id: response
                    .header("X-Tps-trace-ID")
                    .filter(|trace_id| !trace_id.is_empty())
                    .map(str::to_owned),
            }
            .into());
        }

        // 兼容扁平结构与 `{"data": {...}}` 信封结构（botgo `queryTokenRsp`）。
        let (access_token, expires_in) = match parsed.access_token {
            Some(token) if !token.is_empty() => (token, parsed.expires_in),
            _ => {
                let Some(data) = parsed.data else {
                    // 不回显响应体：它可能包含 token。
                    return Err(Error::InvalidResponse(
                        "access_token 响应中没有 access_token".to_owned(),
                    ));
                };
                match data.access_token.filter(|token| !token.is_empty()) {
                    Some(token) => (token, data.expires_in.or(parsed.expires_in)),
                    None => {
                        return Err(Error::InvalidResponse(
                            "access_token 响应中没有 access_token".to_owned(),
                        ));
                    }
                }
            }
        };

        let expires_in = match expires_in {
            Some(raw) => raw.to_secs()?,
            None => DEFAULT_EXPIRES_IN_SECS,
        };
        if expires_in == 0 {
            return Err(Error::InvalidResponse(
                "access_token 响应的 expires_in 为 0".to_owned(),
            ));
        }

        Ok(CachedToken {
            access_token,
            expires_at: now.saturating_add(expires_in),
        })
    }
}

/// token 请求体（字段名对齐官方：`appId` / `clientSecret`）。
#[derive(Debug, Serialize)]
struct TokenRequest<'a> {
    #[serde(rename = "appId")]
    app_id: &'a str,
    #[serde(rename = "clientSecret")]
    client_secret: &'a str,
}

/// token 响应（同时兼容扁平结构与 `data` 信封结构）。
#[derive(Debug, Deserialize)]
struct TokenResponse {
    #[serde(default)]
    code: Option<serde_json::Value>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    expires_in: Option<ExpiresIn>,
    #[serde(default)]
    data: Option<TokenData>,
}

/// `data` 信封。
#[derive(Debug, Deserialize)]
struct TokenData {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    expires_in: Option<ExpiresIn>,
}

/// `expires_in` 兼容数字与字符串。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ExpiresIn {
    Number(i64),
    Text(String),
}

impl ExpiresIn {
    fn to_secs(&self) -> Result<u64> {
        let value = match self {
            Self::Number(value) => *value,
            Self::Text(text) => text
                .parse()
                .map_err(|err| Error::InvalidResponse(format!("expires_in 不是合法整数: {err}")))?,
        };
        u64::try_from(value)
            .map_err(|_| Error::InvalidResponse(format!("expires_in 非法: {value}")))
    }
}

/// JSON `code` 归一为 `i64`（兼容字符串形态）。
fn parse_code(raw: &serde_json::Value) -> Option<i64> {
    raw.as_i64()
        .or_else(|| raw.as_str().and_then(|text| text.parse().ok()))
}
