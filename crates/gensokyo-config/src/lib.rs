//! gensokyo-config：`config.yml` 映射、`Get*()` 等价访问器、快照热重载与首启模板生成。
//!
//! M1a 起 [`Settings`] 已与 Go `structs.Settings`（含嵌套结构体）逐字段对齐；M1b 补齐
//! [`Settings`] 及其嵌套结构体的访问器（见 [`accessors`]）；M1c 落地 [`ConfigManager`]
//! （`ArcSwap` 快照 + 字段级 diff + 重启项判定）、[`ConfigWatcher`]（`notify` 去抖监听）、
//! [`ensure_config_file`]（首启模板生成）与 [`ensure_config_complete`]（缺键回写，Go
//! `ensureConfigComplete`）；[`cleanup_config_file`] 在读盘前清洗旧版 Go 留下的重复
//! `settings:` / 损坏顶层结构（Go `cleanupDuplicateSettings`）。

pub mod accessors;
pub mod complete;
pub mod diff;
pub mod loader;
pub mod manager;
pub mod settings;
pub mod template;
pub mod watcher;

pub use complete::{CompleteError, CompletionReport, ensure_config_complete};
pub use diff::{RESTART_REQUIRED_TAGS, diff_tags, is_restart_required};
pub use loader::{LoadError, cleanup_config_file, load, parse_str};
pub use manager::{ConfigManager, ReloadError, ReloadOutcome};
pub use settings::Settings;
pub use template::{CONFIG_TEMPLATE, TemplateError, ensure_config_file};
pub use watcher::{ConfigWatcher, WatcherError, WatcherHandle};

/// 早期告警出口：`tracing` 订阅器还没装好时，事件会被静默丢弃。
///
/// 二进制在 `logging::init` **之前**就要跑首启模板生成、结构清洗与补键回写，这三个路径
/// 会真的改写 `config.yml`；只发 `tracing::warn!` 的话生产上「文件已经变了、日志里查不到」。
/// 订阅器未安装时这里直接写 stderr（进程管理器照样收走），装好之后交给 `tracing`
/// （stderr + WebUI 日志流），不会重复出两行。
///
/// 订阅器起来之后的告警不要走这里，直接用 `tracing::warn!` 带结构化字段。
pub fn warn_early(message: &str) {
    if tracing::dispatcher::has_been_set() {
        tracing::warn!("{message}");
    } else {
        eprintln!("[warn] {message}");
    }
}
