//! 面板 / 自定义菜单 / 自动审批策略 action（Go 的官方 v2 群管理扩展）。
//!
//! # 覆盖范围
//!
//! | action | Go 对应 | 官方端点 |
//! |--------|---------|----------|
//! | `create_panel` | `handlers/create_panel.go` | `POST /v2/panels` |
//! | `set_panel` | `handlers/set_panel.go` | `PUT /v2/panels/{panel_id}` |
//! | `delete_panel` | `handlers/delete_panel.go` | `DELETE /v2/panels/{panel_id}` |
//! | `get_panel` | `handlers/get_panel.go` | `GET /v2/panels/{panel_id}` |
//! | `get_panel_list` | `handlers/get_panel_list.go` | `GET /v2/panels` |
//! | `set_panel_target` | `handlers/set_panel_target.go` | `PUT /v2/panels/{panel_id}/target` |
//! | `set_custom_menu` | `handlers/set_custom_menu.go` | `PUT /v2/menu` |
//! | `get_custom_menu` | `handlers/get_custom_menu.go` | `GET /v2/menu` |
//! | `join_approval_strategy_create` | `handlers/join_approval_strategy.go` | `POST /v2/groups/join_approval_strategy` |
//! | `join_approval_strategy_list` | 同上 | `GET /v2/groups/join_approval_strategy` |
//! | `join_approval_strategy_update` | 同上 | `PATCH /v2/groups/join_approval_strategy/{id}` |
//! | `join_approval_strategy_execute` | 同上 | `POST .../{id}/execute` |
//! | `join_approval_strategy_whitelist` | 同上 | `POST .../{id}/whitelist_users` |
//! | `join_approval_strategy_delete` | 同上 | `DELETE .../{id}` |
//!
//! # 参数口径
//!
//! 必填字段照 Go 的 `callapi.ParamsContent` 校验：`scope` / `panel_id` / `panel` / `menu` /
//! `strategy_id` / `op` / 关联对象列表，缺失或类型不对回 `1400`
//! （[`crate::handler::RETCODE_BAD_REQUEST`]）；上游失败回 `1500`
//! （[`crate::util::RETCODE_UPSTREAM`]）。
//!
//! `panel` / `menu` 走严格解码（未知字段直接 1400）：`PUT` 是覆盖式写入，拼错的字段名被静默
//! 丢弃等于清空线上已有配置——与 Go 的 `decodeStrictJSON` 同口径。`get_custom_menu` 没有参数
//! 契约，Go 直接调 `GetMenu()`。
//!
//! # 刻意的差异（相对 Go）
//!
//! - **关联对象列表**：Go 用 `idmap.StoreIDv2` / `RetrieveRowByIDv2` 双向换算；这里复用
//!   [`ActionContext::resolve_openid`]，虚拟号未装配 idmap 时回 `1400` 而不是静默跳过。
//!   `get_panel` 的 `user_openids` / `group_openids` 仍按 Go 口径转回虚拟号，反查失败跳过该条。
//! - **`get_panel_list` 分页**：`limit` 缺省 / 0 时取 20（Go 同值），不额外截断——官方自己有限流。

use std::sync::Arc;

use serde::Serialize;
use serde_json::{Map, Value, json};

use gensokyo_core::{Error, Ob11Id, Result};
use gensokyo_protocol::action::{ActionRequest, ActionResponse};
use gensokyo_protocol::{Handler, HandlerFuture};
use gensokyo_qq::dto::{
    CreateJoinApprovalStrategyRequest, CreatePanelRequest, GroupAction, PanelList,
    PanelRecordDetail, PanelTargetRequest, PutMenuRequest, UpdateJoinApprovalStrategyRequest,
    UpdatePanelRequest, WhitelistUsersRequest,
};

use crate::params;
use crate::qqapi::ActionContext;
use crate::unsupported::{required_add_del_op, required_text};
use crate::util::{ob11_id, ok_data, params_object, params_or_empty, respond};

/// 面板 / 审批策略列表的默认单页条数（对齐 Go `get_panel_list` 的 `limit <= 0 → 20`）。
const DEFAULT_PAGE: u32 = 20;

