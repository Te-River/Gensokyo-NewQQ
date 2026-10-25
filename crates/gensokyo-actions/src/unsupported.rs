//! 无上游端点的 action：参数契约照 OneBot 校验后回 `retcode = 1500`（或 Go 的本地固定回执）。
//!
//! # 覆盖范围
//!
//! | action | Go 对应 | 上游端点 | 结果 |
//! |--------|---------|----------|------|
//! | `get_group_at_all_remain` | 无（Go 未注册） | 官方无 @全体成员剩余次数接口 | `1500` |
//! | `set_group_name` | 无（Go 未注册） | 官方无修改群名称接口 | `1500` |
//! | `set_group_admin` | 无（Go 未注册） | 官方无设置 / 取消群管理员接口 | `1500` |
//! | `set_group_special_title` | 无（Go 未注册） | 官方无群头衔接口 | `1500` |
//! | `set_group_remark` | 无（Go 未注册） | 官方无设置群备注接口 | `1500` |
//! | `set_qq_profile` | 无（Go 未注册） | 官方无修改机器人资料接口 | `1500` |
//! | `get_qq_profile` | 无（Go 未注册） | `gensokyo-qq` 尚未暴露机器人资料端点 | `1500` |
//! | `get_group_system_msg` | 无（Go 未注册） | 官方无群系统消息接口 | `1500` |
//! | `get_online_clients` | `handlers/get_online_clients.go` | 官方无在线客户端端点 | 本地回 Go 的固定回执 `{"clients": [], "tiny_id": 0}` |
//!
//! # 为什么是 1500 而不是 100
//!
//! [`crate::group_admin::RETCODE_ACTION_UNSUPPORTED`]（`100`）留给「Go 也明确回 100」的两个写操作
//! （`set_group_whole_ban` / `set_group_card`）。本模块的 action 在 Go 里**根本没有实现**，
//! 与 [`crate::users`] 的同名项同一口径：`1500` + 一句说明，不伪造空数据。
//!
//! # 参数先于 1500
//!
//! 参数形状仍按 OneBot 契约校验：`group_id` / `user_id` / 必填文本缺失或非法 → `1400`。
//! 这样客户端能区分「参数写错」和「功能没有」，也不必为一个不存在的功能去猜 ID 格式。
//! `set_group_admin` 的 `enable`、`set_group_special_title` 的 `duration` 这类可选参数同样校验
//! 类型——静默吞掉非法值会让客户端以为设置生效了。
//!
//! # 唯一有本地结果的是 `get_online_clients`
//!
//! Go 的实现不触达上游：固定回 `clients: []` + `tiny_id: 0`（见 `handlers/get_online_clients.go`）。
//! 本 crate 的 [`crate::qqapi::ActionContext`] 不持有 WS 客户端集合，无法给出真实列表，因此逐字段
//! 照抄 Go 的空回执——这是 Go 的既定语义，不是「查不到就编空」。
//!
//! # 参数校验与 `1500` 的小工具
//!
//! [`checked_unsupported`] / [`required_text`] / [`required_object`] / [`required_add_del_op`]
//! 供 [`crate::panels`] 与 [`crate::interaction`] 里同样「端点未落地」的 action 复用，避免三份
//! 拷贝。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use gensokyo_core::{Error, Result};
use gensokyo_protocol::action::{ActionRequest, ActionResponse};
use gensokyo_protocol::{Handler, HandlerFuture};

use crate::params;
use crate::util::{RETCODE_UPSTREAM, bad_request, failed, ok_data, params_object, params_or_empty};

/// `get_group_at_all_remain` 的说明。
const AT_ALL_REMAIN: &str = "get_group_at_all_remain: QQ 官方未提供 @全体成员剩余次数接口";
/// `set_group_name` 的说明。
const SET_GROUP_NAME: &str = "set_group_name: QQ 官方未提供修改群名称接口";
/// `set_group_admin` 的说明。
const SET_GROUP_ADMIN: &str = "set_group_admin: QQ 官方未提供设置 / 取消群管理员接口";
/// `set_group_special_title` 的说明。
const SET_GROUP_SPECIAL_TITLE: &str = "set_group_special_title: QQ 官方未提供群头衔接口";
/// `set_group_remark` 的说明。
const SET_GROUP_REMARK: &str = "set_group_remark: QQ 官方未提供设置群备注接口";
/// `set_qq_profile` 的说明。
const SET_QQ_PROFILE: &str = "set_qq_profile: QQ 官方未提供修改机器人资料接口";
/// `get_qq_profile` 的说明。
const GET_QQ_PROFILE: &str = "get_qq_profile: gensokyo-qq 尚未暴露机器人资料端点";
/// `get_group_system_msg` 的说明。
const GET_GROUP_SYSTEM_MSG: &str = "get_group_system_msg: QQ 官方未提供群系统消息接口";

