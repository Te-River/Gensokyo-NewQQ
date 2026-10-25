//! QQ API 能力抽象：handler 只认 [`QqApi`]，不直接依赖 `gensokyo-qq` 的传输层。
//!
//! # 为什么是装箱 future 而不是 `async fn`
//!
//! [`ActionContext`] 里存的是 `Arc<dyn QqApi>`，trait 必须 dyn-compatible；trait 里的
//! `async fn`（AFIT）不满足这一点。与 [`gensokyo_protocol::Handler`] 同一取舍：一次装箱分配，
//! 换来 runtime 可以把分发放进任意 tokio 任务。
//!
//! # 生产实现与测试缝隙
//!
//! 生产侧只有 [`gensokyo_qq::OpenApiClient`] 一个实现（下面对 `T: HttpTransport` 全量转发）；
//! 单测用 `FakeQqApi` 记录入参并注入上游错误，因此 handler 的「参数翻译 + 错误映射」可以纯本地
//! 验证，不碰网络。
//!
//! # 未覆盖的端点
//!
//! 本 trait 只声明 M1k 需要的 6 个方法。入群审批、黑名单、菜单、指令面板等仍直接在
//! `gensokyo-qq` 上调用，等对应 action 落地时再补。

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use gensokyo_config::ConfigManager;
use gensokyo_qq::dto::{MemberMuteState, RestrictChatSetting};
use gensokyo_qq::{
    BotInGroupState, GroupInfo, GroupMember, GroupMemberList, HttpTransport, JoinRequestList,
    OpenApiClient,
};
use gensokyo_storage::IdMapStore;

use crate::rfc3339;

/// [`QqApi`] 的异步返回，形态与 [`gensokyo_protocol::HandlerFuture`] 一致。
pub type QqFuture<'a, T> = Pin<Box<dyn Future<Output = gensokyo_qq::Result<T>> + Send + 'a>>;

/// handler 可见的 QQ 上游能力。
pub trait QqApi: Send + Sync + fmt::Debug {
    /// 群基本信息。
    fn get_group_info<'a>(&'a self, group_openid: &'a str) -> QqFuture<'a, GroupInfo>;

    /// 单个群成员信息。
    fn get_group_member_info<'a>(
        &'a self,
        group_openid: &'a str,
        member_openid: &'a str,
    ) -> QqFuture<'a, GroupMember>;

    /// 群成员列表单页（`limit` 为单页条数）。
    fn get_group_member_list<'a>(
        &'a self,
        group_openid: &'a str,
        limit: u32,
    ) -> QqFuture<'a, GroupMemberList>;

    /// 群成员禁言；`duration_secs <= 0` 表示立即解除。
    fn set_group_ban<'a>(
        &'a self,
        group_openid: &'a str,
        member_openid: &'a str,
        duration_secs: i64,
    ) -> QqFuture<'a, ()>;

    /// 机器人在群内的状态。
    fn get_group_bot_state<'a>(&'a self, group_openid: &'a str) -> QqFuture<'a, BotInGroupState>;

    /// 入群申请列表；`cursor` 为空表示首页，`limit` 为单页条数。
    fn get_group_join_request_list<'a>(
        &'a self,
        group_openid: &'a str,
        cursor: Option<&'a str>,
        limit: Option<u32>,
    ) -> QqFuture<'a, JoinRequestList>;
}

impl<T: HttpTransport> QqApi for OpenApiClient<T> {
    fn get_group_info<'a>(&'a self, group_openid: &'a str) -> QqFuture<'a, GroupInfo> {
        // 显式 `OpenApiClient::` 前缀：同名 trait 方法存在时，方法调用语法容易递归到自己
        Box::pin(OpenApiClient::get_group_info(self, group_openid))
    }

    fn get_group_member_info<'a>(
        &'a self,
        group_openid: &'a str,
        member_openid: &'a str,
    ) -> QqFuture<'a, GroupMember> {
        Box::pin(OpenApiClient::get_member_info(
            self,
            group_openid,
            member_openid,
        ))
    }

    fn get_group_member_list<'a>(
        &'a self,
        group_openid: &'a str,
        limit: u32,
    ) -> QqFuture<'a, GroupMemberList> {
        Box::pin(OpenApiClient::get_group_members(
            self,
            group_openid,
            None,
            Some(limit),
        ))
    }

    fn set_group_ban<'a>(
        &'a self,
        group_openid: &'a str,
        member_openid: &'a str,
        duration_secs: i64,
    ) -> QqFuture<'a, ()> {
        Box::pin(async move {
            let setting =
                restrict_chat_setting(member_openid, duration_secs, rfc3339::now_unix_secs());
            OpenApiClient::set_restrict_chat_setting(self, group_openid, &setting).await
        })
    }

    fn get_group_bot_state<'a>(&'a self, group_openid: &'a str) -> QqFuture<'a, BotInGroupState> {
        Box::pin(OpenApiClient::get_bot_state(self, group_openid))
    }

    fn get_group_join_request_list<'a>(
        &'a self,
        group_openid: &'a str,
        cursor: Option<&'a str>,
        limit: Option<u32>,
    ) -> QqFuture<'a, JoinRequestList> {
        Box::pin(OpenApiClient::get_join_request_list(
            self,
            group_openid,
            cursor,
            limit,
        ))
    }
}

