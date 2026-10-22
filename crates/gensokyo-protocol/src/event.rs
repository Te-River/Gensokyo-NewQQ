//! OneBot V11 事件模型（入站方向：QQ → OneBot 客户端）。
//!
//! M0 只定型公共头与事件分类，具体字段随 M1 的 `Processor/` 迁移逐项补齐。
//! `message` 字段（字符串或消息段数组，取决于 `array_value`）与消息段类型一起在 M1 落地，
//! 避免 M0 先引入一个已知不完整的消息段表示。

use serde::{Deserialize, Serialize};

/// 一条上报事件。
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "post_type", rename_all = "snake_case")]
pub enum Event {
    /// 消息事件。
    Message(MessageEvent),
    /// 通知事件。
    Notice(NoticeEvent),
    /// 请求事件。
    Request(RequestEvent),
    /// 元事件。
    MetaEvent(MetaEvent),
    /// 未知事件：QQ 新增 `post_type` 时不应让整条链路报错。
    #[serde(other)]
    Unknown,
}

/// 消息类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    /// 私聊。
    Private,
    /// 群聊。
    Group,
}

/// 消息事件。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MessageEvent {
    /// 事件时间戳（秒）。
    pub time: i64,
    /// 机器人自身 ID（虚拟）。
    pub self_id: i64,
    /// 私聊 / 群聊。
    pub message_type: MessageType,
    /// 消息 ID（虚拟）。
    pub message_id: i64,
    /// 发送者 ID（虚拟）。
    pub user_id: i64,
    /// 群 ID（虚拟，仅群消息）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<i64>,
    /// 原始消息文本。
    #[serde(default)]
    pub raw_message: String,
}

/// 通知事件。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NoticeEvent {
    /// 事件时间戳（秒）。
    pub time: i64,
    /// 机器人自身 ID（虚拟）。
    pub self_id: i64,
    /// 原始通知类型（`group_increase` / `group_ban` / `poke` …）：QQ 仍在扩展，先保留字符串。
    pub notice_type: String,
    /// 子类型（`approve` / `invite` / `leave` …）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_type: Option<String>,
    /// 群 ID（虚拟）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<i64>,
    /// 目标用户 ID（虚拟）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<i64>,
}

/// 请求事件。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RequestEvent {
    /// 事件时间戳（秒）。
    pub time: i64,
    /// 机器人自身 ID（虚拟）。
    pub self_id: i64,
    /// 原始请求类型（`friend` / `group`）。
    pub request_type: String,
    /// 群 ID（虚拟）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<i64>,
    /// 发起用户 ID（虚拟）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<i64>,
    /// 验证信息。
    #[serde(default)]
    pub comment: String,
    /// 请求 flag（用于同意 / 拒绝）。
    #[serde(default)]
    pub flag: String,
}

/// 元事件。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MetaEvent {
    /// 事件时间戳（秒）。
    pub time: i64,
    /// 机器人自身 ID（虚拟）。
    pub self_id: i64,
    /// `lifecycle` / `heartbeat`。
    pub meta_event_type: String,
    /// 子类型（`connect` / `enable` / `disable` …）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_type: Option<String>,
    /// 心跳状态。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<HeartbeatStatus>,
}

/// 心跳状态。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HeartbeatStatus {
    /// 是否在线。
    #[serde(default)]
    pub online: bool,
    /// 连接是否健康。
    #[serde(default)]
    pub good: bool,
}
