//! 启动时的配置自愈：补缺失的 `settings` 直接子项（Go `config.ensureConfigComplete`）。
//!
//! > **已接线（P36）**：`gensokyo-bin/src/startup.rs::load_config` 在 `ensure_config_file`
//! > 之后、`ConfigManager::new` **之前**调用本模块——补进来的键因此本次启动即生效（Go 写完
//! > 是重启进程），补键失败只 warn 且不动文件。契约与缺口见
//! > `reports/refactor/17-go-rust-parity.md` §E。
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
//! - **写盘**：Go 用 `O_APPEND` 直接追加；这里走 [`crate::loader::write_atomic`]（同目录
//!   `config.yml.<pid>.tmp` → rename，首启生成与加载期清洗共用同一套），避免写一半留下半截
//!   配置；rename 前把原文件的权限位搬到临时文件上，0600 的配置不会被放宽成 0644。
//! - **不补注释**：Go 的 `addCommentsToConfigTemp`（:268-331）只在 WebUI 重建整份配置时用，
//!   不在本函数路径上；追加的块自带模板里的块内注释，其余行不动。
//! - **追加前提（顶层 `settings:` 块映射）**：追加的是缩进两格的子项，只有顶层 `settings:`
//!   块映射能承接。Go 靠 `cleanupDuplicateSettings`（`config.go:717-792`）在 `LoadConfig`
//!   早期把缺失的顶层键补回来，本函数没有那一步，于是自己判定挂载点（[`settings_mount`]）：
//!   空文件 / 纯注释文件先补 `version: 1\nsettings:\n` 外壳；其余没有顶层 `settings:` 块映射
//!   的形态（settings 正文、裸 `version: 1`、`settings: null` / 流式映射）返回
//!   [`CompleteError::NoSettings`] 且**一个字节都不写**——对它们追加会产出非法 YAML
//!   （`did not find expected key` / `mapping values are not allowed in this context`），
//!   把本来能载入的配置变成起不来的进程。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::template::CONFIG_TEMPLATE;

/// 直接子项的缩进（模板统一两空格，Go `keyExistsInConfig` 也只认 2，`config.go:702-705`）。
const SETTINGS_CHILD_INDENT: usize = 2;

/// 空文档追加前补的外壳：没有它，补出来的子项会挂在文档根上（「settings 正文」形态），
/// 而 Go 的 `config.yml` 恒为 `{version, settings}`。
const EMPTY_DOCUMENT_SHELL: &str = "version: 1\nsettings:\n";

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
    /// 顶层没有 `settings:` 块映射，追加缩进两格的子项会产出非法 YAML。
    #[error(
        "配置 {} 没有顶层 `settings:` 块映射（settings 正文、`settings: null` / 流式映射或裸 \
         `version: 1` 之类的文档），追加 `settings` 子项会写出非法 YAML；请先补上顶层 \
         `settings:` 块",
        path.display()
    )]
    NoSettings {
        /// 配置路径。
        path: PathBuf,
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
/// 读/写失败、现有文件不是合法 YAML，或没有顶层 `settings:` 块映射时返回 [`CompleteError`]；
/// 后两种都在写盘前放弃，文件逐字节不变。
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

    // 追加段是缩进两格的 `settings` 子项，只有顶层 `settings:` 块映射接得住；接不住的形态
    // 一律拒绝写盘，由调用方 warn（文件保持原样），绝不把可载入的配置改成非法 YAML。
    let shell = match settings_mount(&content) {
        SettingsMount::Block => "",
        SettingsMount::EmptyDocument => EMPTY_DOCUMENT_SHELL,
        SettingsMount::Missing => {
            return Err(CompleteError::NoSettings {
                path: path.to_path_buf(),
            });
        }
    };

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
        let mut body = content;
        if !shell.is_empty() {
            // 外壳必须从新的一行开始，否则会粘到最后一行（注释）的行尾。
            if !body.is_empty() && !body.ends_with('\n') {
                body.push('\n');
            }
            body.push_str(shell);
        }
        write_rewritten(path, &body, &additions)?;
    }
    Ok(report)
}

/// 现有文档能否承接缩进两格的 `settings` 子项。
#[derive(Debug)]
enum SettingsMount {
    /// 顶层 `settings:` 块映射（`settings:` 后为空或只有注释）：可以直接追加。
    Block,
    /// 空文档 / 纯注释文档：没有内容可承接，但可以先补 [`EMPTY_DOCUMENT_SHELL`]。
    EmptyDocument,
    /// 非空文档却没有顶层 `settings:` 块映射。
    Missing,
}

