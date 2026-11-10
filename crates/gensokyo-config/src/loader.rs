//! `config.yml` 加载。
//!
//! 兼容两种文档形态：Go 真源的 `{version, settings}` 整份配置（`template/config_template.go`
//! 生成的就是它），以及 M0 起测试/调用方直接给的 `settings` 正文。
//!
//! 读盘后先清洗（[`cleanup_config_file`]，Go `config/config.go:717-792` 的
//! `cleanupDuplicateSettings`）：换行统一为 LF、截断重复的顶层 `settings:`、顶层键丢失时
//! 重建 `version:` / `settings:` 外壳；有改动才原子写回（Go `config.go:61-64`），且写回前
//! 先自检「清洗结果能否解析」——修不干净就不落盘，宁可留原文件。不先清洗的话
//! `serde_norway` 遇到重复顶层键会直接报解析错误（不会静默取其中一个）。

use std::borrow::Cow;
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
    let text = read_sanitized(path)?;
    parse_with_path(&text, path.to_path_buf())
}

/// 只做结构清洗（必要时写回），不解析（启动链在补键之前调用）。
///
/// 补键（[`crate::ensure_config_complete`]）只在合法 YAML 上追加，而重复的顶层 `settings:`
/// 会被它的预检查判成语法坏——所以清洗必须排在补键前面，一次启动才能把两种损坏都修掉。
///
/// # Errors
///
/// 读取失败时返回 [`LoadError::Read`]；写回失败只 `warn`（Go 同样忽略 `os.WriteFile` 的错误），
/// 不算错误——本次加载用的仍是内存里清洗过的内容。
pub fn cleanup_config_file(path: impl AsRef<Path>) -> Result<(), LoadError> {
    read_sanitized(path.as_ref()).map(|_| ())
}

/// 读文件 → 清洗 → 自检 → 有改动就原子写回；返回本次启动使用的全文。
fn read_sanitized(path: &Path) -> Result<String, LoadError> {
    let original = fs::read_to_string(path).map_err(|source| LoadError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let cleaned = cleanup_duplicate_settings(&original);
    if cleaned.text.as_ref() == original {
        return Ok(original);
    }

    // 写回前自检：重建逻辑一旦有 bug，用户原文件当场被烧掉（P43 评审实测过 `cos:` 子项被
    // 拍平、`---` 被当内容写出非法 YAML 两种）。清洗结果解析不了就绝不落盘。
    if let Err(err) = parse_str(&cleaned.text) {
        crate::warn_early(&format!(
            "config.yml 清洗结果无法解析，已放弃写回并保留原文件 path={} error={err} \
             truncated={} rebuilt={} normalized={}",
            path.display(),
            cleaned.truncated,
            cleaned.rebuilt,
            cleaned.normalized
        ));
        // 原文仍可解析 ⇒ 只是清洗有 bug，退回原文，本次启动不受影响；
        // 原文本来就坏 ⇒ 两者都用不了，交内存里的清洗结果走正常的报错路径。
        return if parse_str(&original).is_ok() {
            Ok(original)
        } else {
            Ok(cleaned.text.into_owned())
        };
    }

    // Go `config.go:61-64` 原地 `os.WriteFile` 并忽略写错误；这里走 temp + rename 并搬权限位
    // （`0600` 的 config.yml 里存着 client_secret），写一半崩掉不会留下半截配置。
    match write_atomic(path, cleaned.text.as_ref()) {
        Ok(()) => crate::warn_early(&format!(
            "config.yml 含重复的 settings: 或顶层结构损坏，已清洗并写回（本次启动即生效） \
             path={} truncated={} rebuilt={} normalized={}",
            path.display(),
            cleaned.truncated,
            cleaned.rebuilt,
            cleaned.normalized
        )),
        Err(err) => crate::warn_early(&format!(
            "config.yml 清洗结果写回失败，本次按内存中的清洗结果加载 path={} error={err}",
            path.display()
        )),
    }
    Ok(cleaned.text.into_owned())
}

/// 原子写回：同目录临时文件 → rename，rename 前把原文件权限位搬到临时文件。
///
/// 配置写盘只有这一份实现：加载期清洗（本文件）与补键写回（`complete.rs`）、首启模板生成
/// （`template.rs`）都走这里。
pub(crate) fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    let tmp = tmp_path(path);
    fs::write(&tmp, content.as_bytes())?;
    // rename 连同权限一起替换原文件：不搬权限的话，0600 的 config.yml 会被放宽成新建文件
    // 默认的 0644。
    if let Err(err) = inherit_permissions(path, &tmp) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
}

