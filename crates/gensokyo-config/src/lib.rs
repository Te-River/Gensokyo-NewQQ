//! gensokyo-config：`config.yml` 映射、`Get*()` 等价访问器、热重载。
//!
//! M1a 起 [`Settings`] 已与 Go `structs.Settings`（含嵌套结构体）逐字段对齐；`Get*()`
//! 等价访问器与 `ArcSwap` 快照 + `notify` 热重载在 M1b 落地。

pub mod loader;
pub mod settings;

pub use loader::{LoadError, load, parse_str};
pub use settings::Settings;