/// 判定追加挂载点：顶层（无缩进）且是**块映射头**的 `settings:`。
///
/// 只认 `settings:` 后面不带值（或只有行尾注释）的形态：`settings: null` 与 `settings: {…}`
/// 这类流式写法同样承接不了缩进子项，追加后是非法 YAML。
fn settings_mount(content: &str) -> SettingsMount {
    let text = content.trim_start_matches('\u{feff}');
    let mut has_content = false;
    for line in text.lines() {
        let line = line.trim_end();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // 文档标记与 null 标量不算内容（与 [`crate::loader`] 的空文档判定同口径）：
        // 只有它们的文件仍然可以先补外壳。
        if matches!(trimmed, "---" | "..." | "null" | "Null" | "NULL" | "~") {
            continue;
        }
        has_content = true;
        // 缩进行只能是某个块的值，不可能是顶层键。
        if indent_width(line) != 0 {
            continue;
        }
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        if key.trim() != "settings" {
            continue;
        }
        let rest = rest.trim();
        return if rest.is_empty() || rest.starts_with('#') {
            SettingsMount::Block
        } else {
            SettingsMount::Missing
        };
    }
    if has_content {
        SettingsMount::Missing
    } else {
        SettingsMount::EmptyDocument
    }
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

/// 原内容 + 追加段写入 `同一目录临时文件 → rename`（实现见 [`crate::loader::write_atomic`]，
/// 加载期清洗的写回走同一份 temp + rename + 权限逻辑）。
fn write_rewritten(path: &Path, original: &str, additions: &str) -> Result<(), CompleteError> {
    let mut content = String::with_capacity(original.len() + additions.len());
    content.push_str(original);
    content.push_str(additions);

    crate::loader::write_atomic(path, &content).map_err(|source| CompleteError::Write {
        path: path.to_path_buf(),
        source,
    })
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

    /// 补键走 `tmp → rename`，临时文件是新建的默认权限；原配置若是 0600，补完必须仍是 0600。
    #[cfg(unix)]
    #[test]
    fn rewrite_keeps_the_original_file_mode() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("临时目录");
        let path = write(&dir, &template_without("ws_token"));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("设权限");

        let report = ensure_config_complete(&path).expect("补全失败");
        assert_eq!(report.appended, vec!["ws_token".to_owned()]);

        let mode = fs::metadata(&path).expect("读元数据").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "补键后权限被放宽: {mode:o}");

        let leftovers: Vec<String> = fs::read_dir(dir.path())
            .expect("列目录")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "临时文件未清理: {leftovers:?}");
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

    /// 评审实测：往「settings 正文 / 裸 `version: 1`」追加缩进两格的子项会写出非法 YAML
    /// （`did not find expected key` / `mapping values are not allowed in this context`），
    /// 进程直接起不来。没有顶层 `settings:` 块映射时必须拒绝写盘。
    #[test]
    fn document_without_top_level_settings_is_refused_untouched() {
        // 前两种是评审复现的形态；`settings: null` 与流式映射同样接不住缩进子项。
        for original in [
            "port: \"5700\"\n",
            "version: 1\n",
            "settings: null\n",
            "settings: {port: \"5700\"}\n",
        ] {
            let dir = tempfile::tempdir().expect("临时目录");
            let path = write(&dir, original);

            assert!(
                matches!(
                    ensure_config_complete(&path),
                    Err(CompleteError::NoSettings { .. })
                ),
                "{original:?} 必须拒绝补键"
            );
            assert_eq!(
                fs::read_to_string(&path).expect("读回失败"),
                original,
                "{original:?} 拒绝补键时文件必须逐字节不变"
            );
            let leftovers: Vec<String> = fs::read_dir(dir.path())
                .expect("列目录")
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.contains(".tmp"))
                .collect();
            assert!(
                leftovers.is_empty(),
                "拒绝时不得留下临时文件: {leftovers:?}"
            );
        }
    }

    /// 拒绝补键不能把「本来能载入的 settings 正文」变成起不来的配置。
    #[test]
    fn refused_settings_body_still_loads() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = write(&dir, "port: \"5700\"\n");

        assert!(matches!(
            ensure_config_complete(&path),
            Err(CompleteError::NoSettings { .. })
        ));
        let manager = crate::ConfigManager::new(&path).expect("settings 正文必须仍能载入");
        assert_eq!(manager.snapshot().port, "5700");
    }

    /// 空文件：追加前先补 `{version, settings}` 外壳，产出必须是 Go 的文档形态，
    /// 而不是「根级正文」——后者 loader 虽然也认，但不是 Go 的 `config.yml`。
    #[test]
    fn empty_file_gets_the_version_and_settings_shell() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = write(&dir, "");

        let report = ensure_config_complete(&path).expect("补全失败");
        assert!(report.changed());

        let completed = fs::read_to_string(&path).expect("读回失败");
        assert!(
            completed.starts_with("version: 1\nsettings:\n"),
            "空文件必须先补顶层 version/settings 外壳（开头 60 字节: {:?}）",
            completed.get(..60).unwrap_or(&completed)
        );
        serde_norway::from_str::<serde_norway::Value>(&completed).expect("补全后必须是合法 YAML");

        let settings = crate::loader::parse_str(&completed).expect("补全后必须能载入");
        assert_eq!(settings.ws_server_path, "ws");
        assert!(
            crate::ConfigManager::new(&path).is_ok(),
            "补全后必须能被 ConfigManager 载入"
        );
    }

    /// 纯注释文件与空文件同口径，但注释一个字节都不能丢；原文件没有结尾换行时外壳要另起一行。
    #[test]
    fn comment_only_file_gets_the_shell_and_keeps_comments() {
        let dir = tempfile::tempdir().expect("临时目录");
        let original = "# 我的配置，别删\n# 第二行注释";
        let path = write(&dir, original);

        let report = ensure_config_complete(&path).expect("补全失败");
        assert!(report.changed());

        let completed = fs::read_to_string(&path).expect("读回失败");
        assert!(
            completed.starts_with("# 我的配置，别删\n# 第二行注释\nversion: 1\nsettings:\n"),
            "注释必须原样保留，外壳另起一行: {:?}",
            completed.get(..80).unwrap_or(&completed)
        );
        serde_norway::from_str::<serde_norway::Value>(&completed).expect("补全后必须是合法 YAML");
        assert_eq!(
            crate::loader::parse_str(&completed)
                .expect("补全后必须能载入")
                .ws_server_path,
            "ws"
        );
    }
}
