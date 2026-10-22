//! 存储层统一错误。

use thiserror::Error;

/// gensokyo-storage 错误。
///
/// 不 panic、不 `unwrap`：所有可预期失败都通过本枚举上抛。文件系统 IO 错误由 redb
/// 包装后归入 [`StorageError::Redb`]，因此不再单列不可达的 `Io` 变体。
#[derive(Debug, Error)]
pub enum StorageError {
    /// 键不存在。
    #[error("键不存在: {0}")]
    NotFound(String),
    /// 键存在但 TTL 已过。
    ///
    /// Go 的 `RetrieveMsgID` 把过期并入 `ErrKeyNotFound`；需要单值语义时用
    /// [`StorageError::is_not_found`]。
    #[error("键已过期: {0}")]
    Expired(String),
    /// 数据损坏或非法入参（整数字段不是 8 字节、非法 UTF-8、空消息 ID、空 AppID 等）。
    #[error("数据损坏: {0}")]
    Corrupt(String),
    /// redb 原生错误（打开库、事务、表、提交；含底层 IO 错误）。
    #[error("redb 错误: {0}")]
    Redb(#[from] redb::Error),
}

impl StorageError {
    /// Go `ErrKeyNotFound` 的单值等价判定：`NotFound` 与 `Expired` 都算"查不到"。
    #[must_use]
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound(_) | Self::Expired(_))
    }
}

impl From<StorageError> for gensokyo_core::Error {
    /// 保留 `NotFound`/`Expired` 的可读文案：`core::Error::Storage` 的 `Display` 会加
    /// "存储错误: " 前缀，日志里仍能看到具体是"键不存在"还是"键已过期"。
    fn from(err: StorageError) -> Self {
        Self::Storage(err.to_string())
    }
}
