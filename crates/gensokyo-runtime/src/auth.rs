//! 正向 WS / HTTP API 共用鉴权。
//!
//! 规则对齐 Go `server/wsserver.go` 与 `httpapi/httpapi.go`：配置的 token 为空即放行；
//! 否则 `Authorization: Bearer <token>`（兼容 `Token <token>` 与裸 token 值）或
//! `?access_token=<token>` 任一匹配即通过。
//!
//! Go 对「缺失 token」回 401、「错误 token」回 403；M1h-a 按 OneBot 客户端习惯统一回
//! 401（`status` 只在 401/403 间选一个，客户端都按鉴权失败处理）。

use axum::Json;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// 从 `Authorization` 头取 token：接受 `Bearer` / `Token` 前缀，也接受裸 token 值。
pub(crate) fn header_token(headers: &HeaderMap) -> Option<&str> {
    let raw = headers.get(AUTHORIZATION)?.to_str().ok()?.trim();
    if raw.is_empty() {
        return None;
    }
    let value = match raw.split_once(' ') {
        Some((scheme, value))
            if scheme.eq_ignore_ascii_case("bearer") || scheme.eq_ignore_ascii_case("token") =>
        {
            value.trim()
        }
        _ => raw,
    };
    (!value.is_empty()).then_some(value)
}

/// 配置的 token 为空 → 放行；否则 query / header 任一匹配即通过。
pub(crate) fn is_authorized(
    expected: &str,
    headers: &HeaderMap,
    query_token: Option<&str>,
) -> bool {
    let expected = expected.trim();
    if expected.is_empty() {
        return true;
    }
    query_token.is_some_and(|token| token == expected)
        || header_token(headers).is_some_and(|token| token == expected)
}

/// 统一的 401 响应。
pub(crate) fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "鉴权失败" })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};

    use super::{header_token, is_authorized};

    fn headers_with(value: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", HeaderValue::from_static(value));
        headers
    }

    #[test]
    fn bearer_and_token_prefixes_are_accepted() {
        assert_eq!(header_token(&headers_with("Bearer abc")), Some("abc"));
        assert_eq!(header_token(&headers_with("token abc")), Some("abc"));
        assert_eq!(header_token(&headers_with("abc")), Some("abc"));
    }

    #[test]
    fn empty_configured_token_allows_everything() {
        assert!(is_authorized("", &HeaderMap::new(), None));
        assert!(is_authorized("  ", &HeaderMap::new(), Some("whatever")));
    }

    #[test]
    fn query_or_header_token_must_match() {
        assert!(is_authorized("s3cret", &HeaderMap::new(), Some("s3cret")));
        assert!(is_authorized(
            "s3cret",
            &headers_with("Bearer s3cret"),
            None
        ));
        assert!(!is_authorized("s3cret", &HeaderMap::new(), None));
        assert!(!is_authorized("s3cret", &HeaderMap::new(), Some("wrong")));
        assert!(!is_authorized(
            "s3cret",
            &headers_with("Bearer wrong"),
            Some("wrong")
        ));
    }
}
