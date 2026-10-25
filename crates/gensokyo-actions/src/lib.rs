//! gensokyo-actions：全部出站 action 实现（Go `handlers/` 的出站部分）。
//!
//! # 已实现
//!
//! | 模块 | action | 说明 |
//! |------|--------|------|
//! | [`send`] | — | 参数解析 + 出站计划构建：`params` → [`send::SendRequest`]（[`send::SendTarget`] + [`send::MessageOut`]）。`message` 支持 CQ 字符串、消息段数组与单个消息段对象，CQ 解析复用 `gensokyo_protocol` 的 [`gensokyo_protocol::MessageContent`] / [`gensokyo_protocol::OutboundPlan`]；[`send::SendRequest`] 支持序列化 → 反序列化往返，runtime 据此还原计划。 |
//! | [`handler`] | `send_group_msg`（别名 `send_to_group`）、`send_private_msg`、`send_msg`、`send_group_msg_async`、`send_private_msg_async`、`send_msg_async` | 规划型 [`gensokyo_protocol::Handler`]：成功时返回 `"deferred": true` 的计划，**不发送**；参数缺失 / 非法返回 `retcode = 1400`。三个 `*_async` 与同步版共用同一套规划函数，响应形状逐字段一致。 |
//! | [`group`] | `get_group_info`、`get_group_member_info`、`get_group_member_list`、`set_group_ban`（别名 `get_group_ban`）、`get_group_bot_state`、`get_group_join_request_list` | 真正调用 QQ 上游的查询 / 管理 action，经 [`qqapi::QqApi`] 抽象，成功回 `data`，参数错误 `1400` / 上游错误 `1500`。 |
//! | [`group_admin`] | `set_group_whole_ban`（别名 `get_group_whole_ban`）、`set_group_card`、`set_group_kick`、`set_group_leave`、`get_group_list` | 群管理：前两个照 Go 口径固定回 `retcode = 100`（官方无写入路径），其余 `1500`（端点未落地）。 |
//! | [`messages`] | `delete_msg`、`get_msg`、`get_forward_msg` | 消息撤回 / 查询；参数与 message_id 还原已实现，上游端点未落地时 `1500`。 |
//! | [`users`] | `get_friend_list`、`get_stranger_info`、`send_like` | 用户列表 / 资料 / 点赞；同上游限制回 `1500`。 |
//! | [`media`] | `get_image`、`get_record` | 纯本地：`file://` 归一成本地路径、其余原样回填进 `{"file": …}`；缺 `file` 回 `1400`。 |
//! | [`system`] | `set_restart`、`clean_cache`、`get_cookies`、`get_csrf_token`、`get_credentials` | 运行控制 / 凭证：前两个只记审计日志并回 `{"code": 0}`（不重启、不清缓存），后三个回空凭证。 |
//! | [`unsupported`] | `get_group_at_all_remain`、`set_group_name`、`set_group_admin`、`set_group_special_title`、`set_group_remark`、`set_qq_profile`、`get_qq_profile`、`get_group_system_msg`、`get_online_clients` | 无上游端点的 action：参数照 OneBot 契约校验后回 `1500`；`get_online_clients` 例外，逐字段照抄 Go 的本地回执 `{"clients": [], "tiny_id": 0}`。 |
//! | [`capability`] | `can_send_image`、`can_send_record` | 纯本地判断，固定 `{"yes": true}`。 |
//!
//! # KNOWN_ACTIONS 覆盖率
//!
//! [`gensokyo_protocol::registry::KNOWN_ACTIONS`] 是 Go `callapi.RegisterHandler` 的全集。
//! 本 crate 目前覆盖其中 **22 项**：19 个注册名（[`IMPLEMENTED_ACTIONS`] 与 KNOWN_ACTIONS 的交集）
//! 加 3 个 Go 别名（`send_to_group` / `get_group_ban` / `get_group_whole_ban`，由
//! [`gensokyo_protocol::registry::ACTION_ALIASES`] 指向对应 handler）。
//!
//! 另有 22 个 action 在 Go 里从未注册，登记在 [`EXTENSION_ACTIONS`]（OneBot v11 标准 action 与
//! go-cqhttp 扩展名）：`get_image`、`get_record`、`set_restart`、`clean_cache`、`get_cookies`、
//! `get_csrf_token`、`get_credentials`、`get_group_at_all_remain`、`get_group_system_msg`、
//! `set_group_name`、`set_group_admin`、`set_group_special_title`、`set_group_remark`、
//! `set_qq_profile`、`get_qq_profile`、`get_msg`、`get_forward_msg`、`get_stranger_info`、
//! `send_like`、`can_send_image`、`can_send_record`、`set_group_leave`。
//!
//! 两张清单由单测 `implemented_actions_are_known_or_documented_extensions` 机器核对：每个已实现
//! action 必须落在 KNOWN_ACTIONS 或 EXTENSION_ACTIONS 里，别名必须指向已实现的 handler。
//!
//! # deferred 执行链路
//!
//! 1. [`register::register_all`] 把 41 个 handler 装进 `gensokyo_protocol::Registry`（runtime 装配，
//!    见 `gensokyo-runtime::AppState::with_actions`）。
//! 2. 客户端调 `send_group_msg` / `send_private_msg` / `send_msg`（含 `*_async`）：handler 只做参数
//!    解析，`data` = [`send::SendRequest`] JSON + `"deferred": true`，**不触达上游**。
//! 3. runtime 的 `dispatcher::Dispatcher` 看到 `deferred` 后反序列化 `SendRequest`，把
//!    [`send::MessageOut`] 合成 `MessageToCreate`（文本 + `[CQ:at]` → `<@openid>` + 单个 media），
//!    收件人经 [`qqapi::ActionContext::resolve_send_target`] 还原成上游 OpenID。
//! 4. 按 target 调 [`qqapi::QqApi::send_group_message`] / [`qqapi::QqApi::send_c2c_message`]，
//!    最终回客户端 `data = {"message_id": …}`——计划本身（含 `deferred` 标记）不会泄漏。
//!
//! 尚未接线的部分：媒体上传（图片需先拿到上游 `file_info`）、echo / 懒消息回写。
//!
//! # 尚未实现
//!
//! - **`send_*` 的真实发送**：handler 仍只产出 deferred 计划（[`handler::DEFERRED_FIELD`]），
//!   真正的 QQ 调用在 `gensokyo-runtime` 的 deferred 执行链上完成，链路见下文。
//! - **媒体上传**：`[CQ:image]` / `[CQ:record]` / `[CQ:video]` / `[CQ:file]` 的上传、图床选择
//!   仍未接线；deferred 执行链只接受「已是上游 `file_info`」的图片（[`gensokyo_protocol::MediaRef::Unknown`]），
//!   其余媒体形态回 `1500`，不做静默丢弃。
//! - **入群审批 / 黑名单 / 面板 / 菜单**（KNOWN_ACTIONS 剩余部分）：`set_group_add_request`、
//!   `get_group_member_blacklist`、`set_group_member_blacklist`、`join_approval_strategy_*`、
//!   `create_panel` / `set_panel` / `delete_panel` / `get_panel` / `get_panel_list` /
//!   `set_panel_target`、`set_custom_menu` / `get_custom_menu`、`put_interaction`、
//!   `mark_msg_as_read`、`delete_group_msg`、`send_group_forward_msg`、`get_avatar`、
//!   `get_login_info` / `get_status` / `get_version_info` / `get_robot_share_link`、
//!   `send_group_msg_raw` / `send_private_msg_sse` / `send_private_msg_wakeup`
//!   等——上游端点大多也未在 `gensokyo-qq` 落地。
//!
//! # 与 Go 的分工
//!
//! Go 的 `handlers/send_*_msg.go` 一个函数干完「解析 → 上传 → 发送 → 回写 echo」；这里把
//! **参数解析与 JSON 形状**（可纯本地单测）和 **上游 I/O** 分开：前者在 [`send`] / [`group`] /
//! [`group_admin`] / [`messages`] / [`users`]，后者只在 [`qqapi`] 一处收口，handler 通过
//! [`qqapi::ActionContext`] 拿依赖。echo 回填由 handler 原样带上（runtime 统一处理的语义不变）。

