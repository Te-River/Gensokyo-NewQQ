//! 群管理 action：`set_group_whole_ban` / `set_group_card` / `set_group_kick` /
//! `set_group_leave` / `get_group_list`。
//!
//! # 覆盖范围
//!
//! | action | Go 对应 | 上游端点 | 结果 |
//! |--------|---------|----------|------|
//! | `set_group_whole_ban`（别名 `get_group_whole_ban`） | `handlers/set_group_whole_ban.go` | 官方无全员禁言写入口 | 固定 `retcode = 100` |
//! | `set_group_card` | `handlers/set_group_card.go` | 官方无设置群名片接口 | 固定 `retcode = 100` |
//! | `set_group_kick` | `handlers/set_group_kick.go` | `batch_remove_members` | 端点未在 `gensokyo-qq` 落地 → `1500` |
//! | `set_group_leave` | 无（Go 未注册） | 官方无机器人退群端点 | `1500` |
//! | `get_group_list` | `handlers/get_group_list.go` | 官方无群列表端点 | `1500` |
//!
//! `get_group_ban` 不在这里：它和 Go 一样是 `set_group_ban` 的旧 action 名，别名表
//! （[`gensokyo_protocol::registry::ACTION_ALIASES`]）直接指向
//! [`crate::group::SetGroupBanHandler`]。
//!
//! # 固定 `retcode = 100` 的两个 action
//!
//! `set_group_whole_ban` 与 `set_group_card` 是**故意失败**的：官方 v2 的
//! `POST /restrict_chat_setting` 请求体只有按成员的 `members`，没有全员禁言写入字段；群名片更是
//! 完全没有接口。Go 侧为此返回 `retcode = 100` + 说明（`sendActionResult(..., 100)`），这里逐字
//! 沿用同一口径（[`RETCODE_ACTION_UNSUPPORTED`]），不伪装成功。
//!
//! # 刻意的差异（相对 Go）
//!
//! - **参数错误统一 1400**：Go 的 `set_group_whole_ban` / `set_group_kick` 在 ID 反查失败时也回
//!   `retcode = 100`；这里只有「操作本身不受官方支持」才是 100，参数与 ID 反查失败按全 crate
//!   口径回 1400，避免客户端把「ID 写错」读成「功能不支持」。
//! - **`set_group_whole_ban` 不查 idmap**：Go 先反查 `group_openid` 再固定失败；这个 action 没有
//!   任何写入路径，反查只会把「ID 没绑定」变成额外的失败原因，因此这里只校验 `group_id` 必填。
//! - **`set_group_kick` 不跳过反查失败的成员**：Go 逐个反查、失败的成员跳过并继续整批；这里整条
//!   请求 1400 并指出是哪个 ID，避免静默少踢。
//! - **`set_group_kick` 的批量上限照抄 20**（Go `dedupeTruncateUserIDs`）：`user_id` 与 `user_ids`
//!   合并、去重保序，超出的后半段截断。
//! - **`set_group_card` 不解析参数**：与 Go 一致，无论参数如何都回固定失败回执。

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use gensokyo_core::{Error, Ob11Id, Result};
use gensokyo_protocol::action::{ActionRequest, ActionResponse, GroupInfo};
use gensokyo_protocol::{Extra, Handler, HandlerFuture};

use crate::params;
use crate::qqapi::ActionContext;
use crate::util::{
    bad_request, failed, ob11_id, ok_data, params_object, params_or_empty, resolve_group,
    resolve_identity_text, upstream_error, virtual_id_text,
};

/// Go 的「功能不受官方支持」失败回执（`sendActionResult(..., 100)`）。
pub const RETCODE_ACTION_UNSUPPORTED: i32 = 100;

/// `set_group_whole_ban` 的固定说明（逐字对齐 Go `errWholeBanUnsupported`）。
pub const WHOLE_BAN_MESSAGE: &str = "官方未提供全员禁言写入字段，仅支持按成员禁言";

/// `set_group_card` 的固定说明（逐字对齐 Go `SetGroupCard`）。
pub const GROUP_CARD_MESSAGE: &str = "QQ官方API未提供设置群名片接口,set_group_card无法实现";

/// 官方批量移出单批上限（Go `dedupeTruncateUserIDs`）。
pub const MAX_KICK_BATCH: usize = 20;