/// `get_panel` 的响应：关联对象转成虚拟号（对齐 Go 的 `[]int64` 输出）。
#[derive(Debug, Clone, Serialize)]
struct PanelDetailData {
    /// 面板 ID。
    panel_id: String,
    /// 生效场景。
    scope: String,
    /// 关联对象形态。
    target_type: String,
    /// 面板内容（原样透传）。
    panel: Value,
    /// 创建时间（RFC3339）。
    created_at: String,
    /// 更新时间（RFC3339）。
    updated_at: String,
    /// 面板版本。
    version: i64,
    /// 关联用户虚拟号。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    user_openids: Vec<Ob11Id>,
    /// 关联群虚拟号。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    group_openids: Vec<Ob11Id>,
}

/// 严格解码客户端透传对象（未知字段直接报错，对齐 Go `decodeStrictJSON`）。
fn decode_strict<T>(object: &Map<String, Value>, key: &str) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let value = object
        .get(key)
        .ok_or_else(|| Error::Protocol(format!("缺少 {key}")))?;
    if !value.is_object() {
        return Err(Error::Protocol(format!("{key} 必须是 JSON 对象")));
    }
    serde_json::from_value(value.clone())
        .map_err(|err| Error::Protocol(format!("{key} 参数格式无效: {err}")))
}

/// 关联对象 ID 列表 → 真实 OpenID（任一反查失败整体失败，对齐 Go `resolveOpenIDList`）。
fn resolve_openids(
    context: &ActionContext,
    object: &Map<String, Value>,
    key: &str,
    kind: &str,
) -> Result<Vec<String>> {
    let mut openids = Vec::new();
    for id in params::id_list(object, key)? {
        openids.push(context.resolve_openid(key, &id, kind)?);
    }
    Ok(openids)
}

/// 真实 OpenID → 虚拟号；未装配 idmap 或反查失败时跳过该条（对齐 Go `get_panel`）。
fn virtual_ids(context: &ActionContext, openids: &[String]) -> Vec<Ob11Id> {
    let Some(store) = context.idmap() else {
        return Vec::new();
    };
    let string_mode = context.string_ob11();
    openids
        .iter()
        .filter_map(|openid| {
            store
                .retrieve_virtual_value(openid)
                .ok()
                .map(|(_, vuin)| ob11_id(&vuin.to_string(), string_mode))
        })
        .collect()
}

/// 分页条数：缺省 / 0 用默认值（Go `limit <= 0 → 默认`）。
fn requested_page(object: &Map<String, Value>, default: u32) -> Result<u32> {
    Ok(match params::optional_u32(object, "limit")? {
        None | Some(0) => default,
        Some(limit) => limit,
    })
}

/// `create_panel`。
#[derive(Debug, Clone)]
pub struct CreatePanelHandler {
    context: Arc<ActionContext>,
}

impl CreatePanelHandler {
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
        let body = CreatePanelRequest {
            scope: required_text(object, "scope")?,
            target_type: params::optional_text(object, "target_type")?.unwrap_or_default(),
            user_openids: resolve_openids(&self.context, object, "user_openids", "member_openid")?,
            group_openids: resolve_openids(&self.context, object, "group_openids", "group_openid")?,
            panel: decode_strict(object, "panel")?,
        };
        let response = self.context.api().create_panel(&body).await?;
        ok_data(request, &response)
    }
}

impl Handler for CreatePanelHandler {
    fn name(&self) -> &'static str {
        "create_panel"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `set_panel`。
#[derive(Debug, Clone)]
pub struct SetPanelHandler {
    context: Arc<ActionContext>,
}

impl SetPanelHandler {
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
        let panel_id = required_text(object, "panel_id")?;
        let body = UpdatePanelRequest {
            panel: decode_strict(object, "panel")?,
        };
        let response = self.context.api().update_panel(&panel_id, &body).await?;
        ok_data(request, &response)
    }
}

impl Handler for SetPanelHandler {
    fn name(&self) -> &'static str {
        "set_panel"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `delete_panel`。
#[derive(Debug, Clone)]
pub struct DeletePanelHandler {
    context: Arc<ActionContext>,
}

impl DeletePanelHandler {
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
        let panel_id = required_text(object, "panel_id")?;
        self.context.api().delete_panel(&panel_id).await?;
        // Go 回 `{}`：删除成功没有业务数据
        ok_data(request, &json!({}))
    }
}

