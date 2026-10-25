//! QQ API 能力抽象：handler 只认 [`QqApi`]，不直接依赖 `gensokyo-qq` 的传输层。
//!
//! # 为什么是装箱 future 而不是 `async fn`
//!
//! [`ActionContext`] 里存的是 `Arc<dyn QqApi>`，trait 必须 dyn-compatible；trait 里的
//! `async fn`（AFIT）不满足这一点。与 [`gensokyo_protocol::Handler`] 同一取舍：一次装箱分配，
//! 换来 runtime 可以把分发放进任意 tokio 任务。
//!
//! # 错误口径
//!
//! 统一返回 [`gensokyo_core::Result`]：上游错误映射到 [`Error::Upstream`]（保留业务码）或
//! [`Error::Network`]（传输层），尚未落地的端点返回 [`Error::Unsupported`]，handler 统一映射成
//! `retcode 1500`。
//!
//! # 端点覆盖
//!
//! | 方法 | 上游端点 | 状态 |
//! |------|----------|------|
//! | [`QqApi::get_group_info`] | `GET /v2/groups/{gid}/info` | 真实调用 |
//! | [`QqApi::get_group_member_info`] | `GET /v2/groups/{gid}/members/{mid}` | 真实调用 |
//! | [`QqApi::get_group_member_list`] | `GET /v2/groups/{gid}/members` | 真实调用 |
//! | [`QqApi::set_group_ban`] | `POST /v2/groups/{gid}/restrict_chat_setting` | 真实调用 |
//! | [`QqApi::get_group_bot_state`] | `GET /v2/groups/{gid}/bot_state` | 真实调用 |
//! | [`QqApi::get_group_join_request_list`] | `GET /v2/groups/{gid}/join_request_list` | 真实调用 |
//! | [`QqApi::set_group_kick`] | `batch_remove_members` | 未落地，`Unsupported` |
//! | [`QqApi::set_group_leave`] | 官方无机器人退群端点 | 未落地，`Unsupported` |
//! | [`QqApi::get_group_list`] | 官方无「列出机器人所在群」端点 | 未落地，`Unsupported` |
//! | [`QqApi::delete_msg`] | 消息撤回端点 | 未落地，`Unsupported` |
//! | [`QqApi::get_msg`] | 官方无按 message_id 查询消息端点 | 未落地，`Unsupported` |
//! | [`QqApi::get_forward_msg`] | 官方无合并转发查询端点 | 未落地，`Unsupported` |
//! | [`QqApi::get_friend_list`] | 官方无好友列表端点 | 未落地，`Unsupported` |
//! | [`QqApi::get_stranger_info`] | 官方无陌生人资料端点 | 未落地，`Unsupported` |
//! | [`QqApi::send_like`] | 官方无点赞端点 | 未落地，`Unsupported` |
//!
//! 「未落地」的两层原因都写在各方法的文档里：要么官方 v2 根本没有这个端点（如好友列表、
//! 点赞、退群），要么端点存在但 `gensokyo-qq` 还没把公开方法补齐（如 `batch_remove_members`、
//! 消息撤回）——后者的路径已在本 trait 上留好，`gensokyo-qq` 落地后只需在
//! `OpenApiClient` 的 impl 里补一个转发，handler 不用改。
//!
//! # 生产实现与测试缝隙
//!
//! 生产侧只有 [`gensokyo_qq::OpenApiClient`] 一个实现（对 `T: HttpTransport` 全量转发）；
//! 单测用 `FakeQqApi` 记录入参并注入上游错误，因此 handler 的「参数翻译 + 错误映射」可以纯本地
//! 验证，不碰网络。

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use gensokyo_config::ConfigManager;
use gensokyo_core::{Error, Result};
use gensokyo_protocol::Segment;
use gensokyo_qq::dto::{MemberMuteState, RestrictChatSetting};
use gensokyo_qq::{
    BotInGroupState, GroupInfo, GroupMember, GroupMemberList, HttpTransport, JoinRequestList,
    OpenApiClient,
};
use gensokyo_storage::IdMapStore;

use crate::rfc3339;

