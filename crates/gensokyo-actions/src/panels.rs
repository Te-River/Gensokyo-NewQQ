//! 面板 / 自定义菜单 / 自动审批策略 action（Go 的官方 v2 群管理扩展）。
//!
//! # 覆盖范围
//!
//! | action | Go 对应 | botgo 端点 | 结果 |
//! |--------|---------|-----------|------|
//! | `create_panel` | `handlers/create_panel.go` | `CreatePanel` | `1500` |
//! | `set_panel` | `handlers/set_panel.go` | `UpdatePanel` | `1500` |
//! | `delete_panel` | `handlers/delete_panel.go` | `DeletePanel` | `1500` |
//! | `get_panel` | `handlers/get_panel.go` | `GetPanel` | `1500` |
//! | `get_panel_list` | `handlers/get_panel_list.go` | `GetPanels` | `1500` |
//! | `set_panel_target` | `handlers/set_panel_target.go` | `UpdatePanelTarget` | `1500` |
//! | `set_custom_menu` | `handlers/set_custom_menu.go` | `PutMenu` | `1500` |
//! | `get_custom_menu` | `handlers/get_custom_menu.go` | `GetMenu` | `1500` |
//! | `join_approval_strategy_create` | `handlers/join_approval_strategy.go` | `CreateJoinApprovalStrategy` | `1500` |
//! | `join_approval_strategy_list` | 同上 | `ListJoinApprovalStrategy` | `1500` |
//! | `join_approval_strategy_update` | 同上 | `UpdateJoinApprovalStrategy` | `1500` |
//! | `join_approval_strategy_execute` | 同上 | `ExecuteJoinApprovalStrategy` | `1500` |
//! | `join_approval_strategy_whitelist` | 同上 | `UpdateJoinApprovalStrategyWhitelist` | `1500` |
//! | `join_approval_strategy_delete` | 同上 | `DeleteJoinApprovalStrategy` | `1500` |
//!
//! # 为什么全部是 1500
//!
//! 这些 action 在 Go 里都走 botgo 的 `GroupAPI` 面板 / 菜单 / 审批策略方法；`gensokyo-qq` 只移植了
//! 群信息、成员、禁言、入群申请列表这几组端点（见 [`crate::qqapi::QqApi`]），没有对应的 HTTP
//! 方法。按 [`crate::unsupported`] 的口径回 `1500` + 一句说明，不伪造空列表：客户端看到「功能
//! 不存在」比拿到一个假的空面板更容易排查。
//!
//! # 参数先于 1500
//!
//! 必填字段照 Go 的 `callapi.ParamsContent` 校验——`scope` / `panel_id` / `panel` / `menu` /
//! `strategy_id` / `op` / 关联对象列表，缺失或类型不对回 `1400`
//! （[`crate::handler::RETCODE_BAD_REQUEST`]）。这样客户端能区分「参数写错」和「端点没落地」，
//! 也不必为一个不存在的功能去猜字段名。`get_custom_menu` 是唯一没有参数契约的项：Go 直接调
//! `GetMenu()`，这里只回 `1500`。

use serde_json::{Map, Value};

use gensokyo_core::{Error, Result};
use gensokyo_protocol::action::{ActionRequest, ActionResponse};
use gensokyo_protocol::{Handler, HandlerFuture};

use crate::params;
use crate::unsupported::{
    checked_unsupported, required_add_del_op, required_object, required_text, unsupported,
};