impl Handler for DeletePanelHandler {
    fn name(&self) -> &'static str {
        "delete_panel"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `get_panel`。
#[derive(Debug, Clone)]
pub struct GetPanelHandler {
    context: Arc<ActionContext>,
}

impl GetPanelHandler {
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
        let panel_id = required_text(object, "panel_id")?;
        let detail = self.context.api().get_panel(&panel_id).await?;
        let data = panel_detail(&self.context, detail);
        ok_data(request, &data)
    }
}

impl Handler for GetPanelHandler {
    fn name(&self) -> &'static str {
        "get_panel"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// 详情转响应：关联对象按 Go 口径换成虚拟号。
fn panel_detail(context: &ActionContext, detail: PanelRecordDetail) -> PanelDetailData {
    PanelDetailData {
        user_openids: virtual_ids(context, &detail.user_openids),
        group_openids: virtual_ids(context, &detail.group_openids),
        panel_id: detail.panel_id,
        scope: detail.scope,
        target_type: detail.target_type,
        panel: detail.panel,
        created_at: detail.created_at,
        updated_at: detail.updated_at,
        version: detail.version,
    }
}

/// `get_panel_list`。
#[derive(Debug, Clone)]
pub struct GetPanelListHandler {
    context: Arc<ActionContext>,
}

impl GetPanelListHandler {
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
        let scope = required_text(object, "scope")?;
        let cursor = params::optional_text(object, "cursor")?;
        let limit = requested_page(object, DEFAULT_PAGE)?;
        let list: PanelList = self
            .context
            .api()
            .get_panels(&scope, cursor.as_deref(), Some(limit))
            .await?;
        ok_data(request, &list)
    }
}

impl Handler for GetPanelListHandler {
    fn name(&self) -> &'static str {
        "get_panel_list"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `set_panel_target`。
#[derive(Debug, Clone)]
pub struct SetPanelTargetHandler {
    context: Arc<ActionContext>,
}

impl SetPanelTargetHandler {
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
        let panel_id = required_text(object, "panel_id")?;
        required_add_del_op(object)?;
        let body = PanelTargetRequest {
            op: required_text(object, "op")?,
            user_openids: resolve_openids(&self.context, object, "user_openids", "member_openid")?,
            group_openids: resolve_openids(&self.context, object, "group_openids", "group_openid")?,
        };
        if body.user_openids.is_empty() && body.group_openids.is_empty() {
            return Err(Error::Protocol(
                "user_openids 与 group_openids 至少要有一个".to_owned(),
            ));
        }
        self.context
            .api()
            .update_panel_target(&panel_id, &body)
            .await?;
        ok_data(request, &json!({}))
    }
}

impl Handler for SetPanelTargetHandler {
    fn name(&self) -> &'static str {
        "set_panel_target"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `set_custom_menu`。
#[derive(Debug, Clone)]
pub struct SetCustomMenuHandler {
    context: Arc<ActionContext>,
}

impl SetCustomMenuHandler {
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
        let body = PutMenuRequest {
            menu: decode_strict(object, "menu")?,
        };
        let response = self.context.api().put_menu(&body).await?;
        ok_data(request, &response)
    }
}

impl Handler for SetCustomMenuHandler {
    fn name(&self) -> &'static str {
        "set_custom_menu"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `get_custom_menu`（Go 不读参数，这里也不做参数校验）。
#[derive(Debug, Clone)]
pub struct GetCustomMenuHandler {
    context: Arc<ActionContext>,
}

impl GetCustomMenuHandler {
    /// 构造。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let result = async {
            // Go 不读参数：这里只确认「给了 params 就得是对象」
            params_or_empty(request)?;
            let response = self.context.api().get_menu().await?;
            ok_data(request, &response)
        }
        .await;
        respond(request, result)
    }
}

impl Handler for GetCustomMenuHandler {
    fn name(&self) -> &'static str {
        "get_custom_menu"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `join_approval_strategy_create`。
#[derive(Debug, Clone)]
pub struct JoinApprovalStrategyCreateHandler {
    context: Arc<ActionContext>,
}

impl JoinApprovalStrategyCreateHandler {
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
        let body = CreateJoinApprovalStrategyRequest {
            group_openids: params::id_list(object, "group_openids")?,
            group_ids: group_ids(object)?,
            is_enable: params::optional_text(object, "is_enable")?.unwrap_or_default(),
            expire_at: params::optional_text(object, "expire_at")?.unwrap_or_default(),
            remark: params::optional_text(object, "remark")?.unwrap_or_default(),
        };
        let response = self
            .context
            .api()
            .create_join_approval_strategy(&body)
            .await?;
        ok_data(request, &response)
    }
}

