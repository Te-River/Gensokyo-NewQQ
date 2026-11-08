//! Go `legacy-export` 产出的 idmap JSONL（v1）→ redb 导入器（M1e）。
//!
//! Go 侧在 M4 之前保留一版二进制，idmap 迁移由它导出 JSONL、Rust 只消费 JSONL，
//! 因此这里**不读 bbolt**：表名/key/value 的字节编码已由 [`crate::idmap`] 对齐 Go，
//! 导入器只负责"解析 → 校验 → 一次搬运"。
//!
//! # 文件格式（v1）
//!
//! 一行一个 JSON 对象，`kind` 区分行型（未知 `kind` 忽略并计数，便于前向兼容）。
//! 首行必须是 `meta`，末行必须是 `end`：
//!
//! | `kind` | 字段 | 落表 |
//! |--------|------|------|
//! | `meta` | `format`（固定 `gensokyo-idmap`）、`version`（固定 `1`）、`allocator`（`counter` / `hash`，可选） | 不入库；`allocator` 决定 `--on-vuin-conflict=remap` 时新 vUIN 的取法 |
//! | `identity` | `key`（Go `normalizeIdentity().Key`）、`raw`、`vuin` | `identity_to_vuin` = `vuin`（8 字节大端）；`vuin_to_identity["<vuin>:<key>"]` = `raw` |
//! | `vuin_counter` | `counter` | 身份库 `meta.current_vuin`（只增不减） |
//! | `msg_counter` | `counter` | 消息库 `meta.current_msg`（只增不减） |
//! | `config` | `section`、`key`、`value` | `config["<section>:<key>"]`（`section` 不做归一化，Go `WriteConfig` 原样键） |
//! | `user_info` | `key`（Go `"<rawID>:<UserID>"`）、`value`（任意 JSON，原样紧凑序列化） | `user_info` |
//! | `last_seen` | `vuin`、`raw` | `last_seen["default:<vuin>"]` = `raw` |
//! | `msg_id` | `virtual`、`real`、`expires`（可选） | `msg_to_virtual` / `virtual_to_msg`；有 `expires` 时再写 `expires_at` |
//! | `latest` | `group`、`user`、`msg_id`、`expires`（可选） | `latest_by_group_user`；有 `expires` 时再写 `latest_expires_at` |
//! | `end` | `lines`、`sha256` | 不入库 |
//!
//! 导出文件**不含** Go 的 `vuin_to_identity` 反向键与 `row-*` / `ts:*` 之类辅助键：反表完全由
//! `identity` 行重建，导入器不读也不依赖任何反向键。
//!
//! `identity` 行携带的是 **Go 已归一化**的 key，导入时不再按 `app_id` / `uin` 重新归一化
//! （否则 `raw` 与 `key` 会各走一遍前缀剥离）。因此导出与导入必须使用同一个 AppID：
//! Go 侧与 Rust 侧 `config.yml` 的 `app_id` 不一致时，导入的 key 与后续分配的新 key 永不相等。
//!
//! # 校验
//!
//! - 首个记录必须是 `meta`、最后一个必须是 `end`，两者各出现一次；`end` 之后不允许再有记录。
//! - `meta.format` / `meta.version` 不匹配 → 格式错（不认识的文件不该被"尽量导入"）。
//! - `end.lines` 等于 `end` 之前的记录数。
//! - `end.sha256` 是 `end` 行之前**全部原始字节**（含每行行尾的 `\n`，不含 `end` 行本身）
//!   的 SHA-256 小写十六进制。按字节校验意味着行内字段顺序、转义写法都不影响结果。
//! - 整数一律按 `i64` 解析：`serde_json` 对带小数/指数或超范围的数字只会产出浮点，
//!   `i64` 字段会直接拒绝，因此大于 2^53 的整数不会被浮点化。
//! - 空行、同一 `identity.key` 两个不同 `vuin`、非正 `vuin` / `virtual`、必填串为空
//!   → 格式错（文件自相矛盾时不做"尽量导入"）。
//!
//! # 应用与幂等
//!
//! 身份库一个写事务、消息库一个写事务（redb 的事务不跨库，无法两库原子提交）：身份库先提交，
//! 消息库失败时整个导入可原样重跑——重复导入是幂等的（同 key 同 vUIN 是 no-op，其余是
//! 同值 upsert，计数器只取 max）。
//!
//! 消息库只在文件里出现 `msg_counter` / `msg_id` / `latest` 时才动；echo 的消息缓存
//! 全在内存（`crate::echo`），不参与导入。
//!
//! # vUIN 冲突
//!
//! | 情形 | 默认（[`OnVuinConflict::Fail`]） | [`OnVuinConflict::Remap`] |
//! |------|----------------------------------|---------------------------|
//! | 同 key 同 vUIN | no-op | no-op |
//! | 同 key 不同 vUIN | [`ImportError::Conflict`] | 该 key 换新 vUIN；旧 vUIN 的归属保留（反表旧行不动） |
//! | vUIN 已被别的 key 占用 | [`ImportError::Conflict`] | 该 key 换新 vUIN |
//!
//! 新 vUIN 的取法跟随存储的 `hash_id`：`true` 走 Go `allocateVuinTx` 的 MD5 派生
//! （9 → 18 位，撞号加长），`false` 走 `max(库内计数器, 文件内最大 vUIN) + 1` 并跳过已占用值。
//!
//! # 退出码语义（二进制侧）
//!
//! [`ImportError::Conflict`] → 3、[`ImportError::Format`] → 4、其余（[`ImportError::NotEmpty`] /
//! 存储错误）→ 1。映射在 `crates/gensokyo-bin/src/main.rs`。

