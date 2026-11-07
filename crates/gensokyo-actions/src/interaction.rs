//! 头像 / 已读 / 分享链接 / 互动 / 群消息撤回 / 入群审批 / 黑名单 / 合并转发 / 直接发送 /
//! 快速操作 action。
//!
//! # 覆盖范围
//!
//! | action | Go 对应 | 上游端点 | 结果 |
//! |--------|---------|----------|------|
//! | `get_avatar` | `handlers/get_avatar.go` | 无（`q.qlogo.cn` URL 模板） | 本地拼 URL：`{"message": url, "url": url, "user_id": …}` |
//! | `mark_msg_as_read` | `handlers/mark_msg_as_read.go` | 无（官方无已读上报接口） | 本地空实现：`retcode = 0` + `data = null` |
//! | `.handle_quick_operation` | `handlers/handle_quick_operation.go` | 无（复用发送链） | deferred 计划（`send_group_msg` / `send_private_msg`） |
//! | `.handle_quick_operation_async` | `handlers/handle_quick_operation_async.go` | 同上 | 同上 |
//! | `get_robot_share_link` | `handlers/get_robot_share_link.go` | `POST /v2/generate_url_link` | `{"url": …, "callback_data": …}` |
//! | `put_interaction` | `handlers/put_interaction.go` | `PUT /interactions/{interaction_id}` | `data = null` |
//! | `delete_group_msg` | `handlers/delete_group_msg.go` | `DELETE /v2/groups/{gid}/messages/{mid}` | `data = null` |
//! | `set_group_add_request` | `handlers/set_group_add_request.go` | `POST /v2/groups/{gid}/approval_join_request/{member}` | `data = null` |
//! | `get_group_member_blacklist` | `handlers/get_group_member_blacklist.go` | `GET /v2/groups/{gid}/member_blacklist` | 黑名单分页 |
//! | `set_group_member_blacklist` | `handlers/set_group_member_blacklist.go` | `POST /v2/groups/{gid}/member_blacklist` | `{"fail_openids": […]}` |
//! | `send_group_forward_msg` | `handlers/send_group_forward_msg.go` | 无（Go 逐条转普通消息） | `1500` |
//! | `send_group_msg_raw` | `handlers/send_group_msg_raw.go` | 发送链 | `1500` |
//! | `send_private_msg_sse` | `handlers/send_private_msg_sse.go` | 发送链（SSE 流式） | `1500` |
//! | `send_private_msg_wakeup` | `handlers/send_private_msg_wakeup.go` | 发送链（`IsWakeup`） | `1500` |
//! | `send_private_forward_msg` | 无（go-cqhttp 扩展名） | 无 | `1500` |
//! | `get_group_honor_info` | 无（go-cqhttp 扩展名） | 无 | `1500` |
//! | `set_group_anonymous_ban` | 无（go-cqhttp 扩展名） | 无 | `1500` |
//!
//! # 本地可判定的三项
//!
//! - **`get_avatar`**：Go 用 `https://q.qlogo.cn/qqapp/{app_id}/{openid}/640` 拼头像地址，不触达
//!   上游。这里同样取配置 `app_id` + 反查出的 `user_openid` 拼 URL；响应保留 Go 的
//!   `message` / `user_id` 字段，并额外给 go-cqhttp 标准客户端一个同值的 `url`。
//!   Go 在 `idmap_pro` 下用「群 + 成员」二维反查，本 crate 只有一维虚拟号反查——32 位 OpenID
//!   直通，两者结果一致；虚拟号场景下以 `user_openid` 映射为准。
//! - **`mark_msg_as_read`**：官方没有已读上报端点，Go 回 `retcode = 0` + `status = ok` + 空 `data`
//!   （`gjson.Result` 零值序列化成 `null`）。这里逐字段照抄，不触达上游。
//! - **`.handle_quick_operation` / `.handle_quick_operation_async`**：Go 的
//!   `CreateSendGroupMsgAction` 只做一件事——按 `context.message_type` 把 `operation.reply`
//!   转成一次 `send_group_msg` / `send_private_msg`。本 crate 复用 [`crate::handler`] 的 deferred
//!   计划（[`crate::send::plan_send_group_msg`] / [`crate::send::plan_send_private_msg`]），
//!   响应形状与同步发送 action 一致，真正的发送仍由 runtime 接管。
//!
//! # 仍然回 1500 的项
//!
//! 剩下这些需要运行时发送链的额外支持（原始 JSON 直发、SSE 流式、`IsWakeup` 主动私聊、
//! 合并转发逐条重发），或官方根本没有对应端点（群荣誉、匿名成员禁言）：按
//! [`crate::unsupported`] 的口径回 `1500` + 说明；参数仍照 Go 的 `callapi.ParamsContent`
//! 校验，缺失或类型不对回 `1400`。
//!
//! # 刻意的差异（相对 Go）
//!
//! - **`.handle_quick_operation` 的未知 `message_type`**：Go 返回空串（客户端收不到任何回执），
//!   这里回 `1400` + 说明，便于定位客户端的事件字段错误。
//! - **合并转发**：Go 把 node 列表逐条转成普通消息再发送；本 crate 不做这层降级（会丢转发卡片
//!   语义），直接 `1500`。
//! - **`get_robot_share_link`**：Go 把链接包成一条伪造的 `share_link_generated` notice 事件推给
//!   客户端，action 本身回 `"{}"`。本 crate 的 handler 契约只能回一个响应，因此把链接放进
//!   `data`（`url` + 回显的 `callback_data`），不伪造事件。
//! - **`put_interaction`**：Go 从 `echo`（字符串）取 interaction_id、从顶层 `post_type` 取结果码；
//!   本 crate 的 `ActionRequest` 没有 `post_type` 字段，因此改读 `params.interaction_id`
//!   （缺省回退 `echo`）与 `params.code`。
//! - **黑名单 / 审批的 ID 反查**：Go 逐个反查、失败跳过该条；这里任一反查失败整体回 `1400`，
//!   与 [`crate::group_admin`] 的 `set_group_kick` 同口径——`add` 少一个成员比多一个更安全。

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use gensokyo_core::{Error, Ob11Id, Result};
use gensokyo_protocol::action::{ActionRequest, ActionResponse};
use gensokyo_protocol::{Handler, HandlerFuture};
use gensokyo_qq::dto::{
    ApprovalJoinRequest, GenerateUrlLinkRequest, MemberBlacklistList, MemberBlacklistRequest,
};

