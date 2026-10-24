//! gensokyo-actions：全部出站 action 实现（Go `handlers/` 的出站部分）。
//!
//! # M1i 覆盖范围（文本类出站 action，纯规划）
//! - [`send`]：参数解析 + 出站计划构建——`params` → [`send::SendRequest`]
//!   （[`send::SendTarget`] + [`send::MessageOut`]）。`message` 支持 CQ 字符串、消息段数组与
//!   单个消息段对象，CQ 解析复用 `gensokyo_protocol` 的 [`gensokyo_protocol::MessageContent`] /
//!   [`gensokyo_protocol::OutboundPlan`]；[`send::SendRequest`] 支持序列化 → 反序列化往返，
//!   runtime 据此还原计划。
//! - [`handler`]：`send_group_msg` / `send_private_msg` / `send_msg` 的
//!   [`gensokyo_protocol::Handler`] 适配器，成功时返回 `"deferred": true` 的规划结果，
//!   参数缺失 / 非法时返回 `retcode = 1400`。
//!
//! # 与 Go 的分工
//! Go 的 `handlers/send_*_msg.go` 一个函数干完「解析 → 上传 → 发送 → 回写 echo」；这里只保留
//! 第一步（可纯 JSON 单测），QQ 网络调用与媒体上传由 `gensokyo-runtime` 接线。细节与未做项
//! （媒体上传、SSM、主动消息、idmap / echo 回写、`send_private_msg` 的群临时会话群上下文）
//! 见 [`send`] 模块文档。
//!
//! # 尚未落地
//! 富媒体上传之外的群管理 / 面板类 action 见 [`admin`]（后续里程碑）。

/// 群聊富媒体与其余群聊 action（M1j 起）；文本类见 [`send`] / [`handler`]。
pub mod group {}
/// 私聊 / C2C 富媒体与其余私聊 action（M1j 起）；文本类见 [`send`] / [`handler`]。
pub mod private {}
/// 群管理与面板类 action。
pub mod admin {}

pub mod handler;
pub mod send;

pub use handler::{
    DEFERRED_FIELD, RETCODE_BAD_REQUEST, SendGroupHandler, SendMsgHandler, SendPrivateHandler,
};
pub use send::{
    MessageOut, SendRequest, SendTarget, plan_send_group_msg, plan_send_msg, plan_send_private_msg,
};
