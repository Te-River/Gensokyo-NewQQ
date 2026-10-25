//! 群查询 / 群管理 action（M1k）：把 OneBot 参数翻译成 [`QqApi`] 调用。
//!
//! # 覆盖范围
//!
//! | action | Go 对应 | 上游端点 |
//! |--------|---------|----------|
//! | `get_group_info` | `handlers/get_group_info.go` | `GET /v2/groups/{gid}/info` |
//! | `get_group_member_info` | `handlers/get_group_member_info.go` | `GET /v2/groups/{gid}/members/{mid}` |
//! | `get_group_member_list` | `handlers/get_group_member_list.go` | `GET /v2/groups/{gid}/members` |
//! | `set_group_ban` | `handlers/set_group_ban.go` | `POST /v2/groups/{gid}/restrict_chat_setting` |
//! | `get_group_bot_state` | `handlers/get_group_bot_state.go` | `GET /v2/groups/{gid}/bot_state` |
//! | `get_group_join_request_list` | `handlers/get_group_join_request_list.go` | `GET /v2/groups/{gid}/join_request_list` |
//!
//! `set_group_whole_ban` / `get_group_whole_ban`、`set_group_card`、`set_group_kick`、
//! `set_group_leave`、`get_group_list` 在 [`crate::group_admin`]。
//!
//! 成功响应是 `status=ok` / `retcode=0` + `data`；参数缺失或非法回 `retcode=1400`
//! （[`RETCODE_BAD_REQUEST`]），上游失败回 `retcode=1500`（[`RETCODE_UPSTREAM`]）。`echo` 由
//! handler 原样带在响应上（runtime 统一回填的语义不变：同一值，重复设置无副作用）。
//!
//! # ID 口径
//!
//! ID 归一口径集中在 `util`：参数接受数字或字符串两种形态（`params` 归一），
//! 32 位原生 OpenID 直接当上游 ID 用，其余形态视为虚拟号并经
//! [`gensokyo_storage::IdMapStore`] 反查——与 Go 的 `len(id) == 32` 分支完全一致。未装配
//! idmap 时，虚拟号请求一律 1400，不做静默降级。
//!
//! 响应里的 `group_id` / `user_id` 是**虚拟号**（回填请求值），形态由配置 `string_ob11`
//! 决定（[`gensokyo_protocol::parse_id`] 的数字/字符串双形态）。本 crate 无 idmap 时列表条目的
//! `user_id` 回退 `"0"`，真实 OpenID 始终可从 `member_openid` 附加字段读到。
//!
//! # 刻意的差异（相对 Go）
//!
//! - **`set_group_ban` 的 `duration` 必填**：Go 的 `ParamsContent.Duration` 缺省为 0，等于静默解禁；
//!   这里显式拒绝缺参，避免漏字段把成员放出来。
//! - **`get_group_member_list` 只取单页**：[`QqApi::get_group_member_list`] 没有 cursor 入参，
//!   Go 的「最多 100 页全量拉取 + 页间节流」尚未移植，`next_cursor` 也暂不外露。
//! - **idmap 缺失时的中性值**：Go 侧 `StoreIDv2` 失败直接跳过成员；这里保留条目并置 `user_id=0`，
//!   由调用方按 `member_openid` 决策。
//! - **`joined_at` 解析**：Go 用 `time.Parse(RFC3339)`；这里用 `rfc3339`（无 `time` 依赖），
//!   解析失败同样置 0。

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use gensokyo_core::{Error, Ob11Id, Result};
use gensokyo_protocol::action::{ActionRequest, ActionResponse, GroupInfo, GroupMemberInfo};
use gensokyo_protocol::{Extra, Handler, HandlerFuture};
use gensokyo_qq::dto::{
    BotInGroupState as QqBotState, GroupMember as QqGroupMember, JoinRequest as QqJoinRequest,
    ReviewQa,
};

use crate::params;
use crate::qqapi::ActionContext;
use crate::rfc3339;
pub use crate::util::RETCODE_UPSTREAM;
use crate::util::{
    bad_request, ob11_id, ok_data, page_limit, params_object, resolve_group, resolve_member,
    upstream_error, virtual_id_text,
};

/// `get_group_member_list` 单页默认条数（对齐 Go 的 `GroupMemberList(..., 30)`）。
pub const DEFAULT_MEMBER_PAGE: u32 = 30;

/// `get_group_member_list` 单页上限：客户端要更多也截断，防止一次拉爆官方 QPM。
pub const MAX_MEMBER_PAGE: u32 = 100;

/// 入群申请列表默认条数（官方默认 20）。
pub const DEFAULT_JOIN_REQUEST_LIMIT: u32 = 20;

/// 入群申请列表单页上限（官方上限 50）。
pub const MAX_JOIN_REQUEST_LIMIT: u32 = 50;

