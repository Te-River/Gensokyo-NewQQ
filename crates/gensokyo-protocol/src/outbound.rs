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
//!
//! 没有 `foundItems` key 的段与**形态不命中**的段按 Go legacy（默认 `cq_parse_mode`）分三类：
//!
//! 1. **字面保留**：写回 [`OutboundPlan::text`]，跟着正文一起发出去。
//!    - 无消费点的码：`face` / `node` / `poke` / `group_info` / `json` / 未知码——Go 的 `cqparse`
//!      没有 handler 时回填 `tok.Raw`（`handlers/cqparse/walk.go:61-72`，金样
//!      `cq_legacy_semantics_test.go:173-174`）；
//!    - 形态不命中的码：`music` 非 `type=qq,id=<数字>`（`handlers/cqcode.go:38`）、
//!      `markdown` / `keyboard` 的 `data` 不是 `base64://` 或 `{` 开头、`stream` / `input_notify`
//!      缺参数、`avatar`（见下方已知差异）。
//!
//!    入口统一走 [`push_literal`]（只回写 [`Segment::raw_or_cq`] 的原文切片）：写回的字面再次
//!    经 [`OutboundPlan::to_segments`] 重分词得到同一段，两个方向互为逆运算（用例见
//!    `literal_segments_round_trip_byte_for_byte`）。
//! 2. **静默丢弃**：`ark`（Go 在 `parseMessageContent` 之后由 `ProcessCQInboundOnlyCodes`
//!    整段剥掉，`handlers/cqcode.go:198-205`、`handlers/cqcode_pipeline.go:57-59`）与空 `card`
//!    （`handlers/cqcode_pipeline.go:124-130`：cardData 为空时不入 `foundItems`、正文也不留）。
//! 3. **进 [`OutboundPlan::extra`]**：`member` / `set_group` / `remove`——Go 是**真执行**管理动作
//!    （`handlers/message_parser.go:1166-1173`、`:1175-1181`、`:1611-1617` 还原 CQ →
//!    `handlers/cqcode.go:235-240` 踢人 / 禁言 / 撤回），本仓未实现（P83）；留在 `extra` 让
//!    下游以 `1400` 明确拒绝，静默吞掉会悄悄丢掉管理动作。
//!
//! `avatar` 在 Go 字符串路径会被替换成 `[CQ:image,file=…]` 字面、段路径进 `url_images`
//! （`handlers/avatar.go:44`、`handlers/message_parser.go:991-1016`），本仓协议层拿不到
//! idmap/app_id，暂按原文回写 ⇒ **字节形态与 Go 不同，属已知差异**（parity B4、P80）。
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
//! | input_notify | `type` 恒写、`second` 可缺，值均为字符串 | [`InputNotifyPayload::kind`] 为 `i64`（`type` 缺失 / 非数字的段按字面写回正文），`second` 为 `Option<i64>` |
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
    /// `desc` / `pic` 优先，为空时回退到 `description` / `pic_url`——这是 **Rust 侧扩展**：
    /// Go 的卡片只认 `title` / `desc` / `pic` / `url` 四键、不识别别名（`handlers/message_parser.go`
    /// 的卡片字段收集与 `handlers/send_group_msg.go` 的组装点均只读这四键；差异登记见 parity P77-4）。
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

