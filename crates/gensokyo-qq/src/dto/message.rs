//! QQ 消息 DTO（对齐 `botgo/dto/message_create.go` 的 `MessageToCreate`）。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tracing::warn;

use crate::error::{Error, Result};

/// `msg_type` 取值（与 Go 侧一致）。
pub mod msg_type {
    /// 普通文本。
    pub const TEXT: u32 = 0;
    /// Markdown。
    pub const MARKDOWN: u32 = 2;
    /// 输入状态（仅单聊）。
    pub const INPUT_NOTIFY: u32 = 6;
    /// 富媒体。
    pub const RICH_MEDIA: u32 = 7;
    /// 卡片（仅群聊）。
    pub const CARD: u32 = 8;
}

/// 富媒体文件类型（对齐官方 `file_type`）。
pub mod file_type {
    /// 图片。
    pub const IMAGE: u32 = 1;
    /// 视频。
    pub const VIDEO: u32 = 2;
    /// 语音。
    pub const VOICE: u32 = 3;
    /// 文件。
    pub const FILE: u32 = 4;
}

/// 发送消息结构体（`MessageToCreate`）。
///
/// 只建模当前出站链路真正用到的字段；`keyboard` 结构层级较深，直接用
/// [`serde_json::Value`] 透传，其余未知字段落进 [`MessageToCreate::extra`]，不会丢失。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MessageToCreate {
    /// 文本内容。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// 消息类型，见 [`msg_type`]。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg_type: Option<u32>,
    /// Markdown 消息（`msg_type=2`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markdown: Option<Markdown>,
    /// 消息按钮（`keyboard`），按 JSON 透传。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyboard: Option<Value>,
    /// 富媒体引用（`file_info`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<Media>,
    /// 要回复的消息 ID；为空是主动消息。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg_id: Option<String>,
    /// 要回复的事件 ID；与 `msg_id` / `is_wakeup` 互斥。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    /// 回复消息序号；与 `msg_id` 联合使用。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg_seq: Option<u32>,
    /// 互动召回消息标记；与 `msg_id` / `event_id` 互斥。
    #[serde(skip_serializing_if = "is_false")]
    pub is_wakeup: bool,
    /// 引用消息（频道侧结构，v2 群 / C2C 透传）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_reference: Option<MessageReference>,
    /// 未建模字段原样透传。
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl MessageToCreate {
    /// 纯文本消息。
    #[must_use]
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: Some(content.into()),
            ..Self::default()
        }
    }

    /// Markdown 消息（`msg_type=2`）。
    #[must_use]
    pub fn markdown(content: impl Into<String>) -> Self {
        Self {
            msg_type: Some(msg_type::MARKDOWN),
            markdown: Some(Markdown::content(content)),
            ..Self::default()
        }
    }

    /// 设置 `msg_id`（被动回复）。
    #[must_use]
    pub fn with_msg_id(mut self, msg_id: impl Into<String>) -> Self {
        self.msg_id = Some(msg_id.into());
        self
    }

    /// 设置 `event_id`（被动回复事件）。
    #[must_use]
    pub fn with_event_id(mut self, event_id: impl Into<String>) -> Self {
        self.event_id = Some(event_id.into());
        self
    }

    /// 设置 `msg_seq`。
    #[must_use]
    pub fn with_msg_seq(mut self, msg_seq: u32) -> Self {
        self.msg_seq = Some(msg_seq);
        self
    }

    /// 标记为互动召回消息。
    #[must_use]
    pub fn wakeup(mut self) -> Self {
        self.is_wakeup = true;
        self
    }

    /// 设置消息类型。
    #[must_use]
    pub fn with_msg_type(mut self, msg_type: u32) -> Self {
        self.msg_type = Some(msg_type);
        self
    }

    /// 追加一个未建模字段。
    ///
    /// # 警告
    /// 已建模字段名（`content` / `msg_type` / `markdown` / `keyboard` / `media` / `msg_id` /
    /// `event_id` / `msg_seq` / `is_wakeup` / `message_reference`）会被忽略并记录 `warn`：
    /// `extra` 走 `#[serde(flatten)]`，写入同名 key 会在 JSON 里产生重复键，服务端行为不确定。
    /// 这些字段请用对应的 builder（如 [`MessageToCreate::with_msg_id`]）设置。
    #[must_use]
    pub fn with_extra(mut self, key: impl Into<String>, value: Value) -> Self {
        let key = key.into();
        if MODELLED_FIELDS.contains(&key.as_str()) {
            warn!(field = %key, "with_extra 忽略已建模字段名");
            return self;
        }
        self.extra.insert(key, value);
        self
    }

    /// 校验 QQ 的互斥约束：`msg_id` / `event_id` / `is_wakeup` 至多设置一个。
    ///
    /// botgo 不做此校验（由服务端报错）；本 crate 在发请求前直接拒绝，避免把非法组合
    /// 发到线上。
    pub fn validate(&self) -> Result<()> {
        let reply_modes = [
            self.msg_id.is_some(),
            self.event_id.is_some(),
            self.is_wakeup,
        ];
        if reply_modes.iter().filter(|mode| **mode).count() > 1 {
            return Err(Error::InvalidMessage(
                "msg_id / event_id / is_wakeup 互斥，只能设置其中一个".to_owned(),
            ));
        }
        Ok(())
    }
}