/// `set_group_kick` 的响应数据（对齐 Go 的 `remove_members_result` + `kicked`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KickData {
    /// 请求里的虚拟号（去重保序后的实际提交集合）。
    pub kicked: Vec<Ob11Id>,
    /// 上游批量移出结果。
    pub remove_members_result: String,
    /// 拉黑失败的成员 OpenID。
    pub add_to_member_blacklist_fail_openids: Vec<String>,
}

/// `set_group_whole_ban`（别名 `get_group_whole_ban`）。
///
/// 不持有 [`ActionContext`]：固定失败回执既不需要上游也没有 idmap 语义（官方无写入路径）。
#[derive(Debug, Clone, Copy, Default)]
pub struct SetGroupWholeBanHandler;

impl SetGroupWholeBanHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = match params_object(request) {
            Ok(object) => object,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        if let Err(err) = params::required_id(object, "group_id") {
            return Ok(bad_request(request, &err));
        }
        // 参数形状仍按 OneBot 契约校验（`enable` 必须是布尔）；官方无写入路径，值本身用不到
        if let Err(err) = params::optional_bool(object, "enable") {
            return Ok(bad_request(request, &err));
        }
        Ok(failed(
            request,
            RETCODE_ACTION_UNSUPPORTED,
            WHOLE_BAN_MESSAGE,
        ))
    }
}

impl Handler for SetGroupWholeBanHandler {
    fn name(&self) -> &'static str {
        "set_group_whole_ban"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `set_group_card`（官方无接口，固定回 `retcode = 100`）。
///
/// 不持有 [`ActionContext`]：与 Go 一致，不解析参数也不触达上游。
#[derive(Debug, Clone, Copy, Default)]
pub struct SetGroupCardHandler;

impl SetGroupCardHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        // 与 Go 一致：不解析参数、不区分群聊私聊，直接给固定失败回执
        Ok(failed(
            request,
            RETCODE_ACTION_UNSUPPORTED,
            GROUP_CARD_MESSAGE,
        ))
    }
}

impl Handler for SetGroupCardHandler {
    fn name(&self) -> &'static str {
        "set_group_card"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `set_group_kick`（批量移出，单批 ≤ 20）。
#[derive(Debug, Clone)]
pub struct SetGroupKickHandler {
    context: Arc<ActionContext>,
}

impl SetGroupKickHandler {
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
        let virtual_ids = match collect_user_ids(object) {
            Ok(ids) if ids.is_empty() => {
                return Ok(bad_request(
                    request,
                    &Error::Protocol("user_id 或 user_ids 至少提供一个".to_owned()),
                ));
            }
            Ok(ids) => ids,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let add_blacklist = match add_blacklist(object) {
            Ok(add_blacklist) => add_blacklist,
            Err(err) => return Ok(bad_request(request, &err)),
        };

        let mut openids = Vec::with_capacity(virtual_ids.len());
        for virtual_id in &virtual_ids {
            match resolve_identity_text(&self.context, "user_id", virtual_id, "member_openid") {
                Ok(resolved) => openids.push(resolved.openid),
                Err(err) => return Ok(bad_request(request, &err)),
            }
        }

        let result = match self
            .context
            .api()
            .set_group_kick(&group.openid, &openids, add_blacklist)
            .await
        {
            Ok(result) => result,
            Err(err) => return Ok(upstream_error(request, &err)),
        };

        let string_mode = self.context.string_ob11();
        let data = KickData {
            kicked: virtual_ids
                .iter()
                .map(|virtual_id| ob11_id(virtual_id, string_mode))
                .collect(),
            remove_members_result: result.remove_members_result,
            add_to_member_blacklist_fail_openids: result.add_to_member_blacklist_fail_openids,
        };
        ok_data(request, &data)
    }
}

impl Handler for SetGroupKickHandler {
    fn name(&self) -> &'static str {
        "set_group_kick"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `set_group_leave`（机器人退群）。
#[derive(Debug, Clone)]
pub struct SetGroupLeaveHandler {
    context: Arc<ActionContext>,
}

impl SetGroupLeaveHandler {
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
        // `is_dismiss` 只对群主形态有意义；官方没有退群端点，这里仅校验形状
        if let Err(err) = params::optional_bool(object, "is_dismiss") {
            return Ok(bad_request(request, &err));
        }

