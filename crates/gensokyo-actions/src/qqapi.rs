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
//! | [`QqApi::get_group_member_list`] / [`QqApi::get_group_member_page`] | `GET /v2/groups/{gid}/members` | 真实调用（后者带 `cursor` 分页） |
//! | [`QqApi::set_group_ban`] | `POST /v2/groups/{gid}/restrict_chat_setting` | 真实调用 |
//! | [`QqApi::get_group_bot_state`] | `GET /v2/groups/{gid}/bot_state` | 真实调用 |
//! | [`QqApi::get_group_join_request_list`] | `GET /v2/groups/{gid}/join_request_list` | 真实调用 |
//! | [`QqApi::send_group_message`] | `POST /v2/groups/{gid}/messages` | 真实调用（deferred 执行链） |
//! | [`QqApi::send_c2c_message`] | `POST /v2/users/{uid}/messages` | 真实调用（deferred 执行链） |
//! | [`QqApi::upload_group_file`] | `POST /v2/groups/{gid}/files` | 真实调用（deferred 媒体上传） |
//! | [`QqApi::upload_c2c_file`] | `POST /v2/users/{uid}/files` | 真实调用（deferred 媒体上传） |
//! | [`QqApi::prepare_media_image`] | `POST /v2/media-images/prepare` | 真实调用（素材管理） |
//! | [`QqApi::put_presigned_image`] | `PUT <presigned_url>`（COS） | 真实调用（不带 Authorization） |
//! | [`QqApi::complete_media_image`] | `POST /v2/media-images/complete` | 真实调用（素材管理） |
//! | [`QqApi::list_media_images`] | `GET /v2/media-images?cursor=` | 真实调用（素材管理） |
//! | [`QqApi::delete_media_image`] | `DELETE /v2/media-images/{id}/delete` | 真实调用（素材管理） |
//! | [`QqApi::set_group_kick`] | `POST /v2/groups/{gid}/batch_remove_members` | 真实调用 |
//! | [`QqApi::delete_msg`] | `DELETE /v2/groups/{gid}/messages/{mid}`、`DELETE /v2/users/{uid}/messages/{mid}` | 真实调用 |
//! | [`QqApi::get_panels`] / [`QqApi::create_panel`] / [`QqApi::get_panel`] / [`QqApi::update_panel`] / [`QqApi::delete_panel`] / [`QqApi::update_panel_target`] | `/v2/panels*` | 真实调用 |
//! | [`QqApi::get_menu`] / [`QqApi::put_menu`] | `/v2/menu` | 真实调用 |
//! | [`QqApi::create_join_approval_strategy`] 等 6 个 | `/v2/groups/join_approval_strategy*` | 真实调用 |
//! | [`QqApi::member_blacklist_list`] / [`QqApi::update_member_blacklist`] | `/v2/groups/{gid}/member_blacklist` | 真实调用 |
//! | [`QqApi::approve_join_request`] | `POST /v2/groups/{gid}/approval_join_request/{member}` | 真实调用 |
//! | [`QqApi::put_interaction`] | `PUT /interactions/{id}` | 真实调用（无 `/v2` 前缀） |
//! | [`QqApi::generate_url_link`] | `POST /v2/generate_url_link` | 真实调用 |
//! | [`QqApi::set_group_leave`] | 官方无机器人退群端点 | 未落地，`Unsupported` |
//! | [`QqApi::get_group_list`] | 官方无「列出机器人所在群」端点 | 未落地，`Unsupported` |
//! | [`QqApi::get_msg`] | 官方无按 message_id 查询消息端点 | 未落地，`Unsupported` |
//! | [`QqApi::get_forward_msg`] | 官方无合并转发查询端点 | 未落地，`Unsupported` |
//! | [`QqApi::get_friend_list`] | 官方无好友列表端点 | 未落地，`Unsupported` |
//! | [`QqApi::get_stranger_info`] | 官方无陌生人资料端点 | 未落地，`Unsupported` |
//! | [`QqApi::send_like`] | 官方无点赞端点 | 未落地，`Unsupported` |
//!
//! 「未落地」的两层原因都写在各方法的文档里：要么官方 v2 根本没有这个端点（如好友列表、
//! 点赞、退群、群列表），要么要等运行时的发送链补齐（如 SSE / `IsWakeup` / 原始直发）。
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
use gensokyo_qq::dto::media_image::{
    MediaImageCompleteRequest, MediaImageCompleteResponse, MediaImageList,
    MediaImagePrepareRequest, MediaImagePrepareResponse,
};
use gensokyo_qq::dto::{
    ApprovalJoinRequest, BatchRemoveMembersRequest, CreateJoinApprovalStrategyRequest,
    CreateJoinApprovalStrategyResponse, CreatePanelRequest, CreatePanelResponse, FileUpload,
    GenerateUrlLink, GenerateUrlLinkRequest, JoinApprovalStrategyList, MediaResponse,
    MemberBlacklistList, MemberBlacklistRequest, MemberBlacklistResponse, MemberMuteState,
    MenuResponse, MessageResponse, MessageToCreate, PanelList, PanelRecordDetail,
    PanelTargetRequest, PutMenuRequest, RestrictChatSetting, UpdateJoinApprovalStrategyRequest,
    UpdateJoinApprovalStrategyResponse, UpdatePanelRequest, VersionResponse, WhitelistUsersRequest,
    WhitelistUsersResponse,
};
use gensokyo_qq::{
    BotInGroupState, GroupInfo, GroupMember, GroupMemberList, HttpTransport, JoinRequestList,
    OpenApiClient,
};
use gensokyo_storage::IdMapStore;

