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
    // 空文档 / 纯注释文档：Go yaml.v3 不报错并留下零值，`serde_norway` 却会报 EOF，这里补齐。
    if is_effectively_empty(text) {
        return Ok(Settings::default());
    }
    serde_norway::from_str(text).map_err(|source| LoadError::Parse {
        path,
        source: Box::new(source),
    })
}

/// 是否没有任何 YAML 内容（只含空白与注释行）。
fn is_effectively_empty(text: &str) -> bool {
    text.lines().all(|line| {
        let line = line.trim();
        line.is_empty() || line.starts_with('#')
    })
}
