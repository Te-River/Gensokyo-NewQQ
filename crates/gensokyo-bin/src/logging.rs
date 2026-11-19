//! 最小 tracing 输出器。
//!
//! 工作区根 `Cargo.toml` 没有声明 `tracing-subscriber`（M1j 不改根清单），这里实现一个够用的
//! [`Subscriber`]：按级别过滤、事件写 stderr、span 只分配 id 不输出。
//!
//! stderr 的行格式照 Go 版控制台日志逐字符对齐：
//! `2026/NOV/19 PM 07:27:11 [INFO ] [core] 正文`——本地时间（12 小时制、月份大写三字母）、
//! 级别宽 5 左对齐带方括号、短模块名（`[core]` / `[openapi]` / `[ws]`），最后是拍平的正文。
//! 只给级别标记上色（DEBUG/TRACE 蓝、INFO 绿、WARN 黄、ERROR 红），正文与模块不着色。
//!
//! 同一份已过滤事件还会推进 [`gensokyo_webui::logs::hub`]（环形缓冲 + 广播），供 WebUI 的
//! 日志 WS（`/api/logs`、`/api/{uin}/process/logs`）回放与实时推送——等价 Go `mylog.emitLog`
//! 往包级 `logChannel` 投递。两条出口共用同一个级别过滤，因此 WebUI 看到的行与 stderr 一致；
//! WebUI 里的 `time` / `level` 字段保持原有口径，不跟着 stderr 格式变。
//!
//! 过滤规则（Go 版第三方库不往日志里写 DEBUG，用户反馈「太繁琐」就出在这里）：
//! - 配置级别与 `RUST_LOG` 的裸级别只作用于本仓 crate（`gensokyo`、`gensokyo_*`）；
//!   其余 target（第三方）最多放到 INFO，除非用户显式写了 `target=level`；
//! - `RUST_LOG` 非空时以它为准，支持 `info`、`debug,gensokyo_runtime=trace` 这类写法；
//! - 否则用 `config.log_level`，映射与 Go `mylog.GetLogLevelFromConfig` 一致：
//!   0=debug 1=info 2=warn 3=error，其它值回退 info。

use std::fmt;
use std::io::{IsTerminal as _, Write as _};
use std::sync::atomic::{AtomicU64, Ordering};

use gensokyo_webui::logs::{self, LogEntry};
use tracing::field::{Field, Visit};
use tracing::level_filters::LevelFilter;
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Metadata, Subscriber};

/// 级别标记的前景色（只包住 `[INFO ]`，正文与模块不着色）。
///
/// Go 版的 FATAL 在 tracing 里没有对应级别（进程直接退出），故只覆盖五个级别。
const COLOR_DEBUG: &str = "\x1b[34m";
const COLOR_INFO: &str = "\x1b[32m";
const COLOR_WARN: &str = "\x1b[33m";
const COLOR_ERROR: &str = "\x1b[31m";
const COLOR_RESET: &str = "\x1b[0m";

/// `config.log_level` → 过滤级别（Go `mylog.GetLogLevelFromConfig` 的等价映射）。
#[must_use]
pub fn level_from_config(level: i64) -> LevelFilter {
    match level {
        0 => LevelFilter::DEBUG,
        2 => LevelFilter::WARN,
        3 => LevelFilter::ERROR,
        _ => LevelFilter::INFO,
    }
}

/// 安装全局订阅器；重复安装或安装失败都不 panic，只提示一次。
///
/// `config_color` 是 `config.log_color_enabled`：只有它为真、且 stderr 是终端时才上色，
/// 重定向到文件 / 管道时不写 ANSI。
pub fn init(config_level: i64, config_color: bool) {
    let color = config_color && std::io::stderr().is_terminal();
    if color {
        enable_ansi();
    }
    let fallback = level_from_config(config_level);
    let filter = match std::env::var("RUST_LOG") {
        Ok(spec) if !spec.trim().is_empty() => Filter::parse(&spec, fallback),
        _ => Filter::new(fallback),
    };
    if let Err(err) = tracing::subscriber::set_global_default(StderrSubscriber::new(filter, color))
    {
        eprintln!("tracing 订阅器未安装（沿用上次设置）：{err}");
    }
}