/// [`QqApi`] 的异步返回，形态与 [`gensokyo_protocol::HandlerFuture`] 一致。
pub type QqFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// [`gensokyo_qq::Error`] → 统一错误：业务错误保留 code，其余归到传输层。
pub(crate) fn qq_error(err: gensokyo_qq::Error) -> Error {
    match err {
        gensokyo_qq::Error::Api(api) => Error::Upstream {
            code: api.code,
            message: api.message,
        },
        other => Error::Network(other.to_string()),
    }
}

/// handler 可见的 QQ 上游能力。
pub trait QqApi: Send + Sync + fmt::Debug {
    /// 群基本信息。
    fn get_group_info(&self, group_openid: &str) -> QqFuture<'_, GroupInfo>;

    /// 单个群成员信息。
    fn get_group_member_info(
        &self,
        group_openid: &str,
        member_openid: &str,
    ) -> QqFuture<'_, GroupMember>;

    /// 群成员列表单页（`limit` 为单页条数）。
    fn get_group_member_list(
        &self,
        group_openid: &str,
        limit: u32,
    ) -> QqFuture<'_, GroupMemberList>;

    /// 群成员禁言；`duration_secs <= 0` 表示立即解除。
    fn set_group_ban(
        &self,
        group_openid: &str,
        member_openid: &str,
        duration_secs: i64,
    ) -> QqFuture<'_, ()>;

    /// 机器人在群内的状态。
    fn get_group_bot_state(&self, group_openid: &str) -> QqFuture<'_, BotInGroupState>;

    /// 入群申请列表；`cursor` 为空表示首页，`limit` 为单页条数。
    fn get_group_join_request_list(
        &self,
        group_openid: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> QqFuture<'_, JoinRequestList>;

    /// 批量移出群成员（单批 ≤ 20，`add_blacklist` 同时拉黑）。
    ///
    /// 官方端点 `batch_remove_members` 尚未在 `gensokyo-qq` 暴露公开方法，默认实现返回
    /// [`Error::Unsupported`]。
    fn set_group_kick(
        &self,
        _group_openid: &str,
        _member_openids: &[String],
        _add_blacklist: bool,
    ) -> QqFuture<'_, KickResult> {
        Box::pin(async {
            Err(Error::Unsupported(
                "set_group_kick: gensokyo-qq 尚未暴露 batch_remove_members".to_owned(),
            ))
        })
    }

    /// 机器人退出群（`is_dismiss` 仅对群主形态有意义）。
    ///
    /// 官方 v2 没有机器人退群端点，默认实现返回 [`Error::Unsupported`]。
    fn set_group_leave(&self, _group_openid: &str) -> QqFuture<'_, ()> {
        Box::pin(async {
            Err(Error::Unsupported(
                "set_group_leave: 官方 v2 无机器人退群端点".to_owned(),
            ))
        })
    }

    /// 机器人所在的群列表。
    ///
    /// 官方没有「列出机器人所在群」的端点；Go 侧靠 idmap 枚举群虚拟号后逐群 `info` 拼装，
    /// 而 `gensokyo-storage` 的 [`IdMapStore`] 目前没有枚举接口，因此返回
    /// [`Error::Unsupported`]。
    fn get_group_list(&self) -> QqFuture<'_, Vec<GroupInfo>> {
        Box::pin(async {
            Err(Error::Unsupported(
                "get_group_list: 官方无群列表端点，且 idmap 暂无枚举接口".to_owned(),
            ))
        })
    }

    /// 撤回消息（群聊 / C2C 二选一）。
    ///
    /// 撤回端点尚未在 `gensokyo-qq` 暴露公开方法，默认实现返回 [`Error::Unsupported`]。
    fn delete_msg(&self, _target: RetractRequest) -> QqFuture<'_, ()> {
        Box::pin(async {
            Err(Error::Unsupported(
                "delete_msg: gensokyo-qq 尚未暴露消息撤回端点".to_owned(),
            ))
        })
    }

    /// 按 message_id 查询消息内容。
    ///
    /// 官方 v2 没有按 message_id 查询消息的端点，默认实现返回 [`Error::Unsupported`]。
    fn get_msg(&self, _message_id: &str) -> QqFuture<'_, MsgDetail> {
        Box::pin(async {
            Err(Error::Unsupported(
                "get_msg: 官方 v2 无按 message_id 查询消息的端点".to_owned(),
            ))
        })
    }

    /// 按转发 ID 取合并转发节点。
    ///
    /// 官方 v2 没有合并转发查询端点，默认实现返回 [`Error::Unsupported`]。
    fn get_forward_msg(&self, _forward_id: &str) -> QqFuture<'_, Vec<Segment>> {
        Box::pin(async {
            Err(Error::Unsupported(
                "get_forward_msg: 官方 v2 无合并转发查询端点".to_owned(),
            ))
        })
    }

    /// 好友列表。
    ///
    /// 官方没有好友列表端点；Go 侧读 idmap 全量用户，而 [`IdMapStore`] 没有枚举接口，
    /// 因此返回 [`Error::Unsupported`]。
    fn get_friend_list(&self) -> QqFuture<'_, Vec<FriendEntry>> {
        Box::pin(async {
            Err(Error::Unsupported(
                "get_friend_list: 官方无好友列表端点，且 idmap 暂无枚举接口".to_owned(),
            ))
        })
    }

    /// 陌生人资料（按 `user_openid`）。
    ///
    /// 官方没有陌生人资料端点（成员资料只能按群查询），默认实现返回 [`Error::Unsupported`]。
    fn get_stranger_info(&self, _user_openid: &str) -> QqFuture<'_, StrangerProfile> {
        Box::pin(async {
            Err(Error::Unsupported(
                "get_stranger_info: 官方无陌生人资料端点".to_owned(),
            ))
        })
    }

    /// 给用户主页点赞（`times` 次）。
    ///
    /// 官方没有点赞端点，默认实现返回 [`Error::Unsupported`]。
    fn send_like(&self, _user_openid: &str, _times: u32) -> QqFuture<'_, ()> {
        Box::pin(async { Err(Error::Unsupported("send_like: 官方无点赞端点".to_owned())) })
    }
}