mod apply;
mod jsonl;

use std::path::Path;

use thiserror::Error;

use crate::StorageError;
use crate::idmap::IdMapStore;

#[cfg(test)]
mod tests;

/// 目标库已有数据且未给 `--merge`。
///
/// 非 `--merge` 时导入要求库为空：迁移只在首启做一次，允许写入半库数据会让
/// "同一个身份两个 vUIN" 这类事故静默发生。
#[derive(Debug, Error)]
pub enum ImportError {
    /// JSONL 格式错误（meta / version / sha256 / counts / 字段类型 / 文件内自相矛盾）。
    #[error("导入文件格式错误: {0}")]
    Format(String),
    /// vUIN 冲突（默认策略下的同 key 不同 vUIN，或 vUIN 已被别的 key 占用）。
    #[error("vUIN 冲突: {0}（可用 --on-vuin-conflict=remap 换新 vUIN）")]
    Conflict(String),
    /// 目标库非空且没有 `--merge`。
    #[error("目标库非空: {0}（用 --merge 允许合并导入）")]
    NotEmpty(String),
    /// 读取导入文件失败（不存在、无权限等）。
    #[error("读取导入文件失败: {0}")]
    Read(String),
    /// 存储层错误。
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// vUIN 冲突的处理策略（`--on-vuin-conflict`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OnVuinConflict {
    /// 报 [`ImportError::Conflict`]，不做任何写入。
    #[default]
    Fail,
    /// 给冲突的身份换一个新 vUIN，旧 vUIN 的归属保留。
    Remap,
}

/// 导入选项（`--merge` / `--dry-run` / `--on-vuin-conflict`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportOptions {
    /// 允许写入已有数据的库；默认要求库为空。
    pub merge: bool,
    /// 只解析与校验（含只读的冲突检测），不写库。
    pub dry_run: bool,
    /// vUIN 冲突策略。
    pub on_vuin_conflict: OnVuinConflict,
}

/// 导入结果。各字段是**本次文件**的条数，不含库内既有数据。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// 是否 `--dry-run`（报告来自校验阶段，库未被改动）。
    pub dry_run: bool,
    /// `end.lines`：`end` 之前的记录数（含未知 `kind`）。
    pub records: usize,
    /// 未知 `kind` 的行数（已忽略）。
    pub unknown_kinds: usize,
    /// 新写入的身份条数。
    pub identities_imported: usize,
    /// 幂等命中（同 key 同 vUIN）。
    pub identities_unchanged: usize,
    /// 因冲突换新 vUIN 的身份条数。
    pub identities_remapped: usize,
    /// `config` 行数。
    pub configs: usize,
    /// `user_info` 行数。
    pub user_infos: usize,
    /// `last_seen` 行数。
    pub last_seen: usize,
    /// `msg_id` 行数。
    pub msg_ids: usize,
    /// `latest` 行数。
    pub latest: usize,
    /// 导入后的 `current_vuin`（只增不减）。
    pub vuin_counter: i64,
    /// 导入后的 `current_msg`（文件未涉及消息库时是库内现值）。
    pub msg_counter: i64,
}

/// 从文件导入。
///
/// # Errors
///
/// 文件读取失败归入 [`ImportError::Read`]，其余见 [`ImportError`]。
pub fn import_file(
    store: &IdMapStore,
    path: &Path,
    options: &ImportOptions,
) -> Result<ImportReport, ImportError> {
    let bytes = std::fs::read(path)
        .map_err(|err| ImportError::Read(format!("{}: {err}", path.display())))?;
    import_bytes(store, &bytes, options)
}

/// 从内存字节导入（解析全部完成并通过校验后才会动库）。
///
/// # Errors
///
/// 格式、冲突、目标库非空或存储失败时返回 [`ImportError`]。
pub fn import_bytes(
    store: &IdMapStore,
    bytes: &[u8],
    options: &ImportOptions,
) -> Result<ImportReport, ImportError> {
    let data = jsonl::parse(bytes)?;
    apply::run(store, &data, options)
}