impl Handler for JoinApprovalStrategyCreateHandler {
    fn name(&self) -> &'static str {
        "join_approval_strategy_create"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `join_approval_strategy_list`。
#[derive(Debug, Clone)]
pub struct JoinApprovalStrategyListHandler {
    context: Arc<ActionContext>,
}

impl JoinApprovalStrategyListHandler {
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
        let cursor = params::optional_text(object, "cursor")?;
        let limit = params::optional_u32(object, "limit")?.filter(|limit| *limit > 0);
        let response = self
            .context
            .api()
            .list_join_approval_strategy(cursor.as_deref(), limit)
            .await?;
        ok_data(request, &response)
    }
}

impl Handler for JoinApprovalStrategyListHandler {
    fn name(&self) -> &'static str {
        "join_approval_strategy_list"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `join_approval_strategy_update`。
#[derive(Debug, Clone)]
pub struct JoinApprovalStrategyUpdateHandler {
    context: Arc<ActionContext>,
}

impl JoinApprovalStrategyUpdateHandler {
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
        let strategy_id = required_text(object, "strategy_id")?;
        let op = params::optional_text(object, "op")?;
        let group_action = match op {
            Some(op) if !op.is_empty() => Some(GroupAction {
                op,
                group_openids: params::id_list(object, "group_openids")?,
                group_ids: group_ids(object)?,
            }),
            _ => None,
        };
        let body = UpdateJoinApprovalStrategyRequest {
            is_enable: params::optional_text(object, "is_enable")?.unwrap_or_default(),
            expire_at: params::optional_text(object, "expire_at")?.unwrap_or_default(),
            group_action,
            remark: params::optional_text(object, "remark")?.unwrap_or_default(),
        };
        let response = self
            .context
            .api()
            .update_join_approval_strategy(&strategy_id, &body)
            .await?;
        ok_data(request, &response)
    }
}

impl Handler for JoinApprovalStrategyUpdateHandler {
    fn name(&self) -> &'static str {
        "join_approval_strategy_update"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `join_approval_strategy_execute`。
#[derive(Debug, Clone)]
pub struct JoinApprovalStrategyExecuteHandler {
    context: Arc<ActionContext>,
}

impl JoinApprovalStrategyExecuteHandler {
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
        let strategy_id = required_text(object, "strategy_id")?;
        self.context
            .api()
            .execute_join_approval_strategy(&strategy_id)
            .await?;
        // Go 回 `data: null`
        ok_data(request, &Value::Null)
    }
}

impl Handler for JoinApprovalStrategyExecuteHandler {
    fn name(&self) -> &'static str {
        "join_approval_strategy_execute"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `join_approval_strategy_whitelist`。
#[derive(Debug, Clone)]
pub struct JoinApprovalStrategyWhitelistHandler {
    context: Arc<ActionContext>,
}

impl JoinApprovalStrategyWhitelistHandler {
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
        let strategy_id = required_text(object, "strategy_id")?;
        let body = WhitelistUsersRequest {
            op: params::optional_text(object, "op")?.unwrap_or_default(),
            whitelist_users: params::id_list(object, "whitelist_users")?,
        };
        let response = self
            .context
            .api()
            .update_join_approval_strategy_whitelist(&strategy_id, &body)
            .await?;
        ok_data(request, &response)
    }
}

impl Handler for JoinApprovalStrategyWhitelistHandler {
    fn name(&self) -> &'static str {
        "join_approval_strategy_whitelist"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `join_approval_strategy_delete`。
#[derive(Debug, Clone)]
pub struct JoinApprovalStrategyDeleteHandler {
    context: Arc<ActionContext>,
}

impl JoinApprovalStrategyDeleteHandler {
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
        let strategy_id = required_text(object, "strategy_id")?;
        self.context
            .api()
            .delete_join_approval_strategy(&strategy_id)
            .await?;
        ok_data(request, &Value::Null)
    }
}

