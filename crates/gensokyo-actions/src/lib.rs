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
//! | [`panels`] | `create_panel`、`set_panel`、`delete_panel`、`get_panel`、`get_panel_list`、`set_panel_target`、`set_custom_menu`、`get_custom_menu`、`join_approval_strategy_*`（6 个） | 面板 / 自定义菜单 / 自动审批策略：`gensokyo-qq` 未移植对应 botgo 端点，参数按 Go 校验后回 `1500`。 |
//! | [`interaction`] | `get_avatar`、`mark_msg_as_read`、`.handle_quick_operation`（+ `_async`）、`get_robot_share_link`、`put_interaction`、`delete_group_msg`、`set_group_add_request`、`get_group_member_blacklist`、`set_group_member_blacklist`、`send_group_forward_msg`、`send_private_forward_msg`、`send_group_msg_raw`、`send_private_msg_sse`、`send_private_msg_wakeup`、`get_group_honor_info`、`set_group_anonymous_ban` | 三项本地实现（头像 URL / 已读空回执 / 快速操作转 deferred 计划），其余 `1500`。 |
//! | [`capability`] | `can_send_image`、`can_send_record` | 纯本地判断，固定 `{"yes": true}`。 |
//!
//! # KNOWN_ACTIONS 覆盖率
//!
//! [`gensokyo_protocol::registry::KNOWN_ACTIONS`] 是 Go `callapi.RegisterHandler` 的全集（56 项）。
//! 本 crate 覆盖其中 **53 项**：50 个注册名（[`IMPLEMENTED_ACTIONS`] 与 KNOWN_ACTIONS 的交集）
//! 加 3 个 Go 别名（`send_to_group` / `get_group_ban` / `get_group_whole_ban`，由
//! [`gensokyo_protocol::registry::ACTION_ALIASES`] 指向对应 handler）。
//!
//! 剩下的 3 项 `get_login_info` / `get_status` / `get_version_info` 由 runtime 内置注册（字段对齐
//! Go 的同名 handler，见 `gensokyo-runtime::builtin`），登记在 [`BUILTIN_ACTIONS`]。本 crate 刻意
//! 不重复注册它们：注册表是「后注册者胜出」，重复注册会让两边的实现互相覆盖。
//!
//! 另有 25 个 action 在 Go 里从未注册，登记在 [`EXTENSION_ACTIONS`]（OneBot v11 标准 action 与
//! go-cqhttp 扩展名，如 `get_image`、`get_msg`、`set_restart`、`get_group_honor_info`）。
//!
//! 这三张清单由单测机器核对：每个已实现 action 必须落在 KNOWN_ACTIONS 或 EXTENSION_ACTIONS 里；
//! 别名必须指向已实现的 handler；`KNOWN_ACTIONS` 去掉 [`BUILTIN_ACTIONS`] 后必须全部注册
//! （`register::known_actions_are_registered_here_or_by_the_runtime`）。
//!
//! # deferred 执行链路
//!
//! 1. [`register::register_all`] 把 75 个 handler 装进 `gensokyo_protocol::Registry`（runtime 装配，
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
//! - **`KNOWN_ACTIONS` 里端点未落地的 action**：面板 / 自定义菜单 / 自动审批策略
//!   （[`panels`]）、互动 / 群消息撤回 / 入群申请审批 / 成员黑名单 / 合并转发 / 原始直发 /
//!   SSE / `IsWakeup`（[`interaction`]）都只做参数校验后回 `1500`——`gensokyo-qq` 没有对应的
//!   上游端点，运行时发送链也还没有这几条分支。`get_login_info` / `get_status` /
//!   `get_version_info` 由 runtime 内置（[`BUILTIN_ACTIONS`]），不在本 crate。
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
/// 头像 / 已读 / 分享链接 / 互动 / 撤回 / 审批 / 黑名单 / 转发 / 快速操作 action。
pub mod interaction;
/// 媒体信息 action（图片 / 语音的本地回填）。
pub mod media;
/// 素材管理 action（上传 / 列表 / 删除素材图片）。
pub mod media_image;
/// 消息类 action（撤回 / 消息详情 / 合并转发）。
pub mod messages;
/// 面板 / 自定义菜单 / 自动审批策略 action（Go 的官方 v2 管理扩展）。
pub mod panels;
pub mod qqapi;
/// 统一注册入口：75 个 handler + Go 兼容别名。
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
pub use interaction::{
    AvatarData, DeleteGroupMsgHandler, GetAvatarHandler, GetGroupHonorInfoHandler,
    GetGroupMemberBlacklistHandler, GetRobotShareLinkHandler, MarkMsgAsReadHandler,
    PutInteractionHandler, QuickOperationAsyncHandler, QuickOperationHandler,
    SendGroupForwardMsgHandler, SendGroupMsgRawHandler, SendPrivateForwardMsgHandler,
    SendPrivateMsgSseHandler, SendPrivateMsgWakeupHandler, SetGroupAddRequestHandler,
    SetGroupAnonymousBanHandler, SetGroupMemberBlacklistHandler,
};
pub use media::{GetImageHandler, GetRecordHandler, MediaFileData};
pub use media_image::{
    DeleteMediaImageData, DeleteMediaImageHandler, GetMediaImageListHandler, MediaImageListData,
    RETCODE_MEDIA_UPSTREAM, UploadMediaImageData, UploadMediaImageHandler,
};
pub use messages::{DeleteMsgHandler, GetForwardMsgHandler, GetMsgHandler};
pub use panels::{
    CreatePanelHandler, DeletePanelHandler, GetCustomMenuHandler, GetPanelHandler,
    GetPanelListHandler, JoinApprovalStrategyCreateHandler, JoinApprovalStrategyDeleteHandler,
    JoinApprovalStrategyExecuteHandler, JoinApprovalStrategyListHandler,
    JoinApprovalStrategyUpdateHandler, JoinApprovalStrategyWhitelistHandler, SetCustomMenuHandler,
    SetPanelHandler, SetPanelTargetHandler,
};
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
    ".handle_quick_operation",
    ".handle_quick_operation_async",
    "can_send_image",
    "can_send_record",
    "clean_cache",
    "create_panel",
    "delete_group_msg",
    "delete_media_image",
    "delete_msg",
    "delete_panel",
    "get_avatar",
    "get_cookies",
    "get_credentials",
    "get_csrf_token",
    "get_custom_menu",
    "get_forward_msg",
    "get_friend_list",
    "get_group_at_all_remain",
    "get_group_bot_state",
    "get_group_honor_info",
    "get_group_info",
    "get_group_join_request_list",
    "get_group_list",
    "get_group_member_blacklist",
    "get_group_member_info",
    "get_group_member_list",
    "get_group_system_msg",
    "get_image",
    "get_media_image_list",
    "get_msg",
    "get_online_clients",
    "get_panel",
    "get_panel_list",
    "get_qq_profile",
    "get_record",
    "get_robot_share_link",
    "get_stranger_info",
    "join_approval_strategy_create",
    "join_approval_strategy_delete",
    "join_approval_strategy_execute",
    "join_approval_strategy_list",
    "join_approval_strategy_update",
    "join_approval_strategy_whitelist",
    "mark_msg_as_read",
    "put_interaction",
    "send_group_forward_msg",
    "send_group_msg",
    "send_group_msg_async",
    "send_group_msg_raw",
    "send_like",
    "send_msg",
    "send_msg_async",
    "send_private_forward_msg",
    "send_private_msg",
    "send_private_msg_async",
    "send_private_msg_sse",
    "send_private_msg_wakeup",
    "set_custom_menu",
    "set_group_add_request",
    "set_group_admin",
    "set_group_anonymous_ban",
    "set_group_ban",
    "set_group_card",
    "set_group_kick",
    "set_group_leave",
    "set_group_member_blacklist",
    "set_group_name",
    "set_group_remark",
    "set_group_special_title",
    "set_group_whole_ban",
    "set_panel",
    "set_panel_target",
    "set_qq_profile",
    "set_restart",
    "upload_media_image",
];

