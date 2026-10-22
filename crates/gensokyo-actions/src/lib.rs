//! gensokyo-actions：全部出站 action 实现（Go `handlers/` 的出站部分）。
//!
//! M0 只建立 crate 边界；action 实现与注册清单在 M1 按 `callapi.RegisterHandler` 全集落地。

/// 群聊 action（文本 + 富媒体）。
pub mod group {}
/// 私聊 / C2C action（文本 + 富媒体）。
pub mod private {}
/// 群管理与面板类 action。
pub mod admin {}
