//! 正向 WS / HTTP API 共用鉴权。
//!
//! 规则对齐 Go `server/wsserver.go` 与 `httpapi/httpapi.go`：配置的 token 为空即放行；
//! 否则 `Authorization: Bearer <token>`（兼容 `Token <token>` 与裸 token 值）或
//! `?access_token=<token>` 任一匹配即通过。
//!
//! 与 Go 一致区分两档：**没有** token 回 401 `Missing token`，token 不匹配回 403
//! `Incorrect token`（`server/wsserver.go:85-94`）。

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

/// Go `server/wsserver.go:63-79` 的 token 提取：`Authorization` 头**非空**时只看头
/// （`Bearer ` / `Token ` 前缀剥掉，裸值直接用），头为空才回落到 `?access_token=`。
///
/// 返回 `None` 即 Go 的 `token == ""`——用于把「没给 token」（401）与「给错了」（403）分开。
/// 与 [`header_token`] 不同：这里不 trim，`Bearer `（前缀后为空）按「没给」算，与 Go 的
/// `TrimPrefix` 后 `token == ""` 一致。
pub(crate) fn provided_token<'a>(
    headers: &'a HeaderMap,
    query_token: Option<&'a str>,
) -> Option<&'a str> {
    let token = match headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .filter(|raw| !raw.is_empty())
    {
        Some(raw) => raw
            .strip_prefix("Token ")
            .or_else(|| raw.strip_prefix("Bearer "))
            .unwrap_or(raw),
        None => query_token.unwrap_or(""),
    };
    (!token.is_empty()).then_some(token)
}

/// 401：配置了 token 但请求没给（Go `Missing token`）。
pub(crate) fn missing_token() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "Missing token" })),
    )
        .into_response()
}

/// 403：给了 token 但不匹配（Go `Incorrect token`）。
pub(crate) fn incorrect_token() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({ "error": "Incorrect token" })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};

    use super::{header_token, is_authorized, provided_token};

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

    #[test]
    fn provided_token_distinguishes_missing_from_wrong() {
        // 没给 token：401 分支
        assert_eq!(provided_token(&HeaderMap::new(), None), None);
        assert_eq!(provided_token(&HeaderMap::new(), Some("")), None);
        // 给了错 token：403 分支
        assert_eq!(
            provided_token(&HeaderMap::new(), Some("wrong")),
            Some("wrong")
        );
        assert_eq!(
            provided_token(&headers_with("Bearer wrong"), None),
            Some("wrong")
        );
        // 头存在但剥完为空的通知按「没给」处理（Go token == ""）
        assert_eq!(
            provided_token(&headers_with("Bearer "), Some("s3cret")),
            None
        );
    }
}