/// `get_online_clients` 的响应数据（逐字段对齐 Go `OnlineClientsData`）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OnlineClientsData {
    /// 在线客户端列表；本 crate 不持有 WS 客户端集合，恒为空数组。
    pub clients: Vec<Value>,
    /// 小世界 ID；官方机器人无此概念，恒为 0。
    pub tiny_id: i64,
}

/// 取 `params` 并校验群 ID 必填。
pub(crate) fn group_params(request: &ActionRequest<Value>) -> Result<&Map<String, Value>> {
    let object = params_object(request)?;
    params::required_id(object, "group_id")?;
    Ok(object)
}

/// 取 `params` 并校验成员 ID 必填（`user_id`，OneBot 标准字段名）。
pub(crate) fn group_member_params(request: &ActionRequest<Value>) -> Result<&Map<String, Value>> {
    let object = group_params(request)?;
    params::required_id(object, "user_id")?;
    Ok(object)
}

/// 必填文本字段：缺失 / `null` / 空串 → 参数错误。
pub(crate) fn required_text(object: &Map<String, Value>, key: &str) -> Result<String> {
    params::optional_text(object, key)?.ok_or_else(|| Error::Protocol(format!("缺少 {key}")))
}

/// 必填 JSON 对象字段（`panel` / `menu` / `context` / `operation`）。
pub(crate) fn required_object<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a Map<String, Value>> {
    match object.get(key) {
        Some(Value::Object(value)) => Ok(value),
        None | Some(Value::Null) => Err(Error::Protocol(format!("缺少 {key}"))),
        Some(_) => Err(Error::Protocol(format!("{key} 必须是 JSON 对象"))),
    }
}

/// `op` 只允许 `add` / `del`（面板关联对象、成员黑名单共用）。
pub(crate) fn required_add_del_op(object: &Map<String, Value>) -> Result<()> {
    match params::optional_text(object, "op")?.as_deref() {
        Some("add" | "del") => Ok(()),
        Some(other) => Err(Error::Protocol(format!("op 只支持 add / del: {other}"))),
        None => Err(Error::Protocol("缺少 op".to_owned())),
    }
}

/// `1500` + 固定说明。
pub(crate) fn unsupported(request: &ActionRequest<Value>, message: &str) -> ActionResponse<Value> {
    failed(request, RETCODE_UPSTREAM, message)
}

/// 参数校验（`# 1400`）通过后回 `1500` 的公共流程：先校验再报「功能不存在」。
///
/// `params` 缺失 / `null` 视为空对象（与 [`params_or_empty`] 同口径），供
/// [`crate::panels`] 与 [`crate::interaction`] 中「端点未落地」的 action 共用。
pub(crate) fn checked_unsupported(
    request: &ActionRequest<Value>,
    message: &str,
    validate: fn(&Map<String, Value>) -> Result<()>,
) -> Result<ActionResponse<Value>> {
    let object = match params_or_empty(request) {
        Ok(object) => object,
        Err(err) => return Ok(bad_request(request, &err)),
    };
    match validate(&object) {
        Ok(()) => Ok(unsupported(request, message)),
        Err(err) => Ok(bad_request(request, &err)),
    }
}

/// `get_group_at_all_remain`。
#[derive(Debug, Clone, Copy, Default)]
pub struct GetGroupAtAllRemainHandler;

impl GetGroupAtAllRemainHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        if let Err(err) = group_params(request) {
            return Ok(bad_request(request, &err));
        }
        Ok(unsupported(request, AT_ALL_REMAIN))
    }
}

impl Handler for GetGroupAtAllRemainHandler {
    fn name(&self) -> &'static str {
        "get_group_at_all_remain"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `set_group_name`。
#[derive(Debug, Clone, Copy, Default)]
pub struct SetGroupNameHandler;

impl SetGroupNameHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = match group_params(request) {
            Ok(object) => object,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        if let Err(err) = required_text(object, "group_name") {
            return Ok(bad_request(request, &err));
        }
        Ok(unsupported(request, SET_GROUP_NAME))
    }
}