use crate::group_admin::collect_user_ids;
use crate::handler::respond_to_params;
use crate::messages::resolve_message_id;
use crate::params;
use crate::qqapi::{ActionContext, RetractRequest};
use crate::send::{SendRequest, plan_send_group_msg, plan_send_private_msg};
use crate::unsupported::{checked_unsupported, required_add_del_op, required_object};
use crate::util::{
    bad_request, ob11_id, ok_data, params_object, params_or_empty, resolve_group,
    resolve_identity_text, resolve_member, respond,
};

/// `send_group_forward_msg` 的说明。
const SEND_GROUP_FORWARD_MSG: &str =
    "send_group_forward_msg: 合并转发需要逐条重发 node，本 crate 未接线";
/// `send_group_msg_raw` 的说明。
const SEND_GROUP_MSG_RAW: &str = "send_group_msg_raw: 原始 JSON 直发尚未接入 deferred 发送链";
/// `send_private_msg_sse` 的说明。
const SEND_PRIVATE_MSG_SSE: &str = "send_private_msg_sse: SSE 流式发送尚未接入 deferred 发送链";
/// `send_private_msg_wakeup` 的说明。
const SEND_PRIVATE_MSG_WAKEUP: &str =
    "send_private_msg_wakeup: IsWakeup 主动私聊尚未接入 deferred 发送链";
/// `send_private_forward_msg` 的说明。
const SEND_PRIVATE_FORWARD_MSG: &str =
    "send_private_forward_msg: 合并转发需要逐条重发 node，本 crate 未接线";
/// `get_group_honor_info` 的说明。
const GET_GROUP_HONOR_INFO: &str = "get_group_honor_info: QQ 官方未提供群荣誉信息接口";
/// `set_group_anonymous_ban` 的说明。
const SET_GROUP_ANONYMOUS_BAN: &str = "set_group_anonymous_ban: QQ 官方未提供匿名成员禁言接口";

/// `get_avatar` 的响应数据（Go `AvatarData` + go-cqhttp 标准的 `url`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvatarData {
    /// 头像地址（Go 把 URL 放在 `message` 字段里）。
    pub message: String,
    /// 头像地址（go-cqhttp 标准字段，与 `message` 同值）。
    pub url: String,
    /// 请求里的用户虚拟号（回填请求值）。
    pub user_id: Ob11Id,
}

/// `message` 必填：消息体可以是字符串 / 数组 / 对象，只拒绝缺失与 `null`。
fn required_message(object: &Map<String, Value>) -> Result<()> {
    match object.get("message") {
        None | Some(Value::Null) => Err(Error::Protocol("缺少 message".to_owned())),
        Some(_) => Ok(()),
    }
}

/// `messages` 必须是非空的 node 列表（合并转发）。
fn required_nodes(object: &Map<String, Value>) -> Result<()> {
    match object.get("messages") {
        Some(Value::Array(items)) if !items.is_empty() => Ok(()),
        Some(Value::Array(_)) => Err(Error::Protocol("messages 不能是空数组".to_owned())),
        None | Some(Value::Null) => Err(Error::Protocol("缺少 messages".to_owned())),
        Some(_) => Err(Error::Protocol("messages 必须是数组".to_owned())),
    }
}

/// `group_id` 与 `user_id` 至少给一个（`send_group_msg_raw` 的群 / 私聊二选一）。
fn either_group_or_user(object: &Map<String, Value>) -> Result<()> {
    if params::id_text(object, "group_id")?.is_none()
        && params::id_text(object, "user_id")?.is_none()
    {
        return Err(Error::Protocol(
            "group_id 与 user_id 至少要有一个".to_owned(),
        ));
    }
    Ok(())
}

/// 群黑名单默认单页条数（官方单页上限）。
const DEFAULT_BLACKLIST_PAGE: u32 = 100;

/// 互动事件的 ID：`params.interaction_id` 优先，缺省回退 `echo`（Go 只读 echo）。
fn interaction_id(request: &ActionRequest<Value>, object: &Map<String, Value>) -> Result<String> {
    if let Some(id) = params::optional_text(object, "interaction_id")? {
        return Ok(id);
    }
    match request.echo.as_ref() {
        Some(Value::String(id)) if !id.is_empty() => Ok(id.clone()),
        Some(Value::Number(id)) => Ok(id.to_string()),
        _ => Err(Error::Protocol(
            "put_interaction 需要 interaction_id（params.interaction_id 或 echo）".to_owned(),
        )),
    }
}

