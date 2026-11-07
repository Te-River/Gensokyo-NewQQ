//! 群管理 action：`set_group_whole_ban` / `set_group_card` / `set_group_kick` /
//! `set_group_leave` / `get_group_list`。
//!
//! # 覆盖范围
//!
//! | action | Go 对应 | 上游端点 | 结果 |
//! |--------|---------|----------|------|
//! | `set_group_whole_ban`（别名 `get_group_whole_ban`） | `handlers/set_group_whole_ban.go` | 官方无全员禁言写入口 | 固定 `retcode = 100` |
//! | `set_group_card` | `handlers/set_group_card.go` | 官方无设置群名片接口 | 固定 `retcode = 100` |
//! | `set_group_kick` | `handlers/set_group_kick.go` | `POST /v2/groups/{gid}/batch_remove_members` | 真实调用 |
//! | `set_group_leave` | 无（Go 未注册） | 官方无机器人退群端点 | `1500` |
//! | `get_group_list` | `handlers/get_group_list.go` | idmap 注册表枚举 + 逐群 `GET /v2/groups/{gid}/info` | 真实聚合 |
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
//! - **`get_group_list` 的失败群跳过**：Go 单群 `info` 拉取失败（或反查失败）时保留一行
//!   字段全空的记录；这里整个跳过该群并 warn，列表更干净——不展示「存在但一无所知」的行。
//! - **`get_group_list` 两种模式都枚举全部注册群**：Go 数字模式只处理纯数字 section、
//!   string 模式跳过纯数字 section；这里 32 位形态直接当 OpenID、其余按 vuin 反查，
//!   配置切换后旧注册同样可见。
//! - **`get_group_list` 不回捞 uin 前缀形态的注册串**：`idmap_isolation` 开启时 Go 侧写入
//!   的 section 理论上可能是 `"<uin>:<原始串>"`；它既非 32 位也 parse 不成数字，会被当作
//!   反查失败整群跳过（不额外剥前缀再试 `lookup_virtual_value`）。Rust 入站链注册的是
//!   上报原串、不带前缀，该形态只出现在与 Go 共库的隔离部署里，属已知限制。
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
    RETCODE_UPSTREAM, bad_request, failed, is_openid, ob11_id, ok_data, params_object,
    params_or_empty, resolve_group, resolve_identity_text, upstream_error,
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
///
/// 上游没有「列出机器人所在群」的端点，照 Go 的口径聚合：idmap 注册表
/// （`<群>:type = group`，入站群事件写入）枚举已注册群，逐群串行调
/// `GET /v2/groups/{gid}/info`。单群失败（反查、正查或上游）跳过该群并 warn；
/// 没有任何注册群时回空列表，仍是成功。
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
        let Some(store) = self.context.idmap() else {
            return Ok(failed(
                request,
                RETCODE_UPSTREAM,
                "get_group_list: 未装配 idmap，无法枚举已加入的群",
            ));
        };
        // Go 枚举失败只记日志继续（等效空列表）；这里同口径：warn 后回空列表成功
        let sections = match store.find_config_sections("type", "group") {
            Ok(sections) => sections,
            Err(err) => {
                tracing::warn!(error = %err, "get_group_list: idmap 枚举群失败，返回空列表");
                Vec::new()
            }
        };

        let string_mode = self.context.string_ob11();
        let mut data: Vec<GroupInfo> = Vec::new();
        for section in sections {
            // 32 位形态直接是 group_openid，其余按虚拟号反查；失败跳过该群（相对 Go
            // 留空字段行更干净，见模块文档「刻意的差异」）
            let openid = if is_openid(&section) {
                section.clone()
            } else {
                match section
                    .parse::<i64>()
                    .ok()
                    .and_then(|vuin| store.retrieve_row_by_id(vuin).ok())
                {
                    Some(openid) if !openid.is_empty() => openid,
                    _ => {
                        tracing::warn!(group = %section, "get_group_list: 群 OpenID 反查失败，跳过");
                        continue;
                    }
                }
            };
            let info = match self.context.api().get_group_info(&openid).await {
                Ok(info) => info,
                Err(err) => {
                    tracing::warn!(group = %section, error = %err, "get_group_list: 群信息拉取失败，跳过");
                    continue;
                }
            };

            // 群 ID 形态照现有 string_ob11 分支：string 模式回注册时的原串，
            // 数字模式回虚拟号。正查失败整群跳过（不复用 util::virtual_id_text——它
            // 回退 "0" 会产出 group_id = 0 的伪行，与「失败即跳过」口径矛盾）
            let group_id = if string_mode {
                Ob11Id::Str(section)
            } else {
                match store.retrieve_virtual_value(&openid) {
                    Ok((_, vuin)) => Ob11Id::num(vuin),
                    Err(err) => {
                        tracing::warn!(group = %section, error = %err, "get_group_list: 群虚拟号正查失败，跳过");
                        continue;
                    }
                }
            };
            let mut extra = Extra::new();
            extra.insert("group_memo".to_owned(), json!(info.group_finger_memo));
            extra.insert("group_class_text".to_owned(), json!(info.group_class_text));
            extra.insert("group_tags".to_owned(), json!(info.group_tags));
            // Go 的群列表还带两个官方没有的字段：诚实置 0，不编造
            extra.insert("group_create_time".to_owned(), json!(0));
            extra.insert("group_level".to_owned(), json!(0));
            data.push(GroupInfo {
                group_id,
                group_name: info.group_name,
                member_count: i64::try_from(info.group_member_num).unwrap_or(i64::MAX),
                max_member_count: 0,
                extra,
            });
        }
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
pub(crate) fn collect_user_ids(object: &Map<String, Value>) -> Result<Vec<String>> {
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
        Call, block_on, context, context_with_idmap, failed, ok_data, request,
        request_without_params,
    };
    use crate::util::RETCODE_UPSTREAM;
    use gensokyo_core::Error;
    use gensokyo_protocol::Handler;
    use gensokyo_protocol::Status;
    use gensokyo_qq::GroupInfo as QqGroupInfo;
    use gensokyo_storage::{IdMapOptions, IdMapStore};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const GROUP: &str = "0123456789abcdef0123456789abcdef";
    const GROUP2: &str = "bbbbccccddddeeeeffff000011112222";
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

    /// 在系统临时目录的独占子目录里打开 idmap 存储（redb 文件锁禁止同名并发打开，
    /// 计数器保证同一进程内的用例互不撞路径）；返回目录句柄供用例末尾清理。
    fn fresh_store(tag: &str) -> (std::path::PathBuf, Arc<IdMapStore>) {
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("gensokyo-actions-{tag}-{n}"));
        std::fs::create_dir_all(&dir).expect("创建临时存储目录");
        let options = IdMapOptions::new(dir.join("openid-map.redb"), dir.join("msgid-map.redb"))
            .with_app_id("102001234");
        (
            dir,
            Arc::new(IdMapStore::open(options).expect("打开 idmap 存储失败")),
        )
    }

    fn qq_group(openid: &str, name: &str, memo: &str, members: u64) -> QqGroupInfo {
        QqGroupInfo {
            group_openid: openid.to_owned(),
            group_name: name.to_owned(),
            group_finger_memo: memo.to_owned(),
            group_class_text: "游戏".to_owned(),
            group_tags: vec!["tag".to_owned()],
            group_member_num: members,
        }
    }

    #[test]
    fn group_list_aggregates_registered_groups_in_string_mode() {
        let (dir, store) = fresh_store("list");
        // 虚拟号 section：注册键就是上报形态（十进制 vuin）
        let vuin = store.retrieve_virtual_value(GROUP).expect("分配群 vuin").1;
        store
            .write_config(&vuin.to_string(), "type", "group")
            .expect("写注册");
        // 原始 OpenID section：string 模式的上报形态
        store.write_config(GROUP2, "type", "group").expect("写注册");
        // 噪声：同 subtype 不同 value，不得进列表
        store.write_config("999", "type", "user").expect("写注册");

        let (context, api) = context_with_idmap(store, true);
        {
            let mut slot = api.group_info.lock().expect("群信息槽锁未中毒");
            // 排序后 "1" < GROUP2("bbbb…")，队列按此顺序出队
            slot.push_back(Ok(qq_group(GROUP, "测试群", "简介", 7)));
            slot.push_back(Ok(qq_group(GROUP2, "二号群", "备注", 3)));
        }
        let handler = GetGroupListHandler::new(context);
        let response = block_on(handler.handle(&request("get_group_list", json!({}))))
            .expect("handler 不应返回 Err");
        let data = ok_data(response);

        assert_eq!(
            api.calls(),
            vec![
                Call::GroupInfo(GROUP.to_owned()),
                Call::GroupInfo(GROUP2.to_owned()),
            ],
            "串行逐群拉 info：虚拟号反查出 OpenID，32 位形态直接当 OpenID"
        );
        let rows = data.as_array().expect("data 应为数组").clone();
        assert_eq!(rows.len(), 2, "user 噪声键不应进列表");
        // string 模式：group_id 回注册时的原串
        assert_eq!(rows[0]["group_id"], json!(vuin.to_string()));
        assert_eq!(rows[1]["group_id"], json!(GROUP2));
        assert_eq!(rows[0]["group_name"], json!("测试群"));
        assert_eq!(rows[0]["member_count"], json!(7));
        assert_eq!(rows[0]["max_member_count"], json!(0));
        assert_eq!(rows[0]["group_memo"], json!("简介"));
        assert_eq!(rows[0]["group_class_text"], json!("游戏"));
        assert_eq!(rows[0]["group_tags"], json!(["tag"]));
        assert_eq!(rows[0]["group_create_time"], json!(0));
        assert_eq!(rows[0]["group_level"], json!(0));
        assert_eq!(handler.name(), "get_group_list");
        // 释放 handler（内含 store 的 Arc）后删除临时库目录；Windows 上若句柄
        // 释放不及时导致删除失败，静默交由 OS temp 目录治理，不阻塞用例
        drop(handler);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn group_list_numeric_mode_returns_virtual_number() {
        let (dir, store) = fresh_store("numeric");
        let vuin = store.retrieve_virtual_value(GROUP).expect("分配群 vuin").1;
        store
            .write_config(&vuin.to_string(), "type", "group")
            .expect("写注册");

        let (context, api) = context_with_idmap(store, false);
        api.group_info
            .lock()
            .expect("群信息槽锁未中毒")
            .push_back(Ok(qq_group(GROUP, "测试群", "简介", 7)));
        let handler = GetGroupListHandler::new(context);
        let response = block_on(handler.handle(&request("get_group_list", json!({}))))
            .expect("handler 不应返回 Err");
        let data = ok_data(response);

        assert_eq!(api.calls(), vec![Call::GroupInfo(GROUP.to_owned())]);
        // 数字模式：group_id 是 JSON 数字形态的虚拟号
        assert_eq!(data[0]["group_id"], json!(vuin));
        assert_eq!(data[0]["group_name"], json!("测试群"));
        drop(handler);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn group_list_numeric_mode_never_emits_zero_group_id() {
        // 「反查成功但正查失败」用 idmap 公开 API 造不出来：retrieve_virtual_value 是唯一
        // 写身份表的入口，正表/反表/last_seen 在同一写事务里同步落盘。因此这里退化为
        // 形状断言钉口径：数字模式返回的每一行 group_id 都来自成功的正查（恒为正整数
        // 虚拟号），绝不出现 util::virtual_id_text 回退形态的 0 伪行——该回退正是本
        // handler 弃用 virtual_id_text、改为「正查失败整群跳过」的原因。
        let (dir, store) = fresh_store("zero");
        let vuin = store.retrieve_virtual_value(GROUP).expect("分配群 vuin").1;
        // 32 位直通形态也注册一份：与 vuin section 指向同一身份
        store.write_config(GROUP, "type", "group").expect("写注册");
        store
            .write_config(&vuin.to_string(), "type", "group")
            .expect("写注册");
        // GROUP2 不进身份表：数字模式下它经 32 位直通 + 正查分配拿到新虚拟号，而非 0
        store.write_config(GROUP2, "type", "group").expect("写注册");

        let (context, api) = context_with_idmap(store, false);
        {
            let mut slot = api.group_info.lock().expect("群信息槽锁未中毒");
            // 排序后 GROUP("0123…") < "1"(vuin) < GROUP2("bbbb…")：前两次命中同一 OpenID
            slot.push_back(Ok(qq_group(GROUP, "测试群", "简介", 7)));
            slot.push_back(Ok(qq_group(GROUP, "测试群", "简介", 7)));
            slot.push_back(Ok(qq_group(GROUP2, "二号群", "备注", 3)));
        }
        let handler = GetGroupListHandler::new(context);
        let response = block_on(handler.handle(&request("get_group_list", json!({}))))
            .expect("handler 不应返回 Err");
        let data = ok_data(response);

        assert_eq!(
            api.calls(),
            vec![
                Call::GroupInfo(GROUP.to_owned()),
                Call::GroupInfo(GROUP.to_owned()),
                Call::GroupInfo(GROUP2.to_owned()),
            ]
        );
        let rows = data.as_array().expect("data 应为数组").clone();
        assert_eq!(rows.len(), 3);
        for row in &rows {
            assert!(
                row["group_id"].as_i64().is_some_and(|id| id > 0),
                "数字模式不得产出 group_id = 0 的伪行: {row}"
            );
        }
        assert_eq!(rows[0]["group_id"], json!(vuin));
        assert_eq!(
            rows[1]["group_id"],
            json!(vuin),
            "vuin section 反查回同一 OpenID 后正查同值"
        );
        assert_eq!(
            rows[2]["group_id"],
            json!(vuin + 1),
            "未入表的 OpenID 由正查分配下一个虚拟号（独占库内从 1 起分配，结果确定）"
        );
        drop(handler);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn group_list_skips_unresolvable_and_failed_groups() {
        let (dir, store) = fresh_store("skip");
        store.write_config(GROUP, "type", "group").expect("写注册");
        store.write_config(GROUP2, "type", "group").expect("写注册");
        // 非 32 位且非数字的 section：反查不了，跳过且不触达上游
        store.write_config("abc", "type", "group").expect("写注册");

        let (context, api) = context_with_idmap(store, true);
        {
            let mut slot = api.group_info.lock().expect("群信息槽锁未中毒");
            // 排序后 GROUP("0123…") < "abc" < GROUP2("bbbb…")：
            // GROUP 成功、"abc" 反查失败被跳过（不出队）、GROUP2 上游失败被跳过
            slot.push_back(Ok(qq_group(GROUP, "测试群", "简介", 7)));
            slot.push_back(Err(Error::Upstream {
                code: 4009,
                message: "群不存在".to_owned(),
            }));
        }
        let handler = GetGroupListHandler::new(context);
        let response = block_on(handler.handle(&request("get_group_list", json!({}))))
            .expect("上游失败只跳过该群，整体仍成功");
        let data = ok_data(response);

        let rows = data.as_array().expect("data 应为数组").clone();
        assert_eq!(rows.len(), 1, "反查失败与上游失败的群都整行跳过");
        assert_eq!(rows[0]["group_id"], json!(GROUP));
        assert_eq!(
            api.calls(),
            vec![
                Call::GroupInfo(GROUP.to_owned()),
                Call::GroupInfo(GROUP2.to_owned()),
            ],
            "\"abc\" 在触达上游前就被跳过"
        );
        drop(handler);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn group_list_without_idmap_returns_1500() {
        // 测试上下文不带 idmap：枚举无从谈起，明确失败而不是伪装空列表
        let (context, api) = context();
        let handler = GetGroupListHandler::new(context);
        let response = block_on(handler.handle(&request("get_group_list", json!({}))))
            .expect("失败回执也应是 Ok(失败响应)");
        let (status, retcode, data) = failed(response);
        assert_eq!(status, Status::Failed);
        assert_eq!(retcode, RETCODE_UPSTREAM);
        assert_eq!(data, None);
        assert!(api.calls().is_empty(), "未装配 idmap 不应触达上游");
    }

    #[test]
    fn group_list_empty_registry_is_ok_with_empty_array() {
        // 从未有群事件注册：空列表是成功（Go 同口径）
        let (dir, store) = fresh_store("empty");
        let (context, api) = context_with_idmap(store, true);
        let handler = GetGroupListHandler::new(context);
        let response = block_on(handler.handle(&request_without_params("get_group_list")))
            .expect("handler 不应返回 Err");
        assert_eq!(ok_data(response), json!([]));
        assert!(api.calls().is_empty());
        drop(handler);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn group_list_rejects_non_object_params_with_1400() {
        let (context, api) = context();
        let handler = GetGroupListHandler::new(context);
        let response = block_on(handler.handle(&request("get_group_list", json!("nope"))))
            .expect("参数错误也应是 Ok(失败响应)");
        let (status, retcode, data) = failed(response);
        assert_eq!(status, Status::Failed);
        assert_eq!(retcode, RETCODE_BAD_REQUEST);
        assert_eq!(data, None);
        assert!(api.calls().is_empty());
    }
}
