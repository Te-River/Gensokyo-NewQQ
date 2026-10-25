//! gensokyo-actions：全部出站 action 实现（Go `handlers/` 的出站部分）。
//!
//! # 已实现
//!
//! | 模块 | action | 说明 |
//! |------|--------|------|
//! | [`send`] | — | 参数解析 + 出站计划构建：`params` → [`send::SendRequest`]（[`send::SendTarget`] + [`send::MessageOut`]）。`message` 支持 CQ 字符串、消息段数组与单个消息段对象，CQ 解析复用 `gensokyo_protocol` 的 [`gensokyo_protocol::MessageContent`] / [`gensokyo_protocol::OutboundPlan`]；[`send::SendRequest`] 支持序列化 → 反序列化往返，runtime 据此还原计划。 |
//! | [`handler`] | `send_group_msg`（别名 `send_to_group`）、`send_private_msg`、`send_msg` | 规划型 [`gensokyo_protocol::Handler`]：成功时返回 `"deferred": true` 的计划，**不发送**；参数缺失 / 非法返回 `retcode = 1400`。 |
//! | [`group`] | `get_group_info`、`get_group_member_info`、`get_group_member_list`、`set_group_ban`、`get_group_bot_state`、`get_group_join_request_list` | 真正调用 QQ 上游的查询 / 管理 action（M1k），经 [`qqapi::QqApi`] 抽象，成功回 `data`，参数错误 `1400` / 上游错误 `1500`。 |
//!
//! # 尚未实现
//!
//! - **`send_*` 的真实发送**：仍走 deferred 计划（[`handler::DEFERRED_FIELD`]），媒体上传、
//!   `MessageToCreate`、echo / 懒消息回写由 `gensokyo-runtime` 接线（见 [`send`] 模块文档）。
//! - **媒体类 action**：`[CQ:image]` / `[CQ:record]` / `[CQ:video]` / `[CQ:file]` 的上传与发送、
//!   图床选择，见 [`crate::admin`] 占位。
//! - **其余群管理 / 面板 action**：入群审批（`set_group_add_request`）、全员禁言、踢人、群名片、
//!   成员黑名单、自动审批策略、自定义菜单、指令面板——上游端点也大多未在 `gensokyo-qq` 落地。
//! - **私聊 / C2C 的查询与管理**：见 [`crate::private`] 占位。
//! - **`get_group_list` / `get_stranger_info` / `get_msg` / `delete_msg` 等**：协议类型已在
//!   `gensokyo_protocol::action` 定义，handler 尚未实现。
//!
//! # 与 Go 的分工
//!
//! Go 的 `handlers/send_*_msg.go` 一个函数干完「解析 → 上传 → 发送 → 回写 echo」；这里把
//! **参数解析与 JSON 形状**（可纯本地单测）和 **上游 I/O** 分开：前者在 [`send`] / [`group`]，
//! 后者只在 [`qqapi`] 一处收口，handler 通过 [`qqapi::ActionContext`] 拿依赖。echo 回填由
//! handler 原样带上（runtime 统一处理的语义不变）。

/// `params` 的公共取值规则（ID 数字/字符串归一），供发送类与查询类共用。
mod params;
/// 最小 RFC3339 工具（Unix 秒 ⇄ UTC 时间串）。
mod rfc3339;

/// 群聊查询与管理 action（M1k）。
pub mod group;
/// private / C2C 查询与管理 action（M1j 起）。
pub mod private {}
/// 富媒体上传与面板类 action。
pub mod admin {}

pub mod handler;
pub mod qqapi;
pub mod send;

pub use group::{
    BotStateData, DEFAULT_JOIN_REQUEST_LIMIT, DEFAULT_MEMBER_PAGE, GetGroupBotStateHandler,
    GetGroupInfoHandler, GetGroupJoinRequestListHandler, GetGroupMemberInfoHandler,
    GetGroupMemberListHandler, JoinRequestEntry, JoinRequestListData, MAX_JOIN_REQUEST_LIMIT,
    MAX_MEMBER_PAGE, RETCODE_UPSTREAM, SetGroupBanHandler,
};
pub use handler::{
    DEFERRED_FIELD, RETCODE_BAD_REQUEST, SendGroupHandler, SendMsgHandler, SendPrivateHandler,
};
pub use qqapi::{ActionContext, QqApi, QqFuture};
pub use send::{
    MessageOut, SendRequest, SendTarget, plan_send_group_msg, plan_send_msg, plan_send_private_msg,
};