/// 互动结果码：`params.code`（0~5），兼容 Go 顶层 `post_type` 的文本形态。
fn interaction_code(object: &Map<String, Value>) -> Result<u32> {
    let code = match params::optional_u32(object, "code")? {
        Some(code) => code,
        None => match params::optional_text(object, "post_type")? {
            Some(text) => text
                .parse::<u32>()
                .map_err(|_| Error::Protocol(format!("post_type 必须是 0~5 的数字: {text}")))?,
            None => 0,
        },
    };
    if code > 5 {
        return Err(Error::Protocol(format!("code 只支持 0~5: {code}")));
    }
    Ok(code)
}

/// `send_group_forward_msg`：`group_id` + 非空 `messages`。
fn validate_send_group_forward_msg(object: &Map<String, Value>) -> Result<()> {
    params::required_id(object, "group_id")?;
    required_nodes(object)
}

/// `send_private_forward_msg`：`user_id` + 非空 `messages`。
fn validate_send_private_forward_msg(object: &Map<String, Value>) -> Result<()> {
    params::required_id(object, "user_id")?;
    required_nodes(object)
}

/// `send_group_msg_raw`：`message` 必填 + `group_id` / `user_id` 二选一。
fn validate_send_group_msg_raw(object: &Map<String, Value>) -> Result<()> {
    required_message(object)?;
    either_group_or_user(object)
}

/// `send_private_msg_sse` / `send_private_msg_wakeup`：`user_id` + `message` 必填。
fn validate_private_send(object: &Map<String, Value>) -> Result<()> {
    params::required_id(object, "user_id")?;
    required_message(object)
}

/// `get_group_honor_info`：`group_id` 必填。
fn validate_get_group_honor_info(object: &Map<String, Value>) -> Result<()> {
    params::required_id(object, "group_id")?;
    params::optional_text(object, "type")?;
    Ok(())
}

/// `set_group_anonymous_ban`：`group_id` 必填 + `anonymous` / `flag` 至少一个。
fn validate_set_group_anonymous_ban(object: &Map<String, Value>) -> Result<()> {
    params::required_id(object, "group_id")?;
    let anonymous = object.get("anonymous");
    if let Some(value) = anonymous
        && !value.is_object()
    {
        return Err(Error::Protocol("anonymous 必须是 JSON 对象".to_owned()));
    }
    let flag = params::optional_text(object, "flag")?;
    if anonymous.is_none() && flag.is_none() {
        return Err(Error::Protocol("anonymous 与 flag 至少要有一个".to_owned()));
    }
    params::optional_i64(object, "duration")?;
    Ok(())
}

/// 一次发送的规划函数（[`plan_send_group_msg`] / [`plan_send_private_msg`]）。
type Planner = fn(&Value) -> Result<SendRequest>;

/// 从 `context` / `operation` 还原一次发送（对齐 Go `CreateSendGroupMsgAction`）。
///
/// 返回构造好的发送 `params` 与对应的规划函数：`message_type = group` → 群聊，
/// `private` → 私聊；其余取值按参数错误处理（Go 返回空串，客户端收不到回执）。
fn quick_operation_params(object: &Map<String, Value>) -> Result<(Value, Planner)> {
    let context = required_object(object, "context")?;
    let operation = required_object(object, "operation")?;
    let message_type = params::optional_text(context, "message_type")?
        .ok_or_else(|| Error::Protocol("缺少 context.message_type".to_owned()))?;
    let reply = params::optional_text(operation, "reply")?
        .ok_or_else(|| Error::Protocol("缺少 operation.reply".to_owned()))?;
    match message_type.as_str() {
        "group" => {
            let group_id = params::required_id(context, "group_id")?;
            Ok((
                json!({"group_id": group_id, "message": reply}),
                plan_send_group_msg,
            ))
        }
        "private" => {
            let user_id = params::required_id(context, "user_id")?;
            Ok((
                json!({"user_id": user_id, "message": reply}),
                plan_send_private_msg,
            ))
        }
        other => Err(Error::Protocol(format!(
            "context.message_type 只支持 group / private: {other}"
        ))),
    }
}

/// `.handle_quick_operation` / `.handle_quick_operation_async` 的公共流程。
fn quick_operation(request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
    let object = match params_or_empty(request) {
        Ok(object) => object,
        Err(err) => return Ok(bad_request(request, &err)),
    };
    let (params, planner) = match quick_operation_params(&object) {
        Ok(planned) => planned,
        Err(err) => return Ok(bad_request(request, &err)),
    };
    respond_to_params(request, &params, planner)
}

/// `get_avatar`：本地拼 `q.qlogo.cn` 头像地址。
#[derive(Debug, Clone)]
pub struct GetAvatarHandler {
    context: Arc<ActionContext>,
}

impl GetAvatarHandler {
    /// 构造：需要 `ActionContext`（配置 `app_id` + idmap 反查）。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = match params_object(request) {
            Ok(object) => object,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let requested = match params::required_id(object, "user_id") {
            Ok(user_id) => user_id,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let resolved =
            match resolve_identity_text(&self.context, "user_id", &requested, "user_openid") {
                Ok(resolved) => resolved,
                Err(err) => return Ok(bad_request(request, &err)),
            };
        let app_id = self.context.config().snapshot().get_app_id();
        let url = format!("https://q.qlogo.cn/qqapp/{app_id}/{}/640", resolved.openid);
        let data = AvatarData {
            message: url.clone(),
            url,
            user_id: ob11_id(&requested, self.context.string_ob11()),
        };
        ok_data(request, &data)
    }
}