/// 同目录临时文件名：`config.yml` → `config.yml.<pid>.tmp`。
///
/// 带进程号是为了两个实例同时启动时互不覆盖对方的半成品文件。
fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(format!(".{}.tmp", std::process::id()));
    path.with_file_name(name)
}

/// 把原文件的权限位复制到临时文件；原文件不存在（首启生成）时保持临时文件的默认权限。
///
/// Unix 上 `fs::write` 新建的文件是 `0666 & !umask`（通常 0644），直接 rename 会把原本
/// 0600 的 `config.yml` 放宽——里面存着 `client_secret`。
#[cfg(unix)]
fn inherit_permissions(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    match fs::metadata(from) {
        Ok(meta) => fs::set_permissions(to, fs::Permissions::from_mode(meta.permissions().mode())),
        // 首启没有原文件可继承：临时文件已经是新建文件的默认权限，直接放行。
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Windows 没有 Unix 权限位：权限由上级目录 ACL 继承，rename 不改动，故 no-op。
#[cfg(not(unix))]
fn inherit_permissions(_from: &Path, _to: &Path) -> std::io::Result<()> {
    Ok(())
}

/// 一次清洗的结果。
struct Cleanup<'a> {
    /// 清洗后的文本；没改动时借用原文。
    text: Cow<'a, str>,
    /// 是否截断了重复的 `settings:` 及其后的全部内容。
    truncated: bool,
    /// 是否重建了顶层 `version:` / `settings:` 外壳。
    rebuilt: bool,
    /// 原文是否含 `\r`（换行被统一成 LF）。
    normalized: bool,
}

/// 清洗配置文本：Go `config.go:717-792` `cleanupDuplicateSettings` 的逐行移植（含三处有意修正：
/// 截断只认列 0 的 `settings:`、文档标记不当内容、重建保留行内相对缩进）。
///
/// 旧版 Go 的 bug 会往 `config.yml` 里追加重复的 `settings:`，或让正文丢掉顶层
/// `version:` / `settings:` 外壳；两种损坏都会让 `serde_norway` 拒绝解析。
fn cleanup_duplicate_settings(text: &str) -> Cleanup<'_> {
    // BOM 不属于 YAML 结构：留在首行会让「第一个非空非注释行」判定看到 `\ufeffversion: 1`
    // 这种既不等于 `settings:` 也不以 `version:` 开头的行，把合规文件误判成结构损坏。
    let (bom, body) = match text.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", text),
    };

    // 第一步：换行统一成 LF（Go 依次 ReplaceAll `\r\n`→`\n`、`\r`→空）。
    let normalized = body.contains('\r');
    let source: Cow<'_, str> = if normalized {
        Cow::Owned(body.replace("\r\n", "\n").replace('\r', ""))
    } else {
        Cow::Borrowed(body)
    };
    let mut lines: Vec<&str> = source.split('\n').collect();

    // 第二步：把文件截断到第二个**顶层** `settings:` 之前（Go 的 `cutIndex`）。
    //
    // 只认列 0：正文里缩进的 `settings:` 是某个键的嵌套值，Go 的 `TrimSpace` 口径会把它当成
    // 顶层键、把后面整段截掉（本仓有意偏离，见 parity §E-7）。
    let mut seen = 0usize;
    let cut = lines.iter().position(|line| {
        if line.starts_with("settings:") && line.trim() == "settings:" {
            seen += 1;
        }
        seen == 2
    });
    let mut truncated = false;
    // Go 只在 `cutIndex > 0` 时截断（首个 `settings:` 落在第 0 行时不丢空块）。
    if let Some(index) = cut.filter(|index| *index > 0) {
        lines.truncate(index);
        truncated = true;
    }

    // 第三步：首个内容行（跳过空行、注释、文档标记 `---` / `...` 与 null 标量）既不是
    // `settings:` 也不以 `version:` 开头 ⇒ 顶层键丢了，重建外壳（注释原样保留，其余内容缩进两格）。
    let mut rebuilt = None;
    if let Some(first) = lines.iter().find(|line| is_content_line(line)) {
        let trimmed = first.trim();
        if trimmed != "settings:" && !trimmed.starts_with("version:") {
            rebuilt = Some(rebuild_structure(&lines));
        }
    }

    if !truncated && rebuilt.is_none() && !normalized {
        return Cleanup {
            text: Cow::Borrowed(text),
            truncated,
            rebuilt: false,
            normalized,
        };
    }

    // 第四步：汇总。截断后 `lines` 的 join 就是清洗结果（Go `strings.Join`），没截断时它与
    // `source` 逐字节相同。
    let rebuilt_flag = rebuilt.is_some();
    let inner = rebuilt.unwrap_or_else(|| lines.join("\n"));
    let text = if bom.is_empty() {
        inner
    } else {
        format!("{bom}{inner}")
    };
    Cleanup {
        text: Cow::Owned(text),
        truncated,
        rebuilt: rebuilt_flag,
        normalized,
    }
}

