//! [`OutboundPlan`]：Go 版 `foundItems` 的类型化替代（design §3.3）。
//!
//! Go 用 `map[string][]string` + "循环跳过控制型 key、循环内再回读 reply" 的隐式约定；
//! 这里用字段与枚举把同一语义显式表达，key 映射保持 1:1（见 [`OutboundPlan::from_found_items`]）。
//!
//! # 顺序语义
//! `foundItems` 本身是无序 map，只有同一 key 内部有序。因此 [`OutboundPlan`] 保留
//! **每个桶内部的到达顺序**，跨桶顺序由 [`OutboundPlan::to_segments`] 的固定次序给出：
//!
//! ```text
//! reply → text（含 [CQ:at] 字面量，按 CQ 重新分词）→ active → wakeup
//!       → images → records → videos → files → qqmusic
//!       → markdown → keyboard → card → input_notify → stream → extra
//! ```
//!
//! reply 恒在首位：Go 会把 `reply_msg_id` 注入到**每条**媒体消息的 `MessageReference`，
//! 等价于"引用先于正文"；数组形态下这就是第一个 `reply` 段。
//!
//! # 无法归类的段
//! `face` / `node` / `poke` / `set_group` 等没有对应 `foundItems` key 的段落进
//! [`OutboundPlan::extra`]，按到达顺序原样保留，`to_segments` 时排在最后。
//!
//! # 与 Go 的字段差异
//!
//! `foundItems` 是 `map[string][]string`，这里把同一契约类型化；差异只在表达方式，线上值不变：
//!
//! | 载荷 | Go | 本 crate |
//! |------|----|----------|
//! | markdown | 只搬 base64(JSON) 字节串（`handlers/message_parser.go:1048-1097`） | 原样承载，另提供 [`MarkdownPayload::decode`] / `content` / `custom_template_id` / `keyboard` 类型化视图（对齐 `dto.Markdown`） |
//! | keyboard | 对象 → JSON 串；`base64://` → 解码；其余原样（同文件 `1103-1134`） | 同左（[`KeyboardPayload`]） |
//! | card | 段路径只认 `title`/`desc`/`pic`/`url`（`handlers/message_parser.go:988-1010`），CQ 文本路径收全部 `k=v`（`handlers/cqcode_pipeline.go:114-132`） | 收全部 `k=v`（CQ 路径口径），另提供 [`CardPayload::fields`] 取四个官方字段 |
//! | stream | `foundItems` 只写 `type`/`qq`；`stream_msg_id`/`index`/`msg_seq` 由 `handlers/send_private_msg.go:51-55` 的进程内 `streamCache` 维护，不落 `foundItems` | 升为 [`StreamPayload`] 的可选字段 `id`/`index`/`seq`，只在有值时附加进 JSON（解析时也认 Go 的 `stream_msg_id`/`msg_seq` 键名） |
//! | input_notify | `type` 恒写、`second` 可缺，值均为字符串 | [`InputNotifyPayload::kind`] 为 `i64`（非数字段的整段进 `extra`），`second` 为 `Option<i64>` |
//! | wakeup | 独立 key `wakeup` | 归到 [`ActiveKind::wakeup`]（分组不同，值相同） |
//!
//! `markdown` / `keyboard` / `card` 在此是 `Vec`：Go 对同名 key 是 `append`，一条消息可带多份，
//! 收敛成 `Option` 会丢信息。`stream` / `input_notify` 在 Go 里实际只消费首项，故为 `Option`。

use std::collections::BTreeMap;

use serde_json::Value;

use gensokyo_core::{Error, Ob11Id, Result};

use crate::base64;
use crate::cqcode;
use crate::parse_id;
use crate::segment::{MediaRef, Segment};

/// Go `foundItems` 的等价物：key → 多值，同一 key 内保序。
pub type FoundItems = BTreeMap<String, Vec<String>>;

/// 主动推送标记（对应 Go 的 `active` / `active_type` / `active_sub_type` / `wakeup`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActiveKind {
    /// Go 的 `foundItems["active"]` 标记（`[CQ:active]` 出现过即为 `true`）。
    pub active: bool,
    /// 主动推送大类（`active_type`）。
    pub kind: Option<String>,
    /// 子类型（`active_sub_type`）。
    pub sub_type: Option<String>,
    /// 唤醒目标（`send_private_msg_wakeup` 的 `wakeup` key）。
    pub wakeup: Option<String>,
}

impl ActiveKind {
    /// 所有标记都为空时视为未设置。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.active && self.kind.is_none() && self.sub_type.is_none() && self.wakeup.is_none()
    }
}

/// 文件引用：`file_name` 与文件本身绑定，避免 Go 里"循环跳过再回读"的隐式约定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRef {
    /// 文件来源。
    pub media: MediaRef,
    /// 显示文件名。
    pub file_name: Option<String>,
}

/// Markdown 载荷。
///
/// Go 的 `foundItems["markdown"]` 是 base64(JSON)（`handlers/message_parser.go` 的
/// `mdContentEncoded`）或原始 JSON 串；协议层不重新编码，原样承载，避免把 M2 的编码决策
/// 固化在模型里。读字段用 [`Self::decode`] 系列（对齐 `dto.Markdown`：`content` /
/// `custom_template_id`，内嵌键盘在 `keyboard`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownPayload {
    /// 原始 `data=` 值。
    pub raw: String,
}

impl MarkdownPayload {
    /// 直接承载已编码好的 base64(JSON)。
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        Self { raw: raw.into() }
    }

    /// JSON 文本 → base64(JSON)（Go 的"对象先 marshal 再编码"分支）。
    ///
    /// # Errors
    /// 入参不是合法 JSON 时返回 [`Error::Protocol`]。
    pub fn from_json(raw: &str) -> Result<Self> {
        let value: Value = serde_json::from_str(raw)
            .map_err(|error| Error::Protocol(format!("markdown 载荷不是合法 JSON: {error}")))?;
        Ok(Self {
            raw: base64::encode(value.to_string().as_bytes()),
        })
    }

    /// 解码回 JSON 值。
    ///
    /// # Errors
    /// 非 base64，或解出的字节不是合法 JSON 时返回 [`Error::Protocol`]。
    pub fn decode(&self) -> Result<Value> {
        let bytes = base64::decode(&self.raw)
            .ok_or_else(|| Error::Protocol("markdown 载荷不是合法 base64".to_owned()))?;
        serde_json::from_slice(&bytes).map_err(|error| {
            Error::Protocol(format!("markdown 载荷 base64 解码后不是合法 JSON: {error}"))
        })
    }

    /// `content` 字段（原生 markdown 文本）。
    #[must_use]
    pub fn content(&self) -> Option<String> {
        self.decode().ok()?.get("content").and_then(json_scalar)
    }

    /// `custom_template_id` 字段（群模板 id）。
    #[must_use]
    pub fn custom_template_id(&self) -> Option<String> {
        self.decode()
            .ok()?
            .get("custom_template_id")
            .and_then(json_scalar)
    }

    /// `keyboard` 字段（markdown 内嵌键盘）。
    #[must_use]
    pub fn keyboard(&self) -> Option<Value> {
        self.decode().ok()?.get("keyboard").cloned()
    }
}

