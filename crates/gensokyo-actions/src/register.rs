//! 统一注册入口：把本 crate 的全部 handler 一次性装进 [`Registry`]。
//!
//! 对应 Go 的 `handlers/*.go` 在 `init()` 里逐个 `callapi.RegisterHandler`：Rust 侧没有文件
//! 自注册，改由 runtime 在装配 [`crate::qqapi::ActionContext`] 后调用 [`register_all`]。
//!
//! 覆盖范围 = [`crate::IMPLEMENTED_ACTIONS`]（72 个注册名）+ [`ACTION_ALIASES`] 里指向本 crate
//! 的 3 个 Go 兼容别名（`send_to_group` / `get_group_ban` / `get_group_whole_ban`）。
//! [`gensokyo_protocol::registry::KNOWN_ACTIONS`] 的剩余 3 项（`get_login_info` / `get_status` /
//! `get_version_info`）由 runtime 内置注册，见 [`crate::BUILTIN_ACTIONS`]——重复注册会让后装配方
//! 覆盖前者，因此这里刻意不注册它们。
//!
//! 绑定 [`crate::qqapi::ActionContext`] 的 handler 共享同一个 `Arc`，其余无状态 handler
//! 各建一份（零成本，都是 ZST）。

use std::sync::Arc;

use gensokyo_protocol::Handler;
use gensokyo_protocol::Registry;
use gensokyo_protocol::registry::ACTION_ALIASES;

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
use crate::messages::{DeleteMsgHandler, GetForwardMsgHandler, GetMsgHandler};
use crate::panels::{
    CreatePanelHandler, DeletePanelHandler, GetCustomMenuHandler, GetPanelHandler,
    GetPanelListHandler, JoinApprovalStrategyCreateHandler, JoinApprovalStrategyDeleteHandler,
    JoinApprovalStrategyExecuteHandler, JoinApprovalStrategyListHandler,
    JoinApprovalStrategyUpdateHandler, JoinApprovalStrategyWhitelistHandler, SetCustomMenuHandler,
    SetPanelHandler, SetPanelTargetHandler,
};
use crate::qqapi::ActionContext;
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