/// `MessageToCreate` 已建模的 JSON 字段名；`extra` 不允许占用这些 key（否则 flatten 出重复键）。
const MODELLED_FIELDS: [&str; 10] = [
    "content",
    "msg_type",
    "markdown",
    "keyboard",
    "media",
    "msg_id",
    "event_id",
    "msg_seq",
    "is_wakeup",
    "message_reference",
];

/// Markdown 消息体。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Markdown {
    /// 官方模板 ID。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_id: Option<u32>,
    /// 群聊自定义模板 ID。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_template_id: Option<String>,
    /// 模板参数。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Vec<MarkdownParams>>,
    /// 原生 Markdown 内容。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// 官方强校验图片转存结果（默认不序列化）。
    #[serde(skip_serializing_if = "is_false")]
    pub force_verify_image_resource: bool,
}

impl Markdown {
    /// 原生 Markdown 内容。
    #[must_use]
    pub fn content(content: impl Into<String>) -> Self {
        Self {
            content: Some(content.into()),
            ..Self::default()
        }
    }
}

/// Markdown 模板参数（键值对）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkdownParams {
    /// 参数名。
    pub key: String,
    /// 参数值列表。
    pub values: Vec<String>,
}

/// 引用消息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageReference {
    /// 被引用消息 ID。
    pub message_id: String,
    /// 是否忽略获取被引用消息失败的错误。
    #[serde(default, skip_serializing_if = "is_false")]
    pub ignore_get_message_error: bool,
}

/// 富媒体引用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Media {
    /// 上传接口返回的 `file_info`。
    pub file_info: String,
}

/// 富媒体上传请求体（对齐 `botgo/dto.RichMediaMessage`，`POST /v2/.../files`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FileUpload {
    /// 文件类型，见 [`file_type`]。
    pub file_type: u32,
    /// 公网 URL。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// 无 `data:` 前缀的 base64 内容。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_data: Option<String>,
    /// 是否由服务端直接发送（true 时响应返回 `id`）。
    #[serde(skip_serializing_if = "is_false")]
    pub srv_send_msg: bool,
    /// 文件名（仅 `file_type=4` 有效）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    /// 附带文本内容。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// 要回复的事件 ID（对齐 botgo `RichMediaMessage.EventID`，语义同 `MsgID`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
}

impl FileUpload {
    /// 以 URL 上传。
    #[must_use]
    pub fn from_url(file_type: u32, url: impl Into<String>) -> Self {
        Self {
            file_type,
            url: Some(url.into()),
            ..Self::default()
        }
    }

