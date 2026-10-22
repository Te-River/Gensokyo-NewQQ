//! QQ 上游错误与 crate 统一错误类型。

use std::fmt;

use serde::Deserialize;
use thiserror::Error;

/// QQ OpenAPI 返回的业务 / HTTP 错误。
///
/// 字段口径对齐 `botgo/errs`：`http_status` 永远是响应行的 HTTP 状态码；`code` 优先取响应
/// 体里的业务码，响应体没有业务码时退回 HTTP 状态码（botgo `errs` 的 `OnAfterResponse`
/// 就是把 `resp.StatusCode()` 当 code 传入，所以 botgo `errs.Error.Code` 对应的是这里的
/// `code`，不是 `http_status`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    /// HTTP 状态码（来自响应行）。
    pub http_status: u16,
    /// 上游业务错误码；无业务码时等于 `http_status`。
    pub code: i64,
    /// 上游错误信息，按字符截断到 [`MAX_MESSAGE_CHARS`]；响应体不是 JSON 时为截断后的原文。
    pub message: String,
    /// 上游返回的链路 ID（`X-Tps-trace-ID`）。
    pub trace_id: Option<String>,
}

/// 错误信息保留的最大字符数（超出部分丢弃并追加 `...`）。
const MAX_MESSAGE_CHARS: usize = 200;

impl ApiError {
    /// 从一次非 2xx 响应构造错误。
    ///
    /// 最终 `message`（JSON 的 `message` / `msg` 与非 JSON 原文一视同仁）统一按字符截断到
    /// [`MAX_MESSAGE_CHARS`]，避免把整段响应体带进日志；截断按字符边界，不会切断 UTF-8。
    #[must_use]
    pub fn from_response(http_status: u16, body: &[u8], trace_id: Option<&str>) -> Self {
        let parsed = serde_json::from_slice::<ErrorBody>(body).ok();
        let code = parsed
            .as_ref()
            .and_then(|body| body.code.as_ref())
            .and_then(parse_code)
            .unwrap_or(i64::from(http_status));
        let message = parsed
            .as_ref()
            .and_then(|body| body.message.as_deref().or(body.msg.as_deref()))
            .filter(|message| !message.is_empty())
            .map_or_else(
                || truncate_text(&String::from_utf8_lossy(body)),
                truncate_text,
            );

        Self {
            http_status,
            code,
            message,
            trace_id: trace_id
                .map(str::to_owned)
                .filter(|trace_id| !trace_id.is_empty()),
        }
    }

    /// 是否为鉴权类错误（token 失效 / 无权限）。
    #[must_use]
    pub fn is_auth(&self) -> bool {
        matches!(self.http_status, 401 | 403)
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "QQ OpenAPI 错误: http_status={}, code={}, message={}",
            self.http_status, self.code, self.message
        )?;
        if let Some(trace_id) = &self.trace_id {
            write!(f, ", trace_id={trace_id}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ApiError {}

/// crate 统一错误类型。
#[derive(Debug, Error)]
pub enum Error {
    /// 网络 / 传输层失败（连接、TLS、超时、响应体读取等）。
    #[error("HTTP 传输失败: {0}")]
    Transport(String),
    /// 上游返回了非 2xx 或业务错误码。
    #[error(transparent)]
    Api(#[from] ApiError),
    /// 请求体 JSON 序列化失败。
    #[error("JSON 序列化失败: {0}")]
    Json(#[from] serde_json::Error),
    /// 消息体不满足 QQ 的互斥约束。
    #[error("消息体非法: {0}")]
    InvalidMessage(String),
    /// 上游响应体无法解析为预期结构。
    #[error("响应体非法: {0}")]
    InvalidResponse(String),
    /// rustls provider / 客户端构建失败。
    #[error("TLS 初始化失败: {0}")]
    Tls(String),
}

impl Error {
    /// 若错误来自上游响应，返回对应的 [`ApiError`]。
    #[must_use]
    pub fn api(&self) -> Option<&ApiError> {
        match self {
            Self::Api(api) => Some(api),
            _ => None,
        }
    }
}

/// crate 内 `Result` 别名。
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// 上游错误响应体；`code` 兼容数字与字符串两种形态。
#[derive(Debug, Deserialize)]
struct ErrorBody {
    #[serde(default)]
    code: Option<serde_json::Value>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    msg: Option<String>,
}

/// 把 JSON 里的 `code` 归一为 `i64`（QQ 偶有字符串形态）。
fn parse_code(raw: &serde_json::Value) -> Option<i64> {
    raw.as_i64()
        .or_else(|| raw.as_str().and_then(|text| text.parse().ok()))
}

/// 按字符截断到 [`MAX_MESSAGE_CHARS`]，超长时追加 `...` 标记（不切断 UTF-8）。
fn truncate_text(text: &str) -> String {
    let mut truncated: String = text.chars().take(MAX_MESSAGE_CHARS).collect();
    if text.chars().count() > MAX_MESSAGE_CHARS {
        truncated.push_str("...");
    }
    truncated
}

#[cfg(test)]
mod tests {
    use super::ApiError;

    #[test]
    fn code_falls_back_to_http_status_without_business_code() {
        let error = ApiError::from_response(502, br#"{"message":"boom"}"#, None);
        assert_eq!(error.http_status, 502);
        assert_eq!(error.code, 502, "无业务码时 code 等于 HTTP 状态码");
        assert_eq!(error.message, "boom");
    }

    #[test]
    fn truncates_long_json_message_by_char() {
        let body = serde_json::json!({"code": 10001, "message": "错".repeat(250)}).to_string();
        let error = ApiError::from_response(200, body.as_bytes(), None);
        assert_eq!(error.code, 10001);
        assert_eq!(error.message.chars().count(), 203, "200 字符 + `...`");
        assert!(error.message.starts_with(&"错".repeat(200)));
        assert!(error.message.ends_with("..."));
    }

    #[test]
    fn truncates_non_json_body_by_char() {
        let error = ApiError::from_response(500, "错".repeat(250).as_bytes(), None);
        assert_eq!(error.message.chars().count(), 203);
        assert!(error.message.ends_with("..."));
    }
}
