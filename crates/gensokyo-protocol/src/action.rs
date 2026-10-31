//! OneBot V11 action 请求 / 响应模型（出站方向：OneBot 客户端 → gensokyo）。
//!
//! [`ActionRequest`] / [`ActionResponse`] 保持泛型：handler 注册表用 `serde_json::Value`
//! 兜底，单个 handler 可以自行反序列化成下面的类型化参数结构。参数结构与
//! `handlers/*.go` 的实际取值对齐（含 Gensokyo 扩展字段），未知字段落进 `extra`。
//!
//! 这里只放**模型**，不含任何 QQ API 调用或业务判断（那是 M4 `gensokyo-actions` 的职责）。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use gensokyo_core::Ob11Id;

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
}

impl<P> ActionRequest<P> {
    /// 构造一条请求。
    #[must_use]
    pub fn new(action: impl Into<String>, params: P) -> Self {
        Self {
            action: action.into(),
            params: Some(params),
            echo: None,
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
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ActionResponse<T = ()> {
    /// 状态。
    pub status: Status,
    /// 返回码，0 为成功。
    pub retcode: i32,
    /// 业务数据：恒序列化（None 输出 `null`，对齐 OneBot 响应结构，不省略字段）。
    #[serde(default)]
    pub data: Option<T>,
    /// 错误信息。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// 补充说明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wording: Option<String>,
    /// 原样回传的 echo。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echo: Option<Echo>,
    /// 分页游标（Gensokyo 扩展，放响应顶层）：仅分页类 action 填，
    /// `None` 不序列化，既有响应形态不受影响。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
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
            wording: None,
            echo: None,
            next_cursor: None,
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
            wording: None,
            echo: None,
            next_cursor: None,
        }
    }

    /// 挂上待回传的 echo。
    #[must_use]
    pub fn with_echo(mut self, echo: Option<Echo>) -> Self {
        self.echo = echo;
        self
    }

    /// 挂上分页游标（`None` 不序列化）。
    #[must_use]
    pub fn with_next_cursor(mut self, next_cursor: Option<String>) -> Self {
        self.next_cursor = next_cursor;
        self
    }
}

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

    #[test]
    fn next_cursor_is_optional_top_level_field() {
        // `data` 保持数组（OneBot 兼容），游标只加在响应顶层
        let plain: ActionResponse<Vec<i32>> = ActionResponse::ok(vec![1, 2]);
        let yaml = serde_norway::to_string(&plain).expect("序列化失败");
        assert!(!yaml.contains("next_cursor"), "None 不应序列化: {yaml}");

        let paged: ActionResponse<Vec<i32>> =
            ActionResponse::ok(vec![1]).with_next_cursor(Some("nc".to_owned()));
        let json = serde_json::to_value(&paged).expect("序列化失败");
        assert_eq!(json["next_cursor"], serde_json::json!("nc"));
        assert_eq!(json["data"], serde_json::json!([1]), "data 仍是数组");
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
