//! OneBot V11 action 请求 / 响应模型（出站方向：OneBot 客户端 → gensokyo）。
//!
//! [`ActionRequest`] / [`ActionResponse`] 保持泛型：handler 注册表用 `serde_json::Value`
//! 兜底，单个 handler 可以自行反序列化成下面的类型化参数结构。参数结构与
//! `handlers/*.go` 的实际取值对齐（含 Gensokyo 扩展字段），未知字段落进 `extra`。
//!
//! 这里只放**模型**，不含任何 QQ API 调用或业务判断（那是 M4 `gensokyo-actions` 的职责）。

use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use gensokyo_core::Ob11Id;

use crate::envelope::{CompactData, EnvelopeKind};
use crate::event::Extra;
use crate::segment::{MessageContent, Segment};

/// 请求回传的 `echo`：OneBot 没限定类型，任意 JSON 原样返回。
///
/// Go 侧把 echo 读进 `interface{}` 再原样回写（`callapi.ActionMessage.Echo`），所以数组 /
/// 对象 / 字符串 / 数字都必须能往返，不能收窄成"字符串或整数"。
pub type Echo = Value;

/// 一条 action 请求。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ActionRequest<P> {
    /// action 名称（含向后兼容别名）。
    pub action: String,
    /// action 参数；部分 action（`get_login_info` 等）允许缺省。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<P>,
    /// 原样回传的 echo。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echo: Option<Echo>,
    /// 顶层 `post_type`（Go `callapi.ActionMessage.PostType`）：只有 `put_interaction`
    /// 读它当互动结果码，其余 action 忽略；`None` 时不序列化，反序列化行为与新增前一致。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_type: Option<Value>,
}

impl<P> ActionRequest<P> {
    /// 构造一条请求。
    #[must_use]
    pub fn new(action: impl Into<String>, params: P) -> Self {
        Self {
            action: action.into(),
            params: Some(params),
            echo: None,
            post_type: None,
        }
    }

    /// 挂上待回传的 echo。
    #[must_use]
    pub fn with_echo(mut self, echo: Option<Echo>) -> Self {
        self.echo = echo;
        self
    }
}

/// 响应状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// 成功。
    Ok,
    /// 失败。
    Failed,
    /// 异步（如 `send_forward_msg`）。
    Async,
}

/// 一条 action 响应。
///
/// **键序 = 字母序**：Go 用 `structToMap` 转 map 再 `json.Marshal`，顶层键按字母序
/// （`handlers/message_parser.go:398-404`），所以序列化（见 [`Serialize`] 的手写实现）按字母序
/// 写键；新增键必须插到正确位置，否则 golden 整串断言会失配。
///
/// 线上键集由 [`ActionResponse::envelope`] 决定：A 发送类 / C 查询类是 [`EnvelopeKind::Full`]
/// （现状），B 管理类是 [`EnvelopeKind::Compact`]（Go `sendActionResult` 形状），
/// `send_private_msg_wakeup` 是 [`EnvelopeKind::GoWakeupOk`]（Go 的字面量回执）。
///
/// 唯一换键名的一处是 `mark_msg_as_read`：Go 的响应结构用 `msg` 而不是 `message`，另带
/// `wording`（见 [`ActionResponse::msg`] / [`ActionResponse::wording`]）。
#[derive(Debug, Clone, Deserialize)]
pub struct ActionResponse<T = ()> {
    /// 业务数据：恒序列化（None 输出 `null`，对齐 OneBot 响应结构，不省略字段）。
    #[serde(default)]
    pub data: Option<T>,
    /// 原样回传的 echo：恒序列化（Go 的 `Echo` 没有 omitempty，无请求回执时是 `null`）。
    #[serde(default)]
    pub echo: Option<Echo>,
    /// 群号（Go `ServerResponse.GroupID`）：numeric 模式是虚拟号数字，string 模式是请求原串。
    ///
    /// `None` 不序列化，模拟 Go 的 `group_id,omitempty`：那边反查失败是 0 被吞掉，这里同样
    /// 不写字段，**绝不伪造 0**。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<Ob11Id>,
    /// 错误信息：恒序列化（Go 的 `Message` 没有 omitempty，成功回执是空串）。
    ///
    /// `mark_msg_as_read` 的响应换用同义的 [`Self::msg`] 键，此时本字段不上线（Go 里两者
    /// 不会同时出现）。
    #[serde(default)]
    pub message: Option<String>,
    /// 顶层 `msg`（Go `MarkThisMessageAsReadAPIResponse.Msg`）：与 [`Self::message`] 同义、
    /// 只是键名不同，只有 `mark_msg_as_read` 用。`None` 不序列化，其他 action 仍写 `message`。
    #[serde(default)]
    pub msg: Option<String>,
    /// 返回码，0 为成功。
    pub retcode: i32,
    /// 状态。
    pub status: Status,
    /// 用户号（Go `ServerResponse.UserID`）：群发送不填；私聊两种模式都是数字虚拟号
    /// （照抄 Go `SendC2CResponse` 的数字结构，见 `handlers/message_parser.go:605-609`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Ob11Id>,
    /// 补充说明（Go `MarkThisMessageAsReadAPIResponse.Wording`，没有 omitempty）：只有
    /// `mark_msg_as_read` 的成功回执带该键（空串）；其余 action 不设、`None` 不序列化。
    #[serde(default)]
    pub wording: Option<String>,
    /// 响应信封模式（见 [`crate::envelope`]）：不上线，由分发器按 action 名设置。
    #[serde(skip)]
    pub envelope: EnvelopeKind,
}

