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

/// 从 YAML 文本解析配置。
pub fn parse_str(text: &str) -> Result<Settings, LoadError> {
    serde_norway::from_str(text).map_err(|source| LoadError::Parse {
        path: PathBuf::from("<inline>"),
        source: Box::new(source),
    })
}

/// 从文件加载配置。
pub fn load(path: impl AsRef<Path>) -> Result<Settings, LoadError> {
    let path = path.as_ref();
    let text = fs::read_to_string(path).map_err(|source| LoadError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    serde_norway::from_str(&text).map_err(|source| LoadError::Parse {
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}