impl<T: HttpTransport> QqApi for OpenApiClient<T> {
    // 入参先转成自有字符串：`QqFuture<'_, T>` 的 `'_` 只绑定 `&self`，async 块若直接借用
    // 入参，返回的 future 会比入参活得久。

    fn get_group_info(&self, group_openid: &str) -> QqFuture<'_, GroupInfo> {
        let group_openid = group_openid.to_owned();
        // 显式 `OpenApiClient::` 前缀：同名 trait 方法存在时，方法调用语法容易递归到自己
        Box::pin(async move {
            OpenApiClient::get_group_info(self, &group_openid)
                .await
                .map_err(qq_error)
        })
    }

    fn get_group_member_info(
        &self,
        group_openid: &str,
        member_openid: &str,
    ) -> QqFuture<'_, GroupMember> {
        let group_openid = group_openid.to_owned();
        let member_openid = member_openid.to_owned();
        Box::pin(async move {
            OpenApiClient::get_member_info(self, &group_openid, &member_openid)
                .await
                .map_err(qq_error)
        })
    }

    fn get_group_member_list(
        &self,
        group_openid: &str,
        limit: u32,
    ) -> QqFuture<'_, GroupMemberList> {
        let group_openid = group_openid.to_owned();
        Box::pin(async move {
            OpenApiClient::get_group_members(self, &group_openid, None, Some(limit))
                .await
                .map_err(qq_error)
        })
    }

    fn set_group_ban(
        &self,
        group_openid: &str,
        member_openid: &str,
        duration_secs: i64,
    ) -> QqFuture<'_, ()> {
        let group_openid = group_openid.to_owned();
        let member_openid = member_openid.to_owned();
        Box::pin(async move {
            let setting =
                restrict_chat_setting(&member_openid, duration_secs, rfc3339::now_unix_secs());
            OpenApiClient::set_restrict_chat_setting(self, &group_openid, &setting)
                .await
                .map_err(qq_error)
        })
    }

    fn get_group_bot_state(&self, group_openid: &str) -> QqFuture<'_, BotInGroupState> {
        let group_openid = group_openid.to_owned();
        Box::pin(async move {
            OpenApiClient::get_bot_state(self, &group_openid)
                .await
                .map_err(qq_error)
        })
    }

    fn get_group_join_request_list(
        &self,
        group_openid: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> QqFuture<'_, JoinRequestList> {
        let group_openid = group_openid.to_owned();
        let cursor = cursor.map(str::to_owned);
        Box::pin(async move {
            OpenApiClient::get_join_request_list(self, &group_openid, cursor.as_deref(), limit)
                .await
                .map_err(qq_error)
        })
    }
}

