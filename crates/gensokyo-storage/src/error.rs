//! 存储层统一错误。

use thiserror::Error;

/// gensokyo-storage 错误。
///
/// 不 panic、不 `unwrap`：所有可预期失败都通过本枚举上抛。文件系统 IO 错误由 redb
/// 包装后归入 [`StorageError::Redb`]，因此不再单列不可达的 `Io` 变体。
///
/// `#[non_exhaustive]`：后续新增变体对下游不是破坏性变更（下游匹配需保留通配分支）。
#[derive(Debug, Error)]
#[non_exhaustive]
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
    /// 目标键已被占用（Go `UpdateVirtualValue` 的 `fmt.Errorf("%v :已存在", new)`）。
    ///
    /// 用变体而不是消息后缀区分，gRPC 层据此映射 `AlreadyExists`。
    #[error("{0} :已存在")]
    AlreadyExists(i64),
    /// 字符串键重复（Go `StoreUserInfo` 的 `fmt.Errorf("duplicate key: %s", key)`，
    /// `idmap/service.go:2271-2272`）。载荷是**完整 key**，因此 `Display` 与 Go 文案逐字节一致。
    ///
    /// 与 [`StorageError::AlreadyExists`] 是两种独立语义：那条是 vUIN 重绑定的目标号已被占用，
    /// 本条是"这一行的键已经写过"。重复键在稳态下必然命中（同一好友的第二条私信起），
    /// 不代表库损坏，所以不复用 [`StorageError::Corrupt`]——否则调用方只能把它连同真损坏
    /// 一起吞掉，或把真损坏一起报出来。
    #[error("duplicate key: {0}")]
    DuplicateKey(String),
    /// 数据损坏或非法入参（整数字段不是 8 字节、非法 UTF-8、空消息 ID、空 AppID 等）。
    #[error("数据损坏: {0}")]
    Corrupt(String),
    /// 有界重试耗尽（短链随机码连续冲突，见 `short_url::MAX_RANDOM_ATTEMPTS`）。
    ///
    /// Go 对应路径是无界 `for {}`；这里宁可报错也不挂住进程。
    #[error("重试耗尽: {0}")]
    RetriesExhausted(String),
    /// redb 原生错误（打开库、事务、表、提交；含底层 IO 错误）。
    #[error("redb 错误: {0}")]
    Redb(#[from] redb::Error),
    /// 远程 idmap 桥失败：gRPC Status、RPC/等待超时或桥线程死亡（`idmap/remote.rs`）。
    #[error("远程 idmap 错误: {0}")]
    Remote(String),
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