/// 旧版 conhost 默认不解析 ANSI 转义（Go 1.17+ 由 runtime 自己打开
/// `ENABLE_VIRTUAL_TERMINAL_PROCESSING`）。用 anstream 同款的安全封装打开 stdout/stderr 的 VT；
/// 失败（管道、已分离的控制台）就保持原控制台模式，转义序列按原样输出而不 panic。
#[cfg(windows)]
fn enable_ansi() {
    let _ = anstyle_query::windows::enable_virtual_terminal_processing();
}

/// 非 Windows 终端默认解析 ANSI，无需额外开关。
#[cfg(not(windows))]
fn enable_ansi() {}

/// 级别过滤表。
///
/// `default` 只代表本仓 crate 的级别；第三方 target 走 [`Filter::level`] 里的 INFO 上限。
#[derive(Debug, Clone)]
struct Filter {
    default: LevelFilter,
    /// `(target 前缀, 级别)`，后出现的优先。
    directives: Vec<(String, LevelFilter)>,
}

impl Filter {
    fn new(default: LevelFilter) -> Self {
        Self {
            default,
            directives: Vec::new(),
        }
    }

    fn parse(spec: &str, fallback: LevelFilter) -> Self {
        let mut filter = Self::new(fallback);
        for piece in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match piece.split_once('=') {
                Some((target, level)) => {
                    if let Some(level) = parse_level(level) {
                        filter.directives.push((target.trim().to_owned(), level));
                    }
                }
                None => {
                    if let Some(level) = parse_level(piece) {
                        filter.default = level;
                    }
                }
            }
        }
        filter
    }

    fn level(&self, target: &str) -> LevelFilter {
        if let Some((_, level)) = self
            .directives
            .iter()
            .rev()
            .find(|(prefix, _)| target.starts_with(prefix.as_str()))
        {
            return *level;
        }
        if is_in_repo(target) {
            self.default
        } else if self.default > LevelFilter::INFO {
            // 第三方 target 最多放到 INFO：配置 / `RUST_LOG` 写 debug、trace 都不连带放开它们。
            LevelFilter::INFO
        } else {
            self.default
        }
    }

    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        LevelFilter::from_level(*metadata.level()) <= self.level(metadata.target())
    }

    /// 任一 target 的级别都不能超过它，否则 tracing 会在调用点直接丢弃事件。
    fn max_level(&self) -> LevelFilter {
        self.directives
            .iter()
            .map(|(_, level)| *level)
            .chain(std::iter::once(self.default))
            .max()
            .unwrap_or(LevelFilter::OFF)
    }
}

/// 是否本仓 crate 的 target：`gensokyo`（bin 自身）与 `gensokyo_*`。
fn is_in_repo(target: &str) -> bool {
    target == "gensokyo" || target.starts_with("gensokyo::") || target.starts_with("gensokyo_")
}

fn parse_level(raw: &str) -> Option<LevelFilter> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "off" => Some(LevelFilter::OFF),
        "error" => Some(LevelFilter::ERROR),
        "warn" | "warning" => Some(LevelFilter::WARN),
        "info" => Some(LevelFilter::INFO),
        "debug" => Some(LevelFilter::DEBUG),
        "trace" => Some(LevelFilter::TRACE),
        _ => None,
    }
}

/// 完整 target → Go 版日志里的短模块名。
///
/// - `gensokyo` / `gensokyo::xxx` 是 bin 自身 → `core`；
/// - `gensokyo_<crate>::<模块>` → `<模块>` 的第一段（`gensokyo_qq::openapi` → `openapi`）；
/// - 裸 `gensokyo_<crate>` → `<crate>`（`gensokyo_runtime` → `runtime`）；
/// - 其它（第三方）→ 最后一段（`hyper_util::client::legacy::pool` → `pool`）。
fn short_module(target: &str) -> &str {
    if target == "gensokyo" || target.starts_with("gensokyo::") {
        return "core";
    }
    if let Some(rest) = target.strip_prefix("gensokyo_") {
        let mut segments = rest.split("::");
        let crate_name = segments.next().unwrap_or(rest);
        return segments.next().unwrap_or(crate_name);
    }
    target.rsplit("::").next().unwrap_or(target)
}