impl Handler for GetAvatarHandler {
    fn name(&self) -> &'static str {
        "get_avatar"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `mark_msg_as_read`：本地空实现（官方无已读上报接口，Go 同样不触达上游）。
#[derive(Debug, Clone, Copy, Default)]
pub struct MarkMsgAsReadHandler;

impl MarkMsgAsReadHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        ok_data(request, &Value::Null)
    }
}

impl Handler for MarkMsgAsReadHandler {
    fn name(&self) -> &'static str {
        "mark_msg_as_read"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `.handle_quick_operation`。
#[derive(Debug, Clone, Copy, Default)]
pub struct QuickOperationHandler;

impl QuickOperationHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Handler for QuickOperationHandler {
    fn name(&self) -> &'static str {
        ".handle_quick_operation"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(async move { quick_operation(request) })
    }
}

/// `.handle_quick_operation_async`（Go 与同步版注册到同一函数）。
#[derive(Debug, Clone, Copy, Default)]
pub struct QuickOperationAsyncHandler;

impl QuickOperationAsyncHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Handler for QuickOperationAsyncHandler {
    fn name(&self) -> &'static str {
        ".handle_quick_operation_async"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(async move { quick_operation(request) })
    }
}

/// `get_robot_share_link`。
#[derive(Debug, Clone)]
pub struct GetRobotShareLinkHandler {
    context: Arc<ActionContext>,
}

impl GetRobotShareLinkHandler {
    /// 构造。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let result = self.execute(request).await;
        respond(request, result)
    }

    async fn execute(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = params_or_empty(request)?;
        let callback_data =
            params::optional_text(object.as_ref(), "callback_data")?.unwrap_or_default();
        let link = self
            .context
            .api()
            .generate_url_link(&GenerateUrlLinkRequest {
                callback_data: callback_data.clone(),
            })
            .await?;
        // Go 把链接包成伪造 notice 事件推送；handler 只能回响应，因此放进 data
        ok_data(
            request,
            &json!({ "url": link.url, "callback_data": callback_data }),
        )
    }
}

impl Handler for GetRobotShareLinkHandler {
    fn name(&self) -> &'static str {
        "get_robot_share_link"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `put_interaction`。
#[derive(Debug, Clone)]
pub struct PutInteractionHandler {
    context: Arc<ActionContext>,
}

impl PutInteractionHandler {
    /// 构造。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let result = self.execute(request).await;
        respond(request, result)
    }

    async fn execute(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = params_or_empty(request)?;
        let object = object.as_ref();
        let interaction_id = interaction_id(request, object)?;
        let code = interaction_code(object)?;
        self.context
            .api()
            .put_interaction(&interaction_id, code)
            .await?;
        ok_data(request, &Value::Null)
    }
}

impl Handler for PutInteractionHandler {
    fn name(&self) -> &'static str {
        "put_interaction"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `delete_group_msg`。
#[derive(Debug, Clone)]
pub struct DeleteGroupMsgHandler {
    context: Arc<ActionContext>,
}

impl DeleteGroupMsgHandler {
    /// 构造。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let result = self.execute(request).await;
        respond(request, result)
    }

    async fn execute(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = params_object(request)?;
        let group = resolve_group(&self.context, object)?;
        // `user_id` 只做类型校验：Go 用它区分「撤回自己发的消息」，群撤回路径不看它
        params::id_text(object, "user_id")?;
        let requested = params::required_id(object, "message_id")?;
        let message_id = resolve_message_id(&self.context, &requested)?;
        self.context
            .api()
            .delete_msg(RetractRequest {
                group_openid: Some(group.openid),
                user_openid: None,
                message_id,
            })
            .await?;
        ok_data(request, &Value::Null)
    }
}

impl Handler for DeleteGroupMsgHandler {
    fn name(&self) -> &'static str {
        "delete_group_msg"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `set_group_add_request`。
#[derive(Debug, Clone)]
pub struct SetGroupAddRequestHandler {
    context: Arc<ActionContext>,
}

impl SetGroupAddRequestHandler {
    /// 构造。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let result = self.execute(request).await;
        respond(request, result)
    }

    async fn execute(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = params_object(request)?;
        let group = resolve_group(&self.context, object)?;
        let member = resolve_member(&self.context, object)?;
        // Go 的 `approve` 缺省为 true；`flag` 是入群申请列表里的申请 ID
        let approve = params::optional_bool(object, "approve")?.unwrap_or(true);
        let body = ApprovalJoinRequest {
            op: if approve { "approve" } else { "decline" }.to_owned(),
            join_request_id: params::optional_text(object, "flag")?.unwrap_or_default(),
            reject_reason: params::optional_text(object, "reason")?.unwrap_or_default(),
            add_to_member_blacklist: params::optional_bool(object, "add_to_member_blacklist")?
                .unwrap_or(false),
        };
        self.context
            .api()
            .approve_join_request(&group.openid, &member.openid, &body)
            .await?;
        ok_data(request, &Value::Null)
    }
}

impl Handler for SetGroupAddRequestHandler {
    fn name(&self) -> &'static str {
        "set_group_add_request"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `get_group_member_blacklist`。
#[derive(Debug, Clone)]
pub struct GetGroupMemberBlacklistHandler {
    context: Arc<ActionContext>,
}

impl GetGroupMemberBlacklistHandler {
    /// 构造。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let result = self.execute(request).await;
        respond(request, result)
    }