    /// 以 base64 内容上传。
    #[must_use]
    pub fn from_base64(
        file_type: u32,
        file_data: impl Into<String>,
        file_name: impl Into<String>,
    ) -> Self {
        Self {
            file_type,
            file_data: Some(file_data.into()),
            file_name: Some(file_name.into()),
            ..Self::default()
        }
    }

    /// 设置回复的事件 ID（被动回复的场景，对齐 botgo `RichMediaMessage.EventID`）。
    #[must_use]
    pub fn with_event_id(mut self, event_id: impl Into<String>) -> Self {
        self.event_id = Some(event_id.into());
        self
    }

    /// 校验 `file_type` 与 `url` / `file_data` 二选一。
    pub fn validate(&self) -> Result<()> {
        if self.file_type == 0 {
            return Err(Error::InvalidMessage("file_type 不能为 0".to_owned()));
        }
        match (self.url.is_some(), self.file_data.is_some()) {
            (true, true) => Err(Error::InvalidMessage(
                "url 与 file_data 互斥，只能设置其中一个".to_owned(),
            )),
            (false, false) => Err(Error::InvalidMessage(
                "url 与 file_data 必须设置其中一个".to_owned(),
            )),
            _ => Ok(()),
        }
    }
}

/// 发送消息响应（群 / C2C 文本路径）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MessageResponse {
    /// 消息 ID。
    pub id: String,
    /// RFC3339 时间字符串（botgo `dto.Timestamp` 同为 string）。
    pub timestamp: String,
}

/// 富媒体上传 / 发送响应（对齐 `botgo/dto.MediaResponse`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaResponse {
    /// 文件 UUID。
    pub file_uuid: String,
    /// 文件引用信息。
    pub file_info: String,
    /// 有效期（秒）。
    pub ttl: u64,
    /// `srv_send_msg=true` 时返回的消息 ID。
    pub id: String,
    /// 媒体下载直链（部分文件类型返回）。
    pub raw_url: String,
}

/// serde 布尔省略条件。
fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn file_upload_serializes_event_id_only_when_set() {
        let with_event = FileUpload::from_url(file_type::IMAGE, "https://example.com/a.png")
            .with_event_id("evt-1");
        let value = serde_json::to_value(&with_event).unwrap();
        assert_eq!(value["event_id"], "evt-1");

        let without_event = FileUpload::from_url(file_type::IMAGE, "https://example.com/a.png");
        let value = serde_json::to_value(&without_event).unwrap();
        assert!(
            value.get("event_id").is_none(),
            "event_id 为空时不得出现在 JSON 里: {value}"
        );
    }

    #[test]
    fn file_upload_round_trips_event_id() {
        let raw = r#"{"file_type":1,"url":"https://example.com/a.png","event_id":"evt-9"}"#;
        let upload: FileUpload = serde_json::from_str(raw).unwrap();
        assert_eq!(upload.event_id.as_deref(), Some("evt-9"));
        assert_eq!(serde_json::to_string(&upload).unwrap(), raw);
    }

    #[test]
    fn with_extra_ignores_modelled_field_names() {
        let message = MessageToCreate::text("hi")
            .with_msg_id("m-1")
            .with_extra("msg_id", json!("override"))
            .with_extra("content", json!("override"));
        assert_eq!(message.msg_id.as_deref(), Some("m-1"));
        assert_eq!(message.content.as_deref(), Some("hi"));
        assert!(message.extra.is_empty(), "已建模字段不得落进 extra");

        let value = serde_json::to_value(&message).unwrap();
        assert_eq!(value["msg_id"], "m-1");
        assert_eq!(value["content"], "hi");
    }

    #[test]
    fn with_extra_keeps_unknown_fields() {
        let message = MessageToCreate::text("hi").with_extra("card", json!({"type": "t"}));
        assert_eq!(message.extra.get("card"), Some(&json!({"type": "t"})));

        let value = serde_json::to_value(&message).unwrap();
        assert_eq!(value["card"], json!({"type": "t"}));
        assert_eq!(value["content"], "hi");
    }
}
