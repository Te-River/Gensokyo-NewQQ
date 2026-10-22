//! 统一错误类型。
//!
//! 规则：领域 crate 用 [`Error`] 表达可预期失败，`anyhow` 只允许出现在 `gensokyo-bin` 与
//! `xtask`（design 附录 B）。

use thiserror::Error;

/// gensokyo 统一错误。
#[derive(Debug, Error)]
pub enum Error {
    /// 配置缺失或非法。
    #[error("配置错误: {0}")]
    Config(String),
    /// OneBot 协议层错误（参数、CQ 码、消息段）。
    #[error("协议错误: {0}")]
    Protocol(String),
    /// 存储层错误（redb、bbolt 导入）。
    #[error("存储错误: {0}")]
    Storage(String),
    /// 网络与传输错误。
    #[error("网络错误: {0}")]
    Network(String),
    /// QQ 上游 API 返回的业务错误。
    #[error("上游 API 错误: code={code} message={message}")]
    Upstream {
        /// 上游返回码。
        code: i64,
        /// 上游返回信息。
        message: String,
    },
    /// 尚未实现的 action / 能力。
    #[error("未实现: {0}")]
    Unsupported(String),
    /// IO 错误。
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// 本 crate 的 Result 别名。
pub type Result<T, E = Error> = std::result::Result<T, E>;