/// `params` 的公共取值规则（ID 数字/字符串归一），供发送类与查询类共用。
mod params;
/// 最小 RFC3339 工具（Unix 秒 ⇄ UTC 时间串）。
mod rfc3339;
/// 单测公共装置（`FakeQqApi` 等），仅 `cargo test` 下编译。
#[cfg(test)]
mod testing;
/// handler 公共管道：params 取值、虚拟号 ⇄ OpenID 反查、响应封装。
mod util;

/// 本地上报能力 action（`can_send_image` / `can_send_record`）。
pub mod capability;
/// 群聊查询与管理 action（M1k）。
pub mod group;
/// 群管理 action（全员禁言 / 群名片 / 踢人 / 退群 / 群列表）。
pub mod group_admin;
pub mod handler;
/// 媒体信息 action（图片 / 语音的本地回填）。
pub mod media;
/// 消息类 action（撤回 / 消息详情 / 合并转发）。
pub mod messages;
pub mod qqapi;
/// 统一注册入口：41 个 handler + Go 兼容别名。
pub mod register;
pub mod send;
/// 运行控制与凭证 action（重启 / 清缓存 / cookies / csrf）。
pub mod system;
/// 无上游端点的 action（统一 `1500`，`get_online_clients` 回 Go 的本地固定回执）。
pub mod unsupported;
/// 用户类 action（好友列表 / 陌生人资料 / 点赞）。
pub mod users;