/// `[INFO ]` / `[DEBUG]`：Go `fmt.Sprintf("[%-5s]", level)`，宽 5、左对齐。
fn level_tag(level: Level) -> String {
    format!("[{level:<5}]")
}

/// 级别标记的颜色；正文与模块不上色。
fn level_color(level: Level) -> &'static str {
    match level {
        Level::TRACE | Level::DEBUG => COLOR_DEBUG,
        Level::INFO => COLOR_INFO,
        Level::WARN => COLOR_WARN,
        Level::ERROR => COLOR_ERROR,
    }
}

/// 事件字段收集：`message` 单独存，其余按 `key=value` 追加。
#[derive(Debug, Default)]
struct FieldVisitor {
    message: Option<String>,
    fields: Vec<(String, String)>,
}

impl Visit for FieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record(field, format!("{value:?}"));
    }
}

impl FieldVisitor {
    fn record(&mut self, field: &Field, value: String) {
        if field.name() == "message" {
            self.message = Some(value);
        } else {
            self.fields.push((field.name().to_owned(), value));
        }
    }

    /// 拍平成一行：`<message> <key>=<value> ...`（stderr 与 WebUI 日志共用同一份正文）。
    fn render(&self) -> String {
        let mut body = self.message.clone().unwrap_or_default();
        for (name, value) in &self.fields {
            body.push(' ');
            body.push_str(name);
            body.push('=');
            body.push_str(value);
        }
        body
    }
}

/// 写 stderr 的订阅器。
#[derive(Debug)]
struct StderrSubscriber {
    filter: Filter,
    color: bool,
    next_span_id: AtomicU64,
}

impl StderrSubscriber {
    fn new(filter: Filter, color: bool) -> Self {
        Self {
            filter,
            color,
            next_span_id: AtomicU64::new(1),
        }
    }
}

impl Subscriber for StderrSubscriber {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.filter.enabled(metadata)
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(self.filter.max_level())
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(self.next_span_id.fetch_add(1, Ordering::Relaxed))
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let metadata = event.metadata();
        if !self.filter.enabled(metadata) {
            return;
        }
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        let body = visitor.render();
        let time = log_time();

        // WebUI 日志流：与 stderr 同一份过滤结果，等价 Go `mylog.emitLog` → `logChannel`。
        logs::hub().push(LogEntry {
            time: time.clone(),
            level: metadata.level().to_string(),
            message: body.clone(),
        });

        let tag = level_tag(*metadata.level());
        let module = short_module(metadata.target());
        let mut stderr = std::io::stderr().lock();
        let _ = if self.color {
            writeln!(
                stderr,
                "{time} {}{tag}{COLOR_RESET} [{module}] {body}",
                level_color(*metadata.level())
            )
        } else {
            writeln!(stderr, "{time} {tag} [{module}] {body}")
        };
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

/// 本地时间串，照 Go `strings.ToUpper(time.Now().Format("2006/Jan/02 PM 03:04:05"))`：
/// `2026/NOV/08 PM 03:04:05`（12 小时制、小时补零、无毫秒、无时区后缀）。stderr 与 WebUI 共用。
///
/// Go 的 `time.Now()` 是本地时间；取不到本地偏移时退回 UTC，不让一行日志 panic。
fn log_time() -> String {
    let layout = time::macros::format_description!(
        "[year]/[month repr:short]/[day] [period case:upper] [hour repr:12 padding:zero]:[minute]:[second]"
    );
    time::OffsetDateTime::now_local()
        .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
        .format(layout)
        .map(|stamp| stamp.to_ascii_uppercase())
        .unwrap_or_else(|_| "?".to_owned())
}

#[cfg(test)]
mod tests {
    use tracing::Level;
    use tracing::level_filters::LevelFilter;

    use super::{Filter, level_from_config, level_tag, log_time, parse_level, short_module};

    #[test]
    fn config_level_matches_go_mapping() {
        assert_eq!(level_from_config(0), LevelFilter::DEBUG);
        assert_eq!(level_from_config(1), LevelFilter::INFO);
        assert_eq!(level_from_config(2), LevelFilter::WARN);
        assert_eq!(level_from_config(3), LevelFilter::ERROR);
        assert_eq!(level_from_config(7), LevelFilter::INFO, "其它值回退 info");
        assert_eq!(level_from_config(-1), LevelFilter::INFO);
    }