/// 重建顶层外壳：补 `version: 1`、补 `settings:`，其余内容整体缩进两格。
///
/// `settings:` 无条件补：Go 只在「全文没有任何 `settings:` 行」时才补，而正文上方有内容、
/// `settings:` 掉在后面的形态（`port: "1"\nsettings:`）会让它既不补外壳、又把那行
/// `settings:` 丢掉，输出 `version: 1\n  port: "1"` 这种非法 YAML。重建本来就是「结构已毁、
/// 尽力而为」的路径，这里宁可偏离 Go 也要保证写回的文件仍然可解析。
///
/// 缩进只整体 +2、**不 trim 行内相对缩进**：`TrimSpace` 会把 `cos:` 的 `secret_id` /
/// `bucket` 拍平到与 `cos` 同级，子项静默丢失（Go `config.go:782-784` 同病，本仓有意修正）。
fn rebuild_structure(lines: &[&str]) -> String {
    // 只认列 0 的 `version:`：缩进的 `version:` 是正文里的嵌套值，不能顶替外壳的版本行。
    let has_version = lines.iter().any(|line| line.trim_end().starts_with("version:"));
    let mut rebuilt = String::new();
    if !has_version {
        rebuilt.push_str("version: 1\n");
    }
    rebuilt.push_str("settings:\n");
    for line in lines {
        let bare = line.trim_end();
        // 空行、列 0 的文档标记 / null 标量与列 0 的（重复）`version:` / `settings:` 行不保留：
        // 它们已被新外壳取代（缩进的同名列是数据，照常保留；块标量内容按 YAML 规则必缩进，
        // 里面的 `---` 列不在 0，跟着 `line.trim_end()` 原样留下）。
        if bare.is_empty()
            || is_marker_or_null(bare)
            || bare == "settings:"
            || bare.starts_with("version:")
        {
            continue;
        }
        if bare.trim_start().starts_with('#') {
            rebuilt.push_str(bare); // 注释原样保留（含原有缩进）
        } else {
            rebuilt.push_str("  "); // 内容整体缩进两格，挂到 settings: 下
            rebuilt.push_str(bare);
        }
        rebuilt.push('\n');
    }
    rebuilt
}

/// 有内容的一行：非空、非注释，且不是文档标记 / null 标量。
fn is_content_line(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && !trimmed.starts_with('#') && !is_marker_or_null(trimmed)
}

