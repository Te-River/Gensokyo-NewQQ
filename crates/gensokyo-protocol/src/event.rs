//! OneBot V11 事件模型（入站方向：QQ → OneBot 客户端）。
//!
//! 覆盖 `post_type` 的全部形态：`message` / `message_sent` / `notice` / `request` /
//! `meta_event`。字段口径对照 `Processor/Processor.go` 的上报结构体与 OneBot V11 规范：
//!
//! - `user_id` / `group_id` / `message_id` / `self_id` 一律 [`Ob11Id`]：`string_ob11`
//!   决定线上是 JSON 数字还是 JSON 字符串，形态由类型携带，反序列化两种都收。
//! - `sub_type` / `notice_type` / `real_*` 保持字符串：QQ 仍在扩展，枚举化会在新值上炸。
//! - 每个事件都带 `#[serde(flatten)] extra`，`Processor` 追加的
//!   `real_user_id` / `to_me` / `platform` 等字段不会丢失，也不会让解析失败。
//! - `message` 同时接受 CQ 字符串与消息段数组（[`MessageContent`]），由 `array_value` 决定。

use std::fmt;

use serde::de::Deserializer;
use serde::{Deserialize, Serialize};

use gensokyo_core::Ob11Id;

use crate::segment::MessageContent;

pub use crate::Extra;

/// `post_type` 取值。
pub mod post_type {
    /// 消息事件。
    pub const MESSAGE: &str = "message";
    /// 自身消息事件（部分客户端会上报）。
    pub const MESSAGE_SENT: &str = "message_sent";
    /// 通知事件。
    pub const NOTICE: &str = "notice";
    /// 请求事件。
    pub const REQUEST: &str = "request";
    /// 元事件。
    pub const META_EVENT: &str = "meta_event";
}

/// 一条上报事件。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "post_type", rename_all = "snake_case")]
pub enum Event {
    /// 消息事件。
    Message(MessageEvent),
    /// 自身发出的消息（`post_type = message_sent`）。
    MessageSent(MessageEvent),
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

impl Event {
    /// 事件分类名（与 `post_type` 一致）。
    #[must_use]
    pub fn post_type(&self) -> &'static str {
        match self {
            Self::Message(_) => post_type::MESSAGE,
            Self::MessageSent(_) => post_type::MESSAGE_SENT,
            Self::Notice(_) => post_type::NOTICE,
            Self::Request(_) => post_type::REQUEST,
            Self::MetaEvent(_) => post_type::META_EVENT,
            Self::Unknown => "unknown",
        }
    }

    /// 机器人自身 ID。
    #[must_use]
    pub fn self_id(&self) -> Option<&Ob11Id> {
        match self {
            Self::Message(event) | Self::MessageSent(event) => Some(&event.self_id),
            Self::Notice(event) => Some(&event.self_id),
            Self::Request(event) => Some(&event.self_id),
            Self::MetaEvent(event) => Some(&event.self_id),
            Self::Unknown => None,
        }
    }

    /// 事件时间戳（秒）。
    #[must_use]
    pub fn time(&self) -> Option<i64> {
        match self {
            Self::Message(event) | Self::MessageSent(event) => Some(event.time),
            Self::Notice(event) => Some(event.time),
            Self::Request(event) => Some(event.time),
            Self::MetaEvent(event) => Some(event.time),
            Self::Unknown => None,
        }
    }

    /// 消息事件视图（`message` 与 `message_sent` 都算）。
    #[must_use]
    pub fn message(&self) -> Option<&MessageEvent> {
        match self {
            Self::Message(event) | Self::MessageSent(event) => Some(event),
            _ => None,
        }
    }
}

/// 消息类型（`message_type`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageType {
    /// 私聊。
    Private,
    /// 群聊。
    Group,
    /// 其它（QQ 未来新增；保留原值）。
    Other(String),
}

impl MessageType {
    /// 线上取值。
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Private => "private",
            Self::Group => "group",
            Self::Other(value) => value,
        }
    }
}

impl fmt::Display for MessageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for MessageType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for MessageType {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "private" => Self::Private,
            "group" => Self::Group,
            _ => Self::Other(raw),
        })
    }
}