    async fn execute(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = params_object(request)?;
        let group = resolve_group(&self.context, object)?;
        let cursor = params::optional_text(object, "cursor")?;
        let limit = match params::optional_u32(object, "limit")? {
            None | Some(0) => DEFAULT_BLACKLIST_PAGE,
            Some(limit) => limit,
        };
        let list: MemberBlacklistList = self
            .context
            .api()
            .member_blacklist_list(&group.openid, cursor.as_deref(), Some(limit))
            .await?;
        let data = blacklist_data(&self.context, list);
        ok_data(request, &data)
    }
}

/// 黑名单条目（`user_id` 是虚拟号，与入站上报口径一致，可直接回传增删接口）。
#[derive(Debug, Clone, Serialize)]
struct BlacklistEntry {
    /// 成员虚拟号（未装配 idmap 时跳过该条）。
    user_id: Ob11Id,
    /// union_openid。
    union_openid: String,
    /// member_openid。
    member_openid: String,
    /// 昵称。
    username: String,
    /// 拉黑时间（RFC3339）。
    banned_at: String,
    /// 是否机器人。
    bot: bool,
}

/// 黑名单响应：`user_id` 换成虚拟号，反查失败跳过（对齐 Go）。
#[derive(Debug, Clone, Serialize)]
struct BlacklistData {
    /// 黑名单条目。
    users: Vec<BlacklistEntry>,
    /// 下一页游标。
    next_cursor: String,
}

/// 上游黑名单 → 响应（`member_openid` → 虚拟号）。
fn blacklist_data(context: &ActionContext, list: MemberBlacklistList) -> BlacklistData {
    let string_mode = context.string_ob11();
    let idmap = context.idmap();
    let users = list
        .users
        .into_iter()
        .filter_map(|user| {
            let virtual_id = idmap
                .and_then(|store| store.retrieve_virtual_value(&user.member_openid).ok())
                .map(|(_, vuin)| ob11_id(&vuin.to_string(), string_mode))?;
            Some(BlacklistEntry {
                user_id: virtual_id,
                union_openid: user.union_openid,
                member_openid: user.member_openid,
                username: user.username,
                banned_at: user.banned_at,
                bot: user.bot,
            })
        })
        .collect();
    BlacklistData {
        users,
        next_cursor: list.next_cursor,
    }
}

impl Handler for GetGroupMemberBlacklistHandler {
    fn name(&self) -> &'static str {
        "get_group_member_blacklist"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `set_group_member_blacklist`。
#[derive(Debug, Clone)]
pub struct SetGroupMemberBlacklistHandler {
    context: Arc<ActionContext>,
}

impl SetGroupMemberBlacklistHandler {
    /// 构造。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let result = self.execute(request).await;
        respond(request, result)
    }

    async fn execute(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = params_object(request)?;
        let group = resolve_group(&self.context, object)?;
        required_add_del_op(object)?;
        let virtual_ids = collect_user_ids(object)?;
        if virtual_ids.is_empty() {
            return Err(Error::Protocol(
                "user_id 与 user_ids 至少要有一个".to_owned(),
            ));
        }
        let mut member_openids = Vec::with_capacity(virtual_ids.len());
        for virtual_id in &virtual_ids {
            member_openids.push(
                resolve_identity_text(&self.context, "user_id", virtual_id, "member_openid")?
                    .openid,
            );
        }
        let response = self
            .context
            .api()
            .update_member_blacklist(
                &group.openid,
                &MemberBlacklistRequest {
                    op: params::optional_text(object, "op")?.unwrap_or_default(),
                    member_openids,
                },
            )
            .await?;
        ok_data(request, &response)
    }
}

impl Handler for SetGroupMemberBlacklistHandler {
    fn name(&self) -> &'static str {
        "set_group_member_blacklist"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `send_group_forward_msg`。
#[derive(Debug, Clone, Copy, Default)]
pub struct SendGroupForwardMsgHandler;

impl SendGroupForwardMsgHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        checked_unsupported(
            request,
            SEND_GROUP_FORWARD_MSG,
            validate_send_group_forward_msg,
        )
    }
}

impl Handler for SendGroupForwardMsgHandler {
    fn name(&self) -> &'static str {
        "send_group_forward_msg"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `send_private_forward_msg`（go-cqhttp 扩展名）。
#[derive(Debug, Clone, Copy, Default)]
pub struct SendPrivateForwardMsgHandler;

impl SendPrivateForwardMsgHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        checked_unsupported(
            request,
            SEND_PRIVATE_FORWARD_MSG,
            validate_send_private_forward_msg,
        )
    }
}

impl Handler for SendPrivateForwardMsgHandler {
    fn name(&self) -> &'static str {
        "send_private_forward_msg"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `send_group_msg_raw`。
#[derive(Debug, Clone, Copy, Default)]
pub struct SendGroupMsgRawHandler;

impl SendGroupMsgRawHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        checked_unsupported(request, SEND_GROUP_MSG_RAW, validate_send_group_msg_raw)
    }
}

impl Handler for SendGroupMsgRawHandler {
    fn name(&self) -> &'static str {
        "send_group_msg_raw"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `send_private_msg_sse`。
#[derive(Debug, Clone, Copy, Default)]
pub struct SendPrivateMsgSseHandler;

impl SendPrivateMsgSseHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        checked_unsupported(request, SEND_PRIVATE_MSG_SSE, validate_private_send)
    }
}

