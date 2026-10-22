//! gensokyo-config：`config.yml` 映射、`Get*()` 等价访问器、快照热重载与首启模板生成。
//!
//! M1a 起 [`Settings`] 已与 Go `structs.Settings`（含嵌套结构体）逐字段对齐；M1b 补齐
//! [`Settings`] 及其嵌套结构体的访问器（见 [`accessors`]）；M1c 落地 [`ConfigManager`]
//! （`ArcSwap` 快照 + 字段级 diff + 重启项判定）、[`ConfigWatcher`]（`notify` 去抖监听）
//! 与 [`ensure_config_file`]（首启模板生成）。

pub mod accessors;
pub mod diff;
pub mod loader;
pub mod manager;
pub mod settings;
pub mod template;
pub mod watcher;

pub use diff::{RESTART_REQUIRED_TAGS, diff_tags, is_restart_required};
pub use loader::{LoadError, load, parse_str};
pub use manager::{ConfigManager, ReloadError, ReloadOutcome};
pub use settings::Settings;
pub use template::{CONFIG_TEMPLATE, TemplateError, ensure_config_file};
pub use watcher::{ConfigWatcher, WatcherError, WatcherHandle};