/// 键盘载荷（`foundItems["keyboard"]`，JSON 字符串）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardPayload {
    /// 原始 JSON。
    pub raw: String,
}

/// 图文卡片载荷（`foundItems["card"]`，JSON 字符串）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardPayload {
    /// 原始 JSON。
    pub raw: String,
}

/// 卡片的四个官方字段（`handlers/message_parser.go:991-1003` 只把这些写进卡片）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CardFields {
    /// 卡片标题。
    pub title: Option<String>,
    /// 卡片描述。
    pub desc: Option<String>,
    /// 封面图 URL。
    pub pic: Option<String>,
    /// 跳转链接。
    pub url: Option<String>,
}

impl CardFields {
    /// 四个字段都为空时没有可发送内容。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.desc.is_none() && self.pic.is_none() && self.url.is_none()
    }
}

impl CardPayload {
    /// 由字段表构造：只写有值的键（键序与 Go 的 map marshal 一致，字典序）。
    #[must_use]
    pub fn from_fields(fields: &CardFields) -> Self {
        let mut object = serde_json::Map::new();
        for (key, value) in [
            ("title", &fields.title),
            ("desc", &fields.desc),
            ("pic", &fields.pic),
            ("url", &fields.url),
        ] {
            if let Some(value) = value {
                object.insert(key.to_owned(), Value::String(value.clone()));
            }
        }
        Self {
            raw: Value::Object(object).to_string(),
        }
    }

    /// 取出四个官方字段。
    ///
    /// 别名与 Go `cardFieldsFromMap` 同口径：`desc` / `pic` 优先，为空时回退到
    /// `description` / `pic_url`（Go 两者都认，避免同一份载荷两侧解析结果不同）。
    ///
    /// # Errors
    /// 载荷不是 JSON 对象时返回 [`Error::Protocol`]。
    pub fn fields(&self) -> Result<CardFields> {
        let value: Value = serde_json::from_str(&self.raw)
            .map_err(|error| Error::Protocol(format!("card 载荷不是合法 JSON: {error}")))?;
        if !value.is_object() {
            return Err(Error::Protocol("card 载荷不是 JSON 对象".to_owned()));
        }
        Ok(CardFields {
            title: value.get("title").and_then(json_scalar),
            desc: first_card_alias(&value, &["desc", "description"]),
            pic: first_card_alias(&value, &["pic", "pic_url"]),
            url: value.get("url").and_then(json_scalar),
        })
    }
}

/// 别名取值：按顺序取第一个 trim 后非空的值（Go `firstNonEmptyCardValue`）。
fn first_card_alias(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| value.get(*key).and_then(json_scalar))
        .find(|candidate| !candidate.trim().is_empty())
}

/// 流式消息阶段（`[CQ:stream,type=…]`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamKind {
    /// 首片。
    Start,
    /// 续片。
    Mid,
    /// 终片。
    Finish,
    /// 其它（保留原值）。
    Other(String),
}

impl StreamKind {
    /// 线上取值。
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Start => "start",
            Self::Mid => "mid",
            Self::Finish => "finish",
            Self::Other(value) => value,
        }
    }
}

impl From<&str> for StreamKind {
    fn from(value: &str) -> Self {
        match value {
            "start" => Self::Start,
            "mid" => Self::Mid,
            "finish" => Self::Finish,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// 流式消息载荷。
///
/// Go 的 `foundItems["stream"]` 只带 `type`/`qq`，`stream_msg_id`、`index`、`msg_seq` 由
/// `handlers/send_private_msg.go:51-55` 的进程内 `streamCache` 维护。这里把它们升为可选字段，
/// 由调用方显式给出：`start` 可全省，`mid`/`finish` 缺 `id` 等价于 Go 的"缺少 stream_msg_id，跳过"。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamPayload {
    /// 阶段。
    pub kind: StreamKind,
    /// 目标用户 ID（虚拟）。
    pub qq: Ob11Id,
    /// 首片返回的 `stream_msg_id`（Go `streamInfo.StreamMsgID`）。
    pub id: Option<String>,
    /// 分片序号（Go `streamInfo.Index`）。
    pub index: Option<i64>,
    /// 回复序号（`msg_seq`）。
    pub seq: Option<i64>,
}

/// 输入状态载荷（`[CQ:input_notify,type=…,second=…]`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputNotifyPayload {
    /// 状态类型。
    pub kind: i64,
    /// 持续秒数。
    pub second: Option<i64>,
}

/// 一条出站消息的完整意图。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutboundPlan {
    /// 文本主体（可能含 `[CQ:at]` 等文本内 CQ 码，与 Go 的 `messageText` 一致）。
    pub text: String,
    /// 回复目标消息 ID。
    pub reply_msg_id: Option<Ob11Id>,
    /// 主动推送 / 唤醒标记。
    pub active: Option<ActiveKind>,
    /// Markdown 消息。
    pub markdown: Vec<MarkdownPayload>,
    /// 按钮 / 键盘。
    pub keyboard: Vec<KeyboardPayload>,
    /// 图片。
    pub images: Vec<MediaRef>,
    /// 语音。
    pub records: Vec<MediaRef>,
    /// 视频。
    pub videos: Vec<MediaRef>,
    /// 文件。
    pub files: Vec<FileRef>,
    /// QQ 音乐分享 ID 列表。
    pub qqmusic: Vec<String>,
    /// 图文卡片。
    pub card: Vec<CardPayload>,
    /// 输入状态。
    pub input_notify: Option<InputNotifyPayload>,
    /// 流式消息。
    pub stream: Option<StreamPayload>,
    /// 无法识别的图片（Go `unknown_image`），按到达顺序回退发送。
    pub unknown_images: Vec<MediaRef>,
    /// 无法识别的语音（Go `unknown_record`），按到达顺序回退发送。
    ///
    /// 与图片分开保留：Go 的两路兜底发送不同（`handlers/send_group_msg.go` 分别走图片/语音
    /// 通道），合成一个桶会在回写 `foundItems` 时丢失原类型。
    pub unknown_records: Vec<MediaRef>,
    /// 无 `foundItems` 对应 key 的消息段（`face` / `node` / `poke` …），按到达顺序保留。
    pub extra: Vec<Segment>,
}