impl Handler for SendPrivateMsgSseHandler {
    fn name(&self) -> &'static str {
        "send_private_msg_sse"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `send_private_msg_wakeup`。
#[derive(Debug, Clone, Copy, Default)]
pub struct SendPrivateMsgWakeupHandler;

impl SendPrivateMsgWakeupHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        checked_unsupported(request, SEND_PRIVATE_MSG_WAKEUP, validate_private_send)
    }
}

impl Handler for SendPrivateMsgWakeupHandler {
    fn name(&self) -> &'static str {
        "send_private_msg_wakeup"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `get_group_honor_info`（go-cqhttp 扩展名）。
#[derive(Debug, Clone, Copy, Default)]
pub struct GetGroupHonorInfoHandler;

impl GetGroupHonorInfoHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        checked_unsupported(request, GET_GROUP_HONOR_INFO, validate_get_group_honor_info)
    }
}

impl Handler for GetGroupHonorInfoHandler {
    fn name(&self) -> &'static str {
        "get_group_honor_info"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `set_group_anonymous_ban`（go-cqhttp 扩展名）。
#[derive(Debug, Clone, Copy, Default)]
pub struct SetGroupAnonymousBanHandler;

impl SetGroupAnonymousBanHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        checked_unsupported(
            request,
            SET_GROUP_ANONYMOUS_BAN,
            validate_set_group_anonymous_ban,
        )
    }
}

impl Handler for SetGroupAnonymousBanHandler {
    fn name(&self) -> &'static str {
        "set_group_anonymous_ban"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{
        DeleteGroupMsgHandler, GetAvatarHandler, GetGroupHonorInfoHandler,
        GetGroupMemberBlacklistHandler, GetRobotShareLinkHandler, MarkMsgAsReadHandler,
        PutInteractionHandler, QuickOperationAsyncHandler, QuickOperationHandler,
        SendGroupForwardMsgHandler, SendGroupMsgRawHandler, SendPrivateForwardMsgHandler,
        SendPrivateMsgSseHandler, SendPrivateMsgWakeupHandler, SetGroupAddRequestHandler,
        SetGroupAnonymousBanHandler, SetGroupMemberBlacklistHandler,
    };
    use crate::handler::{DEFERRED_FIELD, RETCODE_BAD_REQUEST};
    use crate::testing::{
        Call, block_on, context, failed, ok_data, request, request_without_params,
    };
    use crate::util::RETCODE_UPSTREAM;
    use gensokyo_protocol::Handler;
    use gensokyo_protocol::Status;
    use gensokyo_qq::dto::{
        ApprovalJoinRequest, GenerateUrlLink, GenerateUrlLinkRequest, MemberBlacklistList,
        MemberBlacklistRequest, MemberBlacklistResponse,
    };

    const GROUP: &str = "0123456789abcdef0123456789abcdef";
    const MEMBER: &str = "fedcba9876543210fedcba9876543210";

    /// 断言 `1500`。
    fn assert_unsupported(handler: &dyn Handler, params: Value) {
        let response = block_on(handler.handle(&request(handler.name(), params)))
            .expect("未实现端点也应是 Ok(失败响应)");
        let (status, retcode, data) = failed(response);
        assert_eq!(status, Status::Failed);
        assert_eq!(retcode, RETCODE_UPSTREAM, "{}", handler.name());
        assert_eq!(data, None);
    }

    /// 断言 `1400`。
    fn assert_bad_request(handler: &dyn Handler, params: Value) {
        let response = block_on(handler.handle(&request(handler.name(), params)))
            .expect("参数错误也应是 Ok(失败响应)");
        let (status, retcode, data) = failed(response);
        assert_eq!(status, Status::Failed);
        assert_eq!(retcode, RETCODE_BAD_REQUEST, "{}", handler.name());
        assert_eq!(data, None);
    }

    #[test]
    fn avatar_is_built_locally_from_app_id_and_openid() {
        let (ctx, api) = context();
        let handler = GetAvatarHandler::new(ctx);

        let response = block_on(handler.handle(&request("get_avatar", json!({"user_id": MEMBER}))))
            .expect("本地拼 URL 不应失败");
        let data = ok_data(response);
        let url = format!("https://q.qlogo.cn/qqapp/102001234/{MEMBER}/640");
        assert_eq!(data["url"], json!(url));
        assert_eq!(data["message"], json!(url));
        assert_eq!(data["user_id"], json!(MEMBER));
        assert!(api.calls().is_empty(), "get_avatar 不应触达上游");
        assert_eq!(handler.name(), "get_avatar");

        assert_bad_request(&handler, json!({}));
        assert_bad_request(&handler, json!({"user_id": "not-a-vuin"}));
    }

    #[test]
    fn mark_msg_as_read_is_a_local_noop_success() {
        let handler = MarkMsgAsReadHandler::new();
        for req in [
            request("mark_msg_as_read", json!({"message_id": 42})),
            request_without_params("mark_msg_as_read"),
        ] {
            let response = block_on(handler.handle(&req)).expect("本地空实现不应失败");
            assert_eq!(ok_data(response), Value::Null);
        }
        assert_eq!(handler.name(), "mark_msg_as_read");
    }