/// `create_panel` 的说明。
const CREATE_PANEL: &str = "create_panel: gensokyo-qq 尚未暴露官方 v2 面板创建端点";
/// `set_panel` 的说明。
const SET_PANEL: &str = "set_panel: gensokyo-qq 尚未暴露官方 v2 面板更新端点";
/// `delete_panel` 的说明。
const DELETE_PANEL: &str = "delete_panel: gensokyo-qq 尚未暴露官方 v2 面板删除端点";
/// `get_panel` 的说明。
const GET_PANEL: &str = "get_panel: gensokyo-qq 尚未暴露官方 v2 面板查询端点";
/// `get_panel_list` 的说明。
const GET_PANEL_LIST: &str = "get_panel_list: gensokyo-qq 尚未暴露官方 v2 面板列表端点";
/// `set_panel_target` 的说明。
const SET_PANEL_TARGET: &str = "set_panel_target: gensokyo-qq 尚未暴露官方 v2 面板关联对象端点";
/// `set_custom_menu` 的说明。
const SET_CUSTOM_MENU: &str = "set_custom_menu: gensokyo-qq 尚未暴露官方 v2 自定义菜单端点";
/// `get_custom_menu` 的说明。
const GET_CUSTOM_MENU: &str = "get_custom_menu: gensokyo-qq 尚未暴露官方 v2 自定义菜单端点";
/// `join_approval_strategy_create` 的说明。
const STRATEGY_CREATE: &str = "join_approval_strategy_create: gensokyo-qq 尚未暴露自动审批策略端点";
/// `join_approval_strategy_list` 的说明。
const STRATEGY_LIST: &str = "join_approval_strategy_list: gensokyo-qq 尚未暴露自动审批策略端点";
/// `join_approval_strategy_update` 的说明。
const STRATEGY_UPDATE: &str = "join_approval_strategy_update: gensokyo-qq 尚未暴露自动审批策略端点";
/// `join_approval_strategy_execute` 的说明。
const STRATEGY_EXECUTE: &str =
    "join_approval_strategy_execute: gensokyo-qq 尚未暴露自动审批策略端点";
/// `join_approval_strategy_whitelist` 的说明。
const STRATEGY_WHITELIST: &str =
    "join_approval_strategy_whitelist: gensokyo-qq 尚未暴露自动审批策略端点";
/// `join_approval_strategy_delete` 的说明。
const STRATEGY_DELETE: &str = "join_approval_strategy_delete: gensokyo-qq 尚未暴露自动审批策略端点";

/// `create_panel`：`scope` + `panel` 必填。
fn validate_create_panel(object: &Map<String, Value>) -> Result<()> {
    required_text(object, "scope")?;
    required_object(object, "panel")?;
    Ok(())
}

/// `set_panel`：`panel_id` + `panel` 必填。
fn validate_set_panel(object: &Map<String, Value>) -> Result<()> {
    required_text(object, "panel_id")?;
    required_object(object, "panel")?;
    Ok(())
}

/// `delete_panel` / `get_panel`：`panel_id` 必填。
fn validate_panel_id(object: &Map<String, Value>) -> Result<()> {
    required_text(object, "panel_id")?;
    Ok(())
}

/// `get_panel_list`：`scope` 必填，`cursor` / `limit` 校验类型。
fn validate_get_panel_list(object: &Map<String, Value>) -> Result<()> {
    required_text(object, "scope")?;
    params::optional_text(object, "cursor")?;
    params::optional_u32(object, "limit")?;
    Ok(())
}

/// `set_panel_target`：`panel_id` + `op` + 至少一个关联对象列表。
fn validate_set_panel_target(object: &Map<String, Value>) -> Result<()> {
    required_text(object, "panel_id")?;
    required_add_del_op(object)?;
    let users = params::id_list(object, "user_openids")?;
    let groups = params::id_list(object, "group_openids")?;
    if users.is_empty() && groups.is_empty() {
        return Err(Error::Protocol(
            "user_openids 与 group_openids 至少要有一个".to_owned(),
        ));
    }
    Ok(())
}

/// `set_custom_menu`：`menu` 必填。
fn validate_set_custom_menu(object: &Map<String, Value>) -> Result<()> {
    required_object(object, "menu")?;
    Ok(())
}

/// 策略创建 / 更新的可选字段：关联群列表与开关字段只校验类型。
fn validate_strategy_fields(object: &Map<String, Value>) -> Result<()> {
    params::id_list(object, "group_openids")?;
    params::id_list(object, "group_ids")?;
    params::optional_text(object, "is_enable")?;
    params::optional_text(object, "expire_at")?;
    params::optional_text(object, "remark")?;
    Ok(())
}

/// `join_approval_strategy_create`：全部字段可选（Go 直接透传官方校验）。
fn validate_strategy_create(object: &Map<String, Value>) -> Result<()> {
    validate_strategy_fields(object)
}

/// `join_approval_strategy_list`：`cursor` / `limit` 校验类型。
fn validate_strategy_list(object: &Map<String, Value>) -> Result<()> {
    params::optional_text(object, "cursor")?;
    params::optional_u32(object, "limit")?;
    Ok(())
}

/// `join_approval_strategy_update`：`strategy_id` 必填 + 可选的更新字段。
fn validate_strategy_update(object: &Map<String, Value>) -> Result<()> {
    required_text(object, "strategy_id")?;
    params::optional_text(object, "op")?;
    validate_strategy_fields(object)
}

