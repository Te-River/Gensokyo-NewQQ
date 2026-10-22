//! gensokyo-media：下载上传、压缩、图床、silk 编解码、QR。
//!
//! M0 只建立 crate 边界；`image` / `reqwest` 依赖在 M1 随管线一起接入。

/// 图片上传、压缩与 base64（`images/`）。
pub mod image {}
/// 图床后端（`imagehosting/` 与 `oss/`）。
pub mod hosting {}
/// silk 音频编解码（`silk/` + `go-silk/` 的 C ABI 封装）。
pub mod silk {}
