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
    /// 卡片（群聊与单聊）。
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
    /// 图文卡片（`msg_type=8`，群聊与单聊均可）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card: Option<MessageCard>,
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

    /// 图文卡片消息（`msg_type=8`，群聊与单聊均可）。
    #[must_use]
    pub fn card(card: MessageCard) -> Self {
        Self {
            msg_type: Some(msg_type::CARD),
            card: Some(card),
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
    /// 已建模字段名（`content` / `msg_type` / `markdown` / `card` / `keyboard` / `media` / `msg_id` /
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
const MODELLED_FIELDS: [&str; 11] = [
    "content",
    "msg_type",
    "markdown",
    "card",
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

/// 图文卡片消息体（`msg_type=8`，官方 `type="tuwen"`）。
///
/// 对齐 `botgo/dto.GroupCard`：`{"type":"tuwen","content":{…}}`，群聊与单聊用同一结构。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageCard {
    /// 卡片类型，官方当前只有 `tuwen`（图文）。
    #[serde(rename = "type")]
    pub card_type: String,
    /// 卡片正文。
    pub content: MessageCardContent,
}

impl MessageCard {
    /// 图文卡片（`type="tuwen"`）。
    #[must_use]
    pub fn tuwen(content: MessageCardContent) -> Self {
        Self {
            card_type: "tuwen".to_owned(),
            content,
        }
    }
}

/// 图文卡片的四个官方字段（对齐 `botgo/dto.GroupCardContent`）。
///
/// 四项**全部必填**（官方 2026-10 消息接口页）：缺字段 / trim 后为空 / 非法都由
/// [`MessageCardContent::validate`] 拒绝为 `1400`。序列化时 `None` 仍省略该 key
/// （Go 是 `omitempty`），因此结构体本身允许构造出不完整的卡片，校验是发送前的闸门。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MessageCardContent {
    /// 标题（必填，1~50 字符）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// 描述（1~200 字符）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 跳转链接（https，≤1024 字节）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// 封面图链接（https，≤1024 字节）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pic_url: Option<String>,
}

/// 卡片字段长度上限（字符数，与官方文档一致）。
const CARD_TITLE_MAX: usize = 50;
/// 卡片描述长度上限（字符数）。
const CARD_DESCRIPTION_MAX: usize = 200;
/// 卡片链接长度上限（**字节数**，与 Go `len()` 一致）。
const CARD_URL_MAX: usize = 1024;

impl MessageCardContent {
    /// 官方字段校验（四字段**全部必填**）。
    ///
    /// - `title` 1~50 字符、`description` 1~200 字符：先 trim 再判空与长度（Go `buildTuwenCard`）；
    /// - `url` / `pic_url` 必须 `https://` 开头（大小写不敏感）且按**字节**不超过 1024
    ///   （Go 用 `len()`，不是字符数）；
    /// - 缺字段或 trim 后为空一律视为缺失（官方四字段必填，缺失即 `1400`）。
    ///
    /// # Errors
    /// 任一字段违反上述约束时返回 [`Error::InvalidMessage`]。
    pub fn validate(&self) -> Result<()> {
        check_card_len("title", self.title.as_deref(), CARD_TITLE_MAX)?;
        check_card_len(
            "description",
            self.description.as_deref(),
            CARD_DESCRIPTION_MAX,
        )?;
        check_card_url("url", self.url.as_deref())?;
        check_card_url("pic_url", self.pic_url.as_deref())?;
        Ok(())
    }
}

/// 卡片文本字段：trim 后 1~`max` 个字符（按 Unicode 字符数，与 Go `RuneCountInString` 一致）。
fn check_card_len(field: &str, value: Option<&str>, max: usize) -> Result<()> {
    let trimmed = value.unwrap_or_default().trim();
    let len = trimmed.chars().count();
    if len == 0 {
        return Err(Error::InvalidMessage(format!(
            "卡片缺少 {field}（官方要求 1~{max} 字符）"
        )));
    }
    if len > max {
        return Err(Error::InvalidMessage(format!(
            "卡片 {field} 长度必须在 1~{max} 字符之间（当前 {len}）"
        )));
    }
    Ok(())
}

/// 卡片链接字段：必填，`https://` 开头（大小写不敏感），按字节不超过 1024。
fn check_card_url(field: &str, value: Option<&str>) -> Result<()> {
    let trimmed = value.unwrap_or_default().trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidMessage(format!(
            "卡片缺少 {field}（官方要求 https 链接，不超过 {CARD_URL_MAX} 字节）"
        )));
    }
    const HTTPS: &[u8] = b"https://";
    if !trimmed
        .as_bytes()
        .get(..HTTPS.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(HTTPS))
    {
        return Err(Error::InvalidMessage(format!(
            "卡片 {field} 必须是 https 链接: {trimmed}"
        )));
    }
    let len = trimmed.len();
    if len > CARD_URL_MAX {
        return Err(Error::InvalidMessage(format!(
            "卡片 {field} 不能超过 {CARD_URL_MAX} 字节（当前 {len}）"
        )));
    }
    Ok(())
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
        let message = MessageToCreate::text("hi").with_extra("foo", json!({"type": "t"}));
        assert_eq!(message.extra.get("foo"), Some(&json!({"type": "t"})));

        let value = serde_json::to_value(&message).unwrap();
        assert_eq!(value["foo"], json!({"type": "t"}));
        assert_eq!(value["content"], "hi");
    }

    #[test]
    fn card_is_a_modelled_field() {
        // `card` 已建模：写入 extra 会被忽略，出站 JSON 里只有结构化卡片
        let message = MessageToCreate::card(MessageCard::tuwen(MessageCardContent {
            title: Some("标题".to_owned()),
            ..MessageCardContent::default()
        }))
        .with_extra("card", json!("override"));

        assert!(message.extra.is_empty(), "已建模字段不得落进 extra");
        assert_eq!(message.card.as_ref().map(|card| card.card_type.as_str()), Some("tuwen"));
    }

    #[test]
    fn markdown_message_carries_content_only() {
        let message = MessageToCreate::markdown("# 标题");
        let value = serde_json::to_value(&message).unwrap();

        assert_eq!(value["msg_type"], 2);
        assert_eq!(value["markdown"], json!({"content": "# 标题"}));
        assert!(
            value["markdown"].get("template_id").is_none()
                && value["markdown"].get("custom_template_id").is_none(),
            "模板字段已废弃，出站不得构造: {value}"
        );
        assert!(value.get("content").is_none(), "markdown 消息只要 markdown 载荷");
    }

    #[test]
    fn card_message_serializes_official_tuwen_shape() {
        let message = MessageToCreate::card(MessageCard::tuwen(MessageCardContent {
            title: Some("标题".to_owned()),
            description: Some("描述".to_owned()),
            url: Some("https://example.com/a".to_owned()),
            pic_url: Some("https://example.com/a.png".to_owned()),
        }));
        let value = serde_json::to_value(&message).unwrap();

        assert_eq!(value["msg_type"], 8);
        assert_eq!(
            value["card"],
            json!({
                "type": "tuwen",
                "content": {
                    "title": "标题",
                    "description": "描述",
                    "url": "https://example.com/a",
                    "pic_url": "https://example.com/a.png",
                }
            })
        );
    }

    /// 四字段齐备的合法卡片（各用例只改其中一项来验证隔离性）。
    fn valid_card() -> MessageCardContent {
        MessageCardContent {
            title: Some("标题".to_owned()),
            description: Some("描述".to_owned()),
            url: Some("https://example.com/a".to_owned()),
            pic_url: Some("https://example.com/a.png".to_owned()),
        }
    }

    #[test]
    fn card_content_absent_fields_are_omitted_in_json_but_rejected() {
        let card = MessageCard::tuwen(MessageCardContent {
            title: Some("只有标题".to_owned()),
            ..MessageCardContent::default()
        });
        let value = serde_json::to_value(&card).unwrap();

        assert_eq!(value, json!({"type": "tuwen", "content": {"title": "只有标题"}}));
        // 序列化仍省略缺省键（Go omitempty），但四字段必填：发送前 validate 必须拒绝
        assert!(
            card.content.validate().is_err(),
            "四字段缺一即 1400，不能靠省略字段绕过"
        );
    }

    #[test]
    fn card_content_requires_all_four_fields() {
        assert!(valid_card().validate().is_ok(), "四字段齐备应放行");

        let mut missing_title = valid_card();
        missing_title.title = None;
        assert!(missing_title.validate().is_err(), "缺 title 必须拒绝");

        let mut missing_description = valid_card();
        missing_description.description = None;
        assert!(
            missing_description.validate().is_err(),
            "缺 description 必须拒绝（旧实现放行）"
        );

        let mut missing_url = valid_card();
        missing_url.url = None;
        assert!(missing_url.validate().is_err(), "缺 url 必须拒绝（旧实现放行）");

        let mut missing_pic = valid_card();
        missing_pic.pic_url = None;
        assert!(
            missing_pic.validate().is_err(),
            "缺 pic_url 必须拒绝（旧实现放行）"
        );
    }

    #[test]
    fn card_content_treats_blank_as_missing() {
        for blank in ["", "   ", "\t\n"] {
            let mut content = valid_card();
            content.title = Some(blank.to_owned());
            assert!(content.validate().is_err(), "空白 title 必须拒绝: {blank:?}");

            let mut content = valid_card();
            content.description = Some(blank.to_owned());
            assert!(
                content.validate().is_err(),
                "空白 description 必须拒绝（官方必填）: {blank:?}"
            );

            let mut content = valid_card();
            content.url = Some(blank.to_owned());
            assert!(
                content.validate().is_err(),
                "空白 url 必须拒绝（官方必填）: {blank:?}"
            );

            let mut content = valid_card();
            content.pic_url = Some(blank.to_owned());
            assert!(
                content.validate().is_err(),
                "空白 pic_url 必须拒绝（官方必填）: {blank:?}"
            );
        }

        // 先 trim 再判长度：两侧空白不影响合法性（Go 同样 TrimSpace）
        let mut padded = valid_card();
        padded.title = Some("  标题  ".to_owned());
        padded.description = Some(" 描述 ".to_owned());
        padded.url = Some(" https://example.com/a ".to_owned());
        padded.pic_url = Some(" https://example.com/a.png ".to_owned());
        assert!(padded.validate().is_ok(), "trim 后合法应放行: {padded:?}");
    }

    #[test]
    fn card_content_accepts_official_boundaries() {
        let at_boundary = MessageCardContent {
            title: Some("标".repeat(50)),
            description: Some("描".repeat(200)),
            url: Some(format!("https://example.com/{}", "a".repeat(1024 - 20))),
            pic_url: Some(format!("https://example.com/{}", "a".repeat(1024 - 20))),
        };
        assert_eq!(at_boundary.title.as_deref().map(|t| t.chars().count()), Some(50));
        assert_eq!(at_boundary.url.as_deref().map(|u| u.len()), Some(1024));
        assert!(at_boundary.validate().is_ok(), "恰好 50/200/1024 应放行");
    }

    #[test]
    fn card_content_rejects_out_of_range_fields() {
        let mut long_title = valid_card();
        long_title.title = Some("标".repeat(51));
        assert!(long_title.validate().is_err(), "51 字符标题必须拒绝");

        let mut long_description = valid_card();
        long_description.description = Some("描".repeat(201));
        assert!(long_description.validate().is_err(), "201 字符描述必须拒绝");

        let mut too_long_url = valid_card();
        too_long_url.url = Some(format!("https://example.com/{}", "a".repeat(1024)));
        assert!(too_long_url.validate().is_err(), "超过 1024 字节的 url 必须拒绝");

        let mut too_long_pic = valid_card();
        too_long_pic.pic_url = Some(format!("https://example.com/{}", "a".repeat(1024)));
        assert!(
            too_long_pic.validate().is_err(),
            "超过 1024 字节的 pic_url 必须拒绝"
        );
    }

    #[test]
    fn card_content_url_length_is_counted_in_bytes() {
        // 406 字符但 1213 字节：Go 用 len()，按字符数会误放行
        let long_url = format!("https://example.com/{}", "汉".repeat(400));
        assert!(long_url.chars().count() < 1024, "字符数没超");
        assert!(long_url.len() > 1024, "字节数超了");

        let mut content = valid_card();
        content.url = Some(long_url);
        assert!(content.validate().is_err(), "URL 长度按字节，超 1024 字节必须拒绝");
    }

    #[test]
    fn card_content_requires_https_links() {
        for url in ["http://example.com/a", "/relative/a.png", "ftp://example.com/a"] {
            let mut content = valid_card();
            content.url = Some(url.to_owned());
            assert!(content.validate().is_err(), "非 https 的 url 必须拒绝: {url}");

            let mut content = valid_card();
            content.pic_url = Some(url.to_owned());
            assert!(
                content.validate().is_err(),
                "非 https 的 pic_url 必须拒绝: {url}"
            );
        }

        // 协议前缀大小写不敏感（Go `strings.ToLower` 后比前缀）
        let mut upper = valid_card();
        upper.url = Some("HTTPS://example.com/a".to_owned());
        assert!(upper.validate().is_ok(), "https 前缀大小写不敏感");
    }
}
