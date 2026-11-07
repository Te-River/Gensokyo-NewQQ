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

/// 入群自动审批策略（`/v2/groups/join_approval_strategy*`）。
///
/// 以下结构体覆盖 Go `GroupAPI` 的审批策略 / 黑名单 / 批量移出 / 审批申请四组端点，
/// 字段与 `botgo/dto/group.go`、`group_member.go` 一一对应；请求侧的 `omitempty`
/// 落成 `skip_serializing_if`，响应侧一律 `#[serde(default)]` 容忍官方加字段。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct JoinApprovalStrategy {
    /// 策略 ID。
    pub strategy_id: String,
    /// 生效群 OpenID。
    pub group_openids: Vec<String>,
    /// 生效群号。
    pub group_ids: Vec<u64>,
    /// 白名单人数。
    pub whitelist_user_count: u64,
    /// `on` / `off`。
    pub is_enable: String,
    /// 失效时间（RFC3339）。
    pub expire_at: String,
    /// 创建时间（RFC3339）。
    pub created_at: String,
    /// 更新时间（RFC3339）。
    pub updated_at: String,
    /// 备注。
    pub remark: String,
}

/// 审批策略列表（`next_cursor` 空串表示末页）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct JoinApprovalStrategyList {
    /// 策略列表。
    pub strategies: Vec<JoinApprovalStrategy>,
    /// 下一页游标。
    pub next_cursor: String,
}

/// 创建审批策略请求（`POST /v2/groups/join_approval_strategy`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CreateJoinApprovalStrategyRequest {
    /// 生效群 OpenID。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub group_openids: Vec<String>,
    /// 生效群号。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub group_ids: Vec<u64>,
    /// `on` / `off`（缺省官方按 `on` 处理）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub is_enable: String,
    /// 失效时间（RFC3339；缺省一年后）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub expire_at: String,
    /// 备注（≤ 255 字符）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub remark: String,
}

/// 创建审批策略响应。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CreateJoinApprovalStrategyResponse {
    /// 策略 ID。
    pub strategy_id: String,
    /// `on` / `off`。
    pub is_enable: String,
    /// 失效时间（RFC3339）。
    pub expire_at: String,
}

/// 审批策略的群增删动作（`op` 为 `add` / `del`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupAction {
    /// `add` / `del`。
    pub op: String,
    /// 群 OpenID。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub group_openids: Vec<String>,
    /// 群号。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub group_ids: Vec<u64>,
}

/// 更新审批策略请求（`PATCH /v2/groups/join_approval_strategy/{strategy_id}`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateJoinApprovalStrategyRequest {
    /// `on` / `off`。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub is_enable: String,
    /// 失效时间（RFC3339）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub expire_at: String,
    /// 群增删动作。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_action: Option<GroupAction>,
    /// 备注。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub remark: String,
}

/// 更新审批策略响应。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateJoinApprovalStrategyResponse {
    /// `on` / `off`。
    pub is_enable: String,
    /// 失效时间（RFC3339）。
    pub expire_at: String,
}

/// 审批策略白名单请求（`POST .../{strategy_id}/whitelist_users`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WhitelistUsersRequest {
    /// `add` / `del`。
    pub op: String,
    /// 白名单用户 ID。
    pub whitelist_users: Vec<String>,
}

/// 审批策略白名单响应。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WhitelistUsersResponse {
    /// 策略 ID。
    pub strategy_id: String,
    /// 白名单人数。
    pub whitelist_user_count: u64,
    /// 更新时间（RFC3339）。
    pub updated_at: String,
}

/// 入群申请审批请求（`POST /v2/groups/{group_id}/approval_join_request/{member_openid}`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ApprovalJoinRequest {
    /// `approve` / `decline`。
    pub op: String,
    /// 申请 ID（`flag`）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub join_request_id: String,
    /// 拒绝理由。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reject_reason: String,
    /// 是否同时拉黑。
    #[serde(skip_serializing_if = "is_false")]
    pub add_to_member_blacklist: bool,
}

/// 批量移出成员请求（`POST /v2/groups/{group_id}/batch_remove_members`，单批 ≤ 20）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BatchRemoveMembersRequest {
    /// 待移出的成员 OpenID。
    pub member_openids: Vec<String>,
    /// 是否同时拉黑。
    #[serde(skip_serializing_if = "is_false")]
    pub add_to_member_blacklist: bool,
}

/// 批量移出响应。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BatchRemoveMembersResponse {
    /// `success` 表示全部成功。
    pub remove_members_result: String,
    /// 拉黑失败的成员 OpenID。
    pub add_to_member_blacklist_fail_openids: Vec<String>,
}

/// 群黑名单增删请求（`POST /v2/groups/{group_id}/member_blacklist`，单批 ≤ 20）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemberBlacklistRequest {
    /// `add` / `del`。
    pub op: String,
    /// 成员 OpenID。
    pub member_openids: Vec<String>,
}

/// 群黑名单增删响应。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemberBlacklistResponse {
    /// 处理失败的成员 OpenID。
    pub fail_openids: Vec<String>,
}

/// 黑名单用户。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BlacklistUser {
    /// union_openid。
    pub union_openid: String,
    /// 成员 OpenID。
    pub member_openid: String,
    /// 昵称。
    pub username: String,
    /// 拉黑时间（RFC3339）。
    pub banned_at: String,
    /// 是否机器人。
    pub bot: bool,
}

/// 群黑名单列表（`next_cursor` 空串表示末页）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemberBlacklistList {
    /// 黑名单用户。
    pub users: Vec<BlacklistUser>,
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

/// `bool` 是否为 `false`（`omitempty` 语义）。
fn is_false(value: &bool) -> bool {
    !*value
}