    #[test]
    fn quick_operation_plans_group_and_private_sends() {
        let group = QuickOperationHandler::new();
        let response = block_on(group.handle(&request(
            ".handle_quick_operation",
            json!({
                "context": {"message_type": "group", "group_id": 10086},
                "operation": {"reply": "收到"},
            }),
        )))
        .expect("快速操作应产出 deferred 计划");
        let data = ok_data(response);
        assert_eq!(data[DEFERRED_FIELD], json!(true));
        assert_eq!(data["target"], json!({"group": "10086"}));
        assert_eq!(data["message"]["text"], json!("收到"));
        assert_eq!(group.name(), ".handle_quick_operation");

        let private = QuickOperationAsyncHandler::new();
        let response = block_on(private.handle(&request(
            ".handle_quick_operation_async",
            json!({
                "context": {"message_type": "private", "user_id": "12345"},
                "operation": {"reply": "在"},
            }),
        )))
        .expect("快速操作应产出 deferred 计划");
        let data = ok_data(response);
        assert_eq!(data["target"], json!({"private": "12345"}));
        assert_eq!(private.name(), ".handle_quick_operation_async");
    }

    #[test]
    fn quick_operation_rejects_malformed_context() {
        let handler = QuickOperationHandler::new();
        assert_bad_request(&handler, json!({}));
        assert_bad_request(
            &handler,
            json!({"context": {"message_type": "group"}, "operation": {"reply": "hi"}}),
        );
        assert_bad_request(
            &handler,
            json!({"context": {"message_type": "channel"}, "operation": {"reply": "hi"}}),
        );
        assert_bad_request(
            &handler,
            json!({"context": {"message_type": "group", "group_id": 1}, "operation": {}}),
        );
    }

    #[test]
    fn share_link_forwards_callback_data_and_returns_url() {
        let (ctx, api) = context();
        *api.share_link.lock().unwrap() = Some(Ok(GenerateUrlLink {
            url: "https://example.com/link".to_owned(),
        }));
        let handler = GetRobotShareLinkHandler::new(ctx);

        let response = block_on(handler.handle(&request(
            "get_robot_share_link",
            json!({"callback_data": "c1"}),
        )))
        .expect("生成分享链接不应失败");
        let data = ok_data(response);
        assert_eq!(data["url"], json!("https://example.com/link"));
        assert_eq!(data["callback_data"], json!("c1"));
        assert_eq!(
            api.calls(),
            vec![Call::ShareLink(GenerateUrlLinkRequest {
                callback_data: "c1".to_owned(),
            })]
        );
        assert_bad_request(&handler, json!({"callback_data": ["c1"]}));
        assert_eq!(handler.name(), "get_robot_share_link");
    }

    #[test]
    fn put_interaction_uses_echo_id_and_result_code() {
        let (ctx, api) = context();
        *api.interaction.lock().unwrap() = Some(Ok(()));
        let handler = PutInteractionHandler::new(ctx);

        let response = block_on(handler.handle(&request(
            "put_interaction",
            json!({"interaction_id": "inter-1", "code": 3}),
        )))
        .expect("回应互动不应失败");
        assert_eq!(ok_data(response), Value::Null);
        assert_eq!(
            api.calls(),
            vec![Call::Interaction("inter-1".to_owned(), 3)]
        );

        // Go 口径：echo 就是 interaction_id，params 缺省时回退它
        let mut by_echo = request("put_interaction", json!({}));
        by_echo.echo = Some(json!("inter-2"));
        *api.interaction.lock().unwrap() = Some(Ok(()));
        let response = block_on(handler.handle(&by_echo)).expect("echo 也应可作 interaction_id");
        assert_eq!(response.echo, Some(json!("inter-2")));
        assert_eq!(
            api.calls(),
            vec![
                Call::Interaction("inter-1".to_owned(), 3),
                Call::Interaction("inter-2".to_owned(), 0),
            ]
        );

        let mut no_echo = request("put_interaction", json!({"code": 1}));
        no_echo.echo = None;
        let missing =
            block_on(handler.handle(&no_echo)).expect("缺 interaction_id 也应是 Ok(失败响应)");
        assert_eq!(missing.status, Status::Failed);
        assert_eq!(missing.retcode, RETCODE_BAD_REQUEST);
        assert_bad_request(&handler, json!({"interaction_id": "i2", "code": 9}));
        assert_eq!(handler.name(), "put_interaction");
    }

    #[test]
    fn delete_group_msg_retracts_through_the_group_endpoint() {
        let (ctx, api) = context();
        *api.delete.lock().unwrap() = Some(Ok(()));
        let handler = DeleteGroupMsgHandler::new(ctx);

        let response = block_on(handler.handle(&request(
            "delete_group_msg",
            json!({"group_id": GROUP, "message_id": "msg-1"}),
        )))
        .expect("撤回群消息不应失败");
        assert_eq!(ok_data(response), Value::Null);
        assert_eq!(
            api.calls(),
            vec![Call::DeleteMsg(
                Some(GROUP.to_owned()),
                None,
                "msg-1".to_owned()
            )]
        );
        assert_bad_request(&handler, json!({"group_id": GROUP}));
        assert_eq!(handler.name(), "delete_group_msg");
    }