impl OutboundPlan {
    /// 是否没有任何可发送内容（reply / active 不算内容，与 Go 的跳过语义一致）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
            && self.markdown.is_empty()
            && self.keyboard.is_empty()
            && self.images.is_empty()
            && self.records.is_empty()
            && self.videos.is_empty()
            && self.files.is_empty()
            && self.qqmusic.is_empty()
            && self.card.is_empty()
            && self.input_notify.is_none()
            && self.stream.is_none()
            && self.unknown_images.is_empty()
            && self.unknown_records.is_empty()
            && self.extra.is_empty()
    }

    /// 由 `foundItems` 构造（key 映射 1:1，见模块文档）。
    ///
    /// `text` 不在 `foundItems` 里（Go 的 `parseMessageContent` 单独返回），调用方自行赋值。
    ///
    /// # Errors
    /// `stream` / `input_notify` 的 JSON 结构非法时返回 [`Error::Protocol`]；其余 key 只做
    /// 原样搬运，不会失败。
    pub fn from_found_items(items: &FoundItems) -> Result<Self> {
        let mut plan = Self::default();

        if let Some(value) = first(items, "reply_msg_id") {
            plan.reply_msg_id = Some(parse_id(value));
        }

        let active = ActiveKind {
            active: items.contains_key("active"),
            kind: first(items, "active_type").map(ToOwned::to_owned),
            sub_type: first(items, "active_sub_type").map(ToOwned::to_owned),
            wakeup: first(items, "wakeup").map(ToOwned::to_owned),
        };
        plan.active = (!active.is_empty()).then_some(active);

        for value in values(items, "markdown") {
            plan.markdown.push(MarkdownPayload { raw: value.clone() });
        }
        for value in values(items, "keyboard") {
            plan.keyboard.push(KeyboardPayload { raw: value.clone() });
        }
        for value in values(items, "card") {
            plan.card.push(CardPayload { raw: value.clone() });
        }
        plan.qqmusic = values(items, "qqmusic").cloned().collect();

        for key in IMAGE_KEYS {
            push_media(&mut plan.images, key, values(items, key));
        }
        for key in RECORD_KEYS {
            push_media(&mut plan.records, key, values(items, key));
        }
        for key in VIDEO_KEYS {
            push_media(&mut plan.videos, key, values(items, key));
        }
        for key in UNKNOWN_IMAGE_KEYS {
            push_media(&mut plan.unknown_images, key, values(items, key));
        }
        for key in UNKNOWN_RECORD_KEYS {
            push_media(&mut plan.unknown_records, key, values(items, key));
        }

        let mut files = Vec::new();
        for key in FILE_KEYS {
            for value in values(items, key) {
                files.push(FileRef {
                    media: media_from_key(key, value),
                    file_name: None,
                });
            }
        }
        // Go 按到达顺序逐条追加 file_name；多出的名字没有归属，直接忽略
        for (file, name) in files.iter_mut().zip(values(items, "file_name")) {
            file.file_name = Some(name.clone());
        }
        plan.files = files;

        if let Some(raw) = first(items, "stream") {
            plan.stream = Some(parse_stream(raw)?);
        }
        if let Some(raw) = first(items, "input_notify") {
            plan.input_notify = Some(parse_input_notify(raw)?);
        }

        Ok(plan)
    }

    /// 还原成 `foundItems`（供 M2 与 Go 语义对拍，以及旧桥接层复用）。
    #[must_use]
    pub fn to_found_items(&self) -> FoundItems {
        let mut items = FoundItems::new();

        if let Some(reply) = &self.reply_msg_id {
            items.insert("reply_msg_id".to_owned(), vec![reply.to_string()]);
        }
        if let Some(active) = &self.active {
            if active.active {
                items.insert("active".to_owned(), vec!["true".to_owned()]);
            }
            if let Some(kind) = &active.kind {
                items.insert("active_type".to_owned(), vec![kind.clone()]);
            }
            if let Some(sub_type) = &active.sub_type {
                items.insert("active_sub_type".to_owned(), vec![sub_type.clone()]);
            }
            if let Some(wakeup) = &active.wakeup {
                items.insert("wakeup".to_owned(), vec![wakeup.clone()]);
            }
        }

        for payload in &self.markdown {
            items
                .entry("markdown".to_owned())
                .or_default()
                .push(payload.raw.clone());
        }
        for payload in &self.keyboard {
            items
                .entry("keyboard".to_owned())
                .or_default()
                .push(payload.raw.clone());
        }
        for payload in &self.card {
            items
                .entry("card".to_owned())
                .or_default()
                .push(payload.raw.clone());
        }
        if !self.qqmusic.is_empty() {
            items.insert("qqmusic".to_owned(), self.qqmusic.clone());
        }

        collect_media(&mut items, "image", &self.images);
        collect_media(&mut items, "record", &self.records);
        collect_media(&mut items, "video", &self.videos);
        // unknown_* 与 Go 的 key 一一对应，不再合并成单一桶
        if !self.unknown_images.is_empty() {
            items.insert(
                "unknown_image".to_owned(),
                self.unknown_images
                    .iter()
                    .map(MediaRef::to_found_item_value)
                    .collect(),
            );
        }
        if !self.unknown_records.is_empty() {
            items.insert(
                "unknown_record".to_owned(),
                self.unknown_records
                    .iter()
                    .map(MediaRef::to_found_item_value)
                    .collect(),
            );
        }

        for file in &self.files {
            items
                .entry(format!("{}_file", media_key(&file.media)))
                .or_default()
                .push(file.media.to_found_item_value());
            if let Some(name) = &file.file_name {
                items
                    .entry("file_name".to_owned())
                    .or_default()
                    .push(name.clone());
            }
        }

        if let Some(stream) = &self.stream {
            let mut object = serde_json::Map::new();
            object.insert(
                "type".to_owned(),
                Value::String(stream.kind.as_str().to_owned()),
            );
            object.insert("qq".to_owned(), Value::String(stream.qq.to_string()));
            if let Some(id) = &stream.id {
                object.insert("id".to_owned(), Value::String(id.clone()));
            }
            if let Some(index) = stream.index {
                object.insert("index".to_owned(), Value::String(index.to_string()));
            }
            if let Some(seq) = stream.seq {
                object.insert("seq".to_owned(), Value::String(seq.to_string()));
            }
            items.insert("stream".to_owned(), vec![Value::Object(object).to_string()]);
        }
        if let Some(notify) = &self.input_notify {
            let mut object = serde_json::Map::new();
            object.insert("type".to_owned(), Value::String(notify.kind.to_string()));
            if let Some(second) = notify.second {
                object.insert("second".to_owned(), Value::String(second.to_string()));
            }
            items.insert(
                "input_notify".to_owned(),
                vec![Value::Object(object).to_string()],
            );
        }

        items
    }

    /// 由消息段序列构造（入站上报 / 出站 CQ 解析共用）。
    ///
    /// - `text` 段按原样拼接进 [`Self::text`]；`at` 段也写回 `text`（保留 CQ 字面量），
    ///   与 Go 的 `messageText` 一致。
    /// - `reply` 段取第一个，落到 [`Self::reply_msg_id`]。
    /// - 其余无法归类的段进 [`Self::extra`]，不静默丢弃。
    #[must_use]
    pub fn from_segments(segments: &[Segment]) -> Self {
        let mut plan = Self::default();
        let mut active = ActiveKind::default();

        for segment in segments {
            match segment.kind.as_str() {
                "text" => plan
                    .text
                    .push_str(segment.text_content().unwrap_or_default()),
                "at" => plan.text.push_str(&segment.to_cq()),
                "reply" => {
                    if plan.reply_msg_id.is_none() {
                        plan.reply_msg_id = segment.id_param("id");
                    }
                }
                "image" => plan.images.push(media_from_segment(segment)),
                "record" => plan.records.push(media_from_segment(segment)),
                "video" => plan.videos.push(media_from_segment(segment)),
                "file" => plan.files.push(FileRef {
                    media: media_from_segment(segment),
                    file_name: segment
                        .get("file_name")
                        .or_else(|| segment.get("name"))
                        .map(ToOwned::to_owned),
                }),
                "markdown" => {
                    let raw = markdown_payload(segment);
                    if raw.is_empty() {
                        // Go 缺 data.data 时记日志后跳过（message_parser.go:1099-1101）；
                        // 这里保留段本身，模型层不静默丢载荷
                        plan.extra.push(segment.clone());
                    } else {
                        plan.markdown.push(MarkdownPayload { raw });
                    }
                }
                "keyboard" => {
                    let raw = keyboard_payload(segment);
                    if raw.is_empty() {
                        plan.extra.push(segment.clone());
                    } else {
                        plan.keyboard.push(KeyboardPayload { raw });
                    }
                }
                "card" => {
                    let raw = card_payload(segment);
                    if raw.is_empty() {
                        plan.extra.push(segment.clone());
                    } else {
                        plan.card.push(CardPayload { raw });
                    }
                }
                "music" => {
                    if let Some(id) = segment.get("id") {
                        plan.qqmusic.push(id.to_owned());
                    } else {
                        plan.extra.push(segment.clone());
                    }
                }
                "stream" => match stream_from_segment(segment) {
                    Some(stream) => plan.stream = Some(stream),
                    None => plan.extra.push(segment.clone()),
                },
                "input_notify" => match input_notify_from_segment(segment) {
                    Some(notify) => plan.input_notify = Some(notify),
                    None => plan.extra.push(segment.clone()),
                },
                "active" => {
                    active.active = true;
                    if let Some(kind) = segment.get("type") {
                        active.kind = Some(kind.to_owned());
                    }
                    if let Some(sub_type) = segment.get("sub_type") {
                        active.sub_type = Some(sub_type.to_owned());
                    }
                }
                "wakeup" => {
                    if let Some(user) = segment.get("userid") {
                        active.wakeup = Some(user.to_owned());
                    }
                }
                _ => plan.extra.push(segment.clone()),
            }
        }

        plan.active = (!active.is_empty()).then_some(active);
        plan
    }

    /// 展开为消息段序列（顺序见模块文档）。
    #[must_use]
    pub fn to_segments(&self) -> Vec<Segment> {
        let mut out = Vec::new();

        if let Some(reply) = &self.reply_msg_id {
            out.push(Segment::reply(reply.clone()));
        }
        if !self.text.is_empty() {
            // text 里可能有 [CQ:at] 字面量，按 CQ 重新分词
            out.extend(cqcode::parse(&self.text));
        }
        if let Some(active) = &self.active {
            if active.active || active.kind.is_some() || active.sub_type.is_some() {
                out.push(Segment::active(
                    active.kind.as_deref(),
                    active.sub_type.as_deref(),
                ));
            }
            if let Some(wakeup) = &active.wakeup {
                out.push(Segment::wakeup(wakeup.clone()));
            }
        }

        out.extend(self.images.iter().map(Segment::image));
        out.extend(self.records.iter().map(Segment::record));
        out.extend(self.videos.iter().map(Segment::video));
        out.extend(
            self.files
                .iter()
                .map(|file| Segment::file(&file.media, file.file_name.as_deref())),
        );
        out.extend(self.qqmusic.iter().map(|id| Segment::music(id.clone())));
        out.extend(
            self.markdown
                .iter()
                .map(|payload| Segment::markdown(payload.raw.as_str())),
        );
        out.extend(
            self.keyboard
                .iter()
                .map(|payload| Segment::keyboard(payload.raw.as_str())),
        );
        out.extend(
            self.card
                .iter()
                .map(|payload| Segment::card(payload.raw.as_str())),
        );

        if let Some(notify) = &self.input_notify {
            out.push(Segment::input_notify(notify.kind, notify.second));
        }
        if let Some(stream) = &self.stream {
            let mut segment = Segment::stream(stream.kind.as_str(), stream.qq.clone());
            if let Some(id) = &stream.id {
                segment = segment.param("id", id.clone());
            }
            if let Some(index) = stream.index {
                segment = segment.param("index", index.to_string());
            }
            if let Some(seq) = stream.seq {
                segment = segment.param("seq", seq.to_string());
            }
            out.push(segment);
        }

        out.extend(self.extra.iter().cloned());
        out
    }
}