pub use capability::{CanSendData, CanSendImageHandler, CanSendRecordHandler};
pub use group::{
    BotStateData, DEFAULT_JOIN_REQUEST_LIMIT, DEFAULT_MEMBER_PAGE, GetGroupBotStateHandler,
    GetGroupInfoHandler, GetGroupJoinRequestListHandler, GetGroupMemberInfoHandler,
    GetGroupMemberListHandler, JoinRequestEntry, JoinRequestListData, MAX_JOIN_REQUEST_LIMIT,
    MAX_MEMBER_PAGE, RETCODE_UPSTREAM, SetGroupBanHandler,
};
pub use group_admin::{
    GROUP_CARD_MESSAGE, GetGroupListHandler, KickData, MAX_KICK_BATCH, RETCODE_ACTION_UNSUPPORTED,
    SetGroupCardHandler, SetGroupKickHandler, SetGroupLeaveHandler, SetGroupWholeBanHandler,
    WHOLE_BAN_MESSAGE,
};
pub use handler::{
    DEFERRED_FIELD, RETCODE_BAD_REQUEST, SendGroupAsyncHandler, SendGroupHandler,
    SendMsgAsyncHandler, SendMsgHandler, SendPrivateAsyncHandler, SendPrivateHandler,
};
pub use media::{GetImageHandler, GetRecordHandler, MediaFileData};
pub use messages::{DeleteMsgHandler, GetForwardMsgHandler, GetMsgHandler};
pub use qqapi::{
    ActionContext, FriendEntry, KickResult, MsgDetail, QqApi, QqFuture, RetractRequest,
    StrangerProfile,
};
pub use register::register_all;
pub use send::{
    MessageOut, SendRequest, SendTarget, plan_send_group_msg, plan_send_msg, plan_send_private_msg,
};
pub use system::{
    CleanCacheHandler, CodeData, CookiesData, CredentialsData, CsrfTokenData, GetCookiesHandler,
    GetCredentialsHandler, GetCsrfTokenHandler, SetRestartHandler,
};
pub use unsupported::{
    GetGroupAtAllRemainHandler, GetGroupSystemMsgHandler, GetOnlineClientsHandler,
    GetQqProfileHandler, OnlineClientsData, SetGroupAdminHandler, SetGroupNameHandler,
    SetGroupRemarkHandler, SetGroupSpecialTitleHandler, SetQqProfileHandler,
};
pub use users::{
    DEFAULT_LIKE_TIMES, FriendInfo, GetFriendListHandler, GetStrangerInfoHandler, MAX_LIKE_TIMES,
    SendLikeHandler, StrangerInfoData,
};