impl Handler for SetGroupNameHandler {
    fn name(&self) -> &'static str {
        "set_group_name"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `set_group_admin`（`enable` 决定设置还是取消）。
#[derive(Debug, Clone, Copy, Default)]
pub struct SetGroupAdminHandler;

impl SetGroupAdminHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = match group_member_params(request) {
            Ok(object) => object,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        if let Err(err) = params::optional_bool(object, "enable") {
            return Ok(bad_request(request, &err));
        }
        Ok(unsupported(request, SET_GROUP_ADMIN))
    }
}

impl Handler for SetGroupAdminHandler {
    fn name(&self) -> &'static str {
        "set_group_admin"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `set_group_special_title`。
#[derive(Debug, Clone, Copy, Default)]
pub struct SetGroupSpecialTitleHandler;

impl SetGroupSpecialTitleHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = match group_member_params(request) {
            Ok(object) => object,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        if let Err(err) = params::optional_text(object, "special_title") {
            return Ok(bad_request(request, &err));
        }
        if let Err(err) = params::optional_i64(object, "duration") {
            return Ok(bad_request(request, &err));
        }
        Ok(unsupported(request, SET_GROUP_SPECIAL_TITLE))
    }
}

impl Handler for SetGroupSpecialTitleHandler {
    fn name(&self) -> &'static str {
        "set_group_special_title"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `set_group_remark`。
#[derive(Debug, Clone, Copy, Default)]
pub struct SetGroupRemarkHandler;

impl SetGroupRemarkHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = match group_params(request) {
            Ok(object) => object,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        if let Err(err) = required_text(object, "remark") {
            return Ok(bad_request(request, &err));
        }
        Ok(unsupported(request, SET_GROUP_REMARK))
    }
}

impl Handler for SetGroupRemarkHandler {
    fn name(&self) -> &'static str {
        "set_group_remark"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `set_qq_profile`。
#[derive(Debug, Clone, Copy, Default)]
pub struct SetQqProfileHandler;

impl SetQqProfileHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        // 字段（nickname / company / email …）全是可选项，没有「缺参数」可言，只校验 params 形状
        if let Err(err) = params_or_empty(request) {
            return Ok(bad_request(request, &err));
        }
        Ok(unsupported(request, SET_QQ_PROFILE))
    }
}

impl Handler for SetQqProfileHandler {
    fn name(&self) -> &'static str {
        "set_qq_profile"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `get_qq_profile`。
#[derive(Debug, Clone, Copy, Default)]
pub struct GetQqProfileHandler;

impl GetQqProfileHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        if let Err(err) = params_or_empty(request) {
            return Ok(bad_request(request, &err));
        }
        Ok(unsupported(request, GET_QQ_PROFILE))
    }
}

impl Handler for GetQqProfileHandler {
    fn name(&self) -> &'static str {
        "get_qq_profile"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `get_group_system_msg`。
#[derive(Debug, Clone, Copy, Default)]
pub struct GetGroupSystemMsgHandler;

impl GetGroupSystemMsgHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        if let Err(err) = params_or_empty(request) {
            return Ok(bad_request(request, &err));
        }
        Ok(unsupported(request, GET_GROUP_SYSTEM_MSG))
    }
}

impl Handler for GetGroupSystemMsgHandler {
    fn name(&self) -> &'static str {
        "get_group_system_msg"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `get_online_clients`：Go 的本地固定回执。
#[derive(Debug, Clone, Copy, Default)]
pub struct GetOnlineClientsHandler;

impl GetOnlineClientsHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        if let Err(err) = params_or_empty(request) {
            return Ok(bad_request(request, &err));
        }
        ok_data(request, &OnlineClientsData::default())
    }
}

