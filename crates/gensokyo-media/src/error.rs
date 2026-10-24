//! crate 统一错误类型。
//!
//! 媒体获取的所有可预期失败都收敛到 [`enum@Error`]（`anyhow` 只允许出现在 `gensokyo-bin` / `xtask`）。

use std::time::Duration;

use thiserror::Error;

/// gensokyo-media 的媒体错误。
#[derive(Debug, Error)]
pub enum Error {
    /// 媒体来源字符串为空，或 `file://` 后的路径为空。
    #[error("媒体来源非法: {0}")]
    InvalidSource(String),
    /// base64 负载无法解码。
    #[error("base64 解码失败: {0}")]
    Base64(String),
    /// `data:` URI 无法解析。
    #[error("data URI 解析失败: {0}")]
    DataUri(String),
    /// URL scheme 不在支持范围（只接受 http / https）。
    #[error("不支持的 scheme: {0}")]
    UnsupportedScheme(String),
    /// 传输层失败（连接、TLS、读响应体）。
    #[error("传输失败: {0}")]
    Transport(String),
    /// 上游响应状态码不是 200。
    #[error("HTTP 状态码异常: {0}")]
    HttpStatus(u16),
    /// 内容超过大小上限；`actual` 在长度未知（流式读满上限）时为 `None`。
    #[error("内容超过大小上限: limit={limit} actual={actual:?}")]
    TooLarge {
        /// 配置的上限（字节）。
        limit: u64,
        /// 已知的实际长度（字节）。
        actual: Option<u64>,
    },
    /// 单次请求总超时（含响应体读取）。
    #[error("请求超时: {0:?}")]
    Timeout(Duration),
    /// 本地文件读写失败。
    #[error("IO 失败: {0}")]
    Io(#[from] std::io::Error),
}

/// crate 内 `Result` 别名。
pub type Result<T, E = Error> = std::result::Result<T, E>;
