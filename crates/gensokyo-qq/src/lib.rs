//! gensokyo-qq：QQ 官方 API 适配层（token 刷新、OpenAPI v1/v2、群管理、事件 DTO）。
//!
//! M0 只建立 crate 边界；模块文件随 M1 的 botgo 导出清单逐项落地。

/// access token 获取与刷新（`botgo/token`）。
pub mod token {}
/// OpenAPI v1/v2 客户端（`botgo/openapi`）。
pub mod openapi {}
/// QQ 侧事件 DTO（botgo 自定义事件类型）。
pub mod dto {}
/// 群管理 API（`botgo` 的 `GroupAPI`）。
pub mod group {}