impl Handler for GetOnlineClientsHandler {
    fn name(&self) -> &'static str {
        "get_online_clients"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        GetGroupAtAllRemainHandler, GetGroupSystemMsgHandler, GetOnlineClientsHandler,
        GetQqProfileHandler, SetGroupAdminHandler, SetGroupNameHandler, SetGroupRemarkHandler,
        SetGroupSpecialTitleHandler, SetQqProfileHandler,
    };
    use crate::handler::RETCODE_BAD_REQUEST;
    use crate::testing::{block_on, failed, ok_data, request, request_without_params};
    use crate::util::RETCODE_UPSTREAM;
    use gensokyo_protocol::Handler;
    use gensokyo_protocol::Status;

    const GROUP: &str = "0123456789abcdef0123456789abcdef";
    const MEMBER: &str = "fedcba9876543210fedcba9876543210";

    /// 断言 `1500` + 带说明的失败响应。
    fn assert_unsupported(handler: &dyn Handler, params: serde_json::Value) {
        let response = block_on(handler.handle(&request(handler.name(), params)))
            .expect("未实现端点也应是 Ok(失败响应)");
        let (status, retcode, data) = failed(response);
        assert_eq!(status, Status::Failed);
        assert_eq!(retcode, RETCODE_UPSTREAM);
        assert_eq!(data, None);
    }

    /// 断言 `1400`。
    fn assert_bad_request(handler: &dyn Handler, params: serde_json::Value) {
        let response = block_on(handler.handle(&request(handler.name(), params)))
            .expect("参数错误也应是 Ok(失败响应)");
        let (status, retcode, data) = failed(response);
        assert_eq!(status, Status::Failed);
        assert_eq!(retcode, RETCODE_BAD_REQUEST);
        assert_eq!(data, None);
    }

    #[test]
    fn at_all_remain_is_unsupported_but_checks_group_id() {
        let handler = GetGroupAtAllRemainHandler::new();
        assert_unsupported(&handler, json!({"group_id": GROUP}));
        assert_bad_request(&handler, json!({}));
        assert_eq!(handler.name(), "get_group_at_all_remain");
    }

    #[test]
    fn group_name_requires_group_id_and_name() {
        let handler = SetGroupNameHandler::new();
        assert_unsupported(&handler, json!({"group_id": GROUP, "group_name": "新名字"}));
        assert_bad_request(&handler, json!({"group_id": GROUP}));
        assert_bad_request(&handler, json!({"group_name": "新名字"}));
        assert_eq!(handler.name(), "set_group_name");
    }

    #[test]
    fn group_admin_requires_member_and_checks_enable() {
        let handler = SetGroupAdminHandler::new();
        assert_unsupported(
            &handler,
            json!({"group_id": GROUP, "user_id": MEMBER, "enable": true}),
        );
        assert_bad_request(&handler, json!({"group_id": GROUP, "enable": true}));
        assert_bad_request(
            &handler,
            json!({"group_id": GROUP, "user_id": MEMBER, "enable": "yes"}),
        );
        assert_eq!(handler.name(), "set_group_admin");
    }

    #[test]
    fn special_title_requires_member_and_checks_type() {
        let handler = SetGroupSpecialTitleHandler::new();
        assert_unsupported(
            &handler,
            json!({"group_id": GROUP, "user_id": MEMBER, "special_title": "头衔", "duration": -1}),
        );
        assert_bad_request(&handler, json!({"group_id": GROUP}));
        assert_bad_request(
            &handler,
            json!({"group_id": GROUP, "user_id": MEMBER, "duration": 1.5}),
        );
        assert_eq!(handler.name(), "set_group_special_title");
    }

    #[test]
    fn group_remark_requires_remark() {
        let handler = SetGroupRemarkHandler::new();
        assert_unsupported(&handler, json!({"group_id": GROUP, "remark": "备注"}));
        assert_bad_request(&handler, json!({"group_id": GROUP}));
        assert_eq!(handler.name(), "set_group_remark");
    }

    #[test]
    fn profile_actions_are_unsupported_without_params() {
        let set = SetQqProfileHandler::new();
        assert_unsupported(&set, json!({"nickname": "bot"}));
        assert_eq!(set.name(), "set_qq_profile");

        let get = GetQqProfileHandler::new();
        assert_unsupported(&get, json!({}));
        assert_eq!(get.name(), "get_qq_profile");

        let system = GetGroupSystemMsgHandler::new();
        assert_unsupported(&system, json!({}));
        assert_eq!(system.name(), "get_group_system_msg");
    }

    #[test]
    fn online_clients_returns_the_go_fixed_receipt() {
        let handler = GetOnlineClientsHandler::new();
        for req in [
            request_without_params("get_online_clients"),
            request("get_online_clients", json!({})),
        ] {
            let response = block_on(handler.handle(&req)).expect("本地固定回执不应失败");
            assert_eq!(ok_data(response), json!({"clients": [], "tiny_id": 0}));
        }
        assert_eq!(handler.name(), "get_online_clients");
    }

    #[test]
    fn non_object_params_fail_with_1400() {
        for handler in [
            &GetQqProfileHandler::new() as &dyn Handler,
            &GetGroupSystemMsgHandler::new(),
            &GetOnlineClientsHandler::new(),
            &SetQqProfileHandler::new(),
        ] {
            let req = request(handler.name(), json!([]));
            let response = block_on(handler.handle(&req)).expect("参数错误也应是 Ok(失败响应)");
            assert_eq!(failed(response).1, RETCODE_BAD_REQUEST);
        }
    }
}
