//! gensokyo-config：`config.yml` 映射、`Get*()` 等价访问器、热重载。
//!
//! M0 只提供 Settings 骨架与加载骨架；字段全量、`ArcSwap` 快照与 `notify` 热重载在 M1
//! 按 Go `structs.Settings` / `config/` 逐项迁移。

pub mod loader;
pub mod settings;

pub use loader::{LoadError, load, parse_str};
pub use settings::Settings;