    #[test]
    fn set_group_add_request_maps_approve_to_op() {
        let (ctx, api) = context();
        *api.join_approval.lock().unwrap() = Some(Ok(()));
        let handler = SetGroupAddRequestHandler::new(ctx);

        let response = block_on(handler.handle(&request(
            "set_group_add_request",
            json!({
                "group_id": GROUP,
                "user_id": MEMBER,
                "approve": false,
                "flag": "f1",
                "reason": "满员",
                "add_to_member_blacklist": true,
            }),
        )))
        .expect("审批入群申请不应失败");
        assert_eq!(ok_data(response), Value::Null);
        assert_eq!(
            api.calls(),
            vec![Call::JoinApproval(
                GROUP.to_owned(),
                MEMBER.to_owned(),
                ApprovalJoinRequest {
                    op: "decline".to_owned(),
                    join_request_id: "f1".to_owned(),
                    reject_reason: "满员".to_owned(),
                    add_to_member_blacklist: true,
                }
            )]
        );
        assert_bad_request(&handler, json!({"group_id": GROUP}));
        assert_bad_request(
            &handler,
            json!({"group_id": GROUP, "user_id": MEMBER, "approve": "yes"}),
        );
        assert_eq!(handler.name(), "set_group_add_request");
    }

    #[test]
    fn blacklist_actions_forward_paging_and_batches() {
        let (ctx, api) = context();
        *api.blacklist_list.lock().unwrap() = Some(Ok(MemberBlacklistList {
            users: Vec::new(),
            next_cursor: "n1".to_owned(),
        }));
        let get = GetGroupMemberBlacklistHandler::new(ctx);

        let response = block_on(get.handle(&request(
            "get_group_member_blacklist",
            json!({"group_id": GROUP}),
        )))
        .expect("查询黑名单不应失败");
        let data = ok_data(response);
        assert_eq!(data["next_cursor"], json!("n1"));
        assert_eq!(data["users"], json!([]));
        assert_eq!(
            api.calls(),
            vec![Call::BlacklistList(GROUP.to_owned(), None, Some(100))]
        );
        assert_bad_request(&get, json!({}));
        assert_eq!(get.name(), "get_group_member_blacklist");

        let (ctx, api) = context();
        *api.blacklist_update.lock().unwrap() = Some(Ok(MemberBlacklistResponse {
            fail_openids: Vec::new(),
        }));
        let set = SetGroupMemberBlacklistHandler::new(ctx);

        let response = block_on(set.handle(&request(
            "set_group_member_blacklist",
            json!({"group_id": GROUP, "op": "add", "user_ids": [MEMBER]}),
        )))
        .expect("增删黑名单不应失败");
        assert_eq!(ok_data(response)["fail_openids"], json!([]));
        assert_eq!(
            api.calls(),
            vec![Call::BlacklistUpdate(
                GROUP.to_owned(),
                MemberBlacklistRequest {
                    op: "add".to_owned(),
                    member_openids: vec![MEMBER.to_owned()],
                }
            )]
        );
        assert_bad_request(&set, json!({"group_id": GROUP, "op": "add"}));
        assert_bad_request(
            &set,
            json!({"group_id": GROUP, "op": "remove", "user_id": MEMBER}),
        );
        assert_eq!(set.name(), "set_group_member_blacklist");
    }

    #[test]
    fn forward_actions_validate_nodes() {
        let group = SendGroupForwardMsgHandler::new();
        assert_unsupported(
            &group,
            json!({"group_id": GROUP, "messages": [{"type": "node", "data": {}}]}),
        );
        assert_bad_request(&group, json!({"group_id": GROUP, "messages": []}));
        assert_bad_request(&group, json!({"group_id": GROUP, "messages": "raw"}));
        assert_eq!(group.name(), "send_group_forward_msg");

        let private = SendPrivateForwardMsgHandler::new();
        assert_unsupported(
            &private,
            json!({"user_id": MEMBER, "messages": [{"type": "node", "data": {}}]}),
        );
        assert_bad_request(&private, json!({"user_id": MEMBER}));
        assert_eq!(private.name(), "send_private_forward_msg");
    }

    #[test]
    fn direct_send_actions_are_unsupported() {
        let raw = SendGroupMsgRawHandler::new();
        assert_unsupported(
            &raw,
            json!({"group_id": GROUP, "message": {"type": "text"}}),
        );
        assert_bad_request(&raw, json!({"group_id": GROUP}));
        assert_bad_request(&raw, json!({"message": "hi"}));
        assert_eq!(raw.name(), "send_group_msg_raw");

        let sse = SendPrivateMsgSseHandler::new();
        assert_unsupported(&sse, json!({"user_id": MEMBER, "message": "hi"}));
        assert_bad_request(&sse, json!({"user_id": MEMBER}));
        assert_eq!(sse.name(), "send_private_msg_sse");

        let wakeup = SendPrivateMsgWakeupHandler::new();
        assert_unsupported(&wakeup, json!({"user_id": MEMBER, "message": "hi"}));
        assert_bad_request(&wakeup, json!({"message": "hi"}));
        assert_eq!(wakeup.name(), "send_private_msg_wakeup");
    }

    #[test]
    fn extension_actions_are_unsupported() {
        let honor = GetGroupHonorInfoHandler::new();
        assert_unsupported(&honor, json!({"group_id": GROUP, "type": "all"}));
        assert_bad_request(&honor, json!({}));
        assert_eq!(honor.name(), "get_group_honor_info");

        let anonymous = SetGroupAnonymousBanHandler::new();
        assert_unsupported(
            &anonymous,
            json!({"group_id": GROUP, "anonymous": {"id": 1}, "duration": 60}),
        );
        assert_unsupported(&anonymous, json!({"group_id": GROUP, "flag": "f1"}));
        assert_bad_request(&anonymous, json!({"group_id": GROUP}));
        assert_bad_request(&anonymous, json!({"group_id": GROUP, "anonymous": "raw"}));
        assert_eq!(anonymous.name(), "set_group_anonymous_ban");
    }
}