/// `set_group_kick` 的上游回执（对齐 Go `BatchRemoveMembersResponse`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct KickResult {
    /// 上游的批量移出结果（官方 `remove_members_result`）。
    #[serde(default)]
    pub remove_members_result: String,
    /// 拉黑失败的成员 OpenID（空数组表示全部成功）。
    #[serde(default)]
    pub add_to_member_blacklist_fail_openids: Vec<String>,
}

/// `delete_msg` 的撤回目标：群聊与 C2C 二选一（与 Go 的 `group_id` / `user_id` 分支对应）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetractRequest {
    /// 群 OpenID（群聊撤回）。
    pub group_openid: Option<String>,
    /// 用户 OpenID（C2C 撤回）。
    pub user_openid: Option<String>,
    /// 真实消息 ID（已按 `string_ob11` / idmap 还原）。
    pub message_id: String,
}

/// `get_msg` 的上游结果（形状为将来接入端点预留）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MsgDetail {
    /// 上游消息 ID。
    pub message_id: String,
    /// 纯文本内容（无消息段时回退到它）。
    pub text: String,
    /// 消息段（按收到的顺序；为空表示只有纯文本）。
    pub segments: Vec<Segment>,
}

/// 好友条目。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FriendEntry {
    /// 好友的 user_openid。
    pub user_openid: String,
    /// 昵称。
    pub nickname: String,
    /// 备注（官方无此字段时为空串）。
    pub remark: String,
}

/// 陌生人资料。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StrangerProfile {
    /// 用户 user_openid。
    pub user_openid: String,
    /// 昵称。
    pub nickname: String,
    /// 性别（官方无此字段时给中性值 `unknown`）。
    pub sex: String,
    /// 年龄（官方无此字段时为 0）。
    pub age: i64,
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
    use super::QqApi;
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

    #[test]
    fn unsupported_endpoints_default_to_a_clear_error() {
        /// 只实现真实端点、其余走默认实现的替身。
        #[derive(Debug)]
        struct BareApi;

        impl super::QqApi for BareApi {
            fn get_group_info(
                &self,
                _group_openid: &str,
            ) -> super::QqFuture<'_, gensokyo_qq::GroupInfo> {
                Box::pin(async { Err(super::Error::Unsupported("未配置".to_owned())) })
            }

            fn get_group_member_info(
                &self,
                _group_openid: &str,
                _member_openid: &str,
            ) -> super::QqFuture<'_, gensokyo_qq::GroupMember> {
                Box::pin(async { Err(super::Error::Unsupported("未配置".to_owned())) })
            }

            fn get_group_member_list(
                &self,
                _group_openid: &str,
                _limit: u32,
            ) -> super::QqFuture<'_, gensokyo_qq::GroupMemberList> {
                Box::pin(async { Err(super::Error::Unsupported("未配置".to_owned())) })
            }

            fn set_group_ban(
                &self,
                _group_openid: &str,
                _member_openid: &str,
                _duration_secs: i64,
            ) -> super::QqFuture<'_, ()> {
                Box::pin(async { Err(super::Error::Unsupported("未配置".to_owned())) })
            }

            fn get_group_bot_state(
                &self,
                _group_openid: &str,
            ) -> super::QqFuture<'_, gensokyo_qq::BotInGroupState> {
                Box::pin(async { Err(super::Error::Unsupported("未配置".to_owned())) })
            }

            fn get_group_join_request_list(
                &self,
                _group_openid: &str,
                _cursor: Option<&str>,
                _limit: Option<u32>,
            ) -> super::QqFuture<'_, gensokyo_qq::JoinRequestList> {
                Box::pin(async { Err(super::Error::Unsupported("未配置".to_owned())) })
            }
        }

        let mut future = std::pin::pin!(QqApi::set_group_leave(&BareApi, "g"));
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        let error = loop {
            match future.as_mut().poll(&mut context) {
                std::task::Poll::Ready(output) => break output.expect_err("默认实现应返回未实现"),
                std::task::Poll::Pending => std::thread::yield_now(),
            }
        };
        assert!(matches!(error, super::Error::Unsupported(_)), "{error}");
    }
}