/// 本 crate 提供 handler 的全部 action 注册名（不含别名），按字典序排列。
pub const IMPLEMENTED_ACTIONS: &[&str] = &[
    "can_send_image",
    "can_send_record",
    "clean_cache",
    "delete_msg",
    "get_cookies",
    "get_credentials",
    "get_csrf_token",
    "get_forward_msg",
    "get_friend_list",
    "get_group_at_all_remain",
    "get_group_bot_state",
    "get_group_info",
    "get_group_join_request_list",
    "get_group_list",
    "get_group_member_info",
    "get_group_member_list",
    "get_group_system_msg",
    "get_image",
    "get_msg",
    "get_online_clients",
    "get_qq_profile",
    "get_record",
    "get_stranger_info",
    "send_group_msg",
    "send_group_msg_async",
    "send_like",
    "send_msg",
    "send_msg_async",
    "send_private_msg",
    "send_private_msg_async",
    "set_group_admin",
    "set_group_ban",
    "set_group_card",
    "set_group_kick",
    "set_group_leave",
    "set_group_name",
    "set_group_remark",
    "set_group_special_title",
    "set_group_whole_ban",
    "set_qq_profile",
    "set_restart",
];

/// 已实现但不在 Go 注册表（[`gensokyo_protocol::registry::KNOWN_ACTIONS`]）里的 action。
///
/// 这些是 OneBot v11 标准 action 或 go-cqhttp 扩展名，Go 侧从未注册；实现它们是为了让标准客户端
/// 不必为 Gensokyo 单独适配。其余已实现 action 都在 KNOWN_ACTIONS 内。
pub const EXTENSION_ACTIONS: &[&str] = &[
    "can_send_image",
    "can_send_record",
    "clean_cache",
    "get_cookies",
    "get_credentials",
    "get_csrf_token",
    "get_forward_msg",
    "get_group_at_all_remain",
    "get_group_system_msg",
    "get_image",
    "get_msg",
    "get_qq_profile",
    "get_record",
    "get_stranger_info",
    "send_like",
    "set_group_admin",
    "set_group_leave",
    "set_group_name",
    "set_group_remark",
    "set_group_special_title",
    "set_qq_profile",
    "set_restart",
];

#[cfg(test)]
mod tests {
    use super::{EXTENSION_ACTIONS, IMPLEMENTED_ACTIONS};
    use gensokyo_protocol::registry::{ACTION_ALIASES, KNOWN_ACTIONS};

    #[test]
    fn implemented_actions_are_known_or_documented_extensions() {
        for action in IMPLEMENTED_ACTIONS {
            assert!(
                KNOWN_ACTIONS.contains(action) || EXTENSION_ACTIONS.contains(action),
                "{action} 既不在 KNOWN_ACTIONS 里，也没登记为扩展 action"
            );
        }
        for action in EXTENSION_ACTIONS {
            assert!(
                IMPLEMENTED_ACTIONS.contains(action),
                "扩展 action {action} 没有对应的 handler"
            );
            assert!(
                !KNOWN_ACTIONS.contains(action),
                "扩展 action {action} 已经出现在 KNOWN_ACTIONS 里，应移出扩展清单"
            );
        }
    }