impl<T> ActionResponse<T> {
    /// 成功响应。
    #[must_use]
    pub fn ok(data: T) -> Self {
        Self {
            status: Status::Ok,
            retcode: 0,
            data: Some(data),
            message: None,
            msg: None,
            wording: None,
            echo: None,
            group_id: None,
            user_id: None,
            envelope: EnvelopeKind::Full,
        }
    }

    /// 失败响应。
    #[must_use]
    pub fn failed(retcode: i32, message: impl Into<String>) -> Self {
        Self {
            status: Status::Failed,
            retcode,
            data: None,
            message: Some(message.into()),
            msg: None,
            wording: None,
            echo: None,
            group_id: None,
            user_id: None,
            envelope: EnvelopeKind::Full,
        }
    }

    /// 挂上待回传的 echo。
    #[must_use]
    pub fn with_echo(mut self, echo: Option<Echo>) -> Self {
        self.echo = echo;
        self
    }
}

/// 手写 `Serialize`：同一个结构体按 [`EnvelopeKind`] 输出两套键集（Go 的两套信封），键序恒为
/// 字母序。`derive` 无法表达「同一个字段有时在、有时不在、有时换形状」，所以这里显式写。
impl<T: Serialize> Serialize for ActionResponse<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        // `GoWakeupOk` 只对成功生效：失败仍是普通 Full（`1400` / `1500` 要带 `message`）
        let envelope = match self.envelope {
            EnvelopeKind::GoWakeupOk if self.status == Status::Ok => EnvelopeKind::GoWakeupOk,
            EnvelopeKind::GoWakeupOk => EnvelopeKind::Full,
            other => other,
        };
        match envelope {
            EnvelopeKind::Full => {
                map.serialize_entry("data", &self.data)?;
                map.serialize_entry("echo", &self.echo)?;
                if let Some(group_id) = &self.group_id {
                    map.serialize_entry("group_id", group_id)?;
                }
                match &self.msg {
                    // Go 的 `msg` 与 `message` 互斥（`MarkThisMessageAsReadAPIResponse` 只有
                    // `msg`）：设了 `msg` 就不再写 `message`。
                    Some(msg) => map.serialize_entry("msg", msg)?,
                    None => {
                        map.serialize_entry(
                            "message",
                            self.message.as_deref().unwrap_or_default(),
                        )?;
                    }
                }
                map.serialize_entry("retcode", &self.retcode)?;
                map.serialize_entry("status", &self.status)?;
                if let Some(user_id) = &self.user_id {
                    map.serialize_entry("user_id", user_id)?;
                }
                if let Some(wording) = &self.wording {
                    map.serialize_entry("wording", wording)?;
                }
            }
            EnvelopeKind::Compact(data) => {
                // Go 的失败路径只有 `sendActionResult`（那里根本没有 data 字段），所以 failed
                // 时无论子形态如何都不写 data。
                if self.status != Status::Failed {
                    match data {
                        CompactData::Omit => {}
                        CompactData::AsIs => map.serialize_entry("data", &self.data)?,
                        CompactData::EmptyObject => map.serialize_entry("data", &EmptyObject {})?,
                        CompactData::Null => map.serialize_entry("data", &Value::Null)?,
                    }
                }
                if let Some(echo) = &self.echo {
                    map.serialize_entry("echo", echo)?;
                }
                map.serialize_entry("message", self.message.as_deref().unwrap_or_default())?;
                map.serialize_entry("retcode", &self.retcode)?;
                map.serialize_entry("status", &self.status)?;
            }
            EnvelopeKind::GoWakeupOk => {
                // Go 的字面量只有这两个键，键序 `status` → `retcode`
                map.serialize_entry("status", &self.status)?;
                map.serialize_entry("retcode", &self.retcode)?;
            }
        }
        map.end()
    }
}

