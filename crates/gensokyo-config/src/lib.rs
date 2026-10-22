//! gensokyo-config：`config.yml` 映射、`Get*()` 等价访问器、热重载。
//!
//! M1a 起 [`Settings`] 已与 Go `structs.Settings`（含嵌套结构体）逐字段对齐；M1b 补齐
//! [`Settings`] 及其嵌套结构体的访问器（见 [`accessors`]）；`ArcSwap` 快照 + `notify`
//! 热重载与首启模板生成在 M1c 落地。

pub mod accessors;
pub mod loader;
pub mod settings;

pub use loader::{LoadError, load, parse_str};
pub use settings::Settings;