        if let Err(err) = self.context.api().set_group_leave(&group.openid).await {
            return Ok(upstream_error(request, &err));
        }
        // OneBot 的 set_group_leave 成功回执没有业务数据
        Ok(ActionResponse::ok(Value::Null).with_echo(request.echo.clone()))
    }
}

impl Handler for SetGroupLeaveHandler {
    fn name(&self) -> &'static str {
        "set_group_leave"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `get_group_list`。
#[derive(Debug, Clone)]
pub struct GetGroupListHandler {
    context: Arc<ActionContext>,
}

impl GetGroupListHandler {
    /// 构造。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        // 没有必填参数（`no_cache` 官方无对应语义），但仍要拒绝非对象的 params
        if let Err(err) = params_or_empty(request) {
            return Ok(bad_request(request, &err));
        }
        let groups = match self.context.api().get_group_list().await {
            Ok(groups) => groups,
            Err(err) => return Ok(upstream_error(request, &err)),
        };

        let string_mode = self.context.string_ob11();
        let data: Vec<GroupInfo> = groups
            .iter()
            .map(|group| {
                let virtual_id = virtual_id_text(&self.context, &group.group_openid);
                let mut extra = Extra::new();
                extra.insert("group_memo".to_owned(), json!(group.group_finger_memo));
                extra.insert("group_class_text".to_owned(), json!(group.group_class_text));
                extra.insert("group_tags".to_owned(), json!(group.group_tags));
                // Go 的群列表还带两个官方没有的字段：诚实置 0，不编造
                extra.insert("group_create_time".to_owned(), json!(0));
                extra.insert("group_level".to_owned(), json!(0));
                GroupInfo {
                    group_id: ob11_id(&virtual_id, string_mode),
                    group_name: group.group_name.clone(),
                    member_count: i64::try_from(group.group_member_num).unwrap_or(i64::MAX),
                    max_member_count: 0,
                    extra,
                }
            })
            .collect();
        ok_data(request, &data)
    }
}

impl Handler for GetGroupListHandler {
    fn name(&self) -> &'static str {
        "get_group_list"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// 合并 `user_id` + `user_ids`：去重保序，截断到官方单批上限。
fn collect_user_ids(object: &Map<String, Value>) -> Result<Vec<String>> {
    let mut collected = Vec::new();
    if let Some(single) = params::optional_text(object, "user_id")? {
        collected.push(single);
    }
    collected.extend(params::id_list(object, "user_ids")?);

    // 单批 ≤ 20，线性查重比建 HashSet 更省
    let mut unique: Vec<String> = Vec::with_capacity(collected.len());
    for id in collected {
        if id.is_empty() || unique.contains(&id) {
            continue;
        }
        unique.push(id);
    }
    unique.truncate(MAX_KICK_BATCH);
    Ok(unique)
}

/// `add_blacklist`（Go 参数名）优先，兼容 OneBot 标准名 `reject_add_request`。
fn add_blacklist(object: &Map<String, Value>) -> Result<bool> {
    if let Some(value) = params::optional_bool(object, "add_blacklist")? {
        return Ok(value);
    }
    Ok(params::optional_bool(object, "reject_add_request")?.unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        GROUP_CARD_MESSAGE, GetGroupListHandler, MAX_KICK_BATCH, RETCODE_ACTION_UNSUPPORTED,
        SetGroupCardHandler, SetGroupKickHandler, SetGroupLeaveHandler, SetGroupWholeBanHandler,
        WHOLE_BAN_MESSAGE,
    };
    use crate::handler::RETCODE_BAD_REQUEST;
    use crate::qqapi::KickResult;
    use crate::testing::{
        Call, block_on, context, failed, ok_data, request, request_without_params,
    };
    use crate::util::RETCODE_UPSTREAM;
    use gensokyo_core::Error;
    use gensokyo_protocol::Handler;
    use gensokyo_protocol::Status;
    use gensokyo_qq::GroupInfo as QqGroupInfo;

    const GROUP: &str = "0123456789abcdef0123456789abcdef";
    const MEMBER: &str = "fedcba9876543210fedcba9876543210";
    const MEMBER2: &str = "aaaabbbbccccddddeeeeffff00001111";

    #[test]
    fn whole_ban_returns_the_go_failure_receipt() {
        let (_context, api) = context();
        let handler = SetGroupWholeBanHandler::new();
        for params in [
            json!({"group_id": GROUP, "enable": true}),
            json!({"group_id": GROUP}),
        ] {
            let response = block_on(handler.handle(&request("set_group_whole_ban", params)))
                .expect("固定失败回执也应是 Ok(失败响应)");
            let (status, retcode, data) = failed(response);
            assert_eq!(status, Status::Failed);
            assert_eq!(retcode, RETCODE_ACTION_UNSUPPORTED);
            assert_eq!(data, None);
        }
        assert!(api.calls().is_empty(), "全员禁言不应触达上游");
        assert_eq!(handler.name(), "set_group_whole_ban");
        assert!(WHOLE_BAN_MESSAGE.contains("全员禁言"));
    }

    #[test]
    fn whole_ban_rejects_missing_group_or_bad_enable_with_1400() {
        let (_context, api) = context();
        let handler = SetGroupWholeBanHandler::new();
        for params in [json!({}), json!({"group_id": GROUP, "enable": "yes"})] {
            let response = block_on(handler.handle(&request("set_group_whole_ban", params)))
                .expect("参数错误也应是 Ok(失败响应)");
            let (status, retcode, data) = failed(response);
            assert_eq!(status, Status::Failed);
            assert_eq!(retcode, RETCODE_BAD_REQUEST);
            assert_eq!(data, None);
        }
        assert!(api.calls().is_empty());
    }

    #[test]
    fn group_card_always_fails_with_100_even_without_params() {
        let (_context, api) = context();
        let handler = SetGroupCardHandler::new();
        let response = block_on(handler.handle(&request_without_params("set_group_card")))
            .expect("固定失败回执也应是 Ok(失败响应)");
        let (status, retcode, data) = failed(response);
        assert_eq!(status, Status::Failed);
        assert_eq!(retcode, RETCODE_ACTION_UNSUPPORTED);
        assert_eq!(data, None);
        assert!(api.calls().is_empty());
        assert_eq!(handler.name(), "set_group_card");
        assert!(GROUP_CARD_MESSAGE.contains("群名片"));
    }

    #[test]
    fn kick_forwards_deduped_openids_and_echoes_kicked() {
        // 测试上下文没有 idmap：虚拟号会被 1400 拒绝，所以这里全部用 32 位原生 OpenID
        let (context, api) = context();
        *api.kick.lock().unwrap() = Some(Ok(KickResult {
            remove_members_result: "ok".to_owned(),
            add_to_member_blacklist_fail_openids: vec![MEMBER.to_owned()],
        }));
        let handler = SetGroupKickHandler::new(context);
        let response = block_on(handler.handle(&request(
            "set_group_kick",
            json!({
                "group_id": GROUP,
                "user_id": MEMBER,
                "user_ids": [MEMBER2, MEMBER, MEMBER2],
                "add_blacklist": true,
            }),
        )))
        .expect("handler 不应返回 Err");
        let data = ok_data(response);

        assert_eq!(
            api.calls(),
            vec![Call::Kick(
                GROUP.to_owned(),
                vec![MEMBER.to_owned(), MEMBER2.to_owned()],
                true,
            )],
            "user_id 与 user_ids 合并去重、顺序保持"
        );
        assert_eq!(data["kicked"], json!([MEMBER, MEMBER2]));
        assert_eq!(data["remove_members_result"], json!("ok"));
        assert_eq!(
            data["add_to_member_blacklist_fail_openids"],
            json!([MEMBER])
        );
        assert_eq!(handler.name(), "set_group_kick");
    }

    #[test]
    fn kick_truncates_to_the_official_batch_limit() {
        let (context, api) = context();
        *api.kick.lock().unwrap() = Some(Ok(KickResult::default()));
        let ids: Vec<String> = (1..=25).map(|index| format!("{index:032}")).collect();
        let handler = SetGroupKickHandler::new(context);
        let response = block_on(handler.handle(&request(
            "set_group_kick",
            json!({"group_id": GROUP, "user_ids": ids}),
        )))
        .expect("handler 不应返回 Err");
        let _ = ok_data(response);

        let calls = api.calls();
        let Call::Kick(_, openids, add_blacklist) = &calls[0] else {
            panic!("应记录一次 Kick 调用: {calls:?}");
        };
        assert_eq!(openids.len(), MAX_KICK_BATCH);
        assert_eq!(openids[0], format!("{:032}", 1));
        assert!(!*add_blacklist, "默认不拉黑");
    }

    #[test]
    fn kick_requires_users_and_group() {
        let (context, api) = context();
        let handler = SetGroupKickHandler::new(context);
        for params in [
            json!({"group_id": GROUP}),
            json!({"group_id": GROUP, "user_ids": []}),
            json!({"group_id": GROUP, "user_id": "42"}),
            json!({"user_id": MEMBER}),
        ] {
            let response = block_on(handler.handle(&request("set_group_kick", params)))
                .expect("参数错误也应是 Ok(失败响应)");
            let (status, retcode, _) = failed(response);
            assert_eq!(status, Status::Failed);
            assert_eq!(retcode, RETCODE_BAD_REQUEST);
        }
        assert!(api.calls().is_empty());
    }

    #[test]
    fn kick_unsupported_endpoint_maps_to_1500() {
        let (context, api) = context();
        *api.kick.lock().unwrap() = Some(Err(Error::Unsupported(
            "set_group_kick: gensokyo-qq 尚未暴露 batch_remove_members".to_owned(),
        )));
        let handler = SetGroupKickHandler::new(context);
        let response = block_on(handler.handle(&request(
            "set_group_kick",
            json!({"group_id": GROUP, "user_id": MEMBER}),
        )))
        .expect("上游错误也应是 Ok(失败响应)");
        let (status, retcode, data) = failed(response);
        assert_eq!(status, Status::Failed);
        assert_eq!(retcode, RETCODE_UPSTREAM);
        assert_eq!(data, None);
    }

    #[test]
    fn leave_forwards_group_and_returns_null() {
        let (context, api) = context();
        *api.leave.lock().unwrap() = Some(Ok(()));
        let handler = SetGroupLeaveHandler::new(context);
        let response = block_on(handler.handle(&request(
            "set_group_leave",
            json!({"group_id": GROUP, "is_dismiss": false}),
        )))
        .expect("handler 不应返回 Err");
        let data = ok_data(response);

        assert_eq!(data, json!(null));
        assert_eq!(api.calls(), vec![Call::Leave(GROUP.to_owned())]);
        assert_eq!(handler.name(), "set_group_leave");
    }

    #[test]
    fn leave_upstream_error_maps_to_1500() {
        let (context, api) = context();
        *api.leave.lock().unwrap() = Some(Err(Error::Upstream {
            code: 11244,
            message: "无权限".to_owned(),
        }));
        let handler = SetGroupLeaveHandler::new(context);
        let response =
            block_on(handler.handle(&request("set_group_leave", json!({"group_id": GROUP}))))
                .expect("上游错误也应是 Ok(失败响应)");
        let (status, retcode, data) = failed(response);
        assert_eq!(status, Status::Failed);
        assert_eq!(retcode, RETCODE_UPSTREAM);
        assert_eq!(data, None);
    }

    #[test]
    fn group_list_maps_entries_and_adds_go_only_fields() {
        let (context, api) = context();
        *api.group_list.lock().unwrap() = Some(Ok(vec![QqGroupInfo {
            group_openid: GROUP.to_owned(),
            group_name: "测试群".to_owned(),
            group_finger_memo: "简介".to_owned(),
            group_class_text: "游戏".to_owned(),
            group_tags: vec!["a".to_owned()],
            group_member_num: 7,
        }]));
        let handler = GetGroupListHandler::new(context);
        let response = block_on(handler.handle(&request("get_group_list", json!({}))))
            .expect("handler 不应返回 Err");
        let data = ok_data(response);

        assert_eq!(api.calls(), vec![Call::GroupList]);
        assert_eq!(data[0]["group_name"], json!("测试群"));
        assert_eq!(data[0]["member_count"], json!(7));
        assert_eq!(data[0]["max_member_count"], json!(0));
        assert_eq!(data[0]["group_memo"], json!("简介"));
        assert_eq!(data[0]["group_create_time"], json!(0));
        assert_eq!(data[0]["group_level"], json!(0));
        assert_eq!(handler.name(), "get_group_list");
    }

    #[test]
    fn group_list_unsupported_maps_to_1500() {
        let (context, api) = context();
        *api.group_list.lock().unwrap() = Some(Err(Error::Unsupported(
            "get_group_list: 官方无群列表端点，且 idmap 暂无枚举接口".to_owned(),
        )));
        let handler = GetGroupListHandler::new(context);
        let response = block_on(handler.handle(&request_without_params("get_group_list")))
            .expect("上游错误也应是 Ok(失败响应)");
        let (status, retcode, data) = failed(response);
        assert_eq!(status, Status::Failed);
        assert_eq!(retcode, RETCODE_UPSTREAM);
        assert_eq!(data, None);
        assert_eq!(api.calls(), vec![Call::GroupList]);
    }
}
