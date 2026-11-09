//! 启动时的配置自愈：补缺失的 `settings` 直接子项（Go `config.ensureConfigComplete`）。
//!
//! > **未接线**：启动调用待 P36 里程碑（`gensokyo-bin/src/startup.rs` 目前只调
//! > `ensure_config_file`，不调本模块；接线点与验收见 `reports/refactor/17-go-rust-parity.md` §E）。
//!
//! Go 的流程（`config/config.go:350-407`，`LoadConfig` 在 `fastload=false` 时调用）：
//!
//! 1. 把现有配置与默认模板解析进 `Config`，用反射找"零值但模板非零"的字段；
//! 2. 用**文本**比对模板与现有文件的 key 集（`getMissingSettingsByText`，:443-506）：
//!    - 模板有、配置没有的 key 记为缺失；
//!    - 只保留父 key 为 `settings` 的直接子项（第四轮过滤，:497-503）——深层子 key 缺一个
//!      不会单独追加一行，整个块由祖先 key 负责；
//! 3. 从模板里把每个缺失 key 的**整块**切出来（key 行 + 缩进更深的后续行，含块内注释，
//!    `extractMissingConfigLines`，:568-610）；
//! 4. 逐块追加到配置文件末尾（`appendToConfigFile`，:612-688），写成功则
//!    `sys.RestartApplication()` 重启进程生效。
//!
//! 本实现的策略与 Go 相同：**纯文本追加**，只补缺失的 `settings` 直接子项，已有行一个字节
//! 都不改——因此用户的取值、顺序与注释全部保留。补全失败的典型原因（文件不是 YAML）不在
//! 本函数判断：Go 在反射那一步先 `yaml.Unmarshal`，解析失败直接返回错误。这里同样先按
//! [`crate::loader::parse_str`] 校验一次，语法坏的配置不会被"补键"掩盖。
//!
//! # 与 Go 的差异（明确列出）
//!
//! - **不重启进程**：Go 写完调用 `sys.RestartApplication()`（`config.go:402`）。Rust 的进程
//!   生命周期归 runtime 管，本函数只返回 [`CompletionReport`]，由调用方决定是否重启/热载。
//! - **反射路径省略**：Go 的反射分支只可能命中 `version`（`Settings` 是 struct 被跳过，
//!   `config.go:430-433`），而第四轮又把"无父 key"的项全部丢掉——`version` 永远进不了
//!   追加列表。因此行为等价，省掉一份结构体镜像比对。
//! - **追加顺序**：Go 遍历 map，块顺序随机；这里按模板中的出现顺序追加，结果可复现。
//! - **写盘**：Go 用 `O_APPEND` 直接追加；这里先在同目录写 `config.yml.tmp` 再 rename，
//!   避免写一半留下半截配置（与 [`crate::template`] 的首启生成同口径）。
//! - **不补注释**：Go 的 `addCommentsToConfigTemp`（:268-331）只在 WebUI 重建整份配置时用，
//!   不在本函数路径上；追加的块自带模板里的块内注释，其余行不动。
//! - **`settings:` 缺失的前提**：Go 的 `cleanupDuplicateSettings`（`config.go:717-792`）在
//!   `LoadConfig` 早期把缺失的 `version:`/`settings:` 顶层键补回来，本函数假定配置已有
//!   `settings:`（与 Go 一致：缺 `settings:` 时同样只会追加一堆缩进两格的 key）。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::template::CONFIG_TEMPLATE;

/// 直接子项的缩进（模板统一两空格，Go `keyExistsInConfig` 也只认 2，`config.go:702-705`）。
const SETTINGS_CHILD_INDENT: usize = 2;

/// 一次补全的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompletionReport {
    /// 被追加的 YAML key，按模板顺序。
    pub appended: Vec<String>,
    /// 实际写入的行数（不含跳过的空行）。
    pub lines_written: usize,
}

impl CompletionReport {
    /// 是否真的改了文件（Go 用 `written > 0` 判定是否重启）。
    #[must_use]
    pub fn changed(&self) -> bool {
        self.lines_written > 0
    }
}

/// 补全失败。
#[derive(Debug, Error)]
pub enum CompleteError {
    /// 读取配置文件失败。
    #[error("读取配置 {} 失败: {source}", path.display())]
    Read {
        /// 配置路径。
        path: PathBuf,
        /// 底层 IO 错误。
        #[source]
        source: std::io::Error,
    },
    /// 配置文件不是合法 YAML（Go 在反射前先 `yaml.Unmarshal`，同样直接报错）。
    #[error("解析配置 {} 失败: {detail}", .path.display())]
    Parse {
        /// 配置路径。
        path: PathBuf,
        /// 解析错误文案。
        detail: String,
    },
    /// 写回配置文件失败。
    #[error("写回配置 {} 失败: {source}", path.display())]
    Write {
        /// 配置路径。
        path: PathBuf,
        /// 底层 IO 错误。
        #[source]
        source: std::io::Error,
    },
}