const IMAGE_KEYS: &[&str] = &["local_image", "url_image", "url_images", "base64_image"];
const RECORD_KEYS: &[&str] = &["local_record", "url_record", "url_records", "base64_record"];
const VIDEO_KEYS: &[&str] = &["local_video", "url_video", "url_videos", "base64_video"];
const FILE_KEYS: &[&str] = &[
    "local_file",
    "url_file",
    "url_files",
    "base64_file",
    "unknown_file",
];
const UNKNOWN_IMAGE_KEYS: &[&str] = &["unknown_image"];
const UNKNOWN_RECORD_KEYS: &[&str] = &["unknown_record"];

fn values<'a>(items: &'a FoundItems, key: &str) -> impl Iterator<Item = &'a String> {
    items.get(key).into_iter().flatten()
}

fn first<'a>(items: &'a FoundItems, key: &str) -> Option<&'a String> {
    values(items, key).next()
}

fn media_from_key(key: &str, value: &str) -> MediaRef {
    if key.starts_with("local_") {
        MediaRef::Local(value.into())
    } else if key.starts_with("url_") {
        MediaRef::Url(value.to_owned())
    } else if key.starts_with("base64_") {
        MediaRef::Base64(value.to_owned())
    } else {
        MediaRef::Unknown(value.to_owned())
    }
}

fn push_media<'a>(bucket: &mut Vec<MediaRef>, key: &str, values: impl Iterator<Item = &'a String>) {
    for value in values {
        bucket.push(media_from_key(key, value));
    }
}

fn media_from_segment(segment: &Segment) -> MediaRef {
    MediaRef::from_wire(segment.get("file").unwrap_or_default())
}

/// `[CQ:markdown,data=…]` → `foundItems["markdown"]` 契约（base64(JSON)）。
///
/// Go `handlers/message_parser.go:1048-1097`：`data` 是对象时 marshal 后编码；字符串先认
/// `base64://` 前缀，再认裸 base64（必须解出 JSON），最后按旧格式 JSON 串处理。
fn markdown_payload(segment: &Segment) -> String {
    let Some(data) = segment.get("data") else {
        return String::new();
    };
    if let Some(encoded) = data.strip_prefix("base64://") {
        return encoded.to_owned();
    }
    // 裸 base64：解出合法 JSON 才算（Go 段路径先试 base64，再试 JSON）
    let is_base64_json = base64::decode(data)
        .is_some_and(|decoded| serde_json::from_slice::<Value>(&decoded).is_ok());
    if is_base64_json {
        return data.to_owned();
    }
    if let Ok(value) = serde_json::from_str::<Value>(data) {
        return base64::encode(value.to_string().as_bytes());
    }
    // 既不是 base64 也不是 JSON：保留原文，模型层不静默丢载荷（Go 这里记日志后跳过）
    data.to_owned()
}

