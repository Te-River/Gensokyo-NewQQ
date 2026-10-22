//! gensokyo-core：统一错误类型、OneBot 虚拟 ID、日志与指标门面。
//!
//! 依赖方向的最底层 crate，不依赖任何其它 `gensokyo-*` crate。

pub mod error;
pub mod ids;
pub mod telemetry;

pub use error::{Error, Result};
pub use ids::Ob11Id;
