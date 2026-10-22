//! 群管理 DTO（对齐 `botgo/dto/group.go` 与 `group_member.go`）。

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// 群基本信息（`GET /v2/groups/{group_id}/info`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupInfo {
    /// 群 OpenID。
    pub group_openid: String,
    /// 群名。
    pub group_name: String,
    /// 群简介。
    pub group_finger_memo: String,
    /// 群分类文本。
    pub group_class_text: String,
    /// 群标签。
    pub group_tags: Vec<String>,
    /// 群成员数。
    pub group_member_num: u64,
}

/// 机器人群内状态（`GET /v2/groups/{group_id}/bot_state`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BotInGroupState {
    /// 机器人的 member_openid。
    pub member_openid: String,
    /// 入群时间（RFC3339）。
    pub joined_at: String,
    /// 是否允许主动推送。
    pub allow_proactive_msg: bool,
    /// 接收消息设置：`all` / `only_mention` / `mention_and_context`。
    pub recv_msg_setting: String,
    /// 机器人群内角色：`member` / `owner` / `admin`。
    pub member_role: String,
}

/// 入群验证信息；兼容纯字符串与对象两种官方形态。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "VerifyInfoRepr")]
pub struct VerifyInfo {
    /// 验证方式（如 `admin_review_qa`）。
    pub method: String,
    /// 验证消息文本。
    #[serde(rename = "verify_message")]
    pub message: String,
    /// 验证问答列表。
    pub review_qa_list: Vec<ReviewQa>,
}

/// 入群验证问答。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReviewQa {
    /// 问题。
    pub question: String,
    /// 答案。
    pub answer: String,
}

/// `verify_info` 的线上形态。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum VerifyInfoRepr {
    Text(String),
    Object {
        #[serde(default)]
        method: String,
        #[serde(default, rename = "verify_message")]
        message: String,
        #[serde(default)]
        review_qa_list: Vec<ReviewQa>,
    },
}

impl From<VerifyInfoRepr> for VerifyInfo {
    fn from(repr: VerifyInfoRepr) -> Self {
        match repr {
            VerifyInfoRepr::Text(message) => Self {
                message,
                ..Self::default()
            },
            VerifyInfoRepr::Object {
                method,
                message,
                review_qa_list,
            } => Self {
                method,
                message,
                review_qa_list,
            },
        }
    }
}

/// 入群申请自动审批信息；官方对象形态与历史 `true` / `false` 形态都容忍。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "AutoApprovedRepr")]
pub struct AutoApprovedInfo {
    /// 命中的策略 ID。
    pub strategy_id: String,
    /// 是否已自动审批（不参与序列化，对齐 botgo 的 `json:"-"`）。
    #[serde(skip)]
    pub approved: bool,
}

/// `auto_approved` 的线上形态。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AutoApprovedRepr {
    Object {
        #[serde(default)]
        strategy_id: String,
    },
    Bool(bool),
}

impl From<AutoApprovedRepr> for AutoApprovedInfo {
    fn from(repr: AutoApprovedRepr) -> Self {
        match repr {
            AutoApprovedRepr::Object { strategy_id } => Self {
                strategy_id,
                approved: true,
            },
            AutoApprovedRepr::Bool(approved) => Self {
                strategy_id: String::new(),
                approved,
            },
        }
    }
}

/// 入群申请（`GET /v2/groups/{group_id}/join_request_list`）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JoinRequest {
    /// 申请 ID，审批时回传。
    pub join_request_id: String,
    /// 安全提示。
    pub risk_tips: String,
    /// 申请人 union_openid。
    pub union_openid: String,
    /// 申请人 member_openid。
    pub member_openid: String,
    /// 申请人昵称。
    pub username: String,
    /// 申请时间（兼容 RFC3339 字符串与历史数字时间戳）。
    #[serde(deserialize_with = "string_or_number")]
    pub apply_at: String,
    /// 申请来源：`self_apply` / `invited`。
    pub apply_source: String,
    /// 邀请人 OpenID。
    pub invited_by: String,
    /// 是否机器人。
    pub bot: bool,
    /// 验证信息。
    pub verify_info: VerifyInfo,
    /// 自动审批信息（仅事件携带）。
    pub auto_approved: Option<AutoApprovedInfo>,
}

/// 入群申请列表响应。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JoinRequestList {
    /// 申请列表。
    pub list: Vec<JoinRequest>,
    /// 下一页游标；空串表示已到末页。
    pub next_cursor: String,
}

/// 群禁言设置：GET 响应与 POST 请求体共用。
///
/// POST 只接受 `members`（按成员 `op` 增量生效），`global_rule` 为 `None` 时不参与序列化。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RestrictChatSetting {
    /// 群级禁言规则。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_rule: Option<GlobalMuteRule>,
    /// 成员禁言条目。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<MemberMuteState>,
}

/// 群级禁言规则。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GlobalMuteRule {
    /// `none` / `always` / `schedule`。
    pub mode: String,
    /// 定时禁言规则。
    pub schedule_rules: Vec<MuteScheduleRule>,
    /// 周期性禁言规则。
    pub recurring_rules: Vec<MuteRecurringRule>,
}

/// 定时禁言规则。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MuteScheduleRule {
    /// 任务 ID。
    pub task_id: String,
    /// 开始时间（RFC3339）。
    pub start_at: String,
    /// 结束时间（RFC3339）。
    pub end_at: String,
    /// 是否启用。
    pub enabled: bool,
}

/// 周期性禁言规则。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MuteRecurringRule {
    /// 任务 ID。
    pub task_id: String,
    /// 周几（1~7，1 = 周一）。
    pub weekdays: Vec<u8>,
    /// 开始时间（`HH:mm`）。
    pub start_time: String,
    /// 结束时间（`HH:mm`）。
    pub end_time: String,
    /// 是否启用。
    pub enabled: bool,
}

/// 成员禁言条目。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemberMuteState {
    /// `add` 增加 / `update` 更新 / `del` 解除。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
    /// 成员 OpenID。
    pub member_openid: String,
    /// 禁言到期时间（RFC3339；`op=del` 时传空串立即解除）。
    pub mute_expire_at: String,
    /// 昵称（GET 响应携带）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// union_openid（GET 响应携带）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub union_openid: Option<String>,
}

/// 群成员信息。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupMember {
    /// 成员 OpenID。
    pub member_openid: String,
    /// 昵称。
    pub username: String,
    /// 角色：`member` / `owner` / `admin`。
    pub member_role: String,
    /// 是否机器人。
    pub bot: bool,
    /// 入群时间（RFC3339）。
    pub joined_at: String,
    /// union_openid。
    pub union_openid: String,
}

/// 群成员列表响应。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupMemberList {
    /// 成员列表。
    pub members: Vec<GroupMember>,
    /// 下一页游标。
    pub next_cursor: String,
}

/// 把 JSON 字符串或数字反序列化为字符串（`apply_at` 历史形态兼容）。
fn string_or_number<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::String(text) => Ok(text),
        Value::Number(number) => Ok(number.to_string()),
        Value::Null => Ok(String::new()),
        other => Err(serde::de::Error::custom(format!(
            "期望字符串或数字，收到 {other}"
        ))),
    }
}