/// `sub_type` 常用取值（消息 / 通知 / 请求共用，QQ 可能扩展）。
pub mod sub_type {
    /// 私聊：好友。
    pub const FRIEND: &str = "friend";
    /// 私聊：群临时会话。
    pub const GROUP: &str = "group";
    /// 群聊：普通成员消息。
    pub const NORMAL: &str = "normal";
    /// 群聊：匿名消息。
    pub const ANONYMOUS: &str = "anonymous";
    /// 群聊：系统提示。
    pub const NOTICE: &str = "notice";
    /// 通知：成员加入（主动同意）。
    pub const APPROVE: &str = "approve";
    /// 通知：成员加入（被邀请）。
    pub const INVITE: &str = "invite";
    /// 通知：成员减少（主动退群）。
    pub const LEAVE: &str = "leave";
    /// 通知：成员减少（被踢）。
    pub const KICK: &str = "kick";
    /// 通知：成员减少（自己被踢）。
    pub const KICK_ME: &str = "kick_me";
    /// 通知：禁言。
    pub const BAN: &str = "ban";
    /// 通知：解除禁言。
    pub const LIFT_BAN: &str = "lift_ban";
    /// 通知：设置管理员。
    pub const SET: &str = "set";
    /// 通知：取消管理员。
    pub const UNSET: &str = "unset";
    /// 通知：戳一戳。
    pub const POKE: &str = "poke";
    /// 通知：群荣誉。
    pub const HONOR: &str = "honor";
    /// 通知：运气王。
    pub const LUCKY_KING: &str = "lucky_king";
    /// 请求：加好友 / 加群。
    pub const ADD: &str = "add";
    /// 元事件：连接。
    pub const CONNECT: &str = "connect";
    /// 元事件：启用。
    pub const ENABLE: &str = "enable";
    /// 元事件：停用。
    pub const DISABLE: &str = "disable";
}