/// `join_approval_strategy_execute` / `_delete`：`strategy_id` 必填。
fn validate_strategy_id(object: &Map<String, Value>) -> Result<()> {
    required_text(object, "strategy_id")?;
    Ok(())
}

/// `join_approval_strategy_whitelist`：`strategy_id` 必填 + `op` / 白名单列表校验类型。
fn validate_strategy_whitelist(object: &Map<String, Value>) -> Result<()> {
    required_text(object, "strategy_id")?;
    params::optional_text(object, "op")?;
    params::id_list(object, "whitelist_users")?;
    Ok(())
}

/// `create_panel`。
#[derive(Debug, Clone, Copy, Default)]
pub struct CreatePanelHandler;

impl CreatePanelHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        checked_unsupported(request, CREATE_PANEL, validate_create_panel)
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
#[derive(Debug, Clone, Copy, Default)]
pub struct SetPanelHandler;

impl SetPanelHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        checked_unsupported(request, SET_PANEL, validate_set_panel)
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
#[derive(Debug, Clone, Copy, Default)]
pub struct DeletePanelHandler;

impl DeletePanelHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        checked_unsupported(request, DELETE_PANEL, validate_panel_id)
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
#[derive(Debug, Clone, Copy, Default)]
pub struct GetPanelHandler;

impl GetPanelHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        checked_unsupported(request, GET_PANEL, validate_panel_id)
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

/// `get_panel_list`。
#[derive(Debug, Clone, Copy, Default)]
pub struct GetPanelListHandler;

impl GetPanelListHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        checked_unsupported(request, GET_PANEL_LIST, validate_get_panel_list)
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
#[derive(Debug, Clone, Copy, Default)]
pub struct SetPanelTargetHandler;

impl SetPanelTargetHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        checked_unsupported(request, SET_PANEL_TARGET, validate_set_panel_target)
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
#[derive(Debug, Clone, Copy, Default)]
pub struct SetCustomMenuHandler;

impl SetCustomMenuHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        checked_unsupported(request, SET_CUSTOM_MENU, validate_set_custom_menu)
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
#[derive(Debug, Clone, Copy, Default)]
pub struct GetCustomMenuHandler;

impl GetCustomMenuHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        Ok(unsupported(request, GET_CUSTOM_MENU))
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
#[derive(Debug, Clone, Copy, Default)]
pub struct JoinApprovalStrategyCreateHandler;

impl JoinApprovalStrategyCreateHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        checked_unsupported(request, STRATEGY_CREATE, validate_strategy_create)
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
#[derive(Debug, Clone, Copy, Default)]
pub struct JoinApprovalStrategyListHandler;

impl JoinApprovalStrategyListHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        checked_unsupported(request, STRATEGY_LIST, validate_strategy_list)
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
#[derive(Debug, Clone, Copy, Default)]
pub struct JoinApprovalStrategyUpdateHandler;

impl JoinApprovalStrategyUpdateHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        checked_unsupported(request, STRATEGY_UPDATE, validate_strategy_update)
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
#[derive(Debug, Clone, Copy, Default)]
pub struct JoinApprovalStrategyExecuteHandler;

impl JoinApprovalStrategyExecuteHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        checked_unsupported(request, STRATEGY_EXECUTE, validate_strategy_id)
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
#[derive(Debug, Clone, Copy, Default)]
pub struct JoinApprovalStrategyWhitelistHandler;

impl JoinApprovalStrategyWhitelistHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        checked_unsupported(request, STRATEGY_WHITELIST, validate_strategy_whitelist)
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
#[derive(Debug, Clone, Copy, Default)]
pub struct JoinApprovalStrategyDeleteHandler;

impl JoinApprovalStrategyDeleteHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        checked_unsupported(request, STRATEGY_DELETE, validate_strategy_id)
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
    use crate::testing::{block_on, failed, request, request_without_params};
    use crate::util::RETCODE_UPSTREAM;
    use gensokyo_protocol::Handler;
    use gensokyo_protocol::Status;

    const PANEL: &str = "panel-1";
    const STRATEGY: &str = "strategy-1";

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
    fn panel_crud_actions_require_their_go_fields() {
        let create = CreatePanelHandler::new();
        assert_unsupported(&create, json!({"scope": "group", "panel": {"items": []}}));
        assert_bad_request(&create, json!({"panel": {}}));
        assert_bad_request(&create, json!({"scope": "group", "panel": []}));
        assert_eq!(create.name(), "create_panel");

        let set = SetPanelHandler::new();
        assert_unsupported(&set, json!({"panel_id": PANEL, "panel": {"items": []}}));
        assert_bad_request(&set, json!({"panel_id": PANEL}));
        assert_eq!(set.name(), "set_panel");

        let delete = DeletePanelHandler::new();
        assert_unsupported(&delete, json!({"panel_id": PANEL}));
        assert_bad_request(&delete, json!({}));
        assert_eq!(delete.name(), "delete_panel");

        let get = GetPanelHandler::new();
        assert_unsupported(&get, json!({"panel_id": PANEL}));
        assert_bad_request(&get, json!({"panel_id": ""}));
        assert_eq!(get.name(), "get_panel");
    }

    #[test]
    fn panel_list_and_target_follow_go_validation() {
        let list = GetPanelListHandler::new();
        assert_unsupported(
            &list,
            json!({"scope": "group", "cursor": "c1", "limit": 20}),
        );
        assert_bad_request(&list, json!({"cursor": "c1"}));
        assert_eq!(list.name(), "get_panel_list");

        let target = SetPanelTargetHandler::new();
        assert_unsupported(
            &target,
            json!({"panel_id": PANEL, "op": "add", "group_openids": ["0123456789abcdef0123456789abcdef"]}),
        );
        assert_bad_request(&target, json!({"panel_id": PANEL, "op": "add"}));
        assert_bad_request(
            &target,
            json!({"panel_id": PANEL, "op": "remove", "group_openids": ["1"]}),
        );
        assert_eq!(target.name(), "set_panel_target");
    }

    #[test]
    fn custom_menu_actions_are_unsupported() {
        let set = SetCustomMenuHandler::new();
        assert_unsupported(&set, json!({"menu": {"button": []}}));
        assert_bad_request(&set, json!({"menu": "raw"}));
        assert_eq!(set.name(), "set_custom_menu");

        let get = GetCustomMenuHandler::new();
        assert_unsupported(&get, json!({}));
        let response = block_on(get.handle(&request_without_params("get_custom_menu")))
            .expect("未实现端点也应是 Ok(失败响应)");
        assert_eq!(failed(response).1, RETCODE_UPSTREAM);
        assert_eq!(get.name(), "get_custom_menu");
    }

    #[test]
    fn strategy_actions_require_strategy_id() {
        let create = JoinApprovalStrategyCreateHandler::new();
        assert_unsupported(
            &create,
            json!({"group_openids": ["0123456789abcdef0123456789abcdef"]}),
        );
        assert_bad_request(&create, json!({"group_openids": "not-a-list"}));
        assert_eq!(create.name(), "join_approval_strategy_create");

        let list = JoinApprovalStrategyListHandler::new();
        assert_unsupported(&list, json!({"cursor": "c1", "limit": 10}));
        assert_bad_request(&list, json!({"limit": -1}));
        assert_eq!(list.name(), "join_approval_strategy_list");

        let update = JoinApprovalStrategyUpdateHandler::new();
        assert_unsupported(&update, json!({"strategy_id": STRATEGY, "is_enable": "on"}));
        assert_bad_request(&update, json!({"is_enable": "on"}));
        assert_eq!(update.name(), "join_approval_strategy_update");

        let execute = JoinApprovalStrategyExecuteHandler::new();
        assert_unsupported(&execute, json!({"strategy_id": STRATEGY}));
        assert_bad_request(&execute, json!({}));
        assert_eq!(execute.name(), "join_approval_strategy_execute");

        let whitelist = JoinApprovalStrategyWhitelistHandler::new();
        assert_unsupported(
            &whitelist,
            json!({"strategy_id": STRATEGY, "op": "add", "whitelist_users": ["123456"]}),
        );
        assert_bad_request(
            &whitelist,
            json!({"strategy_id": STRATEGY, "whitelist_users": [1.5]}),
        );
        assert_eq!(whitelist.name(), "join_approval_strategy_whitelist");

        let delete = JoinApprovalStrategyDeleteHandler::new();
        assert_unsupported(&delete, json!({"strategy_id": STRATEGY}));
        assert_bad_request(&delete, json!({"strategy_id": ""}));
        assert_eq!(delete.name(), "join_approval_strategy_delete");
    }
}