/// `{}` 的序列化载体（Go `sendActionResultWithData(..., map[string]interface{}{})`）。
#[derive(Serialize)]
struct EmptyObject {}

/// 无参数 action 的参数占位（`get_login_info` / `get_status` / `get_version_info` 等）。
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct EmptyParams {
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `send_group_msg` / `send_to_group` / `send_group_msg_async` 参数。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SendGroupMsgParams {
    /// 群 ID（虚拟）。
    pub group_id: Ob11Id,
    /// 消息内容：CQ 字符串或消息段数组。
    #[serde(default)]
    pub message: MessageContent,
    /// 是否按纯文本发送（不解析 CQ 码）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_escape: Option<bool>,
    /// 附加字段（`msg_type` / `active` 等 Gensokyo 扩展）。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `send_private_msg` / `send_private_msg_wakeup` 参数。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SendPrivateMsgParams {
    /// 用户 ID（虚拟）。
    pub user_id: Ob11Id,
    /// 群 ID（虚拟，群临时会话时带上）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<Ob11Id>,
    /// 消息内容。
    #[serde(default)]
    pub message: MessageContent,
    /// 是否按纯文本发送。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_escape: Option<bool>,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `send_msg` 参数（群聊 / 私聊由 `message_type` 决定）。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SendMsgParams {
    /// 消息类型。
    pub message_type: crate::event::MessageType,
    /// 用户 ID（私聊必填）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Ob11Id>,
    /// 群 ID（群聊必填）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<Ob11Id>,
    /// 消息内容。
    #[serde(default)]
    pub message: MessageContent,
    /// 是否按纯文本发送。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_escape: Option<bool>,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `get_group_info` 参数。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GetGroupInfoParams {
    /// 群 ID（虚拟）。
    pub group_id: Ob11Id,
    /// 是否绕过缓存。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_cache: Option<bool>,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `get_group_member_info` 参数。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GetGroupMemberInfoParams {
    /// 群 ID（虚拟）。
    pub group_id: Ob11Id,
    /// 成员 ID（虚拟）。
    pub user_id: Ob11Id,
    /// 是否绕过缓存。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_cache: Option<bool>,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `get_group_member_list` 参数。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GetGroupMemberListParams {
    /// 群 ID（虚拟）。
    pub group_id: Ob11Id,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `set_group_ban` / `get_group_ban` 参数。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SetGroupBanParams {
    /// 群 ID（虚拟）。
    pub group_id: Ob11Id,
    /// 成员 ID（虚拟）。
    pub user_id: Ob11Id,
    /// 禁言时长（秒），0 为解除。
    pub duration: i64,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `set_group_kick` 参数。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SetGroupKickParams {
    /// 群 ID（虚拟）。
    pub group_id: Ob11Id,
    /// 成员 ID（虚拟）。
    pub user_id: Ob11Id,
    /// 是否同时拒绝再次加群。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reject_add_request: Option<bool>,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `set_group_card` 参数。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SetGroupCardParams {
    /// 群 ID（虚拟）。
    pub group_id: Ob11Id,
    /// 成员 ID（虚拟）。
    pub user_id: Ob11Id,
    /// 新群名片；空串表示清除。
    #[serde(default)]
    pub card: String,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `set_group_whole_ban` / `get_group_whole_ban` 参数。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SetGroupWholeBanParams {
    /// 群 ID（虚拟）。
    pub group_id: Ob11Id,
    /// 是否开启全员禁言。
    #[serde(default)]
    pub enable: bool,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `delete_msg` 参数。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct DeleteMsgParams {
    /// 消息 ID（虚拟）。
    pub message_id: Ob11Id,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `get_msg` 参数。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GetMsgParams {
    /// 消息 ID（虚拟）。
    pub message_id: Ob11Id,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `send_like` 参数。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SendLikeParams {
    /// 用户 ID（虚拟）。
    pub user_id: Ob11Id,
    /// 点赞次数。
    #[serde(default = "default_like_times")]
    pub times: i64,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

fn default_like_times() -> i64 {
    1
}

/// `get_stranger_info` 参数。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GetStrangerInfoParams {
    /// 用户 ID（虚拟）。
    pub user_id: Ob11Id,
    /// 是否绕过缓存。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_cache: Option<bool>,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `get_group_list` 参数。
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct GetGroupListParams {
    /// 是否绕过缓存。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_cache: Option<bool>,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `send_group_forward_msg` / `send_forward_msg` 参数。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SendForwardMsgParams {
    /// 群 ID（虚拟，群聊转发时必填）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<Ob11Id>,
    /// 用户 ID（虚拟，私聊转发时必填）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Ob11Id>,
    /// 合并转发节点（消息段数组）。
    #[serde(default)]
    pub messages: Vec<Segment>,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `send_msg` / `send_group_msg` / `send_private_msg` 的响应数据。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SendMsgResult {
    /// 消息 ID（虚拟）。
    pub message_id: Ob11Id,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `get_msg` 的响应数据。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GetMsgResult {
    /// 消息 ID（虚拟）。
    #[serde(default)]
    pub message_id: Option<Ob11Id>,
    /// 消息内容。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<MessageContent>,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `get_login_info` 的响应数据。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct LoginInfo {
    /// 机器人用户 ID（虚拟）。
    pub user_id: Ob11Id,
    /// 机器人昵称。
    #[serde(default)]
    pub nickname: String,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `get_status` 的响应数据。
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct StatusResult {
    /// 是否在线。
    #[serde(default)]
    pub online: bool,
    /// 连接是否健康。
    #[serde(default)]
    pub good: bool,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `get_version_info` 的响应数据。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct VersionInfo {
    /// 应用名。
    #[serde(default)]
    pub app_name: String,
    /// 应用版本。
    #[serde(default)]
    pub app_version: String,
    /// 协议版本。
    #[serde(default)]
    pub protocol_version: String,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `get_group_info` / `get_group_list` 的响应数据。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GroupInfo {
    /// 群 ID（虚拟）。
    pub group_id: Ob11Id,
    /// 群名。
    #[serde(default)]
    pub group_name: String,
    /// 成员数。
    #[serde(default)]
    pub member_count: i64,
    /// 最大成员数。
    #[serde(default)]
    pub max_member_count: i64,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// `get_group_member_info` / `get_group_member_list` 的响应数据。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GroupMemberInfo {
    /// 群 ID（虚拟）。
    pub group_id: Ob11Id,
    /// 成员 ID（虚拟）。
    pub user_id: Ob11Id,
    /// 昵称。
    #[serde(default)]
    pub nickname: String,
    /// 群名片。
    #[serde(default)]
    pub card: String,
    /// 性别。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sex: Option<String>,
    /// 年龄。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age: Option<i64>,
    /// 地区。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<String>,
    /// 入群时间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_time: Option<i64>,
    /// 最后发言时间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sent_time: Option<i64>,
    /// 等级。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    /// 角色：`owner` / `admin` / `member`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// 是否不良记录成员。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unfriendly: Option<bool>,
    /// 群头衔。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// 头衔过期时间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_expire_time: Option<i64>,
    /// 是否允许修改群名片。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_changeable: Option<bool>,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// 泛型兜底请求（注册表入口用的形态）。
pub type RawActionRequest = ActionRequest<Value>;

/// 泛型兜底响应（注册表出口用的形态）。
pub type RawActionResponse = ActionResponse<Value>;

fn is_empty(map: &Extra) -> bool {
    map.is_empty()
}

#[cfg(test)]
mod tests {
    use super::{ActionRequest, ActionResponse, GetGroupInfoParams, SendGroupMsgParams, Status};
    use crate::envelope::{CompactData, EnvelopeKind};
    use crate::event::MessageType;

    #[test]
    fn data_is_always_serialized_even_when_none() {
        // 契约：data 字段恒出现，失败响应为 null（对齐 OneBot 客户端按字段读取的习惯）
        let failed: ActionResponse<String> = ActionResponse::failed(-1, "boom");
        let yaml = serde_norway::to_string(&failed).expect("序列化失败");
        assert!(yaml.contains("data: null"), "data 必须保留为 null: {yaml}");
        assert_eq!(failed.status, Status::Failed);

        let ok: ActionResponse<String> = ActionResponse::ok("x".to_owned());
        let yaml = serde_norway::to_string(&ok).expect("序列化失败");
        assert!(yaml.contains("data: x"), "data 必须带值: {yaml}");
        assert_eq!(ok.status, Status::Ok);
    }

    /// 分页游标在 `data` 内（Go 的 `get_panel_list` / `get_group_join_request_list` 都这样放），
    /// 响应顶层没有 `next_cursor` 字段（`get_group_member_list` 在 Go 里连游标都不给）。
    #[test]
    fn paging_cursor_lives_in_data() {
        let response: ActionResponse<serde_json::Value> =
            ActionResponse::ok(serde_json::json!({ "next_cursor": "nc", "records": [] }));
        assert_eq!(
            serde_json::to_string(&response).expect("序列化失败"),
            r#"{"data":{"next_cursor":"nc","records":[]},"echo":null,"message":"","retcode":0,"status":"ok"}"#
        );
    }

    /// 键序：字段声明序 = 线上键序（Go 的 map 字母序），整串钉死。
    #[test]
    fn envelope_serializes_keys_alphabetically() {
        let mut response: ActionResponse<serde_json::Value> =
            ActionResponse::ok(serde_json::json!({ "message_id": 1 }));
        response.group_id = Some(crate::parse_id("456"));
        assert_eq!(
            serde_json::to_string(&response).expect("序列化失败"),
            r#"{"data":{"message_id":1},"echo":null,"group_id":456,"message":"","retcode":0,"status":"ok"}"#
        );

        let mut response: ActionResponse<serde_json::Value> =
            ActionResponse::ok(serde_json::json!({ "message_id": "m-1" }));
        response.user_id = Some(crate::parse_id("789"));
        assert_eq!(
            serde_json::to_string(&response).expect("序列化失败"),
            r#"{"data":{"message_id":"m-1"},"echo":null,"message":"","retcode":0,"status":"ok","user_id":789}"#
        );
    }

    /// `data` 内的键也是字母序：依赖 `serde_json::Map` 是 BTreeMap（未开 `preserve_order`）。
    /// 谁给 `serde_json` 打开 `preserve_order`，这条会立刻红。
    #[test]
    fn data_keys_are_alphabetical() {
        let response: ActionResponse<serde_json::Value> =
            ActionResponse::ok(serde_json::json!({ "message_id": 1, "alpha": 2, "zeta": 3 }));
        assert_eq!(
            serde_json::to_string(&response).expect("序列化失败"),
            r#"{"data":{"alpha":2,"message_id":1,"zeta":3},"echo":null,"message":"","retcode":0,"status":"ok"}"#
        );
    }

    /// `message` / `echo` 恒在：成功时 `message` 是空串、无 echo 时是 `null`；失败时 `message`
    /// 带错误串、`data` 是 `null`，且不发 `group_id` / `user_id`。
    #[test]
    fn message_and_echo_are_always_serialized() {
        let ok: ActionResponse<String> = ActionResponse::ok("x".to_owned());
        assert_eq!(
            serde_json::to_string(&ok).expect("序列化失败"),
            r#"{"data":"x","echo":null,"message":"","retcode":0,"status":"ok"}"#
        );

        let failed: ActionResponse<String> = ActionResponse::failed(1400, "boom");
        assert_eq!(
            serde_json::to_string(&failed).expect("序列化失败"),
            r#"{"data":null,"echo":null,"message":"boom","retcode":1400,"status":"failed"}"#
        );
    }

    /// `mark_msg_as_read` 的 Go 形状：`msg` 顶替 `message`，`wording` 紧跟其后（两者都没有
    /// omitempty，成功回执里是空串）。其余 action 一个键都不多。
    #[test]
    fn mark_read_uses_msg_and_wording() {
        let mut response: ActionResponse<serde_json::Value> =
            ActionResponse::ok(serde_json::Value::Null);
        response.msg = Some(String::new());
        response.wording = Some(String::new());
        assert_eq!(
            serde_json::to_string(&response).expect("序列化失败"),
            r#"{"data":null,"echo":null,"msg":"","retcode":0,"status":"ok","wording":""}"#
        );

        let plain: ActionResponse<serde_json::Value> = ActionResponse::ok(serde_json::Value::Null);
        let json = serde_json::to_string(&plain).expect("序列化失败");
        assert!(!json.contains("msg"), "msg 只属于 mark_msg_as_read: {json}");
        assert!(
            !json.contains("wording"),
            "wording 只属于 mark_msg_as_read: {json}"
        );
    }

    // ---- P86c：B 管理类的 Compact 信封（Go `sendActionResult` / `sendActionResultWithData`） ----

    fn compact<T>(mut response: ActionResponse<T>, data: CompactData) -> ActionResponse<T> {
        response.envelope = EnvelopeKind::Compact(data);
        response
    }

    fn to_json<T: serde::Serialize>(response: &ActionResponse<T>) -> String {
        serde_json::to_string(response).expect("序列化失败")
    }

    /// 无 `data` 的成功回执：`echo` 带 omitempty（无请求回执时整个键消失）。
    #[test]
    fn compact_omits_data_and_empty_echo() {
        let echo: ActionResponse<serde_json::Value> = compact(
            ActionResponse::ok(serde_json::Value::Null),
            CompactData::Omit,
        )
        .with_echo(Some(serde_json::json!("e1")));
        assert_eq!(
            to_json(&echo),
            r#"{"echo":"e1","message":"","retcode":0,"status":"ok"}"#
        );

        let bare: ActionResponse<serde_json::Value> = compact(
            ActionResponse::ok(serde_json::Value::Null),
            CompactData::Omit,
        );
        assert_eq!(
            to_json(&bare),
            r#"{"message":"","retcode":0,"status":"ok"}"#
        );
    }

    /// 失败：Go 的 B 类只有 `retcode 100` 这一档，且失败路径没有 `data`（即使子形态是 AsIs）。
    #[test]
    fn compact_failure_has_no_data() {
        let failed: ActionResponse<serde_json::Value> = compact(
            ActionResponse::failed(100, "无法反查群 OpenID"),
            CompactData::AsIs,
        )
        .with_echo(Some(serde_json::json!("e1")));
        assert_eq!(
            to_json(&failed),
            r#"{"echo":"e1","message":"无法反查群 OpenID","retcode":100,"status":"failed"}"#
        );
    }

    /// `data` 的三种形态：取 handler 值 / 恒 `{}` / 恒 `null`。
    #[test]
    fn compact_data_shapes() {
        let with_data: ActionResponse<serde_json::Value> = compact(
            ActionResponse::ok(serde_json::json!({
                "add_to_member_blacklist_fail_openids": [],
                "kicked": [],
                "remove_members_result": "",
            })),
            CompactData::AsIs,
        )
        .with_echo(Some(serde_json::json!("e1")));
        assert_eq!(
            to_json(&with_data),
            r#"{"data":{"add_to_member_blacklist_fail_openids":[],"kicked":[],"remove_members_result":""},"echo":"e1","message":"","retcode":0,"status":"ok"}"#
        );

        let empty: ActionResponse<serde_json::Value> = compact(
            ActionResponse::ok(serde_json::Value::Null),
            CompactData::EmptyObject,
        )
        .with_echo(Some(serde_json::json!("e1")));
        assert_eq!(
            to_json(&empty),
            r#"{"data":{},"echo":"e1","message":"","retcode":0,"status":"ok"}"#
        );

        let null: ActionResponse<serde_json::Value> = compact(
            ActionResponse::ok(serde_json::Value::Null),
            CompactData::Null,
        )
        .with_echo(Some(serde_json::json!("e1")));
        assert_eq!(
            to_json(&null),
            r#"{"data":null,"echo":"e1","message":"","retcode":0,"status":"ok"}"#
        );
    }

    /// 同一个响应体在两种信封下产出不同线上形状（默认仍是 Full）。
    #[test]
    fn full_and_compact_envelopes_differ() {
        let mut response: ActionResponse<serde_json::Value> =
            ActionResponse::ok(serde_json::Value::Null).with_echo(Some(serde_json::json!("e1")));
        assert_eq!(response.envelope, EnvelopeKind::Full);
        assert_eq!(
            to_json(&response),
            r#"{"data":null,"echo":"e1","message":"","retcode":0,"status":"ok"}"#
        );

        response.envelope = EnvelopeKind::Compact(CompactData::Omit);
        assert_eq!(
            to_json(&response),
            r#"{"echo":"e1","message":"","retcode":0,"status":"ok"}"#
        );
    }

    /// C 查询类仍是 Full：`data` / `message` / `echo` 恒在（`get_group_info` 的 `message` 是
    /// `"success"`，不是空串——`handlers/get_group_info.go:65`）。
    #[test]
    fn query_actions_keep_the_full_envelope() {
        let mut response: ActionResponse<serde_json::Value> =
            ActionResponse::ok(serde_json::json!({ "group_id": 456, "group_name": "测试群" }))
                .with_echo(Some(serde_json::json!("e1")));
        response.message = Some("success".to_owned());
        assert_eq!(
            crate::envelope::envelope_for("get_group_info"),
            EnvelopeKind::Full
        );
        assert_eq!(
            to_json(&response),
            r#"{"data":{"group_id":456,"group_name":"测试群"},"echo":"e1","message":"success","retcode":0,"status":"ok"}"#
        );
    }

    /// `send_private_msg_wakeup` 的成功回执只有 `status` + `retcode`（Go 字面量），
    /// 失败回落到 Full：`1400` / `1500` 的 `data` / `message` / `echo` 照旧。
    #[test]
    fn wakeup_envelope_drops_data_and_echo() {
        let mut ok = ActionResponse::ok(serde_json::json!({ "message_id": 1 }))
            .with_echo(Some(serde_json::json!("e1")));
        ok.envelope = EnvelopeKind::GoWakeupOk;
        assert_eq!(to_json(&ok), r#"{"status":"ok","retcode":0}"#);

        let mut failed = ActionResponse::<serde_json::Value>::failed(1500, "boom")
            .with_echo(Some(serde_json::json!("e1")));
        failed.envelope = EnvelopeKind::GoWakeupOk;
        assert_eq!(
            to_json(&failed),
            r#"{"data":null,"echo":"e1","message":"boom","retcode":1500,"status":"failed"}"#
        );
    }

    /// 顶层 `post_type` 是可选字段：缺席时不序列化，其它 action 的请求/响应逐字节不变。
    #[test]
    fn top_level_post_type_is_optional() {
        let request: ActionRequest<SendGroupMsgParams> = serde_json::from_str(
            r#"{"action":"send_group_msg","params":{"group_id":42,"message":"hi"}}"#,
        )
        .expect("反序列化失败");
        assert_eq!(request.post_type, None);
        assert_eq!(
            serde_json::to_string(&request).expect("序列化失败"),
            r#"{"action":"send_group_msg","params":{"group_id":42,"message":"hi"}}"#
        );

        let with: ActionRequest<serde_json::Value> =
            serde_json::from_str(r#"{"action":"put_interaction","echo":"i","post_type":0}"#)
                .expect("反序列化失败");
        assert_eq!(with.post_type, Some(serde_json::json!(0)));
        assert_eq!(
            serde_json::to_string(&with).expect("序列化失败"),
            r#"{"action":"put_interaction","echo":"i","post_type":0}"#
        );
    }

    #[test]
    fn send_group_msg_params_parse_real_json() {
        let request: ActionRequest<SendGroupMsgParams> = serde_json::from_str(
            r#"{"action":"send_group_msg","params":{"group_id":"123456","message":"hi[CQ:at,qq=1]","auto_escape":false,"msg_type":0},"echo":"e1"}"#,
        )
        .expect("反序列化失败");
        assert_eq!(request.action, "send_group_msg");
        let params = request.params.expect("应有参数");
        assert_eq!(params.group_id.as_str(), Some("123456"));
        assert_eq!(params.message.as_str(), Some("hi[CQ:at,qq=1]"));
        assert_eq!(params.auto_escape, Some(false));
        assert_eq!(
            params
                .extra
                .get("msg_type")
                .and_then(|value| value.as_i64()),
            Some(0)
        );
    }

    #[test]
    fn action_request_accepts_missing_params() {
        let request: ActionRequest<serde_json::Value> =
            serde_json::from_str(r#"{"action":"get_status"}"#).expect("反序列化失败");
        assert!(request.params.is_none());
        assert!(request.echo.is_none());
    }

    #[test]
    fn get_group_info_params_parse_numeric_id() {
        let params: GetGroupInfoParams =
            serde_json::from_str(r#"{"group_id":42,"no_cache":true}"#).expect("反序列化失败");
        assert_eq!(params.group_id.as_i64(), Some(42));
        assert_eq!(params.no_cache, Some(true));
    }

    #[test]
    fn send_msg_discriminates_message_type() {
        let request: ActionRequest<super::SendMsgParams> = serde_json::from_str(
            r#"{"action":"send_msg","params":{"message_type":"private","user_id":7,"message":"x"}}"#,
        )
        .expect("反序列化失败");
        let params = request.params.expect("应有参数");
        assert_eq!(params.message_type, MessageType::Private);
        assert!(params.group_id.is_none());
    }

    #[test]
    fn echo_accepts_every_json_shape() {
        for raw in [
            r#""e1""#,
            "42",
            "-1",
            r#"[1,"a",null]"#,
            r#"{"k":"v","n":7}"#,
        ] {
            let request: ActionRequest<serde_json::Value> =
                serde_json::from_str(&format!(r#"{{"action":"get_status","echo":{raw}}}"#))
                    .unwrap_or_else(|error| panic!("echo={raw} 反序列化失败: {error}"));
            assert_eq!(
                serde_json::to_string(&request.echo).expect("序列化失败"),
                raw.to_owned(),
                "echo 应原样往返"
            );

            let mut response = ActionResponse::<()>::ok(());
            response.echo = request.echo.clone();
            assert_eq!(
                serde_json::to_string(&response.echo).expect("序列化失败"),
                raw.to_owned(),
                "响应 echo 应原样往返"
            );
        }
    }

    #[test]
    fn response_echo_round_trips() {
        let response: ActionResponse<super::SendMsgResult> = ActionResponse::ok(
            serde_json::from_str(r#"{"message_id":"99"}"#).expect("反序列化失败"),
        )
        .with_echo(Some(serde_json::json!("e1")));
        let json = serde_json::to_string(&response).expect("序列化失败");
        assert!(json.contains(r#""echo":"e1""#), "echo 应原样回传: {json}");
        assert!(
            json.contains(r#""message_id":"99""#),
            "字符串形态应保留: {json}"
        );
    }
}