/// `get_group_bot_state` 的响应数据（Gensokyo 扩展 action，字段与 Go `GroupBotState` 对齐）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotStateData {
    /// 群虚拟号。
    pub group_id: Ob11Id,
    /// 入群时间（Unix 秒；`joined_at` 解析失败为 0）。
    pub join_time: i64,
    /// 是否允许主动推送（`allow_proactive_msg` 的旧字段别名）。
    pub can_push: bool,
    /// 接收消息设置（`recv_msg_setting` 的旧字段别名）。
    pub push_msg_setting: String,
    /// 机器人群内角色（`member_role` 的旧字段别名）。
    pub role: String,
    /// 入群时间（官方 RFC3339 原文）。
    pub joined_at: String,
    /// 是否允许主动推送。
    pub allow_proactive_msg: bool,
    /// 接收消息设置：`all` / `only_mention` / `mention_and_context`。
    pub recv_msg_setting: String,
    /// 机器人群内角色：`member` / `owner` / `admin`。
    pub member_role: String,
    /// 机器人自己的 member_openid。
    pub member_openid: String,
}

/// 入群申请条目（字段与 Go `JoinRequestItem` 对齐，可直接回传审批类 action）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JoinRequestEntry {
    /// 群虚拟号。
    pub group_id: Ob11Id,
    /// 申请人虚拟号（无 idmap 时为 `"0"`）。
    pub user_id: Ob11Id,
    /// 审批时回传的申请 ID。
    pub flag: String,
    /// 申请人昵称。
    pub username: String,
    /// 申请时间（官方 RFC3339 原文）。
    pub apply_at: String,
    /// 申请来源：`self_apply` / `invited`。
    pub apply_source: String,
    /// 邀请人 OpenID。
    pub invited_by: String,
    /// 申请人 union_openid。
    pub union_openid: String,
    /// 是否机器人。
    pub bot: bool,
    /// 安全提示。
    pub risk_tips: String,
    /// 验证消息文本。
    pub verify_info: String,
    /// 验证方式。
    pub verify_method: String,
    /// 验证问答。
    pub review_qa_list: Vec<ReviewQa>,
    /// 是否命中自动审批。
    pub auto_approved: bool,
}

/// `get_group_join_request_list` 的响应数据。
///
/// `join_requests` 是 `list` 的兼容别名（内容相同），`next_index` 是 `next_cursor` 的旧字段
/// 别名——只有 `next_cursor` 能落进 `i32` 时才同步，否则置 0（对齐 Go 的迁移提示）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JoinRequestListData {
    /// 申请列表。
    pub list: Vec<JoinRequestEntry>,
    /// `list` 的兼容别名。
    pub join_requests: Vec<JoinRequestEntry>,
    /// 下一页游标；空串表示末页。
    pub next_cursor: String,
    /// 旧字段别名。
    pub next_index: i64,
}

/// `get_group_info`。
#[derive(Debug, Clone)]
pub struct GetGroupInfoHandler {
    context: Arc<ActionContext>,
}

impl GetGroupInfoHandler {
    /// 构造。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = match params_object(request) {
            Ok(object) => object,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let group = match resolve_group(&self.context, object) {
            Ok(group) => group,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let info = match self.context.api().get_group_info(&group.openid).await {
            Ok(info) => info,
            Err(err) => return Ok(upstream_error(request, &err)),
        };

        let mut extra = Extra::new();
        extra.insert("group_memo".to_owned(), json!(info.group_finger_memo));
        extra.insert("group_class_text".to_owned(), json!(info.group_class_text));
        extra.insert("group_tags".to_owned(), json!(info.group_tags));
        let data = GroupInfo {
            group_id: ob11_id(&group.requested, self.context.string_ob11()),
            group_name: info.group_name,
            member_count: i64::try_from(info.group_member_num).unwrap_or(i64::MAX),
            // 官方不给群成员上限：保持 OneBot 字段形状，填 0 而不是编造默认值
            max_member_count: 0,
            extra,
        };
        ok_data(request, &data)
    }
}