/// 补齐配置文件里缺失的 `settings` 直接子项，返回本次追加了什么。
///
/// # Errors
///
/// 读/写失败或现有文件不是合法 YAML 时返回 [`CompleteError`]。
pub fn ensure_config_complete(path: impl AsRef<Path>) -> Result<CompletionReport, CompleteError> {
    let path = path.as_ref();
    let content = fs::read_to_string(path).map_err(|source| CompleteError::Read {
        path: path.to_path_buf(),
        source,
    })?;

    // 先确认现有文件可解析：语法坏的配置不该被追加掩盖成"修好了"。
    crate::loader::parse_str(&content).map_err(|err| CompleteError::Parse {
        path: path.to_path_buf(),
        detail: err.to_string(),
    })?;

    let present = extract_keys(&content);
    let mut report = CompletionReport::default();
    let mut additions = String::new();
    for block in settings_child_blocks(CONFIG_TEMPLATE) {
        if present.contains(block.key.as_str()) {
            continue;
        }
        for line in block.lines.iter().filter(|line| !line.trim().is_empty()) {
            // Go `appendToConfigFile` 每行都先写一个 `\n`：文件以换行结尾时自然多一个空行
            additions.push('\n');
            additions.push_str(line);
            report.lines_written += 1;
        }
        report.appended.push(block.key);
    }

    if report.lines_written > 0 {
        write_rewritten(path, &content, &additions)?;
    }
    Ok(report)
}

/// 模板里的一个 `settings` 直接子项：key 与其整块原文（含块内注释）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct KeyBlock {
    key: String,
    lines: Vec<String>,
}

/// 按缩进切出模板的 `settings` 直接子项块（Go `extractMissingConfigLines` 的整块口径）。
fn settings_child_blocks(template: &str) -> Vec<KeyBlock> {
    let lines: Vec<&str> = template.lines().collect();
    let mut blocks = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start();
        let indent = indent_width(line);
        if indent == SETTINGS_CHILD_INDENT
            && !trimmed.starts_with('#')
            && let Some((key, _)) = trimmed.split_once(':')
        {
            let key = key.trim();
            let mut block = vec![line.to_owned()];
            let mut next = index + 1;
            while next < lines.len() {
                let candidate = lines[next];
                if candidate.trim().is_empty() {
                    block.push(candidate.to_owned());
                    next += 1;
                    continue;
                }
                if indent_width(candidate) <= indent {
                    break;
                }
                block.push(candidate.to_owned());
                next += 1;
            }
            blocks.push(KeyBlock {
                key: key.to_owned(),
                lines: block,
            });
            index = next;
        } else {
            index += 1;
        }
    }
    blocks
}

/// 文件里出现过的 YAML key 集（Go `extractKeysFromString`：非注释行冒号前、去空白）。
///
/// 与 Go 一样按"任意缩进"收集，因此深层子 key 的同名出现也会让顶层 key 被当成已存在——
/// 这是 Go 的既有判定口径，保留以免追加出重复块。
fn extract_keys(content: &str) -> HashSet<String> {
    let mut keys = HashSet::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, _)) = line.split_once(':') {
            keys.insert(key.trim().to_owned());
        }
    }
    keys
}