    #[test]
    fn implemented_actions_are_sorted_and_unique() {
        let mut sorted = IMPLEMENTED_ACTIONS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            IMPLEMENTED_ACTIONS.len(),
            "IMPLEMENTED_ACTIONS 有重复项"
        );
        assert_eq!(
            sorted,
            IMPLEMENTED_ACTIONS.to_vec(),
            "IMPLEMENTED_ACTIONS 必须按字典序"
        );
    }

    #[test]
    fn aliases_point_at_implemented_handlers() {
        for (alias, target) in ACTION_ALIASES {
            assert!(
                IMPLEMENTED_ACTIONS.contains(target),
                "别名 {alias} 指向未实现的 {target}"
            );
        }
    }

    /// 每个登记名都必须真有一个 handler，且 handler 的 `name()` 与登记名逐字一致。
    #[test]
    fn every_implemented_action_has_a_handler_with_that_name() {
        use std::sync::Arc;

        use gensokyo_protocol::Handler;

        use crate::capability::{CanSendImageHandler, CanSendRecordHandler};
        use crate::group::{
            GetGroupBotStateHandler, GetGroupInfoHandler, GetGroupJoinRequestListHandler,
            GetGroupMemberInfoHandler, GetGroupMemberListHandler, SetGroupBanHandler,
        };
        use crate::group_admin::{
            GetGroupListHandler, SetGroupCardHandler, SetGroupKickHandler, SetGroupLeaveHandler,
            SetGroupWholeBanHandler,
        };
        use crate::handler::{
            SendGroupAsyncHandler, SendGroupHandler, SendMsgAsyncHandler, SendMsgHandler,
            SendPrivateAsyncHandler, SendPrivateHandler,
        };
        use crate::media::{GetImageHandler, GetRecordHandler};
        use crate::messages::{DeleteMsgHandler, GetForwardMsgHandler, GetMsgHandler};
        use crate::system::{
            CleanCacheHandler, GetCookiesHandler, GetCredentialsHandler, GetCsrfTokenHandler,
            SetRestartHandler,
        };
        use crate::unsupported::{
            GetGroupAtAllRemainHandler, GetGroupSystemMsgHandler, GetOnlineClientsHandler,
            GetQqProfileHandler, SetGroupAdminHandler, SetGroupNameHandler, SetGroupRemarkHandler,
            SetGroupSpecialTitleHandler, SetQqProfileHandler,
        };
        use crate::users::{GetFriendListHandler, GetStrangerInfoHandler, SendLikeHandler};

        let (context, _api) = crate::testing::context();
        let handlers: Vec<Arc<dyn Handler>> = vec![
            Arc::new(CanSendImageHandler::new()),
            Arc::new(CanSendRecordHandler::new()),
            Arc::new(CleanCacheHandler::new()),
            Arc::new(DeleteMsgHandler::new(context.clone())),
            Arc::new(GetCookiesHandler::new()),
            Arc::new(GetCredentialsHandler::new()),
            Arc::new(GetCsrfTokenHandler::new()),
            Arc::new(GetForwardMsgHandler::new(context.clone())),
            Arc::new(GetFriendListHandler::new(context.clone())),
            Arc::new(GetGroupAtAllRemainHandler::new()),
            Arc::new(GetGroupBotStateHandler::new(context.clone())),
            Arc::new(GetGroupInfoHandler::new(context.clone())),
            Arc::new(GetGroupJoinRequestListHandler::new(context.clone())),
            Arc::new(GetGroupListHandler::new(context.clone())),
            Arc::new(GetGroupMemberInfoHandler::new(context.clone())),
            Arc::new(GetGroupMemberListHandler::new(context.clone())),
            Arc::new(GetGroupSystemMsgHandler::new()),
            Arc::new(GetImageHandler::new()),
            Arc::new(GetMsgHandler::new(context.clone())),
            Arc::new(GetOnlineClientsHandler::new()),
            Arc::new(GetQqProfileHandler::new()),
            Arc::new(GetRecordHandler::new()),
            Arc::new(GetStrangerInfoHandler::new(context.clone())),
            Arc::new(SendGroupAsyncHandler::new()),
            Arc::new(SendGroupHandler::new()),
            Arc::new(SendLikeHandler::new(context.clone())),
            Arc::new(SendMsgAsyncHandler::new()),
            Arc::new(SendMsgHandler::new()),
            Arc::new(SendPrivateAsyncHandler::new()),
            Arc::new(SendPrivateHandler::new()),
            Arc::new(SetGroupAdminHandler::new()),
            Arc::new(SetGroupBanHandler::new(context.clone())),
            Arc::new(SetGroupCardHandler::new()),
            Arc::new(SetGroupKickHandler::new(context.clone())),
            Arc::new(SetGroupLeaveHandler::new(context.clone())),
            Arc::new(SetGroupNameHandler::new()),
            Arc::new(SetGroupRemarkHandler::new()),
            Arc::new(SetGroupSpecialTitleHandler::new()),
            Arc::new(SetGroupWholeBanHandler::new()),
            Arc::new(SetQqProfileHandler::new()),
            Arc::new(SetRestartHandler::new()),
        ];

        let mut names: Vec<&str> = handlers.iter().map(|handler| handler.name()).collect();
        names.sort_unstable();
        assert_eq!(names, IMPLEMENTED_ACTIONS.to_vec());
    }
}