impl Handler for GetGroupInfoHandler {
    fn name(&self) -> &'static str {
        "get_group_info"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `get_group_member_info`。
#[derive(Debug, Clone)]
pub struct GetGroupMemberInfoHandler {
    context: Arc<ActionContext>,
}

impl GetGroupMemberInfoHandler {
    /// 构造。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = match params_object(request) {
            Ok(object) => object,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let group = match resolve_group(&self.context, object) {
            Ok(group) => group,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let member = match resolve_member(&self.context, object) {
            Ok(member) => member,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let profile = match self
            .context
            .api()
            .get_group_member_info(&group.openid, &member.openid)
            .await
        {
            Ok(profile) => profile,
            Err(err) => return Ok(upstream_error(request, &err)),
        };

        let string_mode = self.context.string_ob11();
        let data = member_info(
            &ob11_id(&group.requested, string_mode),
            &ob11_id(&member.requested, string_mode),
            &profile,
        );
        ok_data(request, &data)
    }
}

impl Handler for GetGroupMemberInfoHandler {
    fn name(&self) -> &'static str {
        "get_group_member_info"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `get_group_member_list`。
#[derive(Debug, Clone)]
pub struct GetGroupMemberListHandler {
    context: Arc<ActionContext>,
}

impl GetGroupMemberListHandler {
    /// 构造。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = match params_object(request) {
            Ok(object) => object,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let group = match resolve_group(&self.context, object) {
            Ok(group) => group,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let limit = match page_limit(object, DEFAULT_MEMBER_PAGE, MAX_MEMBER_PAGE) {
            Ok(limit) => limit,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let page = match self
            .context
            .api()
            .get_group_member_list(&group.openid, limit)
            .await
        {
            Ok(page) => page,
            Err(err) => return Ok(upstream_error(request, &err)),
        };

        let string_mode = self.context.string_ob11();
        let group_id = ob11_id(&group.requested, string_mode);
        let members: Vec<GroupMemberInfo> = page
            .members
            .iter()
            .map(|member| {
                let user_id = virtual_id_text(&self.context, &member.member_openid);
                member_info(&group_id, &ob11_id(&user_id, string_mode), member)
            })
            .collect();
        ok_data(request, &members)
    }
}

impl Handler for GetGroupMemberListHandler {
    fn name(&self) -> &'static str {
        "get_group_member_list"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `set_group_ban`（别名 `get_group_ban` 由 runtime 的别名表指向本 handler）。
#[derive(Debug, Clone)]
pub struct SetGroupBanHandler {
    context: Arc<ActionContext>,
}

impl SetGroupBanHandler {
    /// 构造。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = match params_object(request) {
            Ok(object) => object,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let group = match resolve_group(&self.context, object) {
            Ok(group) => group,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let member = match resolve_member(&self.context, object) {
            Ok(member) => member,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let duration = match params::optional_i64(object, "duration") {
            Ok(Some(duration)) => duration,
            Ok(None) => {
                return Ok(bad_request(
                    request,
                    &Error::Protocol("缺少 duration".to_owned()),
                ));
            }
            Err(err) => return Ok(bad_request(request, &err)),
        };
        if let Err(err) = self
            .context
            .api()
            .set_group_ban(&group.openid, &member.openid, duration)
            .await
        {
            return Ok(upstream_error(request, &err));
        }

        // OneBot 的 set_group_ban 成功回执没有业务数据
        Ok(ActionResponse::ok(Value::Null).with_echo(request.echo.clone()))
    }
}

impl Handler for SetGroupBanHandler {
    fn name(&self) -> &'static str {
        "set_group_ban"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `get_group_bot_state`。
#[derive(Debug, Clone)]
pub struct GetGroupBotStateHandler {
    context: Arc<ActionContext>,
}

impl GetGroupBotStateHandler {
    /// 构造。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = match params_object(request) {
            Ok(object) => object,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let group = match resolve_group(&self.context, object) {
            Ok(group) => group,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let state = match self.context.api().get_group_bot_state(&group.openid).await {
            Ok(state) => state,
            Err(err) => return Ok(upstream_error(request, &err)),
        };

        let data = bot_state(
            &ob11_id(&group.requested, self.context.string_ob11()),
            &state,
        );
        ok_data(request, &data)
    }
}

impl Handler for GetGroupBotStateHandler {
    fn name(&self) -> &'static str {
        "get_group_bot_state"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `get_group_join_request_list`。
#[derive(Debug, Clone)]
pub struct GetGroupJoinRequestListHandler {
    context: Arc<ActionContext>,
}

impl GetGroupJoinRequestListHandler {
    /// 构造。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = match params_object(request) {
            Ok(object) => object,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let group = match resolve_group(&self.context, object) {
            Ok(group) => group,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let cursor = match params::optional_text(object, "cursor") {
            Ok(cursor) => cursor,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let limit = match page_limit(object, DEFAULT_JOIN_REQUEST_LIMIT, MAX_JOIN_REQUEST_LIMIT) {
            Ok(limit) => limit,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let page = match self
            .context
            .api()
            .get_group_join_request_list(&group.openid, cursor.as_deref(), Some(limit))
            .await
        {
            Ok(page) => page,
            Err(err) => return Ok(upstream_error(request, &err)),
        };

        let string_mode = self.context.string_ob11();
        let group_id = ob11_id(&group.requested, string_mode);
        let entries: Vec<JoinRequestEntry> = page
            .list
            .iter()
            .map(|item| {
                let user_id = virtual_id_text(&self.context, &item.member_openid);
                join_request_entry(&group_id, &ob11_id(&user_id, string_mode), item)
            })
            .collect();
        let data = JoinRequestListData {
            next_index: next_index(&page.next_cursor),
            join_requests: entries.clone(),
            list: entries,
            next_cursor: page.next_cursor,
        };
        ok_data(request, &data)
    }
}

impl Handler for GetGroupJoinRequestListHandler {
    fn name(&self) -> &'static str {
        "get_group_join_request_list"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// 原来的成员 DTO → OneBot 成员信息。
fn member_info(group_id: &Ob11Id, user_id: &Ob11Id, member: &QqGroupMember) -> GroupMemberInfo {
    let mut extra = Extra::new();
    extra.insert("member_openid".to_owned(), json!(member.member_openid));
    extra.insert("union_openid".to_owned(), json!(member.union_openid));
    extra.insert("bot".to_owned(), json!(member.bot));
    extra.insert("joined_at".to_owned(), json!(member.joined_at));
    GroupMemberInfo {
        group_id: group_id.clone(),
        user_id: user_id.clone(),
        nickname: member.username.clone(),
        // 官方无独立群名片：与 Go 一致用昵称填充
        card: member.username.clone(),
        // 官方无性别字段：与 Go 一致给中性值
        sex: Some("unknown".to_owned()),
        age: None,
        area: None,
        join_time: rfc3339::parse(&member.joined_at),
        last_sent_time: None,
        level: None,
        role: Some(normalize_role(&member.member_role)),
        unfriendly: None,
        title: None,
        title_expire_time: None,
        card_changeable: None,
        extra,
    }
}

/// 法庭外的角色归一：官方 `member_role` 与 OneBot `role` 枚举一致，未知值保底 `member`。
fn normalize_role(role: &str) -> String {
    match role {
        "owner" | "admin" | "member" => role.to_owned(),
        _ => "member".to_owned(),
    }
}

/// 机器人群内状态 → 响应数据（新旧字段双写，对齐 Go `GroupBotState`）。
fn bot_state(group_id: &Ob11Id, state: &QqBotState) -> BotStateData {
    BotStateData {
        group_id: group_id.clone(),
        join_time: rfc3339::parse(&state.joined_at).unwrap_or(0),
        can_push: state.allow_proactive_msg,
        push_msg_setting: state.recv_msg_setting.clone(),
        role: state.member_role.clone(),
        joined_at: state.joined_at.clone(),
        allow_proactive_msg: state.allow_proactive_msg,
        recv_msg_setting: state.recv_msg_setting.clone(),
        member_role: state.member_role.clone(),
        member_openid: state.member_openid.clone(),
    }
}

/// 入群申请 → OneBot 条目。
fn join_request_entry(
    group_id: &Ob11Id,
    user_id: &Ob11Id,
    item: &QqJoinRequest,
) -> JoinRequestEntry {
    JoinRequestEntry {
        group_id: group_id.clone(),
        user_id: user_id.clone(),
        flag: item.join_request_id.clone(),
        username: item.username.clone(),
        apply_at: item.apply_at.clone(),
        apply_source: item.apply_source.clone(),
        invited_by: item.invited_by.clone(),
        union_openid: item.union_openid.clone(),
        bot: item.bot,
        risk_tips: item.risk_tips.clone(),
        verify_info: item.verify_info.message.clone(),
        verify_method: item.verify_info.method.clone(),
        review_qa_list: item.verify_info.review_qa_list.clone(),
        auto_approved: item
            .auto_approved
            .as_ref()
            .is_some_and(|approved| approved.approved),
    }
}

/// `next_cursor` → 旧字段 `next_index`：只有能落进 `i32` 的十进制游标才同步，否则 0。
fn next_index(next_cursor: &str) -> i64 {
    next_cursor
        .parse::<i64>()
        .ok()
        .filter(|value| *value <= i64::from(i32::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::sync::{Arc, Mutex, OnceLock};

    use serde_json::{Value, json};

    use super::{
        BotStateData, DEFAULT_JOIN_REQUEST_LIMIT, GetGroupBotStateHandler, GetGroupInfoHandler,
        GetGroupJoinRequestListHandler, GetGroupMemberInfoHandler, GetGroupMemberListHandler,
        JoinRequestListData, MAX_JOIN_REQUEST_LIMIT, RETCODE_UPSTREAM, SetGroupBanHandler,
    };
    use crate::handler::RETCODE_BAD_REQUEST;
    use crate::qqapi::{ActionContext, QqApi, QqFuture};
    use gensokyo_config::ConfigManager;
    use gensokyo_core::{Error, Result};
    use gensokyo_protocol::Handler;
    use gensokyo_protocol::Status;
    use gensokyo_protocol::action::{ActionRequest, ActionResponse};
    use gensokyo_qq::ApiError;
    use gensokyo_qq::dto::{
        AutoApprovedInfo, BotInGroupState, GroupInfo as QqGroupInfo, GroupMember as QqGroupMember,
        GroupMemberList as QqGroupMemberList, JoinRequest as QqJoinRequest,
        JoinRequestList as QqJoinRequestList, ReviewQa, VerifyInfo,
    };

    const GROUP: &str = "0123456789abcdef0123456789abcdef";
    const MEMBER: &str = "fedcba9876543210fedcba9876543210";

    /// 记录一次的入参，断言 handler 把参数翻成了什么。
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        GroupInfo(String),
        MemberInfo(String, String),
        MemberList(String, u32),
        SetBan(String, String, i64),
        BotState(String),
        JoinRequestList(String, Option<String>, Option<u32>),
    }

    #[derive(Debug, Default)]
    struct FakeQqApi {
        calls: Mutex<Vec<Call>>,
        group_info: Mutex<Option<Result<QqGroupInfo, ApiError>>>,
        member_info: Mutex<Option<Result<QqGroupMember, ApiError>>>,
        member_list: Mutex<Option<Result<QqGroupMemberList, ApiError>>>,
        ban: Mutex<Option<ApiError>>,
        bot_state: Mutex<Option<Result<BotInGroupState, ApiError>>>,
        join_requests: Mutex<Option<Result<QqJoinRequestList, ApiError>>>,
    }

    impl FakeQqApi {
        fn record(&self, call: Call) {
            self.calls.lock().expect("调用记录锁未中毒").push(call);
        }

        fn calls(&self) -> Vec<Call> {
            self.calls.lock().expect("调用记录锁未中毒").clone()
        }
    }

    fn take<T>(slot: &Mutex<Option<Result<T, ApiError>>>) -> Result<T> {
        slot.lock()
            .expect("响应槽锁未中毒")
            .take()
            .expect("测试未配置该方法的响应")
            .map_err(|err| Error::Upstream {
                code: err.code,
                message: err.message,
            })
    }

    impl QqApi for FakeQqApi {
        fn get_group_info(&self, group_openid: &str) -> QqFuture<'_, QqGroupInfo> {
            // 先把借用转成所有权：async 块只捕获 `&self`，返回的 future 才只依赖 self 的生命周期
            let group_openid = group_openid.to_owned();
            Box::pin(async move {
                self.record(Call::GroupInfo(group_openid));
                take(&self.group_info)
            })
        }

        fn get_group_member_info(
            &self,
            group_openid: &str,
            member_openid: &str,
        ) -> QqFuture<'_, QqGroupMember> {
            let group_openid = group_openid.to_owned();
            let member_openid = member_openid.to_owned();
            Box::pin(async move {
                self.record(Call::MemberInfo(group_openid, member_openid));
                take(&self.member_info)
            })
        }

        fn get_group_member_list(
            &self,
            group_openid: &str,
            limit: u32,
        ) -> QqFuture<'_, QqGroupMemberList> {
            let group_openid = group_openid.to_owned();
            Box::pin(async move {
                self.record(Call::MemberList(group_openid, limit));
                take(&self.member_list)
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
                self.record(Call::SetBan(group_openid, member_openid, duration_secs));
                let error = self.ban.lock().expect("禁言槽锁未中毒").take();
                match error {
                    Some(error) => Err(Error::Upstream {
                        code: error.code,
                        message: error.message,
                    }),
                    None => Ok(()),
                }
            })
        }

        fn get_group_bot_state(&self, group_openid: &str) -> QqFuture<'_, BotInGroupState> {
            let group_openid = group_openid.to_owned();
            Box::pin(async move {
                self.record(Call::BotState(group_openid));
                take(&self.bot_state)
            })
        }

        fn get_group_join_request_list(
            &self,
            group_openid: &str,
            cursor: Option<&str>,
            limit: Option<u32>,
        ) -> QqFuture<'_, QqJoinRequestList> {
            let group_openid = group_openid.to_owned();
            let cursor = cursor.map(str::to_owned);
            Box::pin(async move {
                self.record(Call::JoinRequestList(group_openid, cursor, limit));
                take(&self.join_requests)
            })
        }

