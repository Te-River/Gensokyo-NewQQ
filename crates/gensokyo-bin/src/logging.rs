//! 最小 tracing 输出器。
//!
//! 工作区根 `Cargo.toml` 没有声明 `tracing-subscriber`（M1j 不改根清单），这里实现一个够用的
//! [`Subscriber`]：按级别过滤、事件写 stderr、span 只分配 id 不输出。
//!
//! 过滤规则：
//! - `RUST_LOG` 非空时以它为准，支持 `info`、`debug,gensokyo_runtime=trace` 这类写法
//!   （逗号分隔的 `level` 或 `target=level`，未知片段忽略）；
//! - 否则用 `config.log_level`，映射与 Go `mylog.GetLogLevelFromConfig` 一致：
//!   0=debug 1=info 2=warn 3=error，其它值回退 info。

use std::fmt;
use std::io::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

use tracing::field::{Field, Visit};
use tracing::level_filters::LevelFilter;
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

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
pub fn init(config_level: i64) {
    let fallback = level_from_config(config_level);
    let filter = match std::env::var("RUST_LOG") {
        Ok(spec) if !spec.trim().is_empty() => Filter::parse(&spec, fallback),
        _ => Filter::new(fallback),
    };
    if let Err(err) = tracing::subscriber::set_global_default(StderrSubscriber::new(filter)) {
        eprintln!("tracing 订阅器未安装（沿用上次设置）：{err}");
    }
}

/// 级别过滤表。
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
        self.directives
            .iter()
            .rev()
            .find(|(prefix, _)| target.starts_with(prefix.as_str()))
            .map_or(self.default, |(_, level)| *level)
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
}

/// 写 stderr 的订阅器。
#[derive(Debug)]
struct StderrSubscriber {
    filter: Filter,
    next_span_id: AtomicU64,
}

impl StderrSubscriber {
    fn new(filter: Filter) -> Self {
        Self {
            filter,
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

        let mut stderr = std::io::stderr().lock();
        let _ = write!(
            stderr,
            "{} {} {}: ",
            timestamp(),
            metadata.level(),
            metadata.target()
        );
        if let Some(message) = &visitor.message {
            let _ = write!(stderr, "{message}");
        }
        for (name, value) in &visitor.fields {
            let _ = write!(stderr, " {name}={value}");
        }
        let _ = writeln!(stderr);
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

/// RFC3339 UTC 时间戳；格式化失败时给 `?`，不因为一行日志 panic。
fn timestamp() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "?".to_owned())
}

#[cfg(test)]
mod tests {
    use tracing::level_filters::LevelFilter;

    use super::{Filter, level_from_config, parse_level};

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
    fn off_and_aliases_are_recognised() {
        assert_eq!(parse_level("OFF"), Some(LevelFilter::OFF));
        assert_eq!(parse_level("warning"), Some(LevelFilter::WARN));
        assert_eq!(parse_level("nonsense"), None);
    }
}