/// [`gensokyo_protocol::registry::KNOWN_ACTIONS`] 里由 runtime 内置注册、本 crate 不重复注册的
/// action（`gensokyo-runtime::builtin`，字段对齐 Go 同名 handler）。
///
/// 注册表是「后注册者胜出」，两边都登记会让实现互相覆盖：内置项留在 runtime，本 crate 只声明
/// 这份名单供覆盖率单测核对。
pub const BUILTIN_ACTIONS: &[&str] = &["get_login_info", "get_status", "get_version_info"];

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
    "get_group_honor_info",
    "get_group_system_msg",
    "get_image",
    "get_msg",
    "get_qq_profile",
    "get_record",
    "get_stranger_info",
    "send_like",
    "send_private_forward_msg",
    "set_group_admin",
    "set_group_anonymous_ban",
    "set_group_leave",
    "set_group_name",
    "set_group_remark",
    "set_group_special_title",
    "set_qq_profile",
    "set_restart",
];

#[cfg(test)]
mod tests {
    use super::{BUILTIN_ACTIONS, EXTENSION_ACTIONS, IMPLEMENTED_ACTIONS};
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

    /// runtime 内置的 3 项不能出现在本 crate 的注册清单里：注册表「后注册者胜出」，两边都登记
    /// 会让实现互相覆盖。
    #[test]
    fn builtin_actions_are_not_registered_here() {
        let implemented_known = IMPLEMENTED_ACTIONS
            .iter()
            .filter(|action| KNOWN_ACTIONS.contains(action))
            .count();
        assert_eq!(
            BUILTIN_ACTIONS.len(),
            KNOWN_ACTIONS.len() - implemented_known - ACTION_ALIASES.len(),
            "KNOWN_ACTIONS 只允许「本 crate 注册名 + Go 别名 + runtime 内置」三种来源"
        );
        for action in BUILTIN_ACTIONS {
            assert!(
                !IMPLEMENTED_ACTIONS.contains(action),
                "内置 action {action} 不应由本 crate 注册"
            );
            assert!(
                !EXTENSION_ACTIONS.contains(action),
                "内置 action {action} 属于 KNOWN_ACTIONS，不应登记为扩展 action"
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
        use crate::interaction::{
            DeleteGroupMsgHandler, GetAvatarHandler, GetGroupHonorInfoHandler,
            GetGroupMemberBlacklistHandler, GetRobotShareLinkHandler, MarkMsgAsReadHandler,
            PutInteractionHandler, QuickOperationAsyncHandler, QuickOperationHandler,
            SendGroupForwardMsgHandler, SendGroupMsgRawHandler, SendPrivateForwardMsgHandler,
            SendPrivateMsgSseHandler, SendPrivateMsgWakeupHandler, SetGroupAddRequestHandler,
            SetGroupAnonymousBanHandler, SetGroupMemberBlacklistHandler,
        };
        use crate::media::{GetImageHandler, GetRecordHandler};
        use crate::media_image::{
            DeleteMediaImageHandler, GetMediaImageListHandler, UploadMediaImageHandler,
        };
        use crate::messages::{DeleteMsgHandler, GetForwardMsgHandler, GetMsgHandler};
        use crate::panels::{
            CreatePanelHandler, DeletePanelHandler, GetCustomMenuHandler, GetPanelHandler,
            GetPanelListHandler, JoinApprovalStrategyCreateHandler,
            JoinApprovalStrategyDeleteHandler, JoinApprovalStrategyExecuteHandler,
            JoinApprovalStrategyListHandler, JoinApprovalStrategyUpdateHandler,
            JoinApprovalStrategyWhitelistHandler, SetCustomMenuHandler, SetPanelHandler,
            SetPanelTargetHandler,
        };
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
            Arc::new(QuickOperationHandler::new()),
            Arc::new(QuickOperationAsyncHandler::new()),
            Arc::new(CanSendImageHandler::new()),
            Arc::new(CanSendRecordHandler::new()),
            Arc::new(CleanCacheHandler::new()),
            Arc::new(CreatePanelHandler::new()),
            Arc::new(DeleteGroupMsgHandler::new()),
            Arc::new(DeleteMediaImageHandler::new(context.clone())),
            Arc::new(DeleteMsgHandler::new(context.clone())),
            Arc::new(DeletePanelHandler::new()),
            Arc::new(GetAvatarHandler::new(context.clone())),
            Arc::new(GetCookiesHandler::new()),
            Arc::new(GetCredentialsHandler::new()),
            Arc::new(GetCsrfTokenHandler::new()),
            Arc::new(GetCustomMenuHandler::new()),
            Arc::new(GetForwardMsgHandler::new(context.clone())),
            Arc::new(GetFriendListHandler::new(context.clone())),
            Arc::new(GetGroupAtAllRemainHandler::new()),
            Arc::new(GetGroupBotStateHandler::new(context.clone())),
            Arc::new(GetGroupHonorInfoHandler::new()),
            Arc::new(GetGroupInfoHandler::new(context.clone())),
            Arc::new(GetGroupJoinRequestListHandler::new(context.clone())),
            Arc::new(GetGroupListHandler::new(context.clone())),
            Arc::new(GetGroupMemberBlacklistHandler::new()),
            Arc::new(GetGroupMemberInfoHandler::new(context.clone())),
            Arc::new(GetGroupMemberListHandler::new(context.clone())),
            Arc::new(GetGroupSystemMsgHandler::new()),
            Arc::new(GetImageHandler::new()),
            Arc::new(GetMediaImageListHandler::new(context.clone())),
            Arc::new(GetMsgHandler::new(context.clone())),
            Arc::new(GetOnlineClientsHandler::new()),
            Arc::new(GetPanelHandler::new()),
            Arc::new(GetPanelListHandler::new()),
            Arc::new(GetQqProfileHandler::new()),
            Arc::new(GetRecordHandler::new()),
            Arc::new(GetRobotShareLinkHandler::new()),
            Arc::new(GetStrangerInfoHandler::new(context.clone())),
            Arc::new(JoinApprovalStrategyCreateHandler::new()),
            Arc::new(JoinApprovalStrategyDeleteHandler::new()),
            Arc::new(JoinApprovalStrategyExecuteHandler::new()),
            Arc::new(JoinApprovalStrategyListHandler::new()),
            Arc::new(JoinApprovalStrategyUpdateHandler::new()),
            Arc::new(JoinApprovalStrategyWhitelistHandler::new()),
            Arc::new(MarkMsgAsReadHandler::new()),
            Arc::new(PutInteractionHandler::new()),
            Arc::new(SendGroupForwardMsgHandler::new()),
            Arc::new(SendGroupHandler::new()),
            Arc::new(SendGroupAsyncHandler::new()),
            Arc::new(SendGroupMsgRawHandler::new()),
            Arc::new(SendLikeHandler::new(context.clone())),
            Arc::new(SendMsgHandler::new()),
            Arc::new(SendMsgAsyncHandler::new()),
            Arc::new(SendPrivateForwardMsgHandler::new()),
            Arc::new(SendPrivateHandler::new()),
            Arc::new(SendPrivateAsyncHandler::new()),
            Arc::new(SendPrivateMsgSseHandler::new()),
            Arc::new(SendPrivateMsgWakeupHandler::new()),
            Arc::new(SetCustomMenuHandler::new()),
            Arc::new(SetGroupAddRequestHandler::new()),
            Arc::new(SetGroupAdminHandler::new()),
            Arc::new(SetGroupAnonymousBanHandler::new()),
            Arc::new(SetGroupBanHandler::new(context.clone())),
            Arc::new(SetGroupCardHandler::new()),
            Arc::new(SetGroupKickHandler::new(context.clone())),
            Arc::new(SetGroupLeaveHandler::new(context.clone())),
            Arc::new(SetGroupMemberBlacklistHandler::new()),
            Arc::new(SetGroupNameHandler::new()),
            Arc::new(SetGroupRemarkHandler::new()),
            Arc::new(SetGroupSpecialTitleHandler::new()),
            Arc::new(SetGroupWholeBanHandler::new()),
            Arc::new(SetPanelHandler::new()),
            Arc::new(SetPanelTargetHandler::new()),
            Arc::new(SetQqProfileHandler::new()),
            Arc::new(SetRestartHandler::new()),
            Arc::new(UploadMediaImageHandler::new(context.clone())),
        ];

        let mut names: Vec<&str> = handlers.iter().map(|handler| handler.name()).collect();
        names.sort_unstable();
        assert_eq!(names, IMPLEMENTED_ACTIONS.to_vec());
    }
}
