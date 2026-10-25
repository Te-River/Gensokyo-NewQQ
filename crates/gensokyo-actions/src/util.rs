//! handler 公共管道：params 取值、虚拟号 ⇄ OpenID 反查、响应封装。
//!
//! 群查询、群管理、消息、用户四组 action 共用同一套口径，集中在这里避免四份拷贝：
//!
//! - **ID 口径**：32 位原生 OpenID 直通上游；其余形态视为虚拟号，经
//!   [`gensokyo_storage::IdMapStore`] 反查——与 Go 的 `len(id) == 32` 分支一致。
//!   未装配 idmap 时虚拟号请求一律 `1400`，不做静默降级。
//! - **响应 ID**：`group_id` / `user_id` 回填请求值（虚拟号），形态由配置 `string_ob11`
//!   决定（[`gensokyo_protocol::parse_id`] 的数字 / 字符串双形态）。
//! - **retcode**：参数错误 `1400`（[`crate::handler::RETCODE_BAD_REQUEST`]），上游失败与
//!   未实现端点 `1500`（[`RETCODE_UPSTREAM`]）。`echo` 一律原样回传。

use std::borrow::Cow;

use serde::Serialize;
use serde_json::{Map, Value};

use gensokyo_core::{Error, Ob11Id, Result};
use gensokyo_protocol::action::{ActionRequest, ActionResponse};

use crate::handler::RETCODE_BAD_REQUEST;
use crate::params;
use crate::qqapi::ActionContext;

/// 上游失败 / 未实现端点（OneBot 语义的 `retcode`），与 `gensokyo_runtime` 的上游错误码同值。
pub const RETCODE_UPSTREAM: i32 = 1500;

/// 32 位 OpenID 的长度（Go `len(id) == 32`）。
const OPENID_LEN: usize = 32;

/// 反查结果：请求里的虚拟号文本 + 真实 OpenID。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedId {
    /// 请求给出的虚拟号 / 文本形态（响应里回填这个）。
    pub(crate) requested: String,
    /// 真实的 32 位 OpenID（上游入参）。
    pub(crate) openid: String,
}

/// 取 `params` 对象；缺失或非对象时返回 1400 用的参数错误。
pub(crate) fn params_object(request: &ActionRequest<Value>) -> Result<&Map<String, Value>> {
    match request.params.as_ref() {
        Some(params) => params::object(params),
        None => Err(Error::Protocol("params 缺失或不是 JSON 对象".to_owned())),
    }
}

/// 取 `params`；缺失 / `null` 视为空对象（无必填参数的 action 用）。
pub(crate) fn params_or_empty(
    request: &ActionRequest<Value>,
) -> Result<Cow<'_, Map<String, Value>>> {
    match request.params.as_ref() {
        None | Some(Value::Null) => Ok(Cow::Owned(Map::new())),
        Some(params) => Ok(Cow::Borrowed(params::object(params)?)),
    }
}

/// 群 ID → 上游 `group_openid`：32 位 OpenID 直通，虚拟号经 idmap 反查。
pub(crate) fn resolve_group(
    context: &ActionContext,
    object: &Map<String, Value>,
) -> Result<ResolvedId> {
    let requested = params::required_id(object, "group_id")?;
    resolve_identity_text(context, "group_id", &requested, "group_openid")
}

/// 成员 ID：`member_openid` 优先（32 位真实 OpenID），否则 `user_id` 按同一套规则反查。
pub(crate) fn resolve_member(
    context: &ActionContext,
    object: &Map<String, Value>,
) -> Result<ResolvedId> {
    if let Some(openid) = params::optional_text(object, "member_openid")? {
        let requested = match params::optional_text(object, "user_id")? {
            Some(user_id) => user_id,
            None => virtual_id_text(context, &openid),
        };
        return Ok(ResolvedId { requested, openid });
    }

    let requested = params::required_id(object, "user_id")?;
    resolve_identity_text(context, "user_id", &requested, "member_openid")
}