/// `[CQ:keyboard,data=…]` → `foundItems["keyboard"]` 契约（JSON 串）。
///
/// Go `handlers/message_parser.go:1103-1134`：`data` 为对象时 marshal；字符串带
/// `base64://` 前缀时解码，否则按原始 JSON 原样保留。
fn keyboard_payload(segment: &Segment) -> String {
    let Some(data) = segment.get("data") else {
        return String::new();
    };
    let Some(encoded) = data.strip_prefix("base64://") else {
        return data.to_owned();
    };
    match base64::decode(encoded).and_then(|bytes| String::from_utf8(bytes).ok()) {
        Some(text) => text,
        // 解码失败保留原文（Go 在这里记日志后丢弃）
        None => data.to_owned(),
    }
}

/// `[CQ:card,…]` → `foundItems["card"]` 契约（JSON 对象串）。
///
/// Go 两条路径同构：`handlers/cqcode_pipeline.go:114-132` 把 `[CQ:card,…]` 的全部 `k=v`
/// 聚合成 `map[string]string`；`handlers/message_parser.go:988-1010` 从段 `data` 取
/// title/desc/pic/url。`data=` 形态（已是 JSON 串）原样保留。
fn card_payload(segment: &Segment) -> String {
    if let Some(data) = segment.get("data") {
        return data.to_owned();
    }
    if segment.params.is_empty() {
        return String::new();
    }
    let mut object = serde_json::Map::new();
    for (key, value) in &segment.params {
        object.insert(key.clone(), Value::String(value.clone()));
    }
    Value::Object(object).to_string()
}

fn media_key(media: &MediaRef) -> &'static str {
    match media {
        MediaRef::Local(_) => "local",
        MediaRef::Url(_) => "url",
        MediaRef::Base64(_) => "base64",
        MediaRef::Unknown(_) => "unknown",
    }
}

fn collect_media(items: &mut FoundItems, prefix: &str, media: &[MediaRef]) {
    for reference in media {
        let key = match reference {
            MediaRef::Local(_) => format!("local_{prefix}"),
            MediaRef::Url(_) => format!("url_{prefix}"),
            MediaRef::Base64(_) => format!("base64_{prefix}"),
            MediaRef::Unknown(_) => format!("unknown_{prefix}"),
        };
        items
            .entry(key)
            .or_default()
            .push(reference.to_found_item_value());
    }
}

impl MediaRef {
    /// 还原成 `foundItems` 的值（本地路径不加 `file://`，与 Go 写入形态一致）。
    fn to_found_item_value(&self) -> String {
        match self {
            Self::Local(path) => path.to_string_lossy().into_owned(),
            Self::Url(url) => url.clone(),
            Self::Base64(data) => data.clone(),
            Self::Unknown(raw) => raw.clone(),
        }
    }
}

fn stream_from_segment(segment: &Segment) -> Option<StreamPayload> {
    Some(StreamPayload {
        kind: StreamKind::from(segment.get("type")?),
        qq: segment.id_param("qq")?,
        id: segment
            .get("id")
            .or_else(|| segment.get("stream_msg_id"))
            .map(ToOwned::to_owned),
        index: segment.get("index").and_then(|raw| raw.parse::<i64>().ok()),
        seq: segment
            .get("seq")
            .or_else(|| segment.get("msg_seq"))
            .and_then(|raw| raw.parse::<i64>().ok()),
    })
}

fn input_notify_from_segment(segment: &Segment) -> Option<InputNotifyPayload> {
    Some(InputNotifyPayload {
        kind: segment.get("type")?.parse().ok()?,
        second: segment.get("second").and_then(|raw| raw.parse().ok()),
    })
}

/// JSON 里的字符串/数字统一转文本（Go 的 `map[string]string` 两种都出现过）。
fn json_scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn parse_stream(raw: &str) -> Result<StreamPayload> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|error| Error::Protocol(format!("stream 载荷不是合法 JSON: {error}")))?;
    let kind = value
        .get("type")
        .and_then(json_scalar)
        .ok_or_else(|| Error::Protocol("stream 载荷缺少 type".to_owned()))?;
    let qq = value
        .get("qq")
        .and_then(json_scalar)
        .ok_or_else(|| Error::Protocol("stream 载荷缺少 qq".to_owned()))?;
    Ok(StreamPayload {
        kind: StreamKind::from(kind.as_str()),
        qq: parse_id(&qq),
        // Go 侧的键名是 stream_msg_id / msg_seq，CQ 参数用 id / seq，两种都认
        id: optional_text(&value, &["id", "stream_msg_id"]),
        index: optional_i64(&value, &["index"]),
        seq: optional_i64(&value, &["seq", "msg_seq"]),
    })
}

/// 可选字符串字段（字符串/数字两种写法，Go 的 `map[string]string` 两种都出现过）。
fn optional_text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(json_scalar))
}

/// 可选整数字段：缺省或不可解析都按"未提供"处理（Go 只读 type/qq，其余是扩展键）。
fn optional_i64(value: &Value, keys: &[&str]) -> Option<i64> {
    optional_text(value, keys).and_then(|text| text.parse().ok())
}

fn parse_input_notify(raw: &str) -> Result<InputNotifyPayload> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|error| Error::Protocol(format!("input_notify 载荷不是合法 JSON: {error}")))?;
    let kind = value
        .get("type")
        .and_then(json_scalar)
        .and_then(|text| text.parse::<i64>().ok())
        .ok_or_else(|| Error::Protocol("input_notify 载荷缺少合法 type".to_owned()))?;
    let second = value
        .get("second")
        .and_then(json_scalar)
        .and_then(|text| text.parse::<i64>().ok());
    Ok(InputNotifyPayload { kind, second })
}

#[cfg(test)]
mod tests {
    use gensokyo_core::Ob11Id;

    use super::{
        CardFields, CardPayload, FileRef, FoundItems, InputNotifyPayload, MarkdownPayload,
        MediaRef, OutboundPlan, StreamKind, StreamPayload,
    };
    use crate::segment::Segment;

