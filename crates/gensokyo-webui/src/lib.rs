//! gensokyo-webui：内嵌前端资源（`webui/dist`）与管理 API。
//!
//! M0 只建立 crate 边界；`rust-embed` 与登录鉴权在 M3 落地。

/// 静态资源嵌入。
pub mod assets {}
/// cookie 鉴权。
pub mod auth {}