/// 消息发送者。
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct Sender {
    /// 发送者 ID（虚拟）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Ob11Id>,
    /// 昵称。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    /// 群名片 / 群昵称。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
    /// 角色：`owner` / `admin` / `member`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// 性别。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sex: Option<String>,
    /// 年龄。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age: Option<i64>,
    /// 地区。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<String>,
    /// 等级。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    /// 群头衔。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// QQ 频道 tiny_id（Gensokyo 上报字段）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiny_id: Option<String>,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// 匿名用户信息。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Anonymous {
    /// 匿名 ID（线上可能是数字或字符串）。
    pub id: Ob11Id,
    /// 匿名名称。
    #[serde(default)]
    pub name: String,
    /// 匿名 flag。
    #[serde(default)]
    pub flag: String,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// 文件信息（`group_upload` / `offline_file` 事件内嵌）。
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct File {
    /// 文件 ID。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// 文件名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 文件大小（字节）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<i64>,
    /// 下载地址。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// 消息事件。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct MessageEvent {
    /// 事件时间戳（秒）。
    pub time: i64,
    /// 机器人自身 ID（虚拟）。
    pub self_id: Ob11Id,
    /// 私聊 / 群聊。
    pub message_type: MessageType,
    /// 子类型（`friend` / `group` / `normal` / `anonymous` …）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_type: Option<String>,
    /// 消息 ID（虚拟）。
    pub message_id: Ob11Id,
    /// 发送者 ID（虚拟）。
    pub user_id: Ob11Id,
    /// 群 ID（虚拟，仅群消息）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<Ob11Id>,
    /// 消息内容：CQ 字符串或消息段数组。
    #[serde(default)]
    pub message: MessageContent,
    /// 原始消息文本（含 CQ 码）。
    #[serde(default)]
    pub raw_message: String,
    /// 字体。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<i64>,
    /// 发送者信息。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<Sender>,
    /// 匿名信息（`sub_type = anonymous` 时存在）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anonymous: Option<Anonymous>,
    /// 消息序号（Gensokyo 上报字段）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_seq: Option<i64>,
    /// 是否 @ 了机器人（Gensokyo 上报字段）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_me: Option<bool>,
    /// 附加字段（`real_user_id` / `real_group_id` / `platform` …）。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

impl MessageEvent {
    /// 是否群消息。
    #[must_use]
    pub fn is_group(&self) -> bool {
        self.message_type == MessageType::Group
    }

    /// 是否私聊消息。
    #[must_use]
    pub fn is_private(&self) -> bool {
        self.message_type == MessageType::Private
    }

    /// 纯文本部分（拼接所有 `text` 段，不含 CQ 码与媒体）。
    #[must_use]
    pub fn text(&self) -> String {
        self.message
            .segments()
            .iter()
            .filter_map(|segment| segment.text_content())
            .collect()
    }
}

/// 通知类型（`notice_type`）分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    /// 群文件上传。
    GroupUpload,
    /// 群管理员变动。
    GroupAdmin,
    /// 群成员减少。
    GroupDecrease,
    /// 群成员增加。
    GroupIncrease,
    /// 群禁言。
    GroupBan,
    /// 好友添加。
    FriendAdd,
    /// 群消息撤回。
    GroupRecall,
    /// 私聊消息撤回。
    FriendRecall,
    /// 群名片变更。
    GroupCard,
    /// 离线文件。
    OfflineFile,
    /// 客户端状态。
    ClientStatus,
    /// 群精华消息。
    Essence,
    /// 戳一戳（`notify` + `sub_type = poke`）。
    Poke,
    /// 群荣誉（`notify` + `sub_type = honor`）。
    Honor,
    /// 运气王（`notify` + `sub_type = lucky_king`）。
    LuckyKing,
    /// 群消息表情表态（Gensokyo 扩展）。
    GroupMsgEmojiLike,
    /// 群消息接收（Gensokyo 扩展，`Processor/ProcessGroupMsgReceive.go`）。
    GroupReceive,
    /// 群消息拒收（Gensokyo 扩展，`Processor/ProcessGroupMsgReject.go`）。
    GroupReject,
    /// 内联搜索等交互（Gensokyo 扩展，`Processor/ProcessInlineSearch.go`）。
    Interaction,
    /// 机器人分享链接生成（Gensokyo 扩展，`handlers/get_robot_share_link.go`）。
    ShareLinkGenerated,
    /// 未知通知。
    Unknown,
}

/// 通知事件。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct NoticeEvent {
    /// 事件时间戳（秒）。
    pub time: i64,
    /// 机器人自身 ID（虚拟）。
    pub self_id: Ob11Id,
    /// 通知类型（`group_increase` / `group_ban` / `notify` …）：QQ 仍在扩展，先保留字符串。
    pub notice_type: String,
    /// 子类型（`approve` / `invite` / `ban` / `poke` …）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_type: Option<String>,
    /// 群 ID（虚拟）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<Ob11Id>,
    /// 目标用户 ID（虚拟）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Ob11Id>,
    /// 操作者 ID（虚拟）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_id: Option<Ob11Id>,
    /// 被戳 / 被操作的目标 ID（虚拟）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<Ob11Id>,
    /// 相关消息 ID（虚拟，撤回事件）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<Ob11Id>,
    /// 禁言时长（秒）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<i64>,
    /// 文件信息（上传 / 离线文件）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<File>,
    /// 新群名片。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_new: Option<String>,
    /// 旧群名片。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_old: Option<String>,
    /// 荣誉类型（`talkative` / `performer` / `emotion`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub honor_type: Option<String>,
    /// 客户端是否在线。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub online: Option<bool>,
    /// 客户端名称。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

impl NoticeEvent {
    /// 通知分类（`notice_type` 字符串 → 枚举）。
    #[must_use]
    pub fn kind(&self) -> NoticeKind {
        match self.notice_type.as_str() {
            "group_upload" => NoticeKind::GroupUpload,
            "group_admin" => NoticeKind::GroupAdmin,
            "group_decrease" => NoticeKind::GroupDecrease,
            "group_increase" => NoticeKind::GroupIncrease,
            "group_ban" => NoticeKind::GroupBan,
            "friend_add" => NoticeKind::FriendAdd,
            "group_recall" => NoticeKind::GroupRecall,
            "friend_recall" => NoticeKind::FriendRecall,
            "group_card" => NoticeKind::GroupCard,
            "offline_file" => NoticeKind::OfflineFile,
            "client_status" => NoticeKind::ClientStatus,
            "essence" => NoticeKind::Essence,
            "group_msg_emoji_like" => NoticeKind::GroupMsgEmojiLike,
            "group_receive" => NoticeKind::GroupReceive,
            "group_reject" => NoticeKind::GroupReject,
            "interaction" => NoticeKind::Interaction,
            "share_link_generated" => NoticeKind::ShareLinkGenerated,
            "notify" => match self.sub_type.as_deref() {
                Some(sub_type::POKE) => NoticeKind::Poke,
                Some(sub_type::HONOR) => NoticeKind::Honor,
                Some(sub_type::LUCKY_KING) => NoticeKind::LuckyKing,
                _ => NoticeKind::Unknown,
            },
            _ => NoticeKind::Unknown,
        }
    }
}

/// 请求事件。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RequestEvent {
    /// 事件时间戳（秒）。
    pub time: i64,
    /// 机器人自身 ID（虚拟）。
    pub self_id: Ob11Id,
    /// 请求类型（`friend` / `group`）。
    pub request_type: String,
    /// 子类型（`add` / `invite`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_type: Option<String>,
    /// 发起用户 ID（虚拟）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Ob11Id>,
    /// 群 ID（虚拟）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<Ob11Id>,
    /// 验证信息。
    #[serde(default)]
    pub comment: String,
    /// 请求 flag（用于同意 / 拒绝）。
    #[serde(default)]
    pub flag: String,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// 元事件。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct MetaEvent {
    /// 事件时间戳（秒）。
    pub time: i64,
    /// 机器人自身 ID（虚拟）。
    pub self_id: Ob11Id,
    /// `lifecycle` / `heartbeat`。
    pub meta_event_type: String,
    /// 子类型（`connect` / `enable` / `disable`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_type: Option<String>,
    /// 心跳间隔（秒）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<i64>,
    /// 心跳状态。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<HeartbeatStatus>,
    /// 附加字段。
    #[serde(default, flatten, skip_serializing_if = "is_empty")]
    pub extra: Extra,
}