/// 别名取值：按顺序取第一个 trim 后非空的值（Rust 侧别名扩展的取值规则；Go 无对应函数）。
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
    /// 本仓尚未实现、必须整条拒绝的段（`member` / `set_group` / `remove`），按到达顺序保留。
    ///
    /// Go 对这些码是**真执行**管理动作（见模块文档「无法归类的段」第 3 类），本仓未实现前
    /// 留在 `extra` 由下游回 `1400`，不静默丢弃。
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
    /// - 无消费点 / 形态不命中的段按字面写回 `text`，`ark` 与空 `card` 静默丢弃，
    ///   `member` / `set_group` / `remove` 进 [`Self::extra`]（三类分界见模块文档）。
    #[must_use]
    pub fn from_segments(segments: &[Segment]) -> Self {
        let mut plan = Self::default();
        let mut active = ActiveKind::default();

        for segment in segments {
            // P59 裸媒体码：按 media_wire_value 的臂口径取值为空时 Go 保留 `tok.Raw`
            // 字面、不进 foundItems（resolveMediaValue handlers_media.go:51-59、
            // resolveFileValue :87-91——file 臂只读 file，故 `[CQ:file,url=…]` 同样算裸码）；
            // 在归一点统一把原文放回 text，不产出 `Unknown("")` 空 file_info。
            if is_bare_media_segment(segment) {
                push_literal(&mut plan, segment);
                continue;
            }
            match segment.kind.as_str() {
                "text" => plan
                    .text
                    .push_str(segment.text_content().unwrap_or_default()),
                "at" => {
                    // at 段是透传臂：回写 parse 的原文切片（自产段回退规范化序列化），
                    // 用户的空白形态不被 to_cq() 改写，to_segments 重分词时逐字节还原
                    push_literal(&mut plan, segment);
                }
                "reply" => {
                    if plan.reply_msg_id.is_none() {
                        plan.reply_msg_id = segment.id_param("id");
                    }
                }
                "image" => plan.images.push(media_from_segment(segment)),
                // voice 与 record 同义：Go legacy `case "voice","record"` 同分支
                // （message_parser.go:909/1318），cqparse 键归一 *_record
                // （cqparse/handlers_media.go:48-50）
                "record" | "voice" => plan.records.push(media_from_segment(segment)),
                "video" => plan.videos.push(media_from_segment(segment)),
                "file" => plan.files.push(FileRef {
                    // 取值走 media_wire_value 单一口径：file 臂只读 file（Go
                    // resolveFileValue handlers_media.go:87-91），url 从不参与取值；
                    // 取值为空的形态（含 [CQ:file,url=…]）已被裸码判定拦在上面
                    media: media_from_segment(segment),
                    file_name: segment
                        .get("file_name")
                        .or_else(|| segment.get("name"))
                        .map(ToOwned::to_owned),
                }),
                "markdown" => {
                    let raw = markdown_payload(segment);
                    if raw.is_empty() {
                        // 未命中 Go legacy 的 `data=base64://(.+?)` 正则（handlers/cqcode.go:36-45）
                        // ⇒ 字符串路径把它当普通文本照发，这里按字面写回正文
                        push_literal(&mut plan, segment);
                    } else {
                        plan.markdown.push(MarkdownPayload { raw });
                    }
                }
                "keyboard" => {
                    let raw = keyboard_payload(segment);
                    if raw.is_empty() {
                        // 只认 `base64://` / `{` 两种形态，其余同 markdown 按字面保留
                        push_literal(&mut plan, segment);
                    } else {
                        plan.keyboard.push(KeyboardPayload { raw });
                    }
                }
                "card" => {
                    let raw = card_payload(segment);
                    // Go `cqcode_pipeline.go:124-130`：cardData 为空 ⇒ 不入 foundItems、正文里
                    // 也没留下东西 ⇒ 整段消失（不推 extra、不留字面）
                    if !raw.is_empty() {
                        plan.card.push(CardPayload { raw });
                    }
                }
                // avatar：本仓协议层无 idmap/app_id，无法复刻 Go 的可见形态——Go 字符串路径把
                // `[CQ:avatar,qq=<数字>]` 替换成 `[CQ:image,file=…]` 字面（handlers/avatar.go:14/44），
                // 段路径反查 openid 后进 url_images（handlers/message_parser.go:991-1016）。这里按
                // 原文写回正文（不整条拒发），字节与 Go 不同，属已知差异（P80）。
                "avatar" => push_literal(&mut plan, segment),
                // Go legacy 字符串解析只匹配 `[CQ:music,type=qq,id=<数字>]`
                // （handlers/cqcode.go:38，消费点 cqcode_pipeline.go:170-178）：裸码、非 qq
                // type、非数字 id 都不命中 ⇒ 原样留在 messageText，不因此拒发整条消息。
                "music" => match (segment.get("type"), segment.get("id")) {
                    (Some("qq"), Some(id))
                        if !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()) =>
                    {
                        plan.qqmusic.push(id.to_owned());
                    }
                    _ => push_literal(&mut plan, segment),
                },
                "stream" => match stream_from_segment(segment) {
                    Some(stream) => plan.stream = Some(stream),
                    // 缺 `type` / `qq`、qq 非数字：Go 的冒号正则不命中 ⇒ 字面
                    None => push_literal(&mut plan, segment),
                },
                "input_notify" => match input_notify_from_segment(segment) {
                    Some(notify) => plan.input_notify = Some(notify),
                    // 缺 `type` 或非数字：Go 的 `type=(\d+)` 不命中 ⇒ 字面
                    None => push_literal(&mut plan, segment),
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
                // ark：Go 在 parseMessageContent 之后由 ProcessCQInboundOnlyCodes 整段剥掉
                // （handlers/cqcode.go:198-205、handlers/cqcode_pipeline.go:57-59）——既不发送
                // 也不留字面，这里同样静默丢弃
                "ark" => {}
                // member / set_group / remove：Go **真执行**管理动作（踢人 / 禁言 / 撤回），
                // 链路是 message_parser.go:1166-1173、:1175-1181、:1611-1617 还原 CQ 文本 →
                // cqcode.go:235-240 分派。本仓未实现（P83）：静默吞掉会悄悄丢管理动作，
                // 所以留在 extra 让下游以 1400 明确拒绝
                "member" | "set_group" | "remove" => plan.extra.push(segment.clone()),
                // face / node / poke / group_info / json / 未知码：Go 的 cqparse 没有消费点时
                // 回填 `tok.Raw`（cqparse/walk.go:61-72；金样 cq_legacy_semantics_test.go:173-174）
                // ⇒ 逐字节留在正文
                _ => push_literal(&mut plan, segment),
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
            // text 里可能有 [CQ:at] 字面量，按 CQ 重新分词；parse 出的段携带原文切片
            // （from_segments 的 at 臂已按 raw 原样写入），下游透传消费走 raw_or_cq 不改字节
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

/// 段的「字面保留」出口：把原文写回 [`OutboundPlan::text`]。
///
/// Go legacy 对没有消费点 / 形态不命中的码保留 `tok.Raw`（`handlers/cqparse/walk.go:61-72`，
/// 金样 `cq_legacy_semantics_test.go:173-174`），段既不出站成富媒体、也不整条拒发。
///
/// 只回写 [`Segment::raw_or_cq`]：parse 而来的段逐字节还原原文，[`OutboundPlan::to_segments`]
/// 重分词时得到同一段，两个方向互为逆运算（不自造序列化，自产段才退回 `to_cq`）。
fn push_literal(plan: &mut OutboundPlan, segment: &Segment) {
    plan.text.push_str(&segment.raw_or_cq());
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

/// 媒体段族的**单一取值点**：按 Go 的臂口径从段参数解析出最终参与发送的 wire 值。
///
/// - Go `resolveMediaValue`（`handlers/cqparse/handlers_media.go:51-55`）：`image` /
///   `record` / `voice` / `video` 的 `file` 为空则兜 `url`（另见 legacy 同口径
///   `message_parser.go:870-874/914-918/1218-1222`）；
/// - Go `resolveFileValue`（`:87-91`）：`file` 只读 `file`，`url` **从不参与取值**。
///
/// 取值结果**为空**即 Go 返回 `Replacement: tok.Raw` 的形态（`:56-59`、`:88-91`）：
/// 保留字面、不产 foundItems。规则只在本函数里出现一次，[`media_from_segment`] 与
/// [`is_bare_media_segment`] 共用，各臂不再重复特判。
///
/// 返回 `None` 表示该段不属于媒体族（调用方不应据此判定裸码）。
fn media_wire_value(segment: &Segment) -> Option<&str> {
    match segment.kind.as_str() {
        // Go resolveMediaValue：file 空则兜 url（:51-55），皆空才保字面（:56-59）
        "image" | "record" | "voice" | "video" => {
            let file = segment.get("file").unwrap_or_default();
            if file.is_empty() {
                Some(segment.get("url").unwrap_or_default())
            } else {
                Some(file)
            }
        }
        // Go resolveFileValue：只读 file（:87-91），url 从不参与取值
        "file" => Some(segment.get("file").unwrap_or_default()),
        _ => None,
    }
}

fn media_from_segment(segment: &Segment) -> MediaRef {
    // 取值口径（含按臂差异）见 media_wire_value，本函数只做 wire → MediaRef 的形态映射
    MediaRef::from_wire(media_wire_value(segment).unwrap_or_default())
}

/// 裸媒体码：媒体段族（`image` / `record` / `voice` / `video` / `file`）按
/// [`media_wire_value`] 的**臂口径**解析出的取值为空——对 `image` / `record` /
/// `voice` / `video` 是 `file` 与 `url` 皆空，对 `file` 是 `file` 空（`url` 有无都算）。
///
/// Go 的 `resolveMediaValue` / `resolveFileValue` 对该形态返回 `Replacement: tok.Raw`
/// （`handlers/cqparse/handlers_media.go:56-59`、`:88-91`）——保留字面、不产任何
/// foundItems 键；若在 Rust 侧放行会产出 `MediaRef::Unknown("")`，向上游发出空
/// `file_info`。`from_segments` 的媒体归一点按本判定把原始字面放回 `plan.text`。
///
/// 取值非空的形态各臂行为不变（`file` 臂不兜 `url` 的钉死口径见
/// `file_arm_does_not_fall_back_to_url`）。
fn is_bare_media_segment(segment: &Segment) -> bool {
    media_wire_value(segment).is_some_and(|wire| wire.is_empty())
}

/// `[CQ:markdown,data=…]` → `foundItems["markdown"]` 契约（base64(JSON)）。
///
/// 只认两种形态，其余返回空串（调用方按字面保留整段）：
/// - `base64://…`：取编码体，命中 Go legacy 的 `data=base64://(.+?)` 正则
///   （`handlers/cqcode.go:36-45`）；
/// - `{…}`：JSON 对象，先 marshal 再编码（段路径的旧格式写法，`handlers/message_parser.go:1048-1097`）。
///
/// 空 `data=`、裸 base64、非 JSON 文本都不在 Go legacy 的消费范围内（P81）。
fn markdown_payload(segment: &Segment) -> String {
    let Some(data) = segment.get("data") else {
        return String::new();
    };
    if let Some(encoded) = data.strip_prefix("base64://") {
        return encoded.to_owned();
    }
    if data.starts_with('{')
        && let Ok(value) = serde_json::from_str::<Value>(data)
    {
        return base64::encode(value.to_string().as_bytes());
    }
    String::new()
}

/// `[CQ:keyboard,data=…]` → `foundItems["keyboard"]` 契约（JSON 串）。
///
/// 只认两种形态，其余返回空串（调用方按字面保留整段）：`base64://…`（解码成 JSON 串）与
/// `{…}`（原样 JSON）。Go legacy 的键盘正则同样只吃 `base64://`
/// （`handlers/cqcode.go:36-45` 同族），裸 JSON 在字符串路径里不是合法形态（P81）。
///
/// `base64://` 解码失败也按「不可消费」处理 ⇒ 字面（Go 在这里记日志后丢弃，本仓宁可让
/// 用户看见原文）。
fn keyboard_payload(segment: &Segment) -> String {
    let Some(data) = segment.get("data") else {
        return String::new();
    };
    let Some(encoded) = data.strip_prefix("base64://") else {
        return if data.starts_with('{') {
            data.to_owned()
        } else {
            String::new()
        };
    };
    base64::decode(encoded)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default()
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
    // Go 的冒号正则只认数字 qq（`\[CQ:stream,type:(\w+),qq:(\d+)\]`，handlers/cqcode.go:41）：
    // 缺 qq / 非数字都不命中 ⇒ 调用方按字面保留整段
    let qq = segment.get("qq")?;
    if qq.is_empty() || !qq.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(StreamPayload {
        kind: StreamKind::from(segment.get("type")?),
        qq: parse_id(qq),
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
    use crate::cqcode;
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
        assert_eq!(plan.text, "看图[CQ:face,id=14]", "face 按字面写回正文");
        assert_eq!(
            plan.images,
            vec![
                MediaRef::Url("https://x.com/a.png".to_owned()),
                MediaRef::Url("https://x.com/b.png".to_owned()),
            ]
        );
        assert_eq!(plan.records, vec![MediaRef::Base64("QUJD".to_owned())]);
        assert!(plan.extra.is_empty(), "face 不再进 extra（P81 字面保留）");
    }

    #[test]
    fn from_segments_keeps_at_inside_text() {
        let segments = vec![Segment::at(Ob11Id::Num(10000)), Segment::text(" hi")];
        let plan = OutboundPlan::from_segments(&segments);
        assert_eq!(plan.text, "[CQ:at,qq=10000] hi");
        assert_eq!(plan.to_segments(), segments);
    }

    /// Major 回归用例：at 臂是透传臂，回写 parse 的原文切片——用户的空白形态不被
    /// 规范化改写，`to_segments` 重分词后逐字节还原（自产段仍走规范化，行为不变）。
    #[test]
    fn from_segments_at_arm_keeps_whitespace_bytes() {
        let segments = cqcode::parse("hi[CQ:at , qq = 123]");
        let plan = OutboundPlan::from_segments(&segments);
        assert_eq!(plan.text, "hi[CQ:at , qq = 123]");
        let back = plan.to_segments();
        assert_eq!(back, segments, "往返后 text/at 段语义一致");
        assert_eq!(
            back[1].raw_or_cq(),
            "[CQ:at , qq = 123]",
            "重分词得到的 at 段应仍携带原文切片"
        );
    }

    /// P57-②：`voice` 与 `record` 同义（Go legacy `case "voice","record"` 同分支
    /// `message_parser.go:909/1318`，cqparse 键归一 `cqparse/handlers_media.go:48-50`）。
    /// 运行级钉死：各参数形态下 voice 码与同参 record 码逐字段等价，不落 extra、
    /// 不产出 `*_voice` 键（对拍 Go `cq_parse_regression_test.go:688-717`、
    /// `cq_threepath_matrix_test.go:181-203`）。
    ///
    /// 第二列是**形态标记**：`true` = P59 裸码（取值空、按 Go `tok.Raw` 保留字面），
    /// 口径见 `media_wire_value`。裸与否随数据一起维护，新增 `[CQ:voice,file=,url=]`
    /// 这类裸形态只需加一行，不会误落非裸分支。
    #[test]
    fn from_segments_voice_alias_matches_record_every_shape() {
        let shapes: [(&str, bool); 11] = [
            ("[CQ:voice,file=https://x.com/a.mp3]", false),
            ("[CQ:voice,file=http://x.com/b.mp3]", false),
            ("[CQ:voice,file=base64://dm9pY2U=]", false),
            ("[CQ:voice,file=file:///C:/tmp/a.wav]", false),
            ("[CQ:voice,file=weird://y]", false),
            // Go resolveMediaValue：file 空时兼容 url（handlers_media.go:52-55）
            ("[CQ:voice,url=https://x.com/c.mp3]", false),
            ("[CQ:voice,file=,url=https://x.com/d.mp3]", false),
            // 附加参数（asr）与 record 同口径：出站不读取，不影响归一
            ("[CQ:voice,file=abc.silk,asr=1]", false),
            ("[CQ:VOICE,file=https://x.com/e.mp3]", false),
            // 裸码与裸 record 行为一致（两路都不应有 voice 特判）
            ("[CQ:voice]", true),
            // 两个参数都在但都为空：取值仍是空 → 同属裸码
            ("[CQ:voice,file=,url=]", true),
        ];
        for (voice_code, bare) in shapes {
            let record_code = voice_code
                .replace("voice", "record")
                .replace("VOICE", "RECORD");
            let voice_plan = OutboundPlan::from_segments(&cqcode::parse(voice_code));
            let record_plan = OutboundPlan::from_segments(&cqcode::parse(&record_code));
            // 裸码无取值：P59 后按 Go `Replacement: tok.Raw` 保留**各自**字面，
            // `[CQ:voice]` 与 `[CQ:record]` 文本天然不同，等价断言只适用有值形态
            if !bare {
                assert_eq!(
                    voice_plan, record_plan,
                    "voice 应与同参 record 等价: {voice_code}"
                );
            }
            assert!(
                voice_plan.extra.is_empty(),
                "voice 不应落 extra: {voice_code}"
            );
            let items = voice_plan.to_found_items();
            for key in items.keys() {
                assert!(!key.contains("voice"), "不应产出 *_voice 键: {key}");
            }
            assert_eq!(
                items,
                record_plan.to_found_items(),
                "foundItems 应与同参 record 一致: {voice_code}"
            );
            // 裸码两路都不产 foundItems 键，且只保留字面、不进任何媒体桶
            if bare {
                assert!(
                    items.is_empty() && record_plan.to_found_items().is_empty(),
                    "裸码不该产出 foundItems 键: {voice_code}"
                );
                assert_eq!(voice_plan.text, voice_code, "裸 voice 保留自身字面");
                assert_eq!(
                    record_plan.text, record_code,
                    "裸 record 保留自身字面（与 Go tok.Raw 同）"
                );
                assert!(voice_plan.records.is_empty(), "裸码不进语音桶");
            } else {
                assert_eq!(voice_plan.records.len(), 1, "应落语音通道: {voice_code}");
            }
        }
    }

    /// 具名形态的通道断言：base64 / url / local 三形态分别落语音桶与
    /// `base64_record` / `url_record` / `local_record` 键（Go legacy
    /// `message_parser.go:921-946`；`file=` 与 `url=` 两参数来源同键）。
    #[test]
    fn from_segments_voice_lands_on_record_channel_per_shape() {
        let plan = OutboundPlan::from_segments(&cqcode::parse("[CQ:voice,file=base64://QUJD]"));
        assert_eq!(plan.records, vec![MediaRef::Base64("QUJD".to_owned())]);
        assert!(plan.unknown_records.is_empty(), "base64 形态不该落兜底桶");
        assert!(plan.unknown_images.is_empty(), "不该误落图片通道");
        let items = plan.to_found_items();
        assert_eq!(
            items["base64_record"],
            ["QUJD".to_owned()],
            "base64_record 键与值"
        );

        let url_plan =
            OutboundPlan::from_segments(&cqcode::parse("[CQ:voice,url=http://x/a.silk]"));
        assert_eq!(
            url_plan.records,
            vec![MediaRef::Url("http://x/a.silk".to_owned())],
            "url= 参数与 file= 同落到语音通道"
        );

        let local_plan =
            OutboundPlan::from_segments(&cqcode::parse("[CQ:voice,file=file:///C:/tmp/a.wav]"));
        assert_eq!(
            local_plan.records,
            vec![MediaRef::Local("C:/tmp/a.wav".into())]
        );
        assert_eq!(
            local_plan.to_found_items()["local_record"],
            ["C:/tmp/a.wav".to_owned()],
            "local_record 键与值"
        );
    }

    /// P57-② 评审钉死（1），P59b 修正口径：`file` 臂**没有** url 兜底，逐字对齐 Go
    /// `resolveFileValue`（`handlers/cqparse/handlers_media.go:87-91` 只读 `file`，
    /// `file` 空即 `Replacement: tok.Raw`）。所以带 `url` 而 `file` 空的形态既不兜 url
    /// （不产 `url_file`），也不产 `Unknown("")` 的空 `file_info`——原位保留字面，
    /// `to_found_items()` 为空（Go 对该形态不产任何键，含 Go 不产的 `file_name`）。
    /// 旧版本用例钉的是 `files == [Unknown("")]` + `unknown_file == [""]`，那只对
    /// legacy `message_parser.go:1185-1207` 成立，与本基准（cqparse）相反，故改钉字面。
    /// 谁把 `media_from_segment` 的兜底用回 file 臂，本用例即报警。
    #[test]
    fn file_arm_does_not_fall_back_to_url() {
        for code in [
            "[CQ:file,url=https://x/a.bin]",
            "[CQ:file,file=,url=https://x/a.bin]",
            "[CQ:file,url=https://x/a.bin,file_name=a.bin]",
        ] {
            let plan = OutboundPlan::from_segments(&cqcode::parse(code));
            assert!(
                plan.files.is_empty(),
                "file 臂不兜 url，不该进文件桶: {code}"
            );
            assert!(plan.extra.is_empty(), "file 码不应落 extra: {code}");
            let items = plan.to_found_items();
            for key in [
                "url_file",
                "url_files",
                "local_file",
                "unknown_file",
                "file_name",
            ] {
                assert!(!items.contains_key(key), "不该产出 {key} 键: {items:?}");
            }
            assert!(
                items.is_empty(),
                "空 file 的 file 码不产任何 foundItems 键: {items:?}"
            );
            assert_eq!(plan.text, code, "空 file 的 file 码原位保留字面: {code}");
        }
    }

    /// P57-② 评审钉死（2）：`video` 臂**有** url 兜底（Go
    /// `handlers/message_parser.go:1218-1222` 的 `resolveMediaValue`），
    /// 与 image / record 同口径。
    #[test]
    fn video_arm_falls_back_to_url_when_file_is_empty() {
        let missing_plan =
            OutboundPlan::from_segments(&cqcode::parse("[CQ:video,url=https://x/a.mp4]"));
        assert_eq!(
            missing_plan.videos,
            vec![MediaRef::Url("https://x/a.mp4".to_owned())],
            "file= 缺失时 video 取 url"
        );
        assert_eq!(
            missing_plan.to_found_items()["url_video"],
            ["https://x/a.mp4".to_owned()],
            "url_video 键与值"
        );

        let empty_plan =
            OutboundPlan::from_segments(&cqcode::parse("[CQ:video,file=,url=https://x/b.mp4]"));
        assert_eq!(
            empty_plan.videos,
            vec![MediaRef::Url("https://x/b.mp4".to_owned())],
            "file= 显式为空同样兜底"
        );
        assert_eq!(
            empty_plan.to_found_items()["url_video"],
            ["https://x/b.mp4".to_owned()],
            "url_video 键与值"
        );
    }

    /// P57-② 评审钉死（3）：兜底只在 `file` 为空时生效，`file` 非空则 file 优先
    /// （Go `handlers_media.go:51-55` 的 `if v == ""` 判定）。
    #[test]
    fn media_arm_prefers_file_over_url() {
        let plan = OutboundPlan::from_segments(&cqcode::parse(
            "[CQ:image,file=a.png,url=https://x/b.png]",
        ));
        assert_eq!(
            plan.images,
            vec![MediaRef::Unknown("a.png".to_owned())],
            "file 非空时读 file（无协议前缀按 from_wire 口径落 Unknown）"
        );
        let items = plan.to_found_items();
        assert_eq!(
            items["unknown_image"],
            ["a.png".to_owned()],
            "unknown_image 键与值"
        );
        assert!(
            !items
                .values()
                .flatten()
                .any(|value| value == "https://x/b.png"),
            "file 非空时 url= 不该进 foundItems"
        );
    }

    /// P57-② 评审钉死（4）：url 兜底与 voice 归一两条不变式合在一处——同参
    /// `record` 与 `voice` 都经 `media_from_segment` 落语音通道与 `url_record` 键
    /// （Go `cqparse/handlers_media.go:48-55`）。
    #[test]
    fn record_and_voice_share_the_url_fallback() {
        let record_plan =
            OutboundPlan::from_segments(&cqcode::parse("[CQ:record,file=,url=https://x/a.amr]"));
        let voice_plan =
            OutboundPlan::from_segments(&cqcode::parse("[CQ:voice,file=,url=https://x/a.amr]"));
        assert_eq!(
            record_plan.records,
            vec![MediaRef::Url("https://x/a.amr".to_owned())],
            "record 的 file 为空时兼容 url"
        );
        assert_eq!(voice_plan, record_plan, "voice 应与同参 record 逐字段等价");
        for plan in [&record_plan, &voice_plan] {
            assert_eq!(
                plan.to_found_items()["url_record"],
                ["https://x/a.amr".to_owned()],
                "两条码都落 url_record 键与值"
            );
            assert!(
                plan.unknown_records.is_empty(),
                "兜底成功就不该落语音兜底桶"
            );
        }
    }

    /// P59：裸媒体码（`file` 与 `url` 皆无）按 Go 保留字面原文——
    /// `resolveMediaValue`/`resolveFileValue` 对该形态返回 `Replacement: tok.Raw`、
    /// 不产 foundItems（`handlers/cqparse/handlers_media.go:56-59`、`:88-91`）。
    /// 修复前会产出 `MediaRef::Unknown("")`，向上游发出空 `file_info`。
    #[test]
    fn from_segments_bare_media_code_keeps_literal_text() {
        for code in [
            "[CQ:image]",
            "[CQ:video]",
            "[CQ:file]",
            "[CQ:record]",
            "[CQ:voice]",
        ] {
            let plan = OutboundPlan::from_segments(&cqcode::parse(code));
            assert!(plan.images.is_empty(), "{code} 不该进图片桶");
            assert!(plan.videos.is_empty(), "{code} 不该进视频桶");
            assert!(plan.records.is_empty(), "{code} 不该进语音桶");
            assert!(plan.files.is_empty(), "{code} 不该进文件桶");
            assert!(plan.extra.is_empty(), "{code} 不该落 extra");
            assert_eq!(plan.text, code, "{code} 应原位保留字面原文（含方括号）");
        }
    }

    /// P59：裸码与文本混排时按 Go `Replacement: tok.Raw` 的原位语义保留前后空白，
    /// 逐字节还原到 `plan.text`。
    #[test]
    fn from_segments_bare_media_code_keeps_surrounding_whitespace() {
        let plan = OutboundPlan::from_segments(&cqcode::parse("看 [CQ:image] 好"));
        assert_eq!(
            plan.text, "看 [CQ:image] 好",
            "空白原位保留，码体逐字面回写"
        );
        assert!(plan.images.is_empty(), "裸码不该进图片桶");
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

    /// 出站不可消费的 `[CQ:avatar]` / `[CQ:music]` 按原文留在正文（不再整条 1400）。
    ///
    /// music 与 Go legacy 字符串路径同口径：只认 `[CQ:music,type=qq,id=<数字>]`
    /// （`handlers/cqcode.go:38`，消费点 `handlers/cqcode_pipeline.go:170-178`）。avatar 在 Go
    /// 字符串路径会被替换成 `[CQ:image,file=…]` 字面（`handlers/avatar.go:44`）、段路径发真图，
    /// 本仓暂按原文回写 ⇒ 字节形态与 Go 不同，属已知差异（parity B4、P80）。
    /// 若落进 `extra`，`deferred::unsupported_segment` 会把整条消息变成 1400。
    #[test]
    fn unconsumable_avatar_and_music_stay_literal() {
        for code in [
            "[CQ:avatar]",
            "[CQ:avatar,qq=123]",
            "[CQ:music]",
            "[CQ:music,id=123]",
            "[CQ:music,type=163,id=1]",
            "[CQ:music,type=QQ,id=1]",
            "[CQ:music,type=qq,id=abc]",
        ] {
            let plan = OutboundPlan::from_segments(&cqcode::parse(code));
            assert_eq!(plan.text, code, "应逐字节留在正文: {code}");
            assert!(plan.extra.is_empty(), "不应进 extra（下游会 1400）: {code}");
            assert!(plan.qqmusic.is_empty(), "不应产出 qqmusic: {code}");
        }
    }

    /// 反面对照：`type=qq` + 纯数字 id 仍按 `foundItems["qqmusic"]` 消费，段从正文摘除。
    #[test]
    fn qq_music_with_numeric_id_is_still_consumed() {
        let plan = OutboundPlan::from_segments(&cqcode::parse("听 [CQ:music,type=qq,id=123] 吧"));
        assert_eq!(plan.qqmusic, vec!["123".to_owned()]);
        assert_eq!(plan.text, "听  吧", "音乐段摘除后前后文本原样保留");
        assert!(plan.extra.is_empty());
    }

    #[test]
    fn active_marker_survives_cq_string_round_trip() {
        // 统一契约后 active 段不认冒号兜底（Go `cqUnionParams` 仅 stream 管线传
        // allowColon=true，`handlers/cqcode_pipeline.go:184`），`k=v` 是规范写法
        let plan = OutboundPlan::from_segments(&crate::cqcode::parse("[CQ:active,type=push]"));
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

        // 新契约钉桩：非 stream 段的裸片段只在「有前一键」时并入其值，否则整段丢弃
        // （Go `cqUnionParamsSegments` 的 `lastKey != "" && t != ""` 守卫，
        // `handlers/cqcode.go:97-102`）。存在位仍由段名置位，不丢 `[CQ:active]` 语义。
        for text in ["[CQ:active,type:push]", "[CQ:active,push]"] {
            let colon = OutboundPlan::from_segments(&crate::cqcode::parse(text));
            let active = colon.active.as_ref().expect("应有 active");
            assert!(active.active, "{text} 出现即置存在位");
            assert_eq!(
                active.kind, None,
                "{text} 首参无前置键：type:push / push 按 Go 并集口径丢弃"
            );
        }
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
        // 统一契约后 card 段同样只认 `k=v`（Go card 管线 `cqUnionParams(…, false)`，
        // 冒号兜底仅 stream）；`=` 是规范写法
        let segments = crate::cqcode::parse(
            "[CQ:card,title=标题,desc=描述,pic=https://x.com/p.png,url=https://x.com]",
        );
        let plan = OutboundPlan::from_segments(&segments);

        let fields = plan.card[0].fields().expect("card 应为 JSON 对象");
        assert_eq!(fields.title.as_deref(), Some("标题"));
        assert_eq!(fields.desc.as_deref(), Some("描述"));
        assert_eq!(fields.pic.as_deref(), Some("https://x.com/p.png"));
        assert_eq!(fields.url.as_deref(), Some("https://x.com"));

        // 新契约钉桩：① 冒号形态首参无前置键 → 整段丢弃（Go `cqcode.go:97-102` 同）；
        // ② `=` 之后的裸尾段并入前一键值、`,` 拼接（C3 并集，`user_ids=1,2,3` 三路径等价）
        let colon_first = OutboundPlan::from_segments(&crate::cqcode::parse(
            "[CQ:card,title:标题,desc=描述,pic=https://x.com/p.png,url=https://x.com]",
        ));
        let dropped = colon_first.card[0].fields().expect("card 应为 JSON 对象");
        assert_eq!(dropped.title, None, "首参冒号形态不是 k=v：丢弃，同 Go");
        assert_eq!(
            dropped.desc.as_deref(),
            Some("描述"),
            "其余 k=v 聚合不受影响"
        );

        let merged =
            OutboundPlan::from_segments(&crate::cqcode::parse("[CQ:card,title=标题,副标题]"));
        let merged_fields = merged.card[0].fields().expect("card 应为 JSON 对象");
        assert_eq!(
            merged_fields.title.as_deref(),
            Some("标题,副标题"),
            "裸尾段并入前一键值"
        );

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

        // 两种写法同现时 desc / pic 优先（Rust 别名取值顺序；Go 不识别别名，只认 desc / pic）
        let both = CardPayload {
            raw: r#"{"desc":"简写","description":"全名","pic":"https://a/p.png","pic_url":"https://b/p.png"}"#
                .to_owned(),
        };
        let fields = both.fields().expect("同现应能解析");
        assert_eq!(fields.desc.as_deref(), Some("简写"));
        assert_eq!(fields.pic.as_deref(), Some("https://a/p.png"));

        // 简写为空/空白时回退到官方名（Go 用 TrimSpace 判空）
        let blank = CardPayload {
            raw: r#"{"desc":"  ","description":"全名","pic":"","pic_url":"https://b/p.png"}"#
                .to_owned(),
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

    /// P81：形如 `[CQ:card]`（无参）或 `data=` 的卡片刻度在 Go 里是**静默丢弃**
    /// （`handlers/cqcode_pipeline.go:124-130`：cardData 为空 ⇒ 不入 foundItems、正文也不留）。
    #[test]
    fn empty_card_is_dropped_silently() {
        for code in ["[CQ:card]", "[CQ:card,data=]"] {
            let plan = OutboundPlan::from_segments(&cqcode::parse(code));
            assert!(plan.card.is_empty(), "空卡片不该产出 card 载荷: {code}");
            assert!(
                plan.extra.is_empty(),
                "空卡片不该进 extra（下游 1400）: {code}"
            );
            assert!(plan.text.is_empty(), "空卡片不留字面: {code}");
            assert!(plan.is_empty(), "整条消息应视为空: {code}");
        }
    }

    /// `ark` 在 Go legacy 里被整段剥掉（`handlers/cqcode.go:198-205`、
    /// `handlers/cqcode_pipeline.go:57-59`）：既不发送也不留字面，也不拒发整条消息。
    #[test]
    fn ark_is_stripped_silently() {
        let plan =
            OutboundPlan::from_segments(&cqcode::parse("hi[CQ:ark,ark_type=tuwen,prompt=点我]"));
        assert_eq!(plan.text, "hi", "ark 整段消失，前后文本原样保留");
        assert!(plan.extra.is_empty(), "ark 不该进 extra（下游 1400）");
    }

    /// `member` / `set_group` / `remove` 在 Go 是**真执行**管理动作（踢人 / 禁言 / 撤回，
    /// `handlers/message_parser.go:1166-1173`/`:1175-1181`/`:1611-1617` → `cqcode.go:235-240`）。
    /// 本仓未实现（P83）：留在 `extra` 让下游回 1400，静默吞掉会悄悄丢管理动作。
    #[test]
    fn management_codes_stay_in_extra() {
        for (code, kind) in [
            ("[CQ:member,qq=123,group_id=456]", "member"),
            ("[CQ:set_group,group_id=456,ban=1]", "set_group"),
            ("[CQ:remove,message_id=789]", "remove"),
        ] {
            let plan = OutboundPlan::from_segments(&cqcode::parse(code));
            assert_eq!(plan.extra.len(), 1, "管理码应留在 extra: {code}");
            assert_eq!(plan.extra[0].kind, kind, "段名不变: {code}");
            assert!(plan.text.is_empty(), "管理码不该同时留字面: {code}");
        }
    }

    /// P81 硬验收：字面保留的每一种形态写回正文后，`to_segments` 重分词必须还原成同一计划——
    /// 既不能把字面重新解析成码段（会再走一遍消费臂），也不许改写字节。
    #[test]
    fn literal_segments_round_trip_byte_for_byte() {
        for code in [
            "[CQ:face,id=1]",
            "[CQ:face , id = 14, name=a&#44;b]",
            "[CQ:node,id=1,user_id=2,nickname=n,content=c]",
            "[CQ:poke,qq=10086]",
            "[CQ:group_info]",
            "[CQ:json,data=abc]",
            "[CQ:unknown_code,x=1]",
            "[CQ:markdown]",
            "[CQ:markdown,data=]",
            "[CQ:markdown,data=abc]",
            "[CQ:keyboard]",
            "[CQ:keyboard,data=abc]",
            "[CQ:stream]",
            "[CQ:stream,type=start]",
            "[CQ:stream,type=start,qq=abc]",
            "[CQ:input_notify]",
            "[CQ:input_notify,type=x]",
            "[CQ:avatar,qq=123]",
            "[CQ:music,type=163,id=1]",
        ] {
            let plan = OutboundPlan::from_segments(&cqcode::parse(code));
            assert_eq!(plan.text, code, "应逐字节留在正文: {code}");
            assert!(plan.extra.is_empty(), "字面形态不该进 extra: {code}");
            assert_eq!(
                OutboundPlan::from_segments(&plan.to_segments()),
                plan,
                "字面往返必须幂等: {code}"
            );
        }
    }

    /// 收紧后的载荷形态：`markdown` / `keyboard` 只认 `base64://` 与 `{` 两种前缀，
    /// 其余（含裸 base64 markdown）按字面保留，与 Go legacy 正则一致。
    #[test]
    fn rich_payload_prefixes_are_enforced() {
        let bare_base64 = crate::base64::encode(br#"{"content":"hi"}"#);
        let code = format!("[CQ:markdown,data={bare_base64}]");
        let plan = OutboundPlan::from_segments(&cqcode::parse(&code));
        assert!(plan.markdown.is_empty(), "裸 base64 不再消费: {code}");
        assert_eq!(plan.text, code, "裸 base64 按字面保留");

        let ok = OutboundPlan::from_segments(&cqcode::parse(&format!(
            "[CQ:markdown,data=base64://{bare_base64}]"
        )));
        assert_eq!(ok.markdown[0].raw, bare_base64, "base64:// 仍照旧消费");

        let json = OutboundPlan::from_segments(&cqcode::parse(r#"[CQ:keyboard,data={"id":"k1"}]"#));
        assert_eq!(
            json.keyboard[0].raw, r#"{"id":"k1"}"#,
            "花括号开头的裸 JSON 仍消费"
        );

        let base64 = OutboundPlan::from_segments(&cqcode::parse(
            "[CQ:keyboard,data=base64://eyJpZCI6ImsxIn0=]",
        ));
        assert_eq!(
            base64.keyboard[0].raw, r#"{"id":"k1"}"#,
            "base64:// 解码成 JSON"
        );
    }
}