use crate::rfc3339;
use crate::send::SendTarget;
use crate::util;

/// [`QqApi`] 的异步返回，形态与 [`gensokyo_protocol::HandlerFuture`] 一致。
pub type QqFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// 默认实现的统一拒绝：`gensokyo-qq` 已支持该端点，但替身（测试 / 只跑查询的部署）不必实现。
fn unsupported<T>(feature: &str) -> Result<T> {
    Err(Error::Unsupported(format!(
        "{feature}端点未在当前 QqApi 实现中提供"
    )))
}

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

    /// 群成员列表单页（带游标；`cursor` 为 `None` / 空串时取首页）。
    ///
    /// 独立成型而不给 [`QqApi::get_group_member_list`] 加形参：该方法已被 `deferred.rs`
    /// 等 P-R2 文件里的替身实现，改签名会把改动扩散到别人的地盘。默认实现忽略游标、
    /// 退回首页，与旧行为等价。
    fn get_group_member_page(
        &self,
        group_openid: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> QqFuture<'_, GroupMemberList> {
        let _ = cursor;
        self.get_group_member_list(group_openid, limit)
    }

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

    /// 发送群消息（`send_*` 的 deferred 计划由 runtime 在执行链上调用）。
    ///
    /// 入参取所有权：调用方（deferred 执行链）已经把 [`gensokyo_qq::dto::MessageToCreate`]
    /// 构造好，直接移动进来可省一次克隆。
    fn send_group_message(
        &self,
        group_openid: String,
        msg: MessageToCreate,
    ) -> QqFuture<'_, MessageResponse>;

    /// 发送 C2C 消息（私聊 / 群临时会话）。
    fn send_c2c_message(
        &self,
        user_openid: String,
        msg: MessageToCreate,
    ) -> QqFuture<'_, MessageResponse>;

    /// 上传群会话媒体（`POST /v2/groups/{group_openid}/files`），返回 [`MediaResponse::file_info`]。
    ///
    /// deferred 执行链先取字节再上传：拿到 `file_info` 后填进
    /// [`gensokyo_qq::dto::Media`]，随 [`QqApi::send_group_message`] 发出。
    ///
    /// 默认实现返回 [`Error::Unsupported`]：只跑查询类 action 的替身不必实现媒体上传。
    fn upload_group_file(
        &self,
        _group_openid: &str,
        _file: FileUpload,
    ) -> QqFuture<'_, MediaResponse> {
        Box::pin(async {
            Err(Error::Unsupported(
                "upload_group_file: 当前 QqApi 实现不支持媒体上传".to_owned(),
            ))
        })
    }

    /// 上传 C2C 媒体（`POST /v2/users/{user_openid}/files`）；语义同
    /// [`QqApi::upload_group_file`]。
    fn upload_c2c_file(
        &self,
        _user_openid: &str,
        _file: FileUpload,
    ) -> QqFuture<'_, MediaResponse> {
        Box::pin(async {
            Err(Error::Unsupported(
                "upload_c2c_file: 当前 QqApi 实现不支持媒体上传".to_owned(),
            ))
        })
    }

    /// 素材上传·申请（`POST /v2/media-images/prepare`）。
    ///
    /// 三个素材方法（prepare / put_presigned / complete）由 handler 按序编排：任一步失败都要能
    /// 定位到具体环节，因此不合并成一个「上传」方法。
    ///
    /// 默认实现返回 [`Error::Unsupported`]：只跑查询类 action 的替身不必实现素材管理。
    fn prepare_media_image(
        &self,
        _request: MediaImagePrepareRequest,
    ) -> QqFuture<'_, MediaImagePrepareResponse> {
        Box::pin(async {
            Err(Error::Unsupported(
                "prepare_media_image: 当前 QqApi 实现不支持素材管理".to_owned(),
            ))
        })
    }

    /// 素材上传·直传 COS（`PUT <presigned_url>`）；返回去引号后的 ETag（等于文件 md5）。
    fn put_presigned_image(
        &self,
        _presigned_url: &str,
        _content_type: &str,
        _bytes: Vec<u8>,
    ) -> QqFuture<'_, String> {
        Box::pin(async {
            Err(Error::Unsupported(
                "put_presigned_image: 当前 QqApi 实现不支持素材管理".to_owned(),
            ))
        })
    }

    /// 素材上传·完成（`POST /v2/media-images/complete`）。
    fn complete_media_image(
        &self,
        _request: MediaImageCompleteRequest,
    ) -> QqFuture<'_, MediaImageCompleteResponse> {
        Box::pin(async {
            Err(Error::Unsupported(
                "complete_media_image: 当前 QqApi 实现不支持素材管理".to_owned(),
            ))
        })
    }

    /// 素材列表（`GET /v2/media-images?cursor=`）；`cursor` 为 `None` 表示首页。
    fn list_media_images(&self, _cursor: Option<&str>) -> QqFuture<'_, MediaImageList> {
        Box::pin(async {
            Err(Error::Unsupported(
                "list_media_images: 当前 QqApi 实现不支持素材管理".to_owned(),
            ))
        })
    }

    /// 删除素材（`DELETE /v2/media-images/{image_id}/delete`）。
    fn delete_media_image(&self, _image_id: &str) -> QqFuture<'_, ()> {
        Box::pin(async {
            Err(Error::Unsupported(
                "delete_media_image: 当前 QqApi 实现不支持素材管理".to_owned(),
            ))
        })
    }

    /// 指令面板列表（`GET /v2/panels`）；`cursor` 为空串等同首页。
    ///
    /// 默认实现返回 [`Error::Unsupported`]：只跑查询类 action 的替身不必实现面板管理。
    fn get_panels(
        &self,
        _scope: &str,
        _cursor: Option<&str>,
        _limit: Option<u32>,
    ) -> QqFuture<'_, PanelList> {
        Box::pin(async { unsupported("指令面板") })
    }

    /// 创建指令面板（`POST /v2/panels`）。
    fn create_panel(&self, _request: &CreatePanelRequest) -> QqFuture<'_, CreatePanelResponse> {
        Box::pin(async { unsupported("指令面板") })
    }

    /// 单个指令面板详情（`GET /v2/panels/{panel_id}`）。
    fn get_panel(&self, _panel_id: &str) -> QqFuture<'_, PanelRecordDetail> {
        Box::pin(async { unsupported("指令面板") })
    }

    /// 更新指令面板（`PUT /v2/panels/{panel_id}`，覆盖元素与备注）。
    fn update_panel(
        &self,
        _panel_id: &str,
        _request: &UpdatePanelRequest,
    ) -> QqFuture<'_, VersionResponse> {
        Box::pin(async { unsupported("指令面板") })
    }

    /// 删除指令面板（`DELETE /v2/panels/{panel_id}`）。
    fn delete_panel(&self, _panel_id: &str) -> QqFuture<'_, ()> {
        Box::pin(async { unsupported("指令面板") })
    }

    /// 增删指令面板关联对象（`PUT /v2/panels/{panel_id}/target`）。
    fn update_panel_target(
        &self,
        _panel_id: &str,
        _request: &PanelTargetRequest,
    ) -> QqFuture<'_, ()> {
        Box::pin(async { unsupported("指令面板") })
    }

    /// 获取自定义菜单（`GET /v2/menu`）。
    fn get_menu(&self) -> QqFuture<'_, MenuResponse> {
        Box::pin(async { unsupported("自定义菜单") })
    }

    /// 设置自定义菜单（`PUT /v2/menu`，覆盖式）。
    fn put_menu(&self, _request: &PutMenuRequest) -> QqFuture<'_, VersionResponse> {
        Box::pin(async { unsupported("自定义菜单") })
    }

    /// 创建入群自动审批策略（`POST /v2/groups/join_approval_strategy`）。
    fn create_join_approval_strategy(
        &self,
        _request: &CreateJoinApprovalStrategyRequest,
    ) -> QqFuture<'_, CreateJoinApprovalStrategyResponse> {
        Box::pin(async { unsupported("自动审批策略") })
    }

    /// 审批策略列表（`GET /v2/groups/join_approval_strategy`）。
    fn list_join_approval_strategy(
        &self,
        _cursor: Option<&str>,
        _limit: Option<u32>,
    ) -> QqFuture<'_, JoinApprovalStrategyList> {
        Box::pin(async { unsupported("自动审批策略") })
    }

    /// 更新审批策略（`PATCH /v2/groups/join_approval_strategy/{strategy_id}`）。
    fn update_join_approval_strategy(
        &self,
        _strategy_id: &str,
        _request: &UpdateJoinApprovalStrategyRequest,
    ) -> QqFuture<'_, UpdateJoinApprovalStrategyResponse> {
        Box::pin(async { unsupported("自动审批策略") })
    }

    /// 生效审批策略（`POST .../{strategy_id}/execute`）。
    fn execute_join_approval_strategy(&self, _strategy_id: &str) -> QqFuture<'_, ()> {
        Box::pin(async { unsupported("自动审批策略") })
    }

    /// 增删审批策略白名单（`POST .../{strategy_id}/whitelist_users`）。
    fn update_join_approval_strategy_whitelist(
        &self,
        _strategy_id: &str,
        _request: &WhitelistUsersRequest,
    ) -> QqFuture<'_, WhitelistUsersResponse> {
        Box::pin(async { unsupported("自动审批策略") })
    }

    /// 删除审批策略（`DELETE .../{strategy_id}`）。
    fn delete_join_approval_strategy(&self, _strategy_id: &str) -> QqFuture<'_, ()> {
        Box::pin(async { unsupported("自动审批策略") })
    }

    /// 群黑名单列表（`GET /v2/groups/{gid}/member_blacklist`）。
    fn member_blacklist_list(
        &self,
        _group_openid: &str,
        _cursor: Option<&str>,
        _limit: Option<u32>,
    ) -> QqFuture<'_, MemberBlacklistList> {
        Box::pin(async { unsupported("群黑名单") })
    }

    /// 增删群黑名单（`POST /v2/groups/{gid}/member_blacklist`，单批 ≤ 20）。
    fn update_member_blacklist(
        &self,
        _group_openid: &str,
        _request: &MemberBlacklistRequest,
    ) -> QqFuture<'_, MemberBlacklistResponse> {
        Box::pin(async { unsupported("群黑名单") })
    }

    /// 审批入群申请（`POST /v2/groups/{gid}/approval_join_request/{member_openid}`）。
    fn approve_join_request(
        &self,
        _group_openid: &str,
        _member_openid: &str,
        _request: &ApprovalJoinRequest,
    ) -> QqFuture<'_, ()> {
        Box::pin(async { unsupported("入群审批") })
    }

    /// 回应互动事件（`PUT /interactions/{interaction_id}`，`code` 为 0~5）。
    fn put_interaction(&self, _interaction_id: &str, _code: u32) -> QqFuture<'_, ()> {
        Box::pin(async { unsupported("互动事件") })
    }

    /// 生成机器人资料页分享链接（`POST /v2/generate_url_link`）。
    fn generate_url_link(
        &self,
        _request: &GenerateUrlLinkRequest,
    ) -> QqFuture<'_, GenerateUrlLink> {
        Box::pin(async { unsupported("资料页分享链接") })
    }

    /// 批量移出群成员（单批 ≤ 20，`add_blacklist` 同时拉黑）。
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

    fn get_group_member_page(
        &self,
        group_openid: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> QqFuture<'_, GroupMemberList> {
        let group_openid = group_openid.to_owned();
        // async 块里不能借用入参，游标先转成所有权再透传
        let cursor = cursor.map(str::to_owned);
        Box::pin(async move {
            OpenApiClient::get_group_members(self, &group_openid, cursor.as_deref(), Some(limit))
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

    fn send_group_message(
        &self,
        group_openid: String,
        msg: MessageToCreate,
    ) -> QqFuture<'_, MessageResponse> {
        Box::pin(async move {
            OpenApiClient::send_group_message(self, &group_openid, &msg)
                .await
                .map_err(qq_error)
        })
    }

    fn send_c2c_message(
        &self,
        user_openid: String,
        msg: MessageToCreate,
    ) -> QqFuture<'_, MessageResponse> {
        Box::pin(async move {
            OpenApiClient::send_c2c_message(self, &user_openid, &msg)
                .await
                .map_err(qq_error)
        })
    }

    fn upload_group_file(
        &self,
        group_openid: &str,
        file: FileUpload,
    ) -> QqFuture<'_, MediaResponse> {
        let group_openid = group_openid.to_owned();
        Box::pin(async move {
            OpenApiClient::upload_group_file(self, &group_openid, &file)
                .await
                .map_err(qq_error)
        })
    }

    fn upload_c2c_file(&self, user_openid: &str, file: FileUpload) -> QqFuture<'_, MediaResponse> {
        let user_openid = user_openid.to_owned();
        Box::pin(async move {
            OpenApiClient::upload_c2c_file(self, &user_openid, &file)
                .await
                .map_err(qq_error)
        })
    }

    fn prepare_media_image(
        &self,
        request: MediaImagePrepareRequest,
    ) -> QqFuture<'_, MediaImagePrepareResponse> {
        Box::pin(async move {
            OpenApiClient::prepare_media_image(self, &request)
                .await
                .map_err(qq_error)
        })
    }

    fn put_presigned_image(
        &self,
        presigned_url: &str,
        content_type: &str,
        bytes: Vec<u8>,
    ) -> QqFuture<'_, String> {
        let presigned_url = presigned_url.to_owned();
        let content_type = content_type.to_owned();
        Box::pin(async move {
            OpenApiClient::put_presigned_image(self, &presigned_url, &content_type, &bytes)
                .await
                .map_err(qq_error)
        })
    }

    fn complete_media_image(
        &self,
        request: MediaImageCompleteRequest,
    ) -> QqFuture<'_, MediaImageCompleteResponse> {
        Box::pin(async move {
            OpenApiClient::complete_media_image(self, &request)
                .await
                .map_err(qq_error)
        })
    }

    fn list_media_images(&self, cursor: Option<&str>) -> QqFuture<'_, MediaImageList> {
        let cursor = cursor.map(str::to_owned);
        Box::pin(async move {
            OpenApiClient::list_media_images(self, cursor.as_deref())
                .await
                .map_err(qq_error)
        })
    }

    fn delete_media_image(&self, image_id: &str) -> QqFuture<'_, ()> {
        let image_id = image_id.to_owned();
        Box::pin(async move {
            OpenApiClient::delete_media_image(self, &image_id)
                .await
                .map_err(qq_error)
        })
    }

    fn get_panels(
        &self,
        scope: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> QqFuture<'_, PanelList> {
        let scope = scope.to_owned();
        let cursor = cursor.map(str::to_owned);
        Box::pin(async move {
            OpenApiClient::get_panels(self, &scope, cursor.as_deref(), limit)
                .await
                .map_err(qq_error)
        })
    }

    fn create_panel(&self, request: &CreatePanelRequest) -> QqFuture<'_, CreatePanelResponse> {
        let request = request.clone();
        Box::pin(async move {
            OpenApiClient::create_panel(self, &request)
                .await
                .map_err(qq_error)
        })
    }

    fn get_panel(&self, panel_id: &str) -> QqFuture<'_, PanelRecordDetail> {
        let panel_id = panel_id.to_owned();
        Box::pin(async move {
            OpenApiClient::get_panel(self, &panel_id)
                .await
                .map_err(qq_error)
        })
    }

    fn update_panel(
        &self,
        panel_id: &str,
        request: &UpdatePanelRequest,
    ) -> QqFuture<'_, VersionResponse> {
        let panel_id = panel_id.to_owned();
        let request = request.clone();
        Box::pin(async move {
            OpenApiClient::update_panel(self, &panel_id, &request)
                .await
                .map_err(qq_error)
        })
    }

    fn delete_panel(&self, panel_id: &str) -> QqFuture<'_, ()> {
        let panel_id = panel_id.to_owned();
        Box::pin(async move {
            OpenApiClient::delete_panel(self, &panel_id)
                .await
                .map_err(qq_error)
        })
    }

    fn update_panel_target(
        &self,
        panel_id: &str,
        request: &PanelTargetRequest,
    ) -> QqFuture<'_, ()> {
        let panel_id = panel_id.to_owned();
        let request = request.clone();
        Box::pin(async move {
            OpenApiClient::update_panel_target(self, &panel_id, &request)
                .await
                .map_err(qq_error)
        })
    }

    fn get_menu(&self) -> QqFuture<'_, MenuResponse> {
        Box::pin(async move { OpenApiClient::get_menu(self).await.map_err(qq_error) })
    }

    fn put_menu(&self, request: &PutMenuRequest) -> QqFuture<'_, VersionResponse> {
        let request = request.clone();
        Box::pin(async move {
            OpenApiClient::put_menu(self, &request)
                .await
                .map_err(qq_error)
        })
    }

    fn create_join_approval_strategy(
        &self,
        request: &CreateJoinApprovalStrategyRequest,
    ) -> QqFuture<'_, CreateJoinApprovalStrategyResponse> {
        let request = request.clone();
        Box::pin(async move {
            OpenApiClient::create_join_approval_strategy(self, &request)
                .await
                .map_err(qq_error)
        })
    }

    fn list_join_approval_strategy(
        &self,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> QqFuture<'_, JoinApprovalStrategyList> {
        let cursor = cursor.map(str::to_owned);
        Box::pin(async move {
            OpenApiClient::list_join_approval_strategy(self, cursor.as_deref(), limit)
                .await
                .map_err(qq_error)
        })
    }

    fn update_join_approval_strategy(
        &self,
        strategy_id: &str,
        request: &UpdateJoinApprovalStrategyRequest,
    ) -> QqFuture<'_, UpdateJoinApprovalStrategyResponse> {
        let strategy_id = strategy_id.to_owned();
        let request = request.clone();
        Box::pin(async move {
            OpenApiClient::update_join_approval_strategy(self, &strategy_id, &request)
                .await
                .map_err(qq_error)
        })
    }

    fn execute_join_approval_strategy(&self, strategy_id: &str) -> QqFuture<'_, ()> {
        let strategy_id = strategy_id.to_owned();
        Box::pin(async move {
            OpenApiClient::execute_join_approval_strategy(self, &strategy_id)
                .await
                .map_err(qq_error)
        })
    }

    fn update_join_approval_strategy_whitelist(
        &self,
        strategy_id: &str,
        request: &WhitelistUsersRequest,
    ) -> QqFuture<'_, WhitelistUsersResponse> {
        let strategy_id = strategy_id.to_owned();
        let request = request.clone();
        Box::pin(async move {
            OpenApiClient::update_join_approval_strategy_whitelist(self, &strategy_id, &request)
                .await
                .map_err(qq_error)
        })
    }

    fn delete_join_approval_strategy(&self, strategy_id: &str) -> QqFuture<'_, ()> {
        let strategy_id = strategy_id.to_owned();
        Box::pin(async move {
            OpenApiClient::delete_join_approval_strategy(self, &strategy_id)
                .await
                .map_err(qq_error)
        })
    }

    fn member_blacklist_list(
        &self,
        group_openid: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> QqFuture<'_, MemberBlacklistList> {
        let group_openid = group_openid.to_owned();
        let cursor = cursor.map(str::to_owned);
        Box::pin(async move {
            OpenApiClient::member_blacklist_list(self, &group_openid, cursor.as_deref(), limit)
                .await
                .map_err(qq_error)
        })
    }

    fn update_member_blacklist(
        &self,
        group_openid: &str,
        request: &MemberBlacklistRequest,
    ) -> QqFuture<'_, MemberBlacklistResponse> {
        let group_openid = group_openid.to_owned();
        let request = request.clone();
        Box::pin(async move {
            OpenApiClient::update_member_blacklist(self, &group_openid, &request)
                .await
                .map_err(qq_error)
        })
    }

    fn approve_join_request(
        &self,
        group_openid: &str,
        member_openid: &str,
        request: &ApprovalJoinRequest,
    ) -> QqFuture<'_, ()> {
        let group_openid = group_openid.to_owned();
        let member_openid = member_openid.to_owned();
        let request = request.clone();
        Box::pin(async move {
            OpenApiClient::approve_join_request(self, &group_openid, &member_openid, &request)
                .await
                .map_err(qq_error)
        })
    }

    fn put_interaction(&self, interaction_id: &str, code: u32) -> QqFuture<'_, ()> {
        let interaction_id = interaction_id.to_owned();
        Box::pin(async move {
            OpenApiClient::put_interaction(self, &interaction_id, code)
                .await
                .map_err(qq_error)
        })
    }

    fn generate_url_link(&self, request: &GenerateUrlLinkRequest) -> QqFuture<'_, GenerateUrlLink> {
        let request = request.clone();
        Box::pin(async move {
            OpenApiClient::generate_url_link(self, &request)
                .await
                .map_err(qq_error)
        })
    }

    fn set_group_kick(
        &self,
        group_openid: &str,
        member_openids: &[String],
        add_blacklist: bool,
    ) -> QqFuture<'_, KickResult> {
        let group_openid = group_openid.to_owned();
        let request = BatchRemoveMembersRequest {
            member_openids: member_openids.to_vec(),
            add_to_member_blacklist: add_blacklist,
        };
        Box::pin(async move {
            let response = OpenApiClient::batch_remove_members(self, &group_openid, &request)
                .await
                .map_err(qq_error)?;
            Ok(KickResult {
                remove_members_result: response.remove_members_result,
                add_to_member_blacklist_fail_openids: response.add_to_member_blacklist_fail_openids,
            })
        })
    }

    fn delete_msg(&self, target: RetractRequest) -> QqFuture<'_, ()> {
        let RetractRequest {
            group_openid,
            user_openid,
            message_id,
        } = target;
        Box::pin(async move {
            match (group_openid, user_openid) {
                (Some(group_openid), _) => {
                    OpenApiClient::retract_group_message(self, &group_openid, &message_id)
                        .await
                        .map_err(qq_error)
                }
                (None, Some(user_openid)) => {
                    OpenApiClient::retract_c2c_message(self, &user_openid, &message_id)
                        .await
                        .map_err(qq_error)
                }
                (None, None) => Err(Error::Protocol(
                    "delete_msg 需要 group_id 或 user_id".to_owned(),
                )),
            }
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

    /// ID 文本 → 上游 OpenID：32 位 OpenID 直通，其余按虚拟号经 idmap 反查。
    ///
    /// `label` 是请求字段名（错误文案），`kind` 是被反查的目标（`group_openid` /
    /// `user_openid` / `member_openid`）。口径与 handler 侧完全一致（[`crate::util`]）。
    ///
    /// # Errors
    /// 既不是 32 位 OpenID 也不是虚拟号、未装配 idmap、或虚拟号未绑定时返回
    /// [`Error::Protocol`]。
    pub fn resolve_openid(&self, label: &str, raw: &str, kind: &str) -> Result<String> {
        util::resolve_identity_text(self, label, raw, kind).map(|resolved| resolved.openid)
    }

    /// deferred 计划的收件人 → 上游 OpenID（群聊取 `group_openid`，私聊取 `user_openid`）。
    ///
    /// # Errors
    /// 同 [`ActionContext::resolve_openid`]。
    pub fn resolve_send_target(&self, target: &SendTarget) -> Result<String> {
        match target {
            SendTarget::Group(id) => self.resolve_openid("group_id", id, "group_openid"),
            SendTarget::Private(id) => self.resolve_openid("user_id", id, "user_openid"),
        }
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

            fn send_group_message(
                &self,
                _group_openid: String,
                _msg: gensokyo_qq::dto::MessageToCreate,
            ) -> super::QqFuture<'_, gensokyo_qq::dto::MessageResponse> {
                Box::pin(async { Err(super::Error::Unsupported("未配置".to_owned())) })
            }

            fn send_c2c_message(
                &self,
                _user_openid: String,
                _msg: gensokyo_qq::dto::MessageToCreate,
            ) -> super::QqFuture<'_, gensokyo_qq::dto::MessageResponse> {
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