/// 心跳状态。
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct HeartbeatStatus {
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

fn is_empty(map: &Extra) -> bool {
    map.is_empty()
}

#[cfg(test)]
mod tests {
    use super::{Event, MessageType, NoticeKind, sub_type};
    use crate::segment::Segment;

    fn parse(json: &str) -> Event {
        serde_json::from_str(json).expect("事件反序列化失败")
    }

    #[test]
    fn group_message_in_string_form() {
        let event = parse(
            r#"{"post_type":"message","message_type":"group","sub_type":"normal",
                "time":1700000000,"self_id":10000,"message_id":8888,"user_id":12345,
                "group_id":99999,"raw_message":"你好[CQ:at,qq=10000]",
                "message":"你好[CQ:at,qq=10000]","font":0,
                "sender":{"user_id":12345,"nickname":"阿求","card":"","role":"member","sex":"unknown","age":0},
                "real_user_id":"A1B2C3","to_me":true}"#,
        );
        let message = event.message().expect("应为消息事件");
        assert!(message.is_group());
        assert_eq!(
            message.group_id.as_ref().and_then(|id| id.as_i64()),
            Some(99999)
        );
        assert_eq!(message.text(), "你好");
        assert_eq!(message.message.segments().len(), 2);
        assert_eq!(
            message
                .extra
                .get("real_user_id")
                .and_then(|value| value.as_str()),
            Some("A1B2C3")
        );
        assert_eq!(message.to_me, Some(true));
        assert_eq!(
            message.sender.as_ref().and_then(|s| s.nickname.as_deref()),
            Some("阿求")
        );
    }

    #[test]
    fn private_message_in_array_form_with_string_ids() {
        let event = parse(
            r#"{"post_type":"message","message_type":"private","sub_type":"friend",
                "time":1700000001,"self_id":"10000","message_id":"777","user_id":"12345",
                "message":[{"type":"text","data":{"text":"hi"}},{"type":"image","data":{"file":"https://x.com/a.png"}}],
                "raw_message":"hi[CQ:image,file=https://x.com/a.png]"}"#,
        );
        let message = event.message().expect("应为消息事件");
        assert!(message.is_private());
        assert_eq!(message.message_type, MessageType::Private);
        assert_eq!(message.user_id.as_str(), Some("12345"));
        assert_eq!(message.message.segments()[1].kind, "image");
        assert!(message.message.is_array(), "数组形态应被识别");
    }

    #[test]
    fn anonymous_group_message_keeps_anonymous_block() {
        let event = parse(
            r#"{"post_type":"message","message_type":"group","sub_type":"anonymous",
                "time":1700000002,"self_id":10000,"message_id":1,"user_id":0,"group_id":42,
                "raw_message":"x","message":"x",
                "anonymous":{"id":1234567890,"name":"匿名用户","flag":"abc"}}"#,
        );
        let message = event.message().expect("应为消息事件");
        let anonymous = message.anonymous.as_ref().expect("应有匿名信息");
        assert_eq!(anonymous.name, "匿名用户");
        assert_eq!(anonymous.id.as_i64(), Some(1_234_567_890));
        assert_eq!(message.sub_type.as_deref(), Some(sub_type::ANONYMOUS));
    }

    #[test]
    fn message_sent_uses_same_shape() {
        let event = parse(
            r#"{"post_type":"message_sent","message_type":"group","time":1700000003,
                "self_id":10000,"message_id":2,"user_id":10000,"group_id":42,
                "raw_message":"out","message":"out"}"#,
        );
        assert_eq!(event.post_type(), "message_sent");
        assert_eq!(event.message().expect("应为消息事件").text(), "out");
    }

    #[test]
    fn group_increase_notice() {
        let event = parse(
            r#"{"post_type":"notice","notice_type":"group_increase","sub_type":"approve",
                "time":1700000004,"self_id":10000,"group_id":42,"operator_id":1,"user_id":2}"#,
        );
        let Event::Notice(notice) = event else {
            panic!("应为通知事件");
        };
        assert_eq!(notice.kind(), NoticeKind::GroupIncrease);
        assert_eq!(notice.sub_type.as_deref(), Some(sub_type::APPROVE));
        assert_eq!(notice.operator_id.and_then(|id| id.as_i64()), Some(1));
    }

    #[test]
    fn group_ban_notice() {
        let notice = match parse(
            r#"{"post_type":"notice","notice_type":"group_ban","sub_type":"ban","time":1700000005,
                "self_id":10000,"group_id":42,"operator_id":1,"user_id":2,"duration":600}"#,
        ) {
            Event::Notice(notice) => notice,
            other => panic!("应为通知事件: {other:?}"),
        };
        assert_eq!(notice.kind(), NoticeKind::GroupBan);
        assert_eq!(notice.duration, Some(600));
    }

    #[test]
    fn group_recall_notice() {
        let notice = match parse(
            r#"{"post_type":"notice","notice_type":"group_recall","time":1700000006,
                "self_id":10000,"group_id":42,"operator_id":1,"user_id":2,"message_id":333}"#,
        ) {
            Event::Notice(notice) => notice,
            other => panic!("应为通知事件: {other:?}"),
        };
        assert_eq!(notice.kind(), NoticeKind::GroupRecall);
        assert_eq!(notice.message_id.and_then(|id| id.as_i64()), Some(333));
    }

    #[test]
    fn poke_notice_is_classified_by_sub_type() {
        let notice = match parse(
            r#"{"post_type":"notice","notice_type":"notify","sub_type":"poke","time":1700000007,
                "self_id":10000,"group_id":42,"user_id":2,"target_id":10000}"#,
        ) {
            Event::Notice(notice) => notice,
            other => panic!("应为通知事件: {other:?}"),
        };
        assert_eq!(notice.kind(), NoticeKind::Poke);
        assert_eq!(notice.target_id.and_then(|id| id.as_i64()), Some(10000));
    }

    #[test]
    fn friend_add_notice_and_upload_file() {
        let notice = match parse(
            r#"{"post_type":"notice","notice_type":"group_upload","time":1700000008,
                "self_id":10000,"group_id":42,"user_id":2,
                "file":{"id":"f1","name":"a.zip","size":1024,"url":"https://x.com/a.zip"}}"#,
        ) {
            Event::Notice(notice) => notice,
            other => panic!("应为通知事件: {other:?}"),
        };
        assert_eq!(notice.kind(), NoticeKind::GroupUpload);
        let file = notice.file.as_ref().expect("应有文件信息");
        assert_eq!(file.name.as_deref(), Some("a.zip"));
        assert_eq!(file.size, Some(1024));
    }

    #[test]
    fn friend_request() {
        let Event::Request(request) = parse(
            r#"{"post_type":"request","request_type":"friend","time":1700000009,
                "self_id":10000,"user_id":2,"comment":"hi","flag":"flag-1",
                "real_user_id":"OPENID"}"#,
        ) else {
            panic!("应为请求事件");
        };
        assert_eq!(request.request_type, "friend");
        assert_eq!(request.flag, "flag-1");
        assert_eq!(
            request
                .extra
                .get("real_user_id")
                .and_then(|value| value.as_str()),
            Some("OPENID")
        );
    }

    #[test]
    fn group_invite_request() {
        let Event::Request(request) = parse(
            r#"{"post_type":"request","request_type":"group","sub_type":"invite","time":1700000010,
                "self_id":10000,"group_id":42,"user_id":2,"comment":"","flag":"f2"}"#,
        ) else {
            panic!("应为请求事件");
        };
        assert_eq!(request.request_type, "group");
        assert_eq!(request.group_id.and_then(|id| id.as_i64()), Some(42));
    }

    #[test]
    fn lifecycle_and_heartbeat_meta_events() {
        let lifecycle = parse(
            r#"{"post_type":"meta_event","meta_event_type":"lifecycle","sub_type":"connect",
                "time":1700000011,"self_id":10000}"#,
        );
        assert_eq!(lifecycle.post_type(), "meta_event");
        let Event::MetaEvent(meta) = lifecycle else {
            panic!("应为元事件");
        };
        assert_eq!(meta.meta_event_type, "lifecycle");
        assert_eq!(meta.sub_type.as_deref(), Some(sub_type::CONNECT));

        let heartbeat = parse(
            r#"{"post_type":"meta_event","meta_event_type":"heartbeat","time":1700000012,
                "self_id":10000,"interval":5000,"status":{"online":true,"good":true}}"#,
        );
        let Event::MetaEvent(meta) = heartbeat else {
            panic!("应为元事件");
        };
        assert_eq!(meta.interval, Some(5000));
        assert!(meta.status.expect("应有状态").online);
    }

    #[test]
    fn unknown_post_type_does_not_fail() {
        let event = parse(r#"{"post_type":"future_event","time":1,"self_id":2}"#);
        assert_eq!(event, Event::Unknown);
        assert_eq!(event.post_type(), "unknown");
    }

    #[test]
    fn unknown_message_type_is_kept_as_other() {
        let event = parse(
            r#"{"post_type":"message","message_type":"guild","time":1,"self_id":2,
                "message_id":3,"user_id":4,"message":"x"}"#,
        );
        let message = event.message().expect("应为消息事件");
        assert_eq!(message.message_type.as_str(), "guild");
        assert_eq!(message.message.segments(), vec![Segment::text("x")]);
    }

    #[test]
    fn event_round_trips_through_json() {
        let json = r#"{"post_type":"message","message_type":"group","time":1,"self_id":10000,
            "message_id":2,"user_id":3,"group_id":4,"message":[{"type":"text","data":{"text":"hi"}}]}"#;
        let event = parse(json);
        let encoded = serde_json::to_string(&event).expect("序列化失败");
        let decoded: Event = serde_json::from_str(&encoded).expect("二次反序列化失败");
        assert_eq!(event, decoded);
    }
}