/// 按官方增量语义构造禁言请求体（对齐 Go `applyRestrictChatSetting`）。
///
/// `duration_secs > 0` → `op=add` + `mute_expire_at = now + duration`（RFC3339）；
/// 否则 → `op=del` + 空 `mute_expire_at`（立即解除）。官方 POST 按成员 `op` 增量生效，
/// 不需要先查询当前设置。
pub(crate) fn restrict_chat_setting(
    member_openid: &str,
    duration_secs: i64,
    now_secs: i64,
) -> RestrictChatSetting {
    let member = if duration_secs > 0 {
        MemberMuteState {
            op: Some("add".to_owned()),
            member_openid: member_openid.to_owned(),
            mute_expire_at: rfc3339::format_unix_secs(now_secs.saturating_add(duration_secs)),
            ..MemberMuteState::default()
        }
    } else {
        MemberMuteState {
            op: Some("del".to_owned()),
            member_openid: member_openid.to_owned(),
            ..MemberMuteState::default()
        }
    };
    RestrictChatSetting {
        global_rule: None,
        members: vec![member],
    }
}

/// handler 的依赖容器：QQ 上游 + 配置快照 + 可选的 idmap。
///
/// `idmap` 是 `Option`：M1 阶段的部署可以只跑查询类 action（直接传 32 位 OpenID），
/// 只有需要「虚拟号 ⇄ OpenID」互转的请求才要求装配。
#[derive(Debug, Clone)]
pub struct ActionContext {
    api: Arc<dyn QqApi>,
    config: Arc<ConfigManager>,
    idmap: Option<Arc<IdMapStore>>,
}

impl ActionContext {
    /// 构造（无 idmap）。
    #[must_use]
    pub fn new(api: Arc<dyn QqApi>, config: Arc<ConfigManager>) -> Self {
        Self {
            api,
            config,
            idmap: None,
        }
    }

    /// 挂上 idmap（虚拟 ID 互转）。
    #[must_use]
    pub fn with_idmap(mut self, idmap: Arc<IdMapStore>) -> Self {
        self.idmap = Some(idmap);
        self
    }

    /// QQ 上游。
    #[must_use]
    pub fn api(&self) -> &dyn QqApi {
        self.api.as_ref()
    }

    /// 配置管理器（每次取快照，热重载即时生效）。
    #[must_use]
    pub fn config(&self) -> &ConfigManager {
        self.config.as_ref()
    }

    /// idmap（未装配时为 `None`）。
    #[must_use]
    pub fn idmap(&self) -> Option<&IdMapStore> {
        self.idmap.as_deref()
    }

    /// 当前快照的 `string_ob11`：决定响应里的 ID 是 JSON 数字还是字符串。
    #[must_use]
    pub fn string_ob11(&self) -> bool {
        self.config.snapshot().get_string_ob11()
    }
}

#[cfg(test)]
mod tests {
    use super::restrict_chat_setting;

    #[test]
    fn positive_duration_adds_expiry_in_rfc3339() {
        let setting = restrict_chat_setting("m1", 600, 1_785_910_749);
        assert_eq!(setting.members.len(), 1);
        let member = &setting.members[0];
        assert_eq!(member.op.as_deref(), Some("add"));
        assert_eq!(member.member_openid, "m1");
        assert_eq!(member.mute_expire_at, "2026-08-05T06:29:09Z");
    }

    #[test]
    fn zero_and_negative_duration_release_the_ban() {
        for duration in [0, -30] {
            let setting = restrict_chat_setting("m1", duration, 1_785_910_749);
            let member = &setting.members[0];
            assert_eq!(member.op.as_deref(), Some("del"));
            assert_eq!(member.mute_expire_at, "");
        }
    }

    #[test]
    fn huge_duration_does_not_overflow() {
        let setting = restrict_chat_setting("m1", i64::MAX, 1_785_910_749);
        assert!(setting.members[0].mute_expire_at.ends_with('Z'));
    }
}