/// 注册本 crate 的全部 action（72 个注册名 + Go 兼容别名）。
///
/// 重复注册时后注册者胜出（[`Registry::register`] 的既有语义）；别名目标缺失会 panic——
/// 别名表是编译期常量，装配期炸出来比线上 `1404` 更容易定位。
pub fn register_all(registry: &mut Registry, context: Arc<ActionContext>) {
    let entries: [(&'static str, Arc<dyn Handler>); 72] = [
        (
            ".handle_quick_operation",
            Arc::new(QuickOperationHandler::new()),
        ),
        (
            ".handle_quick_operation_async",
            Arc::new(QuickOperationAsyncHandler::new()),
        ),
        ("can_send_image", Arc::new(CanSendImageHandler::new())),
        ("can_send_record", Arc::new(CanSendRecordHandler::new())),
        ("clean_cache", Arc::new(CleanCacheHandler::new())),
        ("create_panel", Arc::new(CreatePanelHandler::new())),
        ("delete_group_msg", Arc::new(DeleteGroupMsgHandler::new())),
        (
            "delete_msg",
            Arc::new(DeleteMsgHandler::new(Arc::clone(&context))),
        ),
        ("delete_panel", Arc::new(DeletePanelHandler::new())),
        (
            "get_avatar",
            Arc::new(GetAvatarHandler::new(Arc::clone(&context))),
        ),
        ("get_cookies", Arc::new(GetCookiesHandler::new())),
        ("get_credentials", Arc::new(GetCredentialsHandler::new())),
        ("get_csrf_token", Arc::new(GetCsrfTokenHandler::new())),
        ("get_custom_menu", Arc::new(GetCustomMenuHandler::new())),
        (
            "get_forward_msg",
            Arc::new(GetForwardMsgHandler::new(Arc::clone(&context))),
        ),
        (
            "get_friend_list",
            Arc::new(GetFriendListHandler::new(Arc::clone(&context))),
        ),
        (
            "get_group_at_all_remain",
            Arc::new(GetGroupAtAllRemainHandler::new()),
        ),
        (
            "get_group_bot_state",
            Arc::new(GetGroupBotStateHandler::new(Arc::clone(&context))),
        ),
        (
            "get_group_honor_info",
            Arc::new(GetGroupHonorInfoHandler::new()),
        ),
        (
            "get_group_info",
            Arc::new(GetGroupInfoHandler::new(Arc::clone(&context))),
        ),
        (
            "get_group_join_request_list",
            Arc::new(GetGroupJoinRequestListHandler::new(Arc::clone(&context))),
        ),
        (
            "get_group_list",
            Arc::new(GetGroupListHandler::new(Arc::clone(&context))),
        ),
        (
            "get_group_member_blacklist",
            Arc::new(GetGroupMemberBlacklistHandler::new()),
        ),
        (
            "get_group_member_info",
            Arc::new(GetGroupMemberInfoHandler::new(Arc::clone(&context))),
        ),
        (
            "get_group_member_list",
            Arc::new(GetGroupMemberListHandler::new(Arc::clone(&context))),
        ),
        (
            "get_group_system_msg",
            Arc::new(GetGroupSystemMsgHandler::new()),
        ),
        ("get_image", Arc::new(GetImageHandler::new())),
        (
            "get_msg",
            Arc::new(GetMsgHandler::new(Arc::clone(&context))),
        ),
        (
            "get_online_clients",
            Arc::new(GetOnlineClientsHandler::new()),
        ),
        ("get_panel", Arc::new(GetPanelHandler::new())),
        ("get_panel_list", Arc::new(GetPanelListHandler::new())),
        ("get_qq_profile", Arc::new(GetQqProfileHandler::new())),
        ("get_record", Arc::new(GetRecordHandler::new())),
        (
            "get_robot_share_link",
            Arc::new(GetRobotShareLinkHandler::new()),
        ),
        (
            "get_stranger_info",
            Arc::new(GetStrangerInfoHandler::new(Arc::clone(&context))),
        ),
        (
            "join_approval_strategy_create",
            Arc::new(JoinApprovalStrategyCreateHandler::new()),
        ),
        (
            "join_approval_strategy_delete",
            Arc::new(JoinApprovalStrategyDeleteHandler::new()),
        ),
        (
            "join_approval_strategy_execute",
            Arc::new(JoinApprovalStrategyExecuteHandler::new()),
        ),
        (
            "join_approval_strategy_list",
            Arc::new(JoinApprovalStrategyListHandler::new()),
        ),
        (
            "join_approval_strategy_update",
            Arc::new(JoinApprovalStrategyUpdateHandler::new()),
        ),
        (
            "join_approval_strategy_whitelist",
            Arc::new(JoinApprovalStrategyWhitelistHandler::new()),
        ),
        ("mark_msg_as_read", Arc::new(MarkMsgAsReadHandler::new())),
        ("put_interaction", Arc::new(PutInteractionHandler::new())),
        (
            "send_group_forward_msg",
            Arc::new(SendGroupForwardMsgHandler::new()),
        ),
        ("send_group_msg", Arc::new(SendGroupHandler::new())),
        (
            "send_group_msg_async",
            Arc::new(SendGroupAsyncHandler::new()),
        ),
        (
            "send_group_msg_raw",
            Arc::new(SendGroupMsgRawHandler::new()),
        ),
        (
            "send_like",
            Arc::new(SendLikeHandler::new(Arc::clone(&context))),
        ),
        ("send_msg", Arc::new(SendMsgHandler::new())),
        ("send_msg_async", Arc::new(SendMsgAsyncHandler::new())),
        (
            "send_private_forward_msg",
            Arc::new(SendPrivateForwardMsgHandler::new()),
        ),
        ("send_private_msg", Arc::new(SendPrivateHandler::new())),
        (
            "send_private_msg_async",
            Arc::new(SendPrivateAsyncHandler::new()),
        ),
        (
            "send_private_msg_sse",
            Arc::new(SendPrivateMsgSseHandler::new()),
        ),
        (
            "send_private_msg_wakeup",
            Arc::new(SendPrivateMsgWakeupHandler::new()),
        ),
        ("set_custom_menu", Arc::new(SetCustomMenuHandler::new())),
        (
            "set_group_add_request",
            Arc::new(SetGroupAddRequestHandler::new()),
        ),
        ("set_group_admin", Arc::new(SetGroupAdminHandler::new())),
        (
            "set_group_anonymous_ban",
            Arc::new(SetGroupAnonymousBanHandler::new()),
        ),
        (
            "set_group_ban",
            Arc::new(SetGroupBanHandler::new(Arc::clone(&context))),
        ),
        ("set_group_card", Arc::new(SetGroupCardHandler::new())),
        (
            "set_group_kick",
            Arc::new(SetGroupKickHandler::new(Arc::clone(&context))),
        ),
        (
            "set_group_leave",
            Arc::new(SetGroupLeaveHandler::new(Arc::clone(&context))),
        ),
        (
            "set_group_member_blacklist",
            Arc::new(SetGroupMemberBlacklistHandler::new()),
        ),
        ("set_group_name", Arc::new(SetGroupNameHandler::new())),
        ("set_group_remark", Arc::new(SetGroupRemarkHandler::new())),
        (
            "set_group_special_title",
            Arc::new(SetGroupSpecialTitleHandler::new()),
        ),
        (
            "set_group_whole_ban",
            Arc::new(SetGroupWholeBanHandler::new()),
        ),
        ("set_panel", Arc::new(SetPanelHandler::new())),
        ("set_panel_target", Arc::new(SetPanelTargetHandler::new())),
        ("set_qq_profile", Arc::new(SetQqProfileHandler::new())),
        ("set_restart", Arc::new(SetRestartHandler::new())),
    ];

    for (action, handler) in entries {
        let _ = registry.register(action, handler);
    }

    for (alias, target) in ACTION_ALIASES {
        registry.alias(alias, target).expect("别名目标必须已注册");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gensokyo_protocol::Registry;
    use gensokyo_protocol::registry::{ACTION_ALIASES, KNOWN_ACTIONS};

    use super::register_all;
    use crate::testing::context;
    use crate::{BUILTIN_ACTIONS, IMPLEMENTED_ACTIONS};

    fn registry() -> Registry {
        let mut registry = Registry::new();
        let (context, _api) = context();
        register_all(&mut registry, context);
        registry
    }

    #[test]
    fn every_implemented_action_is_registered() {
        let registry = registry();
        assert_eq!(
            registry.missing(IMPLEMENTED_ACTIONS),
            Vec::<&'static str>::new()
        );
    }

    #[test]
    fn aliases_point_at_the_same_handler() {
        let registry = registry();
        for (alias, target) in ACTION_ALIASES {
            let alias_handler = registry.get(alias).expect("别名应已注册");
            let target_handler = registry.get(target).expect("别名目标应已注册");
            assert!(
                Arc::ptr_eq(alias_handler, target_handler),
                "{alias} 应指向 {target} 的同一份实现"
            );
        }
    }

    #[test]
    fn registered_names_are_implemented_actions_plus_aliases() {
        let mut expected: Vec<&'static str> = IMPLEMENTED_ACTIONS.to_vec();
        expected.extend(ACTION_ALIASES.iter().map(|(alias, _)| *alias));
        expected.sort_unstable();

        let registry = registry();
        assert_eq!(registry.actions(), expected);
        assert_eq!(
            registry.len(),
            IMPLEMENTED_ACTIONS.len() + ACTION_ALIASES.len()
        );
    }

    /// `KNOWN_ACTIONS` 是 Go `RegisterHandler` 的全集：除 runtime 内置的 3 项外必须全部注册。
    #[test]
    fn known_actions_are_registered_here_or_by_the_runtime() {
        assert_eq!(KNOWN_ACTIONS.len(), 53, "KNOWN_ACTIONS 是 Go 注册全集");
        let registry = registry();
        assert_eq!(
            registry.missing(KNOWN_ACTIONS),
            BUILTIN_ACTIONS.to_vec(),
            "除 runtime 内置的 3 项外，KNOWN_ACTIONS 必须全部在本 crate 注册"
        );
    }

    #[test]
    fn handler_names_match_their_registration_names() {
        let registry = registry();
        let aliases: Vec<&str> = ACTION_ALIASES.iter().map(|(alias, _)| *alias).collect();
        for action in registry.actions() {
            if aliases.contains(&action) {
                continue;
            }
            let handler = registry.get(action).expect("应已注册");
            assert_eq!(handler.name(), action);
        }
    }
}