/// 任意 ID 文本 → OpenID：32 位直通，数字虚拟号经 idmap 反查。
///
/// `label` 是请求字段名（错误文案），`kind` 是被反查的目标（`group_openid` /
/// `member_openid` / `user_openid`）。
pub(crate) fn resolve_identity_text(
    context: &ActionContext,
    label: &str,
    requested: &str,
    kind: &str,
) -> Result<ResolvedId> {
    if is_openid(requested) {
        return Ok(ResolvedId {
            openid: requested.to_owned(),
            requested: requested.to_owned(),
        });
    }
    let vuin = requested.parse::<i64>().map_err(|_| {
        Error::Protocol(format!("{label} 必须是 32 位 OpenID 或虚拟号: {requested}"))
    })?;
    let store = context
        .idmap()
        .ok_or_else(|| Error::Protocol(format!("未装配 idmap，无法把 {label} 反查成 {kind}")))?;
    let openid = store
        .retrieve_row_by_id(vuin)
        .map_err(|err| Error::Protocol(format!("反查 {kind} 失败: {err}")))?;
    if openid.is_empty() {
        return Err(Error::Protocol(format!(
            "{label} {requested} 未绑定 {kind}"
        )));
    }
    Ok(ResolvedId {
        requested: requested.to_owned(),
        openid,
    })
}

/// 真实 OpenID → 虚拟号文本；未装配 idmap 或映射失败时回退 `"0"`（真实 ID 仍在附加字段里）。
pub(crate) fn virtual_id_text(context: &ActionContext, openid: &str) -> String {
    let Some(store) = context.idmap() else {
        return "0".to_owned();
    };
    match store.retrieve_virtual_value(openid) {
        Ok((_, vuin)) => vuin.to_string(),
        Err(_) => "0".to_owned(),
    }
}

/// 32 位 OpenID 判定（Go `len(id) == 32`；OpenID 是 ASCII，字符数与字节数一致）。
pub(crate) fn is_openid(raw: &str) -> bool {
    raw.chars().count() == OPENID_LEN
}

/// 虚拟号的线上形态：`string_ob11` 为真时发字符串，否则按 `parse_id` 的数字 / 字符串双形态。
pub(crate) fn ob11_id(text: &str, string_mode: bool) -> Ob11Id {
    if string_mode {
        Ob11Id::Str(text.to_owned())
    } else {
        gensokyo_protocol::parse_id(text)
    }
}

/// 分页条数：未传 / `0` 用默认值，超过上限截断。
pub(crate) fn page_limit(object: &Map<String, Value>, default: u32, max: u32) -> Result<u32> {
    match params::optional_u32(object, "limit")? {
        None | Some(0) => Ok(default),
        Some(limit) => Ok(limit.min(max)),
    }
}

/// 成功响应：`data` 来自序列化后的业务结构。
pub(crate) fn ok_data<T: Serialize>(
    request: &ActionRequest<Value>,
    data: &T,
) -> Result<ActionResponse<Value>> {
    let value = serde_json::to_value(data)
        .map_err(|err| Error::Protocol(format!("响应序列化失败: {err}")))?;
    Ok(ActionResponse::ok(value).with_echo(request.echo.clone()))
}

/// 失败响应（`status = failed` + 指定 `retcode`），`message` 是给人看的说明。
pub(crate) fn failed(
    request: &ActionRequest<Value>,
    retcode: i32,
    message: &str,
) -> ActionResponse<Value> {
    ActionResponse::failed(retcode, message.to_owned()).with_echo(request.echo.clone())
}

/// 参数错误响应（`retcode = 1400`）。
pub(crate) fn bad_request(request: &ActionRequest<Value>, err: &Error) -> ActionResponse<Value> {
    failed(request, RETCODE_BAD_REQUEST, &err.to_string())
}

/// 上游错误 / 未实现端点响应（`retcode = 1500`）。
pub(crate) fn upstream_error(request: &ActionRequest<Value>, err: &Error) -> ActionResponse<Value> {
    failed(request, RETCODE_UPSTREAM, &err.to_string())
}
