//! `config.yml` 加载。
//!
//! M0 只做"读文件 + 解析"；热重载（`notify` + `ArcSwap` 快照）与首启模板生成在 M1 落地。

use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::settings::Settings;

/// 配置加载错误。
#[derive(Debug, Error)]
pub enum LoadError {
    /// 读取配置文件失败。
    #[error("读取配置文件 {} 失败: {source}", path.display())]
    Read {
        /// 配置路径。
        path: PathBuf,
        /// 底层 IO 错误。
        #[source]
        source: std::io::Error,
    },
    /// YAML 语法或类型错误。
    #[error("解析配置 {} 失败: {source}", path.display())]
    Parse {
        /// 配置路径（内存解析时为 `<inline>`）。
        path: PathBuf,
        /// YAML 解析错误（`serde_norway::Error` 体积较大，装箱避免 `LoadError` 撑大每个 `Result`）。
        #[source]
        source: Box<serde_norway::Error>,
    },
}

// 上层统一走 `gensokyo_core::Error`，但 `LoadError` 的路径/底层原因必须保留，
// 因此只做 Display 转写，不丢弃 `LoadError` 结构。
impl From<LoadError> for gensokyo_core::Error {
    fn from(err: LoadError) -> Self {
        Self::Config(err.to_string())
    }
}

/// 从 YAML 文本解析配置。
pub fn parse_str(text: &str) -> Result<Settings, LoadError> {
    parse_with_path(text, PathBuf::from("<inline>"))
}

/// 从文件加载配置。
pub fn load(path: impl AsRef<Path>) -> Result<Settings, LoadError> {
    let path = path.as_ref();
    let text = fs::read_to_string(path).map_err(|source| LoadError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    parse_with_path(&text, path.to_path_buf())
}

fn parse_with_path(text: &str, path: PathBuf) -> Result<Settings, LoadError> {
    // 部分编辑器（记事本等）会写 UTF-8 BOM；先剥掉，否则纯 BOM 文件会被当成"有内容"送去解析。
    let text = text.trim_start_matches('\u{feff}');
    // 空文档 / 纯注释文档：Go yaml.v3 不报错并留下零值，`serde_norway` 却会报 EOF，这里补齐。
    if is_effectively_empty(text) {
        return Ok(Settings::default());
    }
    serde_norway::from_str(text).map_err(|source| LoadError::Parse {
        path,
        source: Box::new(source),
    })
}

/// 是否没有任何 YAML 内容：只有空白、注释、文档标记（`---` / `...`）与 null 标量
/// （`null` / `Null` / `NULL` / `~`）。
///
/// Go yaml.v3 对这类文档不报错并留下零值，`serde_norway` 却会报 EOF 或类型错误，这里补齐。
fn is_effectively_empty(text: &str) -> bool {
    text.lines().all(|line| {
        // 先剥行内注释：`--- # 文档标记` 与 `null # 空值` 同样视为无内容。
        // 只用 `#` 切分不会把有内容的行误判为空（有内容时切分后仍非空），
        // 因为 YAML 里 `#` 在引号外才起注释作用，而本函数只关心"是否全空"。
        let line = line.split('#').next().unwrap_or("").trim();
        line.is_empty() || matches!(line, "---" | "..." | "null" | "Null" | "NULL" | "~")
    })
}
