//! 上传重试策略与错误分类。
//!
//! 对齐 Go `imagehosting/hosting.go` 的 `retryUpload`：最多 3 次尝试（含首次），失败后按
//! 1s / 2s 线性退避（第 n 次重试前睡 `n × backoff`）。
//!
//! 与 Go 的差异：Go 对**任何**错误都重试，连 4xx 与响应格式错误也会再打两次；这里只重试
//! 可恢复的错误——传输失败、超时、5xx。4xx（鉴权失败、参数错、`413` 之类）重发不会变好，
//! 只会把一次失败放大成三次。

use std::time::Duration;

use crate::error::Error;

/// 最大尝试次数（含首次），对齐 Go `maxUploadAttempts`。
pub const MAX_ATTEMPTS: u32 = 3;
/// 退避步长：第 n 次重试前睡 `n × 1s`（Go 的 `attempt * time.Second`）。
pub const BACKOFF_STEP: Duration = Duration::from_secs(1);

/// 重试参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// 最大尝试次数（含首次）；`1` 表示不重试。
    pub max_attempts: u32,
    /// 退避步长。
    pub backoff: Duration,
}

impl RetryPolicy {
    /// 生产默认：3 次尝试、1s/2s 退避（Go 口径）。
    pub const DEFAULT: Self = Self {
        max_attempts: MAX_ATTEMPTS,
        backoff: BACKOFF_STEP,
    };

    /// 第 `retry_index` 次重试前的等待（`1` 起算；与 Go `attempt * time.Second` 一致）。
    #[must_use]
    pub const fn delay_before(&self, retry_index: u32) -> Duration {
        self.backoff.saturating_mul(retry_index)
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 这个错误重发一次是否可能成功。
///
/// 只有网络层失败、总超时与 5xx 算可重试；4xx 是请求本身的问题（凭据、参数、体积），
/// 重发只会得到同样的答复。
#[must_use]
pub fn is_retryable(error: &Error) -> bool {
    match error {
        Error::Transport(_) | Error::Timeout(_) => true,
        Error::HttpStatus(status) => (500..600).contains(status),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{BACKOFF_STEP, MAX_ATTEMPTS, RetryPolicy, is_retryable};
    use crate::error::Error;

    #[test]
    fn defaults_match_go() {
        assert_eq!(MAX_ATTEMPTS, 3);
        assert_eq!(BACKOFF_STEP, Duration::from_secs(1));
        assert_eq!(RetryPolicy::DEFAULT.max_attempts, 3);
        assert_eq!(RetryPolicy::default(), RetryPolicy::DEFAULT);
        // Go：第 1 次重试前 1s，第 2 次前 2s
        assert_eq!(RetryPolicy::DEFAULT.delay_before(1), Duration::from_secs(1));
        assert_eq!(RetryPolicy::DEFAULT.delay_before(2), Duration::from_secs(2));
    }

    #[test]
    fn classifies_transient_errors_as_retryable() {
        for error in [
            Error::Transport("连接被拒绝".to_owned()),
            Error::Timeout(Duration::from_secs(30)),
            Error::HttpStatus(500),
            Error::HttpStatus(502),
            Error::HttpStatus(599),
        ] {
            assert!(is_retryable(&error), "{error}");
        }
    }

    #[test]
    fn classifies_client_and_payload_errors_as_permanent() {
        for error in [
            Error::HttpStatus(400),
            Error::HttpStatus(401),
            Error::HttpStatus(404),
            Error::HttpStatus(413),
            Error::HttpStatus(429),
            Error::HostingConfig("COS 未配置".to_owned()),
            Error::HostingResponse("响应不是合法 JSON".to_owned()),
            Error::TooLarge {
                limit: 4,
                actual: Some(5),
            },
        ] {
            assert!(!is_retryable(&error), "{error}");
        }
    }
}
