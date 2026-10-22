//! `config.yml` 加载。
//!
//! 兼容两种文档形态：Go 真源的 `{version, settings}` 整份配置（`template/config_template.go`
//! 生成的就是它），以及 M0 起测试/调用方直接给的 `settings` 正文。
//!
//! **已知缺口（M1d 处理）**：Go `config/config.go:60-64` 会在解析前修复历史 bug 产生的
//! 重复 `settings:` / `version:` 行；这里尚未实现，出现重复顶层键时 `serde_norway` 会
//! 直接报解析错误（不会静默取其中一个）。

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
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

    // 先看顶层有没有 `settings` 键：Go 的 config.yml 是 `{version, settings}`，
    // 而 M0 起的夹具/调用方给的是 settings 正文。探测一次 `Value` 只为决定形态，
    // 真正反序列化仍走带行号的 `from_str`，报错位置不丢。
    let parsed = match serde_norway::from_str::<serde_norway::Value>(text) {
        Ok(serde_norway::Value::Mapping(map)) if map.contains_key("settings") => {
            serde_norway::from_str::<ConfigDocument>(text)
                .map(|doc| doc.settings.unwrap_or_default())
        }
        Ok(_) => serde_norway::from_str::<Settings>(text),
        Err(err) => Err(err),
    };
    parsed.map_err(|source| LoadError::Parse {
        path,
        source: Box::new(source),
    })
}

/// Go `config.Config`：`version` 之外只有 `settings` 一份配置。
///
/// `settings: null` 与缺失同样回退零值（Go yaml.v3 对 null 结构体留零值）。
#[derive(Deserialize)]
struct ConfigDocument {
    settings: Option<Settings>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwraps_go_document_shape() {
        let settings = parse_str("version: 1\nsettings:\n  port: \"5700\"\n  app_id: 42\n")
            .expect("Go 文档形态必须能解析");
        assert_eq!(settings.port, "5700");
        assert_eq!(settings.app_id, 42);
    }

    #[test]
    fn null_settings_falls_back_to_default() {
        let settings =
            parse_str("version: 1\nsettings: null\n").expect("settings: null 必须能解析");
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn plain_settings_body_still_parses() {
        // M0 起的夹具形态（没有 version/settings 外壳）不能回归
        let settings = parse_str("port: \"5700\"\n").expect("settings 正文必须能解析");
        assert_eq!(settings.port, "5700");
    }
}