    fn found(pairs: &[(&str, &[&str])]) -> FoundItems {
        pairs
            .iter()
            .map(|(key, values)| {
                (
                    (*key).to_owned(),
                    values.iter().map(|value| (*value).to_owned()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn maps_every_found_items_key() {
        let items = found(&[
            ("reply_msg_id", &["777"]),
            ("active", &["true"]),
            ("active_type", &["push"]),
            ("active_sub_type", &["2"]),
            ("wakeup", &["12345"]),
            ("local_image", &["C:/tmp/a.png"]),
            ("url_image", &["https://x.com/a.png"]),
            ("url_images", &["x.com/b.png"]),
            ("base64_image", &["QUJD"]),
            ("unknown_image", &["weird://x"]),
            ("unknown_record", &["weird://y"]),
            ("local_record", &["C:/tmp/a.wav"]),
            ("url_record", &["https://x.com/a.wav"]),
            ("local_video", &["C:/tmp/a.mp4"]),
            ("base64_video", &["QUJD"]),
            ("local_file", &["C:/tmp/a.zip"]),
            ("url_file", &["https://x.com/b.zip"]),
            ("file_name", &["a.zip"]),
            ("qqmusic", &["123456"]),
            ("markdown", &["eyJjb250ZW50IjoiaGkifQ=="]),
            ("keyboard", &[r#"{"id":"k1"}"#]),
            ("card", &[r#"{"type":"card"}"#]),
            ("input_notify", &[r#"{"type":"1","second":"5"}"#]),
            ("stream", &[r#"{"type":"start","qq":"99999"}"#]),
        ]);

        let plan = OutboundPlan::from_found_items(&items).expect("映射失败");
        assert_eq!(plan.reply_msg_id, Some(Ob11Id::Num(777)));
        let active = plan.active.expect("应有 active");
        assert!(active.active, "foundItems 里有 active key 即应置位");
        assert_eq!(active.kind.as_deref(), Some("push"));
        assert_eq!(active.sub_type.as_deref(), Some("2"));
        assert_eq!(active.wakeup.as_deref(), Some("12345"));
        assert_eq!(
            plan.images,
            vec![
                MediaRef::Local("C:/tmp/a.png".into()),
                MediaRef::Url("https://x.com/a.png".to_owned()),
                MediaRef::Url("x.com/b.png".to_owned()),
                MediaRef::Base64("QUJD".to_owned()),
            ],
            "图片桶应保持 key 内的到达顺序"
        );
        assert_eq!(plan.records.len(), 2);
        assert_eq!(plan.videos.len(), 2);
        assert_eq!(
            plan.files,
            vec![
                FileRef {
                    media: MediaRef::Local("C:/tmp/a.zip".into()),
                    file_name: Some("a.zip".to_owned()),
                },
                FileRef {
                    media: MediaRef::Url("https://x.com/b.zip".to_owned()),
                    file_name: None,
                },
            ],
            "file_name 按到达顺序绑定，多出的文件没有名字"
        );
        assert_eq!(plan.qqmusic, vec!["123456".to_owned()]);
        assert_eq!(plan.markdown[0].raw, "eyJjb250ZW50IjoiaGkifQ==");
        assert_eq!(plan.keyboard[0].raw, r#"{"id":"k1"}"#);
        assert_eq!(
            plan.card[0],
            CardPayload {
                raw: r#"{"type":"card"}"#.to_owned()
            }
        );
        assert_eq!(
            plan.input_notify,
            Some(InputNotifyPayload {
                kind: 1,
                second: Some(5)
            })
        );
        assert_eq!(
            plan.stream,
            Some(StreamPayload {
                kind: StreamKind::Start,
                qq: Ob11Id::Num(99999),
                id: None,
                index: None,
                seq: None,
            })
        );
        assert_eq!(
            plan.unknown_images,
            vec![MediaRef::Unknown("weird://x".to_owned())],
            "unknown_image 进图片兜底桶"
        );
        assert_eq!(
            plan.unknown_records,
            vec![MediaRef::Unknown("weird://y".to_owned())],
            "unknown_record 进语音兜底桶"
        );
    }

    #[test]
    fn invalid_stream_payload_is_reported() {
        let items = found(&[("stream", &[r#"{"type":"start"}"#])]);
        let error = OutboundPlan::from_found_items(&items).expect_err("缺 qq 应报错");
        assert!(error.to_string().contains("qq"), "错误应点名字段: {error}");

        let items = found(&[("stream", &[r#"{"qq":"1"}"#])]);
        let error = OutboundPlan::from_found_items(&items).expect_err("缺 type 应报错");
        assert!(
            error.to_string().contains("type"),
            "错误应点名字段: {error}"
        );

        let items = found(&[("stream", &["not-json"])]);
        let error = OutboundPlan::from_found_items(&items).expect_err("非 JSON 应报错");
        assert!(
            error.to_string().contains("JSON"),
            "错误应点名字段: {error}"
        );

        let items = found(&[("input_notify", &[r#"{"second":"5"}"#])]);
        assert!(OutboundPlan::from_found_items(&items).is_err());

        let items = found(&[("input_notify", &[r#"{"type":"abc"}"#])]);
        assert!(OutboundPlan::from_found_items(&items).is_err());
    }

    #[test]
    fn from_segments_keeps_reply_and_media_order() {
        let segments = vec![
            Segment::reply(Ob11Id::Num(1)),
            Segment::text("看图"),
            Segment::image(&MediaRef::Url("https://x.com/a.png".to_owned())),
            Segment::image(&MediaRef::Url("https://x.com/b.png".to_owned())),
            Segment::record(&MediaRef::Base64("QUJD".to_owned())),
            Segment::new("face").param("id", "14"),
        ];
        let plan = OutboundPlan::from_segments(&segments);
        assert_eq!(plan.reply_msg_id, Some(Ob11Id::Num(1)));
        assert_eq!(plan.text, "看图");
        assert_eq!(
            plan.images,
            vec![
                MediaRef::Url("https://x.com/a.png".to_owned()),
                MediaRef::Url("https://x.com/b.png".to_owned()),
            ]
        );
        assert_eq!(plan.records, vec![MediaRef::Base64("QUJD".to_owned())]);
        assert_eq!(plan.extra.len(), 1, "face 无对应 key，应进 extra");
    }

    #[test]
    fn from_segments_keeps_at_inside_text() {
        let segments = vec![Segment::at(Ob11Id::Num(10000)), Segment::text(" hi")];
        let plan = OutboundPlan::from_segments(&segments);
        assert_eq!(plan.text, "[CQ:at,qq=10000] hi");
        assert_eq!(plan.to_segments(), segments);
    }

    #[test]
    fn to_segments_uses_documented_order() {
        let plan = OutboundPlan {
            text: "hi".to_owned(),
            reply_msg_id: Some(Ob11Id::Num(7)),
            images: vec![MediaRef::Url("https://x.com/a.png".to_owned())],
            files: vec![FileRef {
                media: MediaRef::Local("C:/tmp/a.zip".into()),
                file_name: Some("a.zip".to_owned()),
            }],
            stream: Some(StreamPayload {
                kind: StreamKind::Start,
                qq: Ob11Id::Num(9),
                id: None,
                index: None,
                seq: None,
            }),
            ..OutboundPlan::default()
        };
        let segments = plan.to_segments();
        let kinds: Vec<&str> = segments.iter().map(|s| s.kind.as_str()).collect();
        assert_eq!(kinds, vec!["reply", "text", "image", "file", "stream"]);
    }

    #[test]
    fn to_found_items_round_trips_back_to_plan() {
        let plan = OutboundPlan {
            reply_msg_id: Some(Ob11Id::str("BAC3N5VPRKPRQ1GJ")),
            text: "hi".to_owned(),
            images: vec![
                MediaRef::Local("C:/tmp/a.png".into()),
                MediaRef::Url("https://x.com/b.png".to_owned()),
                MediaRef::Base64("QUJD".to_owned()),
            ],
            records: vec![MediaRef::Url("https://x.com/a.wav".to_owned())],
            files: vec![FileRef {
                media: MediaRef::Url("https://x.com/a.zip".to_owned()),
                file_name: Some("a.zip".to_owned()),
            }],
            qqmusic: vec!["123".to_owned()],
            stream: Some(StreamPayload {
                kind: StreamKind::Finish,
                qq: Ob11Id::Num(5),
                id: Some("stream-1".to_owned()),
                index: Some(3),
                seq: Some(2),
            }),
            input_notify: Some(InputNotifyPayload {
                kind: 1,
                second: None,
            }),
            ..OutboundPlan::default()
        };
        let items = plan.to_found_items();
        let back = OutboundPlan::from_found_items(&items).expect("还原失败");
        // text 不在 foundItems 里（Go 单独返回 messageText），还原时天然为空
        let mut expected = plan.clone();
        expected.text.clear();
        assert_eq!(back, expected);
    }

    #[test]
    fn bare_active_marker_and_wakeup_survive_round_trip() {
        let segments = vec![Segment::active(None, None)];
        let plan = OutboundPlan::from_segments(&segments);
        assert!(plan.active.as_ref().expect("应有 active").active);
        assert_eq!(plan.to_segments(), segments, "裸 [CQ:active] 应原样往返");
        assert_eq!(
            plan.to_found_items().get("active"),
            Some(&vec!["true".to_owned()])
        );

        let wakeup = vec![Segment::wakeup("12345")];
        let plan = OutboundPlan::from_segments(&wakeup);
        assert_eq!(
            plan.active.as_ref().expect("应有 active").wakeup.as_deref(),
            Some("12345")
        );
        assert_eq!(
            plan.to_segments(),
            wakeup,
            "只有 wakeup 时不应额外产出 [CQ:active]"
        );
    }

    #[test]
    fn active_marker_survives_cq_string_round_trip() {
        let plan = OutboundPlan::from_segments(&crate::cqcode::parse("[CQ:active,type:push]"));
        let active = plan.active.as_ref().expect("应有 active");
        assert!(active.active, "[CQ:active] 出现即置存在位");
        assert_eq!(active.kind.as_deref(), Some("push"));

        let items = plan.to_found_items();
        assert_eq!(items["active"], vec!["true".to_owned()]);
        assert_eq!(items["active_type"], vec!["push".to_owned()]);

        // foundItems → 段：存在位回写成裸 [CQ:active]
        let segments = plan.to_segments();
        assert_eq!(segments[0].kind, "active");
        assert_eq!(OutboundPlan::from_segments(&segments), plan);
    }

    #[test]
    fn markdown_keyboard_and_card_normalize_to_found_items_contract() {
        let markdown: Segment = serde_json::from_str(
            r#"{"type":"markdown","data":{"data":{"content":"hi","extra":["a"]}}}"#,
        )
        .expect("markdown 段反序列化失败");
        let segments = vec![
            markdown,
            Segment::new("keyboard").param("data", r#"{"id":"k1"}"#),
            Segment::new("keyboard").param("data", "base64://eyJpZCI6ImsxIn0="),
            Segment::new("card")
                .param("title", "标题")
                .param("desc", "描述")
                .param("pic", "https://x.com/p.png")
                .param("url", "https://x.com"),
        ];
        let plan = OutboundPlan::from_segments(&segments);

        let decoded = crate::base64::decode(&plan.markdown[0].raw).expect("markdown 应为 base64");
        assert_eq!(
            String::from_utf8(decoded).expect("utf8"),
            r#"{"content":"hi","extra":["a"]}"#,
            "markdown 的对象载荷应 marshal 后 base64"
        );
        assert_eq!(plan.keyboard[0].raw, r#"{"id":"k1"}"#);
        assert_eq!(
            plan.keyboard[1].raw, r#"{"id":"k1"}"#,
            "base64:// 形态应解码成 JSON 串"
        );
        assert_eq!(
            plan.card[0].raw,
            r#"{"desc":"描述","pic":"https://x.com/p.png","title":"标题","url":"https://x.com"}"#,
            "card 的 k=v 应聚合成 JSON 对象"
        );

        // 契约往返：foundItems → plan 与 plan → 段 → plan 都不丢信息
        let items = plan.to_found_items();
        assert_eq!(items["markdown"], vec![plan.markdown[0].raw.clone()]);
        assert_eq!(items["card"], vec![plan.card[0].raw.clone()]);
        assert_eq!(
            OutboundPlan::from_found_items(&items).expect("foundItems 还原失败"),
            plan
        );
        assert_eq!(OutboundPlan::from_segments(&plan.to_segments()), plan);
    }

    #[test]
    fn unknown_media_round_trips_through_its_own_key() {
        let plan = OutboundPlan {
            unknown_images: vec![MediaRef::Unknown("weird://x".to_owned())],
            unknown_records: vec![MediaRef::Unknown("weird://y".to_owned())],
            ..OutboundPlan::default()
        };
        let items = plan.to_found_items();
        assert_eq!(items["unknown_image"], vec!["weird://x".to_owned()]);
        assert_eq!(items["unknown_record"], vec!["weird://y".to_owned()]);
        assert_eq!(
            OutboundPlan::from_found_items(&items).expect("还原失败"),
            plan
        );
    }

    #[test]
    fn empty_plan_detects_content() {
        let mut plan = OutboundPlan::default();
        assert!(plan.is_empty());
        plan.reply_msg_id = Some(Ob11Id::Num(1));
        assert!(plan.is_empty(), "reply 不是可发送内容");
        plan.text = "hi".to_owned();
        assert!(!plan.is_empty());
    }

    #[test]
    fn markdown_payload_exposes_go_fields_and_round_trips() {
        let segment: Segment = serde_json::from_str(
            r##"{"type":"markdown","data":{"data":{"content":"# hi","custom_template_id":"tpl-1","keyboard":{"content":{"rows":[]}}}}}"##,
        )
        .expect("markdown 段反序列化失败");
        let plan = OutboundPlan::from_segments(&[segment]);
        let payload = &plan.markdown[0];

        assert_eq!(payload.content().as_deref(), Some("# hi"));
        assert_eq!(payload.custom_template_id().as_deref(), Some("tpl-1"));
        assert!(payload.keyboard().is_some(), "markdown 内嵌键盘应可读出");
        assert_eq!(
            payload.decode().expect("应为 base64(JSON)")["content"],
            "# hi"
        );

        // foundItems 契约：原样 base64；双向映射都不丢字段
        let items = plan.to_found_items();
        assert_eq!(items["markdown"], vec![payload.raw.clone()]);
        assert_eq!(
            OutboundPlan::from_found_items(&items).expect("还原失败"),
            plan
        );
        assert_eq!(OutboundPlan::from_segments(&plan.to_segments()), plan);

        // JSON → base64 的规范化与 Go 的 `mdContentMap` 分支一致
        assert_eq!(
            MarkdownPayload::from_json(r#"{"content":"hi"}"#).expect("编码失败"),
            MarkdownPayload {
                raw: "eyJjb250ZW50IjoiaGkifQ==".to_owned()
            }
        );
        assert!(MarkdownPayload::from_json("not json").is_err());
        assert!(
            MarkdownPayload {
                raw: "not-base64".to_owned()
            }
            .decode()
            .is_err()
        );
    }

    #[test]
    fn card_payload_aggregates_kv_and_exposes_go_fields() {
        // 冒号与 `=` 混写（Go 的 card 正则只认 `k=v`，冒号是本 crate 的兜底写法）
        let segments = crate::cqcode::parse(
            "[CQ:card,title:标题,desc=描述,pic=https://x.com/p.png,url=https://x.com]",
        );
        let plan = OutboundPlan::from_segments(&segments);

        let fields = plan.card[0].fields().expect("card 应为 JSON 对象");
        assert_eq!(fields.title.as_deref(), Some("标题"));
        assert_eq!(fields.desc.as_deref(), Some("描述"));
        assert_eq!(fields.pic.as_deref(), Some("https://x.com/p.png"));
        assert_eq!(fields.url.as_deref(), Some("https://x.com"));

        let items = plan.to_found_items();
        let raw: serde_json::Value =
            serde_json::from_str(&items["card"][0]).expect("card 应为 JSON");
        assert_eq!(raw["title"], "标题");
        assert_eq!(
            OutboundPlan::from_found_items(&items).expect("还原失败"),
            plan
        );
        assert_eq!(OutboundPlan::from_segments(&plan.to_segments()), plan);

        // 字段表 → 载荷：只写有值的键（与 Go 的 map marshal 同序）
        let payload = CardPayload::from_fields(&CardFields {
            title: Some("t".to_owned()),
            ..CardFields::default()
        });
        assert_eq!(payload.raw, r#"{"title":"t"}"#);
        assert_eq!(
            payload.fields().expect("应为对象"),
            CardFields {
                title: Some("t".to_owned()),
                ..CardFields::default()
            }
        );
        assert!(
            CardPayload {
                raw: r#""x""#.to_owned()
            }
            .fields()
            .is_err(),
            "非对象载荷应报错"
        );
    }

    #[test]
    fn card_payload_accepts_official_field_aliases() {
        // 官方卡片字段名（description / pic_url）与 Go 的 CQ 简写（desc / pic）都要认
        let official = CardPayload {
            raw: r#"{"title":"t","description":"d","pic_url":"https://x.com/p.png","url":"https://x.com"}"#
                .to_owned(),
        };
        let fields = official.fields().expect("官方字段名应能解析");
        assert_eq!(fields.desc.as_deref(), Some("d"));
        assert_eq!(fields.pic.as_deref(), Some("https://x.com/p.png"));

        // 两种写法同现时 desc / pic 优先（Go firstNonEmptyCardValue 的顺序）
        let both = CardPayload {
            raw: r#"{"desc":"简写","description":"全名","pic":"https://a/p.png","pic_url":"https://b/p.png"}"#
                .to_owned(),
        };
        let fields = both.fields().expect("同现应能解析");
        assert_eq!(fields.desc.as_deref(), Some("简写"));
        assert_eq!(fields.pic.as_deref(), Some("https://a/p.png"));

        // 简写为空/空白时回退到官方名（Go 用 TrimSpace 判空）
        let blank = CardPayload {
            raw: r#"{"desc":"  ","description":"全名","pic":"","pic_url":"https://b/p.png"}"#.to_owned(),
        };
        let fields = blank.fields().expect("空白简写应回退");
        assert_eq!(fields.desc.as_deref(), Some("全名"));
        assert_eq!(fields.pic.as_deref(), Some("https://b/p.png"));
    }

    #[test]
    fn stream_payload_covers_all_stages_and_optional_fields() {
        for (kind, expected) in [
            ("start", StreamKind::Start),
            ("mid", StreamKind::Mid),
            ("finish", StreamKind::Finish),
        ] {
            let segment = Segment::stream(kind, Ob11Id::Num(99999))
                .param("id", "stream-1")
                .param("index", "0")
                .param("seq", "2");
            let plan = OutboundPlan::from_segments(&[segment]);

            let stream = plan.stream.as_ref().expect("应有 stream");
            assert_eq!(stream.kind, expected);
            assert_eq!(stream.qq, Ob11Id::Num(99999));
            assert_eq!(stream.id.as_deref(), Some("stream-1"));
            assert_eq!(stream.index, Some(0), "首片 index 为 0，不能被当成缺省");
            assert_eq!(stream.seq, Some(2));

            let items = plan.to_found_items();
            let raw: serde_json::Value =
                serde_json::from_str(&items["stream"][0]).expect("stream 应为 JSON");
            assert_eq!(raw["type"], kind);
            assert_eq!(raw["qq"], "99999");
            assert_eq!(raw["id"], "stream-1");
            assert_eq!(raw["index"], "0");
            assert_eq!(raw["seq"], "2");

            assert_eq!(
                OutboundPlan::from_found_items(&items).expect("还原失败"),
                plan
            );
            assert_eq!(OutboundPlan::from_segments(&plan.to_segments()), plan);
        }

        // CQ 文本：冒号与 `=` 混写（Go 的 streamPattern 原生是 `type:start,qq:123`）
        let plan = OutboundPlan::from_segments(&crate::cqcode::parse(
            "[CQ:stream,type:start,qq=123,id:abc,index=0,seq:2]",
        ));
        let stream = plan.stream.expect("CQ 混写应解析成 stream");
        assert_eq!(stream.kind, StreamKind::Start);
        assert_eq!(stream.qq, Ob11Id::Num(123));
        assert_eq!(stream.id.as_deref(), Some("abc"));
        assert_eq!(stream.index, Some(0));
        assert_eq!(stream.seq, Some(2));

        // Go 只写 type/qq 的形态：可选字段缺省，JSON 里不出现空键
        let plan = OutboundPlan::from_segments(&[Segment::stream("mid", Ob11Id::Num(1))]);
        assert!(plan.stream.as_ref().expect("应有 stream").id.is_none());
        assert_eq!(
            plan.to_found_items()["stream"][0],
            r#"{"qq":"1","type":"mid"}"#
        );
    }

    #[test]
    fn stream_payload_reads_go_stream_chunk_keys() {
        let items = found(&[(
            "stream",
            &[r#"{"type":"mid","qq":"7","stream_msg_id":"abc","index":1,"msg_seq":4}"#],
        )]);
        let plan = OutboundPlan::from_found_items(&items).expect("应接受 Go 的键名");
        let stream = plan.stream.as_ref().expect("应有 stream");
        assert_eq!(stream.kind, StreamKind::Mid);
        assert_eq!(stream.id.as_deref(), Some("abc"));
        assert_eq!(stream.index, Some(1));
        assert_eq!(stream.seq, Some(4));
        // 回写时统一成 CQ 参数口径（id / index / seq）
        assert_eq!(
            plan.to_found_items()["stream"][0],
            r#"{"id":"abc","index":"1","qq":"7","seq":"4","type":"mid"}"#
        );
    }

    #[test]
    fn missing_rich_payloads_stay_visible_in_extra() {
        let segments = vec![
            Segment::new("markdown"),
            Segment::new("keyboard"),
            Segment::new("card"),
        ];
        let plan = OutboundPlan::from_segments(&segments);
        assert!(plan.markdown.is_empty());
        assert!(plan.keyboard.is_empty());
        assert!(plan.card.is_empty());
        assert_eq!(plan.extra, segments, "缺载荷的段进 extra，不静默丢弃");
        assert_eq!(plan.to_found_items().get("markdown"), None);
    }
}