    #[test]
    fn rust_log_directives_are_parsed() {
        let filter = Filter::parse("warn,gensokyo_runtime=debug,unknown", LevelFilter::INFO);
        assert_eq!(filter.level("gensokyo_bin"), LevelFilter::WARN);
        assert_eq!(filter.level("gensokyo_runtime::boot"), LevelFilter::DEBUG);
        assert_eq!(filter.level("hyper"), LevelFilter::WARN);
        assert_eq!(filter.max_level(), LevelFilter::DEBUG);
    }

    #[test]
    fn third_party_targets_are_capped_at_info() {
        // `RUST_LOG=debug` 只放开本仓 crate；第三方仍是 info（用户嫌「太繁琐」的根因）。
        let filter = Filter::parse("debug", LevelFilter::INFO);
        assert_eq!(
            filter.level("gensokyo_runtime::gateway"),
            LevelFilter::DEBUG
        );
        assert_eq!(
            filter.level("gensokyo_storage::idmap::batch"),
            LevelFilter::DEBUG
        );
        assert_eq!(
            filter.level("hyper_util::client::legacy::pool"),
            LevelFilter::INFO
        );

        // 显式指令仍然能单独放开某个第三方 target。
        let filter = Filter::parse("debug,hyper_util=debug", LevelFilter::INFO);
        assert_eq!(
            filter.level("hyper_util::client::legacy::pool"),
            LevelFilter::DEBUG
        );

        // 配置级别比 info 更安静时，第三方跟着配置走，不会反而更吵。
        let filter = Filter::new(LevelFilter::WARN);
        assert_eq!(
            filter.level("hyper_util::client::legacy::pool"),
            LevelFilter::WARN
        );
    }

    #[test]
    fn short_module_follows_go_naming() {
        assert_eq!(short_module("gensokyo"), "core");
        assert_eq!(short_module("gensokyo::cli"), "core");
        assert_eq!(short_module("gensokyo_runtime::gateway"), "gateway");
        assert_eq!(short_module("gensokyo_qq::openapi"), "openapi");
        assert_eq!(short_module("gensokyo_storage::idmap::batch"), "idmap");
        assert_eq!(short_module("gensokyo_storage"), "storage");
        assert_eq!(short_module("hyper_util::client::legacy::pool"), "pool");
    }

    #[test]
    fn level_tag_is_five_wide() {
        assert_eq!(level_tag(Level::INFO), "[INFO ]");
        assert_eq!(level_tag(Level::DEBUG), "[DEBUG]");
        assert_eq!(level_tag(Level::WARN), "[WARN ]");
        assert_eq!(level_tag(Level::ERROR), "[ERROR]");
    }

    #[test]
    fn off_and_aliases_are_recognised() {
        assert_eq!(parse_level("OFF"), Some(LevelFilter::OFF));
        assert_eq!(parse_level("warning"), Some(LevelFilter::WARN));
        assert_eq!(parse_level("nonsense"), None);
    }

    #[test]
    fn log_time_matches_go_layout() {
        let stamp = log_time();
        let (date, clock) = stamp.split_once(' ').expect("应有两段：日期 时间");
        let date_parts: Vec<&str> = date.split('/').collect();
        assert_eq!(date_parts.len(), 3, "{stamp}");
        assert_eq!(date_parts[0].len(), 4, "四位年 {stamp}");
        assert!(
            date_parts[1].len() == 3 && date_parts[1].chars().all(|c| c.is_ascii_uppercase()),
            "月份应是 Go `Jan` 的大写形 {stamp}"
        );
        assert_eq!(date_parts[2].len(), 2, "两位日 {stamp}");

        let (period, time_of_day) = clock.split_once(' ').expect("应有 AM/PM {stamp}");
        assert!(period == "AM" || period == "PM", "{stamp}");
        let hms: Vec<&str> = time_of_day.split(':').collect();
        assert_eq!(hms.len(), 3, "{stamp}");
        let hour: u8 = hms[0].parse().expect("小时应是数字");
        assert!((1..=12).contains(&hour), "12 小时制 {stamp}");
        assert_eq!(hms[0].len(), 2, "小时补零 {stamp}");
    }
}