impl Handler for JoinApprovalStrategyDeleteHandler {
    fn name(&self) -> &'static str {
        "join_approval_strategy_delete"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// 群号列表（`group_ids`）→ `Vec<u64>`。
fn group_ids(object: &Map<String, Value>) -> Result<Vec<u64>> {
    params::id_list(object, "group_ids")?
        .into_iter()
        .map(|id| {
            id.parse::<u64>()
                .map_err(|_| Error::Protocol(format!("group_ids 必须是数字群号: {id}")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{
        CreatePanelHandler, DeletePanelHandler, GetCustomMenuHandler, GetPanelHandler,
        GetPanelListHandler, JoinApprovalStrategyCreateHandler, JoinApprovalStrategyDeleteHandler,
        JoinApprovalStrategyExecuteHandler, JoinApprovalStrategyListHandler,
        JoinApprovalStrategyUpdateHandler, JoinApprovalStrategyWhitelistHandler,
        SetCustomMenuHandler, SetPanelHandler, SetPanelTargetHandler,
    };
    use crate::handler::RETCODE_BAD_REQUEST;
    use crate::testing::{
        Call, block_on, context, failed, ok_data, request, request_without_params,
    };
    use crate::util::RETCODE_UPSTREAM;
    use gensokyo_core::Error;
    use gensokyo_protocol::Handler;
    use gensokyo_protocol::Status;
    use gensokyo_qq::dto::{
        CreateJoinApprovalStrategyRequest, CreateJoinApprovalStrategyResponse, CreatePanelRequest,
        CreatePanelResponse, GroupAction, JoinApprovalStrategyList, MenuResponse, Panel, PanelItem,
        PanelList, PanelRecord, PanelRecordDetail, PanelTargetRequest, PutMenuRequest,
        UpdateJoinApprovalStrategyRequest, UpdateJoinApprovalStrategyResponse, VersionResponse,
        WhitelistUsersRequest, WhitelistUsersResponse,
    };

    const PANEL: &str = "panel-1";
    const STRATEGY: &str = "strategy-1";
    const GROUP: &str = "0123456789abcdef0123456789abcdef";

    /// 断言 `1400`。
    fn assert_bad_request(handler: &dyn Handler, params: Value) {
        let response = block_on(handler.handle(&request(handler.name(), params)))
            .expect("参数错误也应是 Ok(失败响应)");
        let (status, retcode, data) = failed(response);
        assert_eq!(status, Status::Failed);
        assert_eq!(retcode, RETCODE_BAD_REQUEST, "{}", handler.name());
        assert_eq!(data, None);
    }

    /// 断言 `1500`。
    fn assert_upstream_error(handler: &dyn Handler, params: Value) {
        let response = block_on(handler.handle(&request(handler.name(), params)))
            .expect("上游错误也应是 Ok(失败响应)");
        let (status, retcode, data) = failed(response);
        assert_eq!(status, Status::Failed);
        assert_eq!(retcode, RETCODE_UPSTREAM, "{}", handler.name());
        assert_eq!(data, None);
    }

    fn command_item(name: &str) -> PanelItem {
        PanelItem {
            name: name.to_owned(),
            item_type: "command".to_owned(),
            ..PanelItem::default()
        }
    }

    fn upstream() -> Error {
        Error::Upstream {
            code: 500,
            message: "boom".to_owned(),
        }
    }

    #[test]
    fn create_panel_resolves_targets_and_returns_panel_id() {
        let (ctx, api) = context();
        *api.panel_create.lock().unwrap() = Some(Ok(CreatePanelResponse {
            panel_id: "p-new".to_owned(),
        }));
        let handler = CreatePanelHandler::new(ctx);

        let response = block_on(handler.handle(&request(
            "create_panel",
            json!({
                "scope": "group",
                "target_type": "specific",
                "group_openids": [GROUP],
                "panel": {"items": [{"name": "签到", "type": "command"}]},
            }),
        )))
        .expect("创建面板不应失败");

        assert_eq!(ok_data(response)["panel_id"], json!("p-new"));
        assert_eq!(
            api.calls(),
            vec![Call::PanelCreate(CreatePanelRequest {
                scope: "group".to_owned(),
                target_type: "specific".to_owned(),
                user_openids: Vec::new(),
                group_openids: vec![GROUP.to_owned()],
                panel: Panel {
                    items: vec![command_item("签到")],
                    ..Panel::default()
                },
            })]
        );
        assert_eq!(handler.name(), "create_panel");
    }

    #[test]
    fn panel_objects_are_decoded_strictly() {
        let (ctx, api) = context();
        let handler = CreatePanelHandler::new(ctx);
        for params in [
            json!({"panel": {"items": []}}),
            json!({"scope": "group", "panel": []}),
            // 未知字段（拼错的 items）必须 1400，否则 PUT 会用空面板覆盖线上配置
            json!({"scope": "group", "panel": {"item": []}}),
        ] {
            assert_bad_request(&handler, params);
        }
        assert!(api.calls().is_empty(), "参数错误不应触达上游");
    }

    #[test]
    fn set_panel_forwards_panel_and_returns_version() {
        let (ctx, api) = context();
        *api.panel_update.lock().unwrap() = Some(Ok(VersionResponse { version: 7 }));
        let handler = SetPanelHandler::new(ctx);

        let response = block_on(handler.handle(&request(
            "set_panel",
            json!({"panel_id": PANEL, "panel": {"items": [{"name": "帮助", "type": "link", "link": "https://example.com"}]}}),
        )))
        .expect("更新面板不应失败");

        assert_eq!(ok_data(response)["version"], json!(7));
        let calls = api.calls();
        assert!(matches!(&calls[0], Call::PanelUpdate(id, _) if id == PANEL));
        assert_bad_request(&handler, json!({"panel_id": PANEL}));
    }

    #[test]
    fn delete_and_get_panel_use_their_endpoints() {
        let (ctx, api) = context();
        *api.panel_delete.lock().unwrap() = Some(Ok(()));
        let delete = DeletePanelHandler::new(ctx);
        let response =
            block_on(delete.handle(&request("delete_panel", json!({"panel_id": PANEL}))))
                .expect("删除面板不应失败");
        assert_eq!(ok_data(response), json!({}));
        assert_eq!(api.calls(), vec![Call::PanelDelete(PANEL.to_owned())]);
        assert_bad_request(&delete, json!({}));
        assert_eq!(delete.name(), "delete_panel");

        let (ctx, api) = context();
        *api.panel_get.lock().unwrap() = Some(Ok(PanelRecordDetail {
            panel_id: PANEL.to_owned(),
            scope: "group".to_owned(),
            panel: json!({"items": []}),
            group_openids: vec![GROUP.to_owned()],
            ..PanelRecordDetail::default()
        }));
        let get = GetPanelHandler::new(ctx);
        let response = block_on(get.handle(&request("get_panel", json!({"panel_id": PANEL}))))
            .expect("查询面板不应失败");
        let data = ok_data(response);
        assert_eq!(data["panel_id"], json!(PANEL));
        // 未装配 idmap：关联对象反查失败按 Go 口径跳过，不代表 0
        assert!(data.get("group_openids").is_none());
        assert_eq!(api.calls(), vec![Call::PanelGet(PANEL.to_owned())]);
    }

    #[test]
    fn panel_list_defaults_to_go_page_size() {
        let (ctx, api) = context();
        *api.panel_list.lock().unwrap() = Some(Ok(PanelList {
            records: vec![PanelRecord {
                panel_id: PANEL.to_owned(),
                ..PanelRecord::default()
            }],
            next_cursor: "n1".to_owned(),
            is_end: false,
        }));
        let handler = GetPanelListHandler::new(ctx);

        let response = block_on(handler.handle(&request(
            "get_panel_list",
            json!({"scope": "group", "cursor": "c1"}),
        )))
        .expect("面板列表不应失败");
        let data = ok_data(response);
        assert_eq!(data["next_cursor"], json!("n1"));
        assert_eq!(data["records"][0]["panel_id"], json!(PANEL));
        assert_eq!(
            api.calls(),
            vec![Call::PanelList(
                "group".to_owned(),
                Some("c1".to_owned()),
                Some(20)
            )]
        );
        assert_bad_request(&handler, json!({"cursor": "c1"}));
        assert_bad_request(&handler, json!({"scope": "group", "limit": -1}));
    }

    #[test]
    fn set_panel_target_requires_op_and_at_least_one_target() {
        let (ctx, api) = context();
        *api.panel_target.lock().unwrap() = Some(Ok(()));
        let handler = SetPanelTargetHandler::new(ctx);

        let response = block_on(handler.handle(&request(
            "set_panel_target",
            json!({"panel_id": PANEL, "op": "add", "group_openids": [GROUP]}),
        )))
        .expect("增删关联对象不应失败");
        assert_eq!(ok_data(response), json!({}));
        assert_eq!(
            api.calls(),
            vec![Call::PanelTarget(
                PANEL.to_owned(),
                PanelTargetRequest {
                    op: "add".to_owned(),
                    user_openids: Vec::new(),
                    group_openids: vec![GROUP.to_owned()],
                }
            )]
        );
        assert_bad_request(&handler, json!({"panel_id": PANEL, "op": "add"}));
        assert_bad_request(
            &handler,
            json!({"panel_id": PANEL, "op": "remove", "group_openids": [GROUP]}),
        );
    }

    #[test]
    fn custom_menu_round_trip() {
        let (ctx, api) = context();
        *api.menu_put.lock().unwrap() = Some(Ok(VersionResponse { version: 2 }));
        let set = SetCustomMenuHandler::new(ctx);

        let response = block_on(set.handle(&request(
            "set_custom_menu",
            json!({"menu": {"items": [{"name": "指令", "type": "send_message", "send_message": "/help"}]}}),
        )))
        .expect("设置菜单不应失败");
        assert_eq!(ok_data(response)["version"], json!(2));
        let calls = api.calls();
        assert!(
            matches!(&calls[0], Call::MenuPut(PutMenuRequest { menu }) if menu.items.len() == 1)
        );
        assert_bad_request(&set, json!({"menu": {"button": []}}));
        assert_bad_request(&set, json!({"menu": "raw"}));

        let (ctx, api) = context();
        *api.menu_get.lock().unwrap() = Some(Ok(MenuResponse {
            version: 3,
            menu: json!({"items": []}),
        }));
        let get = GetCustomMenuHandler::new(ctx);
        let response = block_on(get.handle(&request_without_params("get_custom_menu")))
            .expect("查询菜单不应失败");
        let data = ok_data(response);
        assert_eq!(data["version"], json!(3));
        assert_eq!(data["menu"], json!({"items": []}));
        assert_eq!(api.calls(), vec![Call::MenuGet]);
        assert_eq!(get.name(), "get_custom_menu");
    }

    #[test]
    fn strategy_create_and_list_forward_their_fields() {
        let (ctx, api) = context();
        *api.strategy_create.lock().unwrap() = Some(Ok(CreateJoinApprovalStrategyResponse {
            strategy_id: STRATEGY.to_owned(),
            is_enable: "on".to_owned(),
            expire_at: "2027-01-01T00:00:00Z".to_owned(),
        }));
        let create = JoinApprovalStrategyCreateHandler::new(ctx);

        let response = block_on(create.handle(&request(
            "join_approval_strategy_create",
            json!({
                "group_openids": [GROUP],
                "group_ids": [123456],
                "is_enable": "on",
                "remark": "自动通过",
            }),
        )))
        .expect("创建策略不应失败");
        assert_eq!(ok_data(response)["strategy_id"], json!(STRATEGY));
        assert_eq!(
            api.calls(),
            vec![Call::StrategyCreate(CreateJoinApprovalStrategyRequest {
                group_openids: vec![GROUP.to_owned()],
                group_ids: vec![123_456],
                is_enable: "on".to_owned(),
                expire_at: String::new(),
                remark: "自动通过".to_owned(),
            })]
        );
        assert_bad_request(&create, json!({"group_openids": "not-a-list"}));

        let (ctx, api) = context();
        *api.strategy_list.lock().unwrap() = Some(Ok(JoinApprovalStrategyList::default()));
        let list = JoinApprovalStrategyListHandler::new(ctx);
        let response = block_on(list.handle(&request(
            "join_approval_strategy_list",
            json!({"cursor": "c1", "limit": 10}),
        )))
        .expect("策略列表不应失败");
        assert_eq!(ok_data(response)["next_cursor"], json!(""));
        assert_eq!(
            api.calls(),
            vec![Call::StrategyList(Some("c1".to_owned()), Some(10))]
        );
        assert_bad_request(&list, json!({"limit": -1}));
    }

    #[test]
    fn strategy_update_execute_whitelist_and_delete_forward_ids() {
        let (ctx, api) = context();
        *api.strategy_update.lock().unwrap() = Some(Ok(UpdateJoinApprovalStrategyResponse {
            is_enable: "off".to_owned(),
            expire_at: String::new(),
        }));
        let update = JoinApprovalStrategyUpdateHandler::new(ctx);

        let response = block_on(update.handle(&request(
            "join_approval_strategy_update",
            json!({"strategy_id": STRATEGY, "op": "del", "group_ids": [42], "is_enable": "off"}),
        )))
        .expect("更新策略不应失败");
        assert_eq!(ok_data(response)["is_enable"], json!("off"));
        assert_eq!(
            api.calls(),
            vec![Call::StrategyUpdate(
                STRATEGY.to_owned(),
                UpdateJoinApprovalStrategyRequest {
                    is_enable: "off".to_owned(),
                    expire_at: String::new(),
                    group_action: Some(GroupAction {
                        op: "del".to_owned(),
                        group_openids: Vec::new(),
                        group_ids: vec![42],
                    }),
                    remark: String::new(),
                }
            )]
        );
        assert_bad_request(&update, json!({"is_enable": "on"}));

        let (ctx, api) = context();
        *api.strategy_execute.lock().unwrap() = Some(Ok(()));
        let execute = JoinApprovalStrategyExecuteHandler::new(ctx);
        let response = block_on(execute.handle(&request(
            "join_approval_strategy_execute",
            json!({"strategy_id": STRATEGY}),
        )))
        .expect("生效策略不应失败");
        assert_eq!(ok_data(response), Value::Null);
        assert_eq!(
            api.calls(),
            vec![Call::StrategyExecute(STRATEGY.to_owned())]
        );
        assert_bad_request(&execute, json!({}));

        let (ctx, api) = context();
        *api.strategy_whitelist.lock().unwrap() = Some(Ok(WhitelistUsersResponse {
            strategy_id: STRATEGY.to_owned(),
            whitelist_user_count: 1,
            updated_at: String::new(),
        }));
        let whitelist = JoinApprovalStrategyWhitelistHandler::new(ctx);
        let response = block_on(whitelist.handle(&request(
            "join_approval_strategy_whitelist",
            json!({"strategy_id": STRATEGY, "op": "add", "whitelist_users": ["123456"]}),
        )))
        .expect("白名单不应失败");
        assert_eq!(ok_data(response)["whitelist_user_count"], json!(1));
        assert_eq!(
            api.calls(),
            vec![Call::StrategyWhitelist(
                STRATEGY.to_owned(),
                WhitelistUsersRequest {
                    op: "add".to_owned(),
                    whitelist_users: vec!["123456".to_owned()],
                }
            )]
        );
        assert_bad_request(
            &whitelist,
            json!({"strategy_id": STRATEGY, "whitelist_users": [1.5]}),
        );

        let (ctx, api) = context();
        *api.strategy_delete.lock().unwrap() = Some(Ok(()));
        let delete = JoinApprovalStrategyDeleteHandler::new(ctx);
        let response = block_on(delete.handle(&request(
            "join_approval_strategy_delete",
            json!({"strategy_id": STRATEGY}),
        )))
        .expect("删除策略不应失败");
        assert_eq!(ok_data(response), Value::Null);
        assert_eq!(api.calls(), vec![Call::StrategyDelete(STRATEGY.to_owned())]);
        assert_bad_request(&delete, json!({"strategy_id": ""}));
    }

    #[test]
    fn upstream_errors_map_to_1500() {
        let (ctx, api) = context();
        *api.panel_get.lock().unwrap() = Some(Err(upstream()));
        let get = GetPanelHandler::new(ctx);
        assert_upstream_error(&get, json!({"panel_id": PANEL}));

        let (ctx, api) = context();
        *api.panel_delete.lock().unwrap() = Some(Err(upstream()));
        let delete = DeletePanelHandler::new(ctx);
        assert_upstream_error(&delete, json!({"panel_id": PANEL}));

        let (ctx, api) = context();
        *api.panel_create.lock().unwrap() = Some(Err(upstream()));
        let create = CreatePanelHandler::new(ctx);
        assert_upstream_error(&create, json!({"scope": "group", "panel": {"items": []}}));
    }
}