/// YAML 的文档标记（`---` / `...`）与 null 标量（`null` / `Null` / `NULL` / `~`）。
///
/// 它们是结构标记而非配置内容（与 [`is_effectively_empty`] 同口径）。
fn is_marker_or_null(trimmed: &str) -> bool {
    matches!(trimmed, "---" | "..." | "null" | "Null" | "NULL" | "~")
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
        line.is_empty() || is_marker_or_null(line)
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

    /// 两份 `settings:`：截断到第二份之前，首份内容零改动。
    #[test]
    fn duplicate_settings_is_truncated_to_the_first_block() {
        let text = "version: 1\nsettings:\n  port: \"5700\"\nsettings:\n  port: \"5701\"\n  app_id: 42\n";
        let cleaned = cleanup_duplicate_settings(text);

        assert!(cleaned.truncated);
        assert!(!cleaned.rebuilt && !cleaned.normalized);
        // 截断走 `strings.Join` 等价路径，末行不再补换行（Go 同）。
        assert_eq!(
            cleaned.text.as_ref(),
            "version: 1\nsettings:\n  port: \"5700\""
        );
        let settings = parse_str(&cleaned.text).expect("截断后必须能解析");
        assert_eq!(settings.port, "5700");
        assert_eq!(settings.app_id, 0, "第二份 settings 的键必须一起丢掉");
    }

    /// 正文形态：补 `version: 1` + `settings:` 外壳，内容缩进两格、注释原样、空行丢弃。
    #[test]
    fn settings_body_is_rebuilt_with_shell() {
        let text = "# 我的配置\nport: \"5700\"\n\n# 第二段\nlog_level: 4\n";
        let cleaned = cleanup_duplicate_settings(text);

        assert!(cleaned.rebuilt);
        assert!(!cleaned.truncated && !cleaned.normalized);
        // 外壳在前，注释与内容按原顺序跟在后面（Go 先 append 外壳、再遍历原文）。
        assert_eq!(
            cleaned.text.as_ref(),
            "version: 1\nsettings:\n# 我的配置\n  port: \"5700\"\n# 第二段\n  log_level: 4\n"
        );
        let settings = parse_str(&cleaned.text).expect("重建后必须能解析");
        assert_eq!(settings.port, "5700");
        assert_eq!(settings.log_level, 4);
    }

    /// 正文上方有内容、`settings:` 掉在后面：Go 会省掉外壳、写出非法 YAML，这里补上外壳。
    #[test]
    fn settings_header_below_content_still_gets_a_valid_shell() {
        let cleaned = cleanup_duplicate_settings("port: \"5700\"\nsettings:\n");

        assert!(cleaned.rebuilt);
        assert_eq!(
            cleaned.text.as_ref(),
            "version: 1\nsettings:\n  port: \"5700\"\n"
        );
        assert_eq!(
            parse_str(&cleaned.text).expect("重建结果必须能解析").port,
            "5700"
        );
    }

    /// 合规文件：逐字节不变，`load` 也不写盘（mtime 不动）。
    #[test]
    fn compliant_file_is_not_rewritten() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        let text = "version: 1\nsettings:\n  port: \"5700\"\n";
        fs::write(&path, text).expect("写配置");
        let before = fs::metadata(&path)
            .expect("元数据")
            .modified()
            .expect("mtime");

        assert_eq!(load(&path).expect("加载失败").port, "5700");
        assert_eq!(fs::read_to_string(&path).expect("读回失败"), text);
        assert_eq!(
            fs::metadata(&path)
                .expect("元数据")
                .modified()
                .expect("mtime"),
            before,
            "没有清洗动作时不得写盘"
        );
    }

    /// 有清洗动作时才写回：截断后的文件落盘，且能解析。
    #[test]
    fn load_writes_back_the_truncated_file() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        fs::write(
            &path,
            "version: 1\nsettings:\n  port: \"5700\"\nsettings:\n  port: \"5701\"\n",
        )
        .expect("写坏配置");

        assert_eq!(load(&path).expect("清洗后必须能加载").port, "5700");
        assert_eq!(
            fs::read_to_string(&path).expect("读回失败"),
            "version: 1\nsettings:\n  port: \"5700\"",
            "截断结果必须写回原文件"
        );
    }

    /// CRLF / 裸 CR 一律折算成 LF。
    #[test]
    fn crlf_is_normalized_to_lf() {
        let cleaned = cleanup_duplicate_settings("version: 1\r\nsettings:\r\n  port: \"5700\"\r\n");
        assert!(cleaned.normalized);
        assert!(!cleaned.truncated && !cleaned.rebuilt);
        assert_eq!(
            cleaned.text.as_ref(),
            "version: 1\nsettings:\n  port: \"5700\"\n"
        );
    }

    /// 幂等：清洗两次与一次结果相同，第二次不再报任何动作。
    #[test]
    fn cleanup_is_idempotent() {
        for text in [
            "version: 1\nsettings:\n  port: \"5700\"\nsettings:\n  port: \"5701\"\n",
            "port: \"5700\"\nlog_level: 4\n",
            "version: 1\r\nsettings:\r\n",
            "# 只有注释\n",
            "",
        ] {
            let once = cleanup_duplicate_settings(text);
            let twice = cleanup_duplicate_settings(&once.text);
            assert_eq!(once.text, twice.text, "二次清洗必须无变化: {text:?}");
            assert!(
                !twice.truncated && !twice.rebuilt && !twice.normalized,
                "二次清洗不得再报动作: {text:?}"
            );
        }
    }

    /// BOM 先剥开、原样接回：不认识 BOM 会把合规文件误判成结构损坏并重建。
    #[test]
    fn bom_is_preserved() {
        let text = "\u{feff}version: 1\nsettings:\n  port: \"5700\"\n";
        let cleaned = cleanup_duplicate_settings(text);
        assert!(!cleaned.rebuilt && !cleaned.truncated && !cleaned.normalized);
        assert_eq!(cleaned.text.as_ref(), text);
    }

    /// 合法 YAML 文档起始符 `---` 不是内容：`---\nversion: …` 既不能判成结构损坏，
    /// 也不能被重建写成 `settings:\n  ---\n  …`（P43 评审实测：非法 YAML，进程起不来）。
    #[test]
    fn document_start_marker_is_not_content() {
        let text = "---\nversion: 1\nsettings:\n  port: \"0\"\n";
        let cleaned = cleanup_duplicate_settings(text);

        assert!(!cleaned.rebuilt && !cleaned.truncated && !cleaned.normalized);
        assert_eq!(cleaned.text.as_ref(), text, "合规文档必须逐字节不变");
        assert_eq!(parse_str(&cleaned.text).expect("必须能解析").port, "0");
    }

    /// `---` 后面的 settings 正文：外壳照补，但文档标记本身不得变成 `settings:` 的内容。
    #[test]
    fn document_marker_is_dropped_when_rebuilding() {
        let cleaned = cleanup_duplicate_settings("---\nport: \"5700\"\n");

        assert!(cleaned.rebuilt);
        assert_eq!(
            cleaned.text.as_ref(),
            "version: 1\nsettings:\n  port: \"5700\"\n"
        );
        assert_eq!(
            parse_str(&cleaned.text).expect("重建结果必须能解析").port,
            "5700"
        );
    }

    /// 重建只整体 +2，行内相对缩进保留：`cos` 的 `secret_id` / `bucket` 必须仍是 `cos` 的子项
    /// （`TrimSpace` 会把它们拍平到与 `cos` 同级，子项静默丢失）。
    #[test]
    fn rebuild_keeps_relative_indent_of_nested_maps() {
        let cleaned = cleanup_duplicate_settings(
            "cos:\n  secret_id: cos-id\n  bucket: bucket-1\nport: \"5700\"\n",
        );

        assert!(cleaned.rebuilt);
        assert_eq!(
            cleaned.text.as_ref(),
            "version: 1\nsettings:\n  cos:\n    secret_id: cos-id\n    bucket: bucket-1\n  port: \"5700\"\n"
        );
        let settings = parse_str(&cleaned.text).expect("重建结果必须能解析");
        assert_eq!(settings.cos.secret_id, "cos-id");
        assert_eq!(settings.cos.bucket, "bucket-1");
        assert_eq!(settings.port, "5700");
    }

    /// 截断只认列 0 的 `settings:`：正文里缩进的 `settings:` 是嵌套值，不得把后面整段截掉。
    #[test]
    fn indented_settings_key_does_not_truncate() {
        let text = "version: 1\nsettings:\n  menu:\n    settings:\n      theme: dark\n  port: \"5700\"\n";
        let cleaned = cleanup_duplicate_settings(text);

        assert!(!cleaned.truncated && !cleaned.rebuilt);
        assert_eq!(cleaned.text.as_ref(), text);
        assert_eq!(parse_str(&cleaned.text).expect("必须能解析").port, "5700");
    }

    /// 不变量：任何能解析为 `Settings` 的文档，清洗后必须仍可解析且 `Settings` 等值。
    #[test]
    fn sanitizing_never_changes_a_parseable_document() {
        for text in [
            // Go 文档形态 + 嵌套 map + 注释 + 空行
            "# 顶部注释\nversion: 1\nsettings:\n  port: \"5700\"\n  cos:\n    secret_id: cos-id\n    bucket: bucket-1\n\n  log_level: 4\n",
            // settings 正文形态（无外壳）
            "port: \"5700\"\nlog_level: 4\n",
            // 文档标记 + CRLF
            "---\r\nversion: 1\r\nsettings:\r\n  port: \"5700\"\r\n",
        ] {
            let before = parse_str(text).expect("样本本身必须可解析");
            let cleaned = cleanup_duplicate_settings(text);

            let after = match parse_str(&cleaned.text) {
                Ok(after) => after,
                Err(err) => panic!("清洗后必须仍可解析: {err}; 清洗结果 {:?}", cleaned.text),
            };
            assert_eq!(after, before, "清洗不得改变解析结果: {text:?}");
            // 已经有合法顶层的文档一律不得重建（重建只属于「结构已毁」的输入）。
            if text.starts_with("version:") || text.starts_with("---\r\nversion:") {
                assert!(!cleaned.rebuilt, "可解析的 Go 文档不得被重建: {text:?}");
            }
        }
    }

    /// BOM / CRLF 的落盘形态：BOM 保留、换行折成 LF、重复块截断——断言实际写回文件的字节。
    #[test]
    fn bom_and_crlf_are_written_back_as_lf() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        fs::write(
            &path,
            "\u{feff}version: 1\r\nsettings:\r\n  port: \"5700\"\r\nsettings:\r\n  port: \"5701\"\r\n",
        )
        .expect("写坏配置");

        assert_eq!(load(&path).expect("清洗后必须能加载").port, "5700");
        assert_eq!(
            fs::read(&path).expect("读回失败"),
            "\u{feff}version: 1\nsettings:\n  port: \"5700\"".as_bytes(),
            "落盘必须是带 BOM 的 LF 文本"
        );
    }
}