/// 前导空白宽度；制表符按 1 计（Go 用 `len(line) - len(TrimLeft(line, " \t"))`）。
fn indent_width(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

/// 原内容 + 追加段写入 `同一目录临时文件 → rename`。
fn write_rewritten(path: &Path, original: &str, additions: &str) -> Result<(), CompleteError> {
    let tmp = tmp_path(path);
    let mut content = String::with_capacity(original.len() + additions.len());
    content.push_str(original);
    content.push_str(additions);

    fs::write(&tmp, content.as_bytes()).map_err(|source| CompleteError::Write {
        path: tmp.clone(),
        source,
    })?;
    if let Err(source) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(CompleteError::Write {
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

/// 同目录临时文件名：`config.yml` → `config.yml.tmp`。
fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".tmp");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 从模板里删掉某个 `settings` 直接子项块，模拟"旧版本配置"。
    fn template_without(key: &str) -> String {
        let mut kept = Vec::new();
        let mut skipping = false;
        for line in CONFIG_TEMPLATE.lines() {
            if skipping {
                if line.trim().is_empty() || indent_width(line) > SETTINGS_CHILD_INDENT {
                    continue;
                }
                skipping = false;
            }
            if indent_width(line) == SETTINGS_CHILD_INDENT
                && line
                    .trim_start()
                    .split_once(':')
                    .is_some_and(|(candidate, _)| candidate.trim() == key)
            {
                skipping = true;
                continue;
            }
            kept.push(line);
        }
        assert_ne!(
            kept.len(),
            CONFIG_TEMPLATE.lines().count(),
            "模板里必须存在 key {key}"
        );
        kept.join("\n")
    }

    fn write(dir: &tempfile::TempDir, content: &str) -> PathBuf {
        let path = dir.path().join("config.yml");
        fs::write(&path, content).expect("写夹具失败");
        path
    }

    #[test]
    fn complete_template_needs_nothing() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = write(&dir, CONFIG_TEMPLATE);

        let report = ensure_config_complete(&path).expect("补全失败");
        assert!(!report.changed(), "完整模板不应被追加: {report:?}");
        assert!(report.appended.is_empty());
        assert_eq!(
            fs::read_to_string(&path).expect("读回失败"),
            CONFIG_TEMPLATE,
            "无缺失时文件必须逐字节不变"
        );
    }

    #[test]
    fn missing_block_is_appended_once_and_only_appends() {
        let dir = tempfile::tempdir().expect("临时目录");
        let original = template_without("ws_token");
        let path = write(&dir, &original);

        let report = ensure_config_complete(&path).expect("补全失败");
        assert_eq!(report.appended, vec!["ws_token".to_owned()]);
        assert!(report.changed());

        let completed = fs::read_to_string(&path).expect("读回失败");
        assert!(
            completed.starts_with(&original),
            "补全只能追加，不能改写已有行"
        );
        assert!(completed.contains("ws_token:"));
        assert!(
            completed.contains("  ws_token: [\"\",\"\",\"\"]"),
            "追加的必须是模板原文（含缩进）: {}",
            &completed[original.len()..]
        );
        // 补全后仍是合法 YAML
        serde_norway::from_str::<serde_norway::Value>(&completed).expect("补全后必须是合法 YAML");

        // 幂等：第二次不再追加，文件不变
        let again = ensure_config_complete(&path).expect("二次补全失败");
        assert!(!again.changed(), "{again:?}");
        assert_eq!(fs::read_to_string(&path).expect("读回失败"), completed);
    }

    #[test]
    fn existing_values_and_comments_are_preserved() {
        let dir = tempfile::tempdir().expect("临时目录");
        let original = template_without("ws_token")
            .replace("  port: \"15630\"", "  port: \"5999\"  # 用户改过的端口");
        let path = write(&dir, &original);

        let report = ensure_config_complete(&path).expect("补全失败");
        assert_eq!(report.appended, vec!["ws_token".to_owned()]);

        let completed = fs::read_to_string(&path).expect("读回失败");
        assert!(
            completed.contains("  port: \"5999\"  # 用户改过的端口"),
            "用户取值与行内注释必须原样保留"
        );
        assert!(completed.contains("#反向ws设置"), "块注释必须保留");
        assert!(completed.contains("ws_token:"));
    }

    #[test]
    fn empty_settings_gets_every_direct_child_but_not_version() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = write(&dir, "version: 1\nsettings:\n");

        let report = ensure_config_complete(&path).expect("补全失败");
        assert!(
            report.appended.len() > 100,
            "settings 直接子项应全部补齐，实际 {}",
            report.appended.len()
        );
        assert!(!report.appended.iter().any(|key| key == "version"));
        assert!(
            report.appended.iter().all(|key| key != "settings"),
            "settings 本身（无父 key）不会被追加（Go 第四轮过滤）"
        );

        let completed = fs::read_to_string(&path).expect("读回失败");
        serde_norway::from_str::<serde_norway::Value>(&completed).expect("补全后必须是合法 YAML");
        // 抽查一个多行块：text_intent 的注释行也要带过来
        assert!(completed.contains("text_intent:"));
        assert!(completed.contains("FriendAddEventHandler"));

        let again = ensure_config_complete(&path).expect("二次补全失败");
        assert!(!again.changed(), "幂等: {again:?}");
    }

    #[test]
    fn invalid_yaml_is_reported_not_patched() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = write(&dir, "version: 1\nsettings:\n  broken: [\n");

        assert!(matches!(
            ensure_config_complete(&path),
            Err(CompleteError::Parse { .. })
        ));
        assert_eq!(
            fs::read_to_string(&path).expect("读回失败"),
            "version: 1\nsettings:\n  broken: [\n",
            "解析失败不得改文件"
        );
    }

    #[test]
    fn missing_file_is_an_error() {
        let dir = tempfile::tempdir().expect("临时目录");
        assert!(matches!(
            ensure_config_complete(dir.path().join("absent.yml")),
            Err(CompleteError::Read { .. })
        ));
    }
}