        // 本组 action 不发送消息：显式报「未配置」，避免依赖默认实现掩盖接线遗漏。
        fn send_group_message(
            &self,
            _group_openid: String,
            _msg: gensokyo_qq::dto::MessageToCreate,
        ) -> QqFuture<'_, gensokyo_qq::dto::MessageResponse> {
            Box::pin(async { Err(Error::Unsupported("未配置 send_group_message".to_owned())) })
        }

        fn send_c2c_message(
            &self,
            _user_openid: String,
            _msg: gensokyo_qq::dto::MessageToCreate,
        ) -> QqFuture<'_, gensokyo_qq::dto::MessageResponse> {
            Box::pin(async { Err(Error::Unsupported("未配置 send_c2c_message".to_owned())) })
        }
    }

    /// 单测里的最小 `block_on`：handler 只 await 一次 FakeQqApi，一次 poll 即 `Ready`。
    fn block_on<F: Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        loop {
            match future.as_mut().poll(&mut context) {
                std::task::Poll::Ready(output) => return output,
                std::task::Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    /// 一份只含默认值的配置快照：临时目录写一个最小 `config.yml` 后立即删除。
    fn config() -> Arc<ConfigManager> {
        static MANAGER: OnceLock<Arc<ConfigManager>> = OnceLock::new();
        MANAGER
            .get_or_init(|| {
                let path = std::env::temp_dir()
                    .join(format!("gensokyo-actions-test-{}.yml", std::process::id()));
                std::fs::write(&path, "{}\n").expect("写临时 config.yml 失败");
                let manager =
                    Arc::new(ConfigManager::new(&path).expect("加载临时 config.yml 失败"));
                let _ = std::fs::remove_file(&path);
                manager
            })
            .clone()
    }

    fn context() -> (Arc<ActionContext>, Arc<FakeQqApi>) {
        let api = Arc::new(FakeQqApi::default());
        let context = Arc::new(ActionContext::new(api.clone(), config()));
        (context, api)
    }

    fn request(action: &str, params: Value) -> ActionRequest<Value> {
        serde_json::from_value(json!({"action": action, "params": params, "echo": "e-1"}))
            .expect("请求应可反序列化")
    }

    fn ok_data(response: ActionResponse<Value>) -> Value {
        assert_eq!(response.status, Status::Ok);
        assert_eq!(response.retcode, 0);
        assert_eq!(response.echo, Some(json!("e-1")));
        response.data.expect("成功响应应带 data")
    }

    fn failed(response: ActionResponse<Value>) -> (Status, i32, Option<Value>) {
        assert_eq!(response.echo, Some(json!("e-1")));
        (response.status, response.retcode, response.data)
    }

    #[test]
    fn get_group_info_maps_upstream_fields() {
        let (context, api) = context();
        *api.group_info.lock().unwrap() = Some(Ok(QqGroupInfo {
            group_openid: GROUP.to_owned(),
            group_name: "测试群".to_owned(),
            group_finger_memo: "简介".to_owned(),
            group_class_text: "游戏".to_owned(),
            group_tags: vec!["a".to_owned()],
            group_member_num: 7,
        }));

        let handler = GetGroupInfoHandler::new(context);
        let response =
            block_on(handler.handle(&request("get_group_info", json!({"group_id": GROUP}))));
        let data = ok_data(response.expect("handler 不应返回 Err"));

        assert_eq!(api.calls(), vec![Call::GroupInfo(GROUP.to_owned())]);
        assert_eq!(data["group_id"], json!(GROUP));
        assert_eq!(data["group_name"], json!("测试群"));
        assert_eq!(data["member_count"], json!(7));
        assert_eq!(data["max_member_count"], json!(0));
        assert_eq!(data["group_memo"], json!("简介"));
        assert_eq!(handler.name(), "get_group_info");
    }

    #[test]
    fn get_group_member_info_maps_joined_at_and_role() {
        let (context, api) = context();
        *api.member_info.lock().unwrap() = Some(Ok(QqGroupMember {
            member_openid: MEMBER.to_owned(),
            username: "小明".to_owned(),
            member_role: "owner".to_owned(),
            bot: false,
            joined_at: "2026-08-05T14:19:09+08:00".to_owned(),
            union_openid: "u-1".to_owned(),
        }));

        let handler = GetGroupMemberInfoHandler::new(context);
        let response = block_on(handler.handle(&request(
            "get_group_member_info",
            json!({"group_id": GROUP, "user_id": MEMBER}),
        )));
        let data = ok_data(response.expect("handler 不应返回 Err"));

        assert_eq!(
            api.calls(),
            vec![Call::MemberInfo(GROUP.to_owned(), MEMBER.to_owned())]
        );
        assert_eq!(data["group_id"], json!(GROUP));
        assert_eq!(data["user_id"], json!(MEMBER));
        assert_eq!(data["nickname"], json!("小明"));
        assert_eq!(data["card"], json!("小明"));
        assert_eq!(data["role"], json!("owner"));
        assert_eq!(data["sex"], json!("unknown"));
        assert_eq!(data["join_time"], json!(1_785_910_749_i64));
        assert_eq!(data["member_openid"], json!(MEMBER));
        assert_eq!(handler.name(), "get_group_member_info");
    }

    #[test]
    fn get_group_member_info_accepts_explicit_member_openid() {
        let (context, api) = context();
        api.member_info.lock().unwrap().replace(Ok(QqGroupMember {
            member_openid: MEMBER.to_owned(),
            ..QqGroupMember::default()
        }));

        let handler = GetGroupMemberInfoHandler::new(context);
        let response = block_on(handler.handle(&request(
            "get_group_member_info",
            json!({"group_id": GROUP, "member_openid": MEMBER}),
        )));
        let data = ok_data(response.expect("handler 不应返回 Err"));

        assert_eq!(
            api.calls(),
            vec![Call::MemberInfo(GROUP.to_owned(), MEMBER.to_owned())]
        );
        // 没有 idmap 时虚拟号回退 "0"，真实 OpenID 仍在附加字段里
        assert_eq!(data["user_id"], json!(0));
        assert_eq!(data["member_openid"], json!(MEMBER));
    }

    #[test]
    fn get_group_member_list_passes_limit_and_returns_array() {
        let (context, api) = context();
        *api.member_list.lock().unwrap() = Some(Ok(QqGroupMemberList {
            members: vec![
                QqGroupMember {
                    member_openid: MEMBER.to_owned(),
                    username: "甲".to_owned(),
                    member_role: "member".to_owned(),
                    ..QqGroupMember::default()
                },
                QqGroupMember {
                    member_openid: "1".repeat(32),
                    username: "乙".to_owned(),
                    member_role: "admin".to_owned(),
                    ..QqGroupMember::default()
                },
            ],
            next_cursor: "nc".to_owned(),
        }));

        let handler = GetGroupMemberListHandler::new(context);
        let response = block_on(handler.handle(&request(
            "get_group_member_list",
            json!({"group_id": GROUP, "limit": 7}),
        )));
        let data = ok_data(response.expect("handler 不应返回 Err"));

        assert_eq!(api.calls(), vec![Call::MemberList(GROUP.to_owned(), 7)]);
        let members = data
            .as_array()
            .expect("get_group_member_list 的 data 是数组");
        assert_eq!(members.len(), 2);
        assert_eq!(members[0]["user_id"], json!(0));
        assert_eq!(members[0]["member_openid"], json!(MEMBER));
        assert_eq!(members[1]["role"], json!("admin"));
        assert_eq!(handler.name(), "get_group_member_list");
    }

    #[test]
    fn get_group_member_list_defaults_and_clamps_limit() {
        for (params, want) in [
            (json!({"group_id": GROUP}), 30),
            (json!({"group_id": GROUP, "limit": 0}), 30),
            (json!({"group_id": GROUP, "limit": "500"}), 100),
        ] {
            let (context, api) = context();
            *api.member_list.lock().unwrap() = Some(Ok(QqGroupMemberList::default()));
            let handler = GetGroupMemberListHandler::new(context);
            block_on(handler.handle(&request("get_group_member_list", params)))
                .expect("handler 不应返回 Err");
            assert_eq!(api.calls(), vec![Call::MemberList(GROUP.to_owned(), want)]);
        }
    }

    #[test]
    fn set_group_ban_forwards_duration_and_returns_null_data() {
        let (context, api) = context();
        let handler = SetGroupBanHandler::new(context);
        let response = block_on(handler.handle(&request(
            "set_group_ban",
            json!({"group_id": GROUP, "user_id": MEMBER, "duration": "600"}),
        )));
        let response = response.expect("handler 不应返回 Err");

        assert_eq!(response.status, Status::Ok);
        assert_eq!(response.retcode, 0);
        assert_eq!(response.data, Some(Value::Null));
        assert_eq!(
            api.calls(),
            vec![Call::SetBan(GROUP.to_owned(), MEMBER.to_owned(), 600)]
        );
        assert_eq!(handler.name(), "set_group_ban");
    }

    #[test]
    fn set_group_ban_requires_duration() {
        let (context, api) = context();
        let handler = SetGroupBanHandler::new(context);
        let response = block_on(handler.handle(&request(
            "set_group_ban",
            json!({"group_id": GROUP, "user_id": MEMBER}),
        )));
        let response = response.expect("参数错误也应是 Ok(失败响应)");

        assert_eq!(
            failed(response),
            (Status::Failed, RETCODE_BAD_REQUEST, None)
        );
        assert!(api.calls().is_empty(), "缺参不应触达上游");
    }

    #[test]
    fn get_group_bot_state_exposes_both_field_sets() {
        let (context, api) = context();
        *api.bot_state.lock().unwrap() = Some(Ok(BotInGroupState {
            member_openid: "bot-openid".to_owned(),
            joined_at: "2026-08-05T14:19:09+08:00".to_owned(),
            allow_proactive_msg: true,
            recv_msg_setting: "only_mention".to_owned(),
            member_role: "admin".to_owned(),
        }));

        let handler = GetGroupBotStateHandler::new(context);
        let response =
            block_on(handler.handle(&request("get_group_bot_state", json!({"group_id": GROUP}))));
        let data = ok_data(response.expect("handler 不应返回 Err"));

        assert_eq!(api.calls(), vec![Call::BotState(GROUP.to_owned())]);
        let state: BotStateData =
            serde_json::from_value(data.clone()).expect("响应应能还原成 BotStateData");
        assert_eq!(state.group_id, gensokyo_core::Ob11Id::str(GROUP));
        assert_eq!(state.join_time, 1_785_910_749);
        assert_eq!(state.can_push, state.allow_proactive_msg);
        assert_eq!(state.push_msg_setting, state.recv_msg_setting);
        assert_eq!(state.role, state.member_role);
        assert_eq!(state.member_openid, "bot-openid");
        assert_eq!(data.as_object().map(|object| object.len()), Some(10));
        assert_eq!(handler.name(), "get_group_bot_state");
    }

    #[test]
    fn get_group_join_request_list_maps_cursor_limit_and_items() {
        let (context, api) = context();
        *api.join_requests.lock().unwrap() = Some(Ok(QqJoinRequestList {
            list: vec![QqJoinRequest {
                join_request_id: "flag-1".to_owned(),
                risk_tips: "风险".to_owned(),
                union_openid: "union-1".to_owned(),
                member_openid: MEMBER.to_owned(),
                username: "申请者".to_owned(),
                apply_at: "2026-08-05T14:19:09+08:00".to_owned(),
                apply_source: "self_apply".to_owned(),
                invited_by: String::new(),
                bot: false,
                verify_info: VerifyInfo {
                    method: "admin_review_qa".to_owned(),
                    message: "问题：答案".to_owned(),
                    review_qa_list: vec![ReviewQa {
                        question: "q".to_owned(),
                        answer: "a".to_owned(),
                    }],
                },
                auto_approved: Some(AutoApprovedInfo {
                    strategy_id: "s-1".to_owned(),
                    approved: true,
                }),
            }],
            next_cursor: "42".to_owned(),
        }));

        let handler = GetGroupJoinRequestListHandler::new(context);
        let response = block_on(handler.handle(&request(
            "get_group_join_request_list",
            json!({"group_id": GROUP, "cursor": "c1", "limit": 200}),
        )));
        let data = ok_data(response.expect("handler 不应返回 Err"));

        assert_eq!(
            api.calls(),
            vec![Call::JoinRequestList(
                GROUP.to_owned(),
                Some("c1".to_owned()),
                Some(MAX_JOIN_REQUEST_LIMIT)
            )]
        );
        let list: JoinRequestListData =
            serde_json::from_value(data).expect("响应应能还原成 JoinRequestListData");
        assert_eq!(list.list, list.join_requests);
        assert_eq!(list.next_cursor, "42");
        assert_eq!(list.next_index, 42);
        let item = &list.list[0];
        assert_eq!(item.flag, "flag-1");
        assert_eq!(item.username, "申请者");
        assert_eq!(item.verify_info, "问题：答案");
        assert_eq!(item.verify_method, "admin_review_qa");
        assert_eq!(item.review_qa_list.len(), 1);
        assert!(item.auto_approved);
        assert_eq!(handler.name(), "get_group_join_request_list");
    }

    #[test]
    fn get_group_join_request_list_defaults_limit_and_resets_wide_next_index() {
        let (context, api) = context();
        *api.join_requests.lock().unwrap() = Some(Ok(QqJoinRequestList {
            list: Vec::new(),
            next_cursor: "1234567890123".to_owned(),
        }));
        let handler = GetGroupJoinRequestListHandler::new(context);
        let response = block_on(handler.handle(&request(
            "get_group_join_request_list",
            json!({"group_id": GROUP}),
        )));
        let data = ok_data(response.expect("handler 不应返回 Err"));

        assert_eq!(
            api.calls(),
            vec![Call::JoinRequestList(
                GROUP.to_owned(),
                None,
                Some(DEFAULT_JOIN_REQUEST_LIMIT)
            )]
        );
        assert_eq!(data["next_cursor"], json!("1234567890123"));
        assert_eq!(
            data["next_index"],
            json!(0),
            "超出 i32 的游标不同步到旧字段"
        );
        assert_eq!(data["list"], json!([]));
    }

    #[test]
    fn upstream_errors_map_to_1500() {
        for (handler, params, slot) in [
            (
                "get_group_info" as &str,
                json!({"group_id": GROUP}),
                "group_info",
            ),
            (
                "get_group_member_info",
                json!({"group_id": GROUP, "user_id": MEMBER}),
                "member_info",
            ),
            (
                "get_group_member_list",
                json!({"group_id": GROUP}),
                "member_list",
            ),
            (
                "get_group_bot_state",
                json!({"group_id": GROUP}),
                "bot_state",
            ),
            (
                "get_group_join_request_list",
                json!({"group_id": GROUP}),
                "join_requests",
            ),
        ] {
            let (context, api) = context();
            let error = ApiError {
                http_status: 500,
                code: 30500,
                message: "upstream boom".to_owned(),
                trace_id: None,
            };
            match slot {
                "group_info" => *api.group_info.lock().unwrap() = Some(Err(error)),
                "member_info" => *api.member_info.lock().unwrap() = Some(Err(error)),
                "member_list" => *api.member_list.lock().unwrap() = Some(Err(error)),
                "bot_state" => *api.bot_state.lock().unwrap() = Some(Err(error)),
                _ => *api.join_requests.lock().unwrap() = Some(Err(error)),
            }

            let handler: Arc<dyn Handler> = match handler {
                "get_group_info" => Arc::new(GetGroupInfoHandler::new(context)),
                "get_group_member_info" => Arc::new(GetGroupMemberInfoHandler::new(context)),
                "get_group_member_list" => Arc::new(GetGroupMemberListHandler::new(context)),
                "get_group_bot_state" => Arc::new(GetGroupBotStateHandler::new(context)),
                _ => Arc::new(GetGroupJoinRequestListHandler::new(context)),
            };
            let request = request(handler.name(), params);
            let response = block_on(handler.handle(&request)).expect("上游错误也应是 Ok(失败响应)");
            let (status, retcode, data) = failed(response);
            assert_eq!(status, Status::Failed);
            assert_eq!(retcode, RETCODE_UPSTREAM, "{}", handler.name());
            assert_eq!(data, None);
        }
    }

    #[test]
    fn set_group_ban_upstream_error_maps_to_1500() {
        let (context, api) = context();
        *api.ban.lock().unwrap() = Some(ApiError {
            http_status: 403,
            code: 11244,
            message: "无权限".to_owned(),
            trace_id: Some("t-1".to_owned()),
        });
        let handler = SetGroupBanHandler::new(context);
        let response = block_on(handler.handle(&request(
            "set_group_ban",
            json!({"group_id": GROUP, "user_id": MEMBER, "duration": 60}),
        )))
        .expect("上游错误也应是 Ok(失败响应)");

        let (status, retcode, data) = failed(response);
        assert_eq!(status, Status::Failed);
        assert_eq!(retcode, RETCODE_UPSTREAM);
        assert_eq!(data, None);
    }

    #[test]
    fn missing_or_virtual_ids_without_idmap_fail_with_1400() {
        for params in [
            json!({}),
            json!({"group_id": 42}),
            json!({"group_id": 42, "user_id": 43}),
        ] {
            let (context, api) = context();
            let handler = GetGroupInfoHandler::new(context);
            let response = block_on(handler.handle(&request("get_group_info", params)))
                .expect("参数错误也应是 Ok(失败响应)");
            let (status, retcode, data) = failed(response);
            assert_eq!(status, Status::Failed);
            assert_eq!(retcode, RETCODE_BAD_REQUEST);
            assert_eq!(data, None);
            assert!(api.calls().is_empty(), "参数错误不应触达上游");
        }
    }

    #[test]
    fn numeric_user_id_with_member_openid_keeps_its_number_shape() {
        let (context, api) = context();
        api.member_info
            .lock()
            .unwrap()
            .replace(Ok(QqGroupMember::default()));

        let handler = GetGroupMemberInfoHandler::new(context);
        let response = block_on(handler.handle(&request(
            "get_group_member_info",
            json!({"group_id": GROUP, "member_openid": MEMBER, "user_id": 42}),
        )));
        let data = ok_data(response.expect("handler 不应返回 Err"));

        assert_eq!(
            api.calls(),
            vec![Call::MemberInfo(GROUP.to_owned(), MEMBER.to_owned())]
        );
        assert_eq!(data["user_id"], json!(42), "数字形态的虚拟号原样回填");
    }
}
