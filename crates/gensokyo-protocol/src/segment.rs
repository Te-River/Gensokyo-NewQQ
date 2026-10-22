//! OneBot V11 消息段（`array_value` 数组形态）与 `message` 字段的两种形态。
//!
//! 对照 Go：`internal/domain/message/segment.go` 的 `Segment{Type, Data}` 把 `data` 存成
//! `map[string]string`（键序丢失）；这里用 `Vec<(String, String)>` 保留参数顺序，CQ 码生成
//! 才能做到与 Go 输出逐字节一致（见 [`crate::cqcode`]）。
//!
//! 参数值一律存**未转义**的原文；转义只发生在生成 CQ 文本时，与解析侧互为逆操作。
//! `data` 里的嵌套对象 / 数组同样落成字符串（紧凑 JSON），因此 `params` 仍是键序稳定的
//! `Vec<(String, String)>`——与 Go `Segment.Data` 的 `map[string]string` 同构，而嵌套形态
//! 由 `foundItems` 契约（base64 / JSON 串）承载。

use std::fmt;
use std::path::PathBuf;

use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use gensokyo_core::Ob11Id;

use crate::base64;
use crate::cqcode::escape;
use crate::parse_id;

/// 文本段类型。
pub const KIND_TEXT: &str = "text";

/// 一个 OneBot 消息段。
///
/// JSON 形态为 `{"type":"at","data":{"qq":"123"}}`；`data` 里的数字/布尔值反序列化时按
/// Go `stringify` 的语义转成字符串（QQ 侧两种写法都出现过）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// 段类型（`text` / `at` / `image` …）。
    pub kind: String,
    /// 参数，保持插入（线上）顺序。
    pub params: Vec<(String, String)>,
}

impl Segment {
    /// 新建一个只有类型、无参数的消息段。
    #[must_use]
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            params: Vec::new(),
        }
    }

    /// 追加参数（值保持未转义原文）。
    #[must_use]
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.push((key.into(), value.into()));
        self
    }

    /// 追加可缺省参数；`None` 时整条参数不落地，避免生成 `key=` 噪声。
    #[must_use]
    pub fn param_opt(mut self, key: &str, value: Option<impl Into<String>>) -> Self {
        if let Some(value) = value {
            self.params.push((key.to_owned(), value.into()));
        }
        self
    }

    /// 读取参数。
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// 读取 ID 型参数：纯数字还原为 [`Ob11Id::Num`]，其余（32 位 OpenID）保持字符串形态。
    #[must_use]
    pub fn id_param(&self, key: &str) -> Option<Ob11Id> {
        self.get(key).map(parse_id)
    }

    /// 是否文本段。
    #[must_use]
    pub fn is_text(&self) -> bool {
        self.kind == KIND_TEXT
    }

    /// 文本段内容（非文本段返回 `None`）。
    #[must_use]
    pub fn text_content(&self) -> Option<&str> {
        if self.is_text() {
            self.get("text")
        } else {
            None
        }
    }

    /// 生成该段的 CQ 文本：文本段原样输出，其余按 `[CQ:type,k=v,...]` 输出并转义参数。
    #[must_use]
    pub fn to_cq(&self) -> String {
        if self.is_text() {
            return self.get("text").unwrap_or_default().to_owned();
        }
        let mut out = String::with_capacity(16 + self.params.len() * 12);
        out.push_str("[CQ:");
        out.push_str(&self.kind);
        for (key, value) in &self.params {
            out.push(',');
            out.push_str(key);
            out.push('=');
            out.push_str(&escape(value));
        }
        out.push(']');
        out
    }

    /// 文本段构造。
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::new(KIND_TEXT).param("text", text)
    }

    /// `[CQ:at,qq=…]`。
    #[must_use]
    pub fn at(id: impl Into<Ob11Id>) -> Self {
        Self::new("at").param("qq", id.into().to_string())
    }

    /// `[CQ:reply,id=…]`。
    #[must_use]
    pub fn reply(id: impl Into<Ob11Id>) -> Self {
        Self::new("reply").param("id", id.into().to_string())
    }

    /// `[CQ:image,file=…]`。
    #[must_use]
    pub fn image(media: &MediaRef) -> Self {
        Self::new("image").param("file", media.to_wire())
    }

    /// `[CQ:record,file=…]`。
    #[must_use]
    pub fn record(media: &MediaRef) -> Self {
        Self::new("record").param("file", media.to_wire())
    }

    /// `[CQ:video,file=…]`。
    #[must_use]
    pub fn video(media: &MediaRef) -> Self {
        Self::new("video").param("file", media.to_wire())
    }

    /// `[CQ:file,file=…,file_name=…]`（Go 的 `ProcessCQFile` 同时认 `file_name` 与 `name`）。
    #[must_use]
    pub fn file(media: &MediaRef, file_name: Option<&str>) -> Self {
        Self::new("file")
            .param("file", media.to_wire())
            .param_opt("file_name", file_name)
    }

    /// `[CQ:face,id=…]`。
    #[must_use]
    pub fn face(id: impl fmt::Display) -> Self {
        Self::new("face").param("id", id.to_string())
    }

    /// `[CQ:music,type=qq,id=…]`（`foundItems` 里对应 `qqmusic`）。
    #[must_use]
    pub fn music(id: impl Into<String>) -> Self {
        Self::new("music").param("type", "qq").param("id", id)
    }

    /// `[CQ:markdown,data=base64://…]`。
    ///
    /// `data` 是 `foundItems["markdown"]` 的同款 base64(JSON)，生成侧补 `base64://` 前缀
    /// 以命中 Go 的 `mdPattern`（`handlers/cqcode.go:36`）。
    #[must_use]
    pub fn markdown(data: impl Into<String>) -> Self {
        Self::new("markdown").param("data", format!("base64://{}", data.into()))
    }

    /// `[CQ:keyboard,data=base64://…]`。
    ///
    /// 入参是 `foundItems["keyboard"]` 同款 JSON 串；生成侧编码成 base64，避免 JSON 里的
    /// `]` / `,` 在 CQ 文本里被转义后又被 Go 的裸 JSON 正则原样收走。
    #[must_use]
    pub fn keyboard(data: impl Into<String>) -> Self {
        let raw = data.into();
        Self::new("keyboard").param(
            "data",
            format!("base64://{}", base64::encode(raw.as_bytes())),
        )
    }

    /// `[CQ:card,title=…,desc=…,pic=…,url=…]`（入参为 JSON 对象）。
    ///
    /// Go 的卡片契约是 `map[string]string`（`handlers/message_parser.go:988-1010`、
    /// `handlers/cqcode_pipeline.go:114-132`）：这里把对象的字符串字段展开成参数，
    /// 非对象或没有字符串字段时退回 `data=` 原文，保证不丢内容。
    #[must_use]
    pub fn card(data: impl Into<String>) -> Self {
        let raw = data.into();
        let mut segment = Self::new("card");
        if let Ok(serde_json::Value::Object(object)) = serde_json::from_str(&raw) {
            for (key, value) in object {
                if let serde_json::Value::String(text) = value {
                    segment = segment.param(key, text);
                }
            }
        }
        if segment.params.is_empty() {
            segment = segment.param("data", raw);
        }
        segment
    }

    /// `[CQ:stream,type=…,qq=…]`。
    #[must_use]
    pub fn stream(kind: impl Into<String>, qq: impl Into<Ob11Id>) -> Self {
        Self::new("stream")
            .param("type", kind)
            .param("qq", qq.into().to_string())
    }

    /// `[CQ:input_notify,type=…,second=…]`。
    #[must_use]
    pub fn input_notify(kind: i64, second: Option<i64>) -> Self {
        Self::new("input_notify")
            .param("type", kind.to_string())
            .param_opt("second", second.map(|value| value.to_string()))
    }

    /// `[CQ:active]` 或 `[CQ:active,type=…,sub_type=…]`。
    #[must_use]
    pub fn active(kind: Option<&str>, sub_type: Option<&str>) -> Self {
        Self::new("active")
            .param_opt("type", kind)
            .param_opt("sub_type", sub_type)
    }

    /// `[CQ:wakeup,userid=…]`。
    #[must_use]
    pub fn wakeup(user: impl Into<String>) -> Self {
        Self::new("wakeup").param("userid", user)
    }

    /// `[CQ:poke,qq=…]`。
    #[must_use]
    pub fn poke(qq: impl Into<Ob11Id>) -> Self {
        Self::new("poke").param("qq", qq.into().to_string())
    }
}

impl fmt::Display for Segment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_cq())
    }
}

impl Serialize for Segment {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("type", &self.kind)?;
        map.serialize_entry("data", &ParamsRef(&self.params))?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for Segment {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = SegmentWire::deserialize(deserializer)?;
        Ok(Self {
            kind: wire.kind,
            params: wire.data.map(|data| data.0).unwrap_or_default(),
        })
    }
}

/// 以 `data` 对象形态序列化参数（顺序 == `Vec` 顺序）。
struct ParamsRef<'a>(&'a [(String, String)]);

impl Serialize for ParamsRef<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, value) in self.0 {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

/// `data` 反序列化结果（顺序由反序列化器给出；JSON 对象按书写顺序）。
#[derive(Debug)]
struct Params(Vec<(String, String)>);

impl<'de> Deserialize<'de> for Params {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(ParamsVisitor).map(Self)
    }
}

struct ParamsVisitor;

impl<'de> Visitor<'de> for ParamsVisitor {
    type Value = Vec<(String, String)>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("消息段 data 对象")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut out = Vec::with_capacity(map.size_hint().unwrap_or(0));
        while let Some((key, value)) = map.next_entry::<String, Option<Scalar>>()? {
            out.push((key, value.map(Scalar::into_string).unwrap_or_default()));
        }
        Ok(out)
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Scalar {
    Text(String),
    Int(i64),
    Uint(u64),
    Float(f64),
    Bool(bool),
    /// 嵌套对象 / 数组：`markdown` 的 `data.data`、`keyboard` 的 JSON、`node` 的 `content`
    /// 都是这种形态（Go 侧 `data` 是 `map[string]interface{}`）。必须放在最后——
    /// `untagged` 按声明顺序匹配，`Value` 会吞掉前面所有标量分支。
    Json(serde_json::Value),
}

impl Scalar {
    fn into_string(self) -> String {
        match self {
            Self::Text(value) => value,
            Self::Int(value) => value.to_string(),
            Self::Uint(value) => value.to_string(),
            Self::Float(value) => value.to_string(),
            Self::Bool(value) => value.to_string(),
            // 归一为紧凑 JSON 文本（键序由 serde_json 决定，与 Go `json.Marshal` 的排序一致）
            Self::Json(value) => value.to_string(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct SegmentWire {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    data: Option<Params>,
}

/// 消息段里媒体引用的来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaRef {
    /// 本地文件（`file://` 形式去掉前缀后的路径）。
    Local(PathBuf),
    /// 远程 URL（`http://` / `https://`）。
    Url(String),
    /// base64 内容（去掉 `base64://` 前缀）。
    Base64(String),
    /// 无法识别、需回退处理的内容（Go 的 `unknown_image` 等）。
    Unknown(String),
}

impl MediaRef {
    /// 按 Go 的前缀口径解析 CQ `file=` 值。
    ///
    /// `file:///C:/x` 在 Go 的 Windows 正则里捕获为 `C:/x`，`file:///tmp/x` 在 Unix 正则里
    /// 捕获为 `/tmp/x`；两者都设计成"去掉 `file://` 后只多一个前导 `/`"。
    #[must_use]
    pub fn from_wire(raw: &str) -> Self {
        if let Some(rest) = raw.strip_prefix("base64://") {
            return Self::Base64(rest.to_owned());
        }
        if raw.starts_with("http://") || raw.starts_with("https://") {
            return Self::Url(raw.to_owned());
        }
        if let Some(rest) = raw.strip_prefix("file://") {
            return Self::Local(PathBuf::from(normalize_file_path(rest)));
        }
        Self::Unknown(raw.to_owned())
    }

    /// 还原成 CQ `file=` 值（本地路径统一成 `file:///…` 形态）。
    #[must_use]
    pub fn to_wire(&self) -> String {
        match self {
            Self::Local(path) => {
                let text = path.to_string_lossy().replace('\\', "/");
                format!("file:///{}", text.trim_start_matches('/'))
            }
            Self::Url(url) => url.clone(),
            Self::Base64(data) => format!("base64://{data}"),
            Self::Unknown(raw) => raw.clone(),
        }
    }
}

fn normalize_file_path(rest: &str) -> String {
    if let Some(tail) = rest.strip_prefix('/') {
        let mut chars = tail.chars();
        if matches!((chars.next(), chars.next()), (Some(drive), Some(':')) if drive.is_ascii_alphabetic())
        {
            return tail.to_owned();
        }
    }
    rest.to_owned()
}

/// `message` 字段的两种线上形态：CQ 字符串或消息段数组。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// 字符串形态（`array_value = false`）。
    Text(String),
    /// 数组形态（`array_value = true`）。
    Array(Vec<Segment>),
}

impl MessageContent {
    /// 是否数组形态。
    #[must_use]
    pub fn is_array(&self) -> bool {
        matches!(self, Self::Array(_))
    }

    /// 字符串形态原始值。
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Array(_) => None,
        }
    }

    /// 展开为消息段：字符串形态按 [`crate::cqcode::parse`] 解析。
    ///
    /// 数组形态直接克隆，不做二次解析（段里已没有 CQ 字面量）。
    #[must_use]
    pub fn segments(&self) -> Vec<Segment> {
        match self {
            Self::Text(text) => crate::cqcode::parse(text),
            Self::Array(segments) => segments.clone(),
        }
    }

    /// 是否为空内容。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Text(text) => text.is_empty(),
            Self::Array(segments) => segments.is_empty(),
        }
    }
}

impl Default for MessageContent {
    fn default() -> Self {
        Self::Text(String::new())
    }
}

impl From<&str> for MessageContent {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<String> for MessageContent {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<Vec<Segment>> for MessageContent {
    fn from(value: Vec<Segment>) -> Self {
        Self::Array(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{MediaRef, MessageContent, Segment};

    #[test]
    fn array_shape_round_trips_with_param_order() {
        let json = r#"{"type":"image","data":{"file":"https://x.com/a.png","subType":0,"url":"https://y.com/b.png"}}"#;
        let segment: Segment = serde_json::from_str(json).expect("反序列化失败");
        assert_eq!(segment.kind, "image");
        assert_eq!(
            segment.params,
            vec![
                ("file".to_owned(), "https://x.com/a.png".to_owned()),
                ("subType".to_owned(), "0".to_owned()),
                ("url".to_owned(), "https://y.com/b.png".to_owned()),
            ],
            "数字值应按 Go stringify 语义转字符串"
        );

        let back = serde_json::to_string(&segment).expect("序列化失败");
        // data 是 map[string]string（Go `Segment.Data` 同构），数字被 stringify 成 "0"
        assert_eq!(
            back,
            r#"{"type":"image","data":{"file":"https://x.com/a.png","subType":"0","url":"https://y.com/b.png"}}"#
        );
    }

    #[test]
    fn text_segment_generates_raw_text_not_cq() {
        assert_eq!(Segment::text("你好").to_cq(), "你好");
        assert_eq!(
            Segment::text("a[CQ:at]b").to_cq(),
            "a[CQ:at]b",
            "文本不转义"
        );
    }

    #[test]
    fn non_text_segment_escapes_params() {
        let segment = Segment::new("at").param("name", "a,b]c&d");
        assert_eq!(segment.to_cq(), "[CQ:at,name=a&#44;b&#93;c&amp;d]");
    }

    #[test]
    fn media_ref_detects_wire_prefixes() {
        assert_eq!(
            MediaRef::from_wire("base64://QUJD"),
            MediaRef::Base64("QUJD".to_owned())
        );
        assert_eq!(
            MediaRef::from_wire("https://x.com/a.png"),
            MediaRef::Url("https://x.com/a.png".to_owned())
        );
        assert_eq!(
            MediaRef::from_wire("file:///C:/tmp/a.png"),
            MediaRef::Local("C:/tmp/a.png".into())
        );
        assert_eq!(
            MediaRef::from_wire("file:///tmp/a.png"),
            MediaRef::Local("/tmp/a.png".into())
        );
        assert_eq!(
            MediaRef::from_wire("weird://x"),
            MediaRef::Unknown("weird://x".to_owned())
        );
    }

    #[test]
    fn media_ref_wire_round_trip() {
        for raw in [
            "base64://QUJD",
            "http://x.com/a.png",
            "file:///C:/tmp/a.png",
            "file:///tmp/a.png",
            "weird://x",
        ] {
            assert_eq!(MediaRef::from_wire(raw).to_wire(), raw, "原样往返: {raw}");
        }
    }

    #[test]
    fn nested_data_values_normalize_to_compact_json() {
        // markdown 的 data.data 是对象
        let markdown: Segment = serde_json::from_str(
            r#"{"type":"markdown","data":{"data":{"content":"hi","extra":["a",1]}}}"#,
        )
        .expect("markdown 段反序列化失败");
        assert_eq!(
            markdown.get("data"),
            Some(r#"{"content":"hi","extra":["a",1]}"#),
            "嵌套对象应归一为紧凑 JSON 字符串"
        );

        // keyboard 的 data 是对象
        let keyboard: Segment =
            serde_json::from_str(r#"{"type":"keyboard","data":{"data":{"id":"k1","rows":[]}}}"#)
                .expect("keyboard 段反序列化失败");
        assert_eq!(keyboard.get("data"), Some(r#"{"id":"k1","rows":[]}"#));

        // node 的 content 是段数组
        let node: Segment = serde_json::from_str(
            r#"{"type":"node","data":{"user_id":"1","nickname":"n","content":[{"type":"text","data":{"text":"hi"}}]}}"#,
        )
        .expect("node 段反序列化失败");
        assert_eq!(node.get("user_id"), Some("1"), "同段内的标量不受影响");
        assert_eq!(
            node.get("content"),
            Some(r#"[{"data":{"text":"hi"},"type":"text"}]"#),
            "数组应归一为紧凑 JSON 字符串（键序与 Go json.Marshal 一致）"
        );

        let back = serde_json::to_string(&markdown).expect("序列化失败");
        assert_eq!(
            back,
            r#"{"type":"markdown","data":{"data":"{\"content\":\"hi\",\"extra\":[\"a\",1]}"}}"#,
            "嵌套值序列化为 JSON 字符串"
        );
    }

    #[test]
    fn card_segment_expands_json_fields_into_params() {
        let segment = Segment::card(
            r#"{"title":"标题","desc":"描述","pic":"https://x.com/p.png","url":"https://x.com"}"#,
        );
        assert_eq!(segment.kind, "card");
        assert_eq!(
            segment.params,
            vec![
                ("desc".to_owned(), "描述".to_owned()),
                ("pic".to_owned(), "https://x.com/p.png".to_owned()),
                ("title".to_owned(), "标题".to_owned()),
                ("url".to_owned(), "https://x.com".to_owned()),
            ],
            "按 Go json.Marshal 的键序展开"
        );
        assert_eq!(
            segment.to_cq(),
            "[CQ:card,desc=描述,pic=https://x.com/p.png,title=标题,url=https://x.com]"
        );

        let fallback = Segment::card("not json");
        assert_eq!(
            fallback.params,
            vec![("data".to_owned(), "not json".to_owned())]
        );
    }

    #[test]
    fn markdown_and_keyboard_segments_carry_base64_wire_form() {
        assert_eq!(
            Segment::markdown("eyJjb250ZW50IjoiaGkifQ==").to_cq(),
            "[CQ:markdown,data=base64://eyJjb250ZW50IjoiaGkifQ==]"
        );
        assert_eq!(
            Segment::keyboard(r#"{"id":"k1"}"#).to_cq(),
            "[CQ:keyboard,data=base64://eyJpZCI6ImsxIn0=]"
        );
    }

    #[test]
    fn message_content_accepts_both_shapes() {
        let text: MessageContent =
            serde_json::from_str("\"hi[CQ:at,qq=1]\"").expect("字符串形态反序列化失败");
        assert_eq!(text.segments().len(), 2);

        let array: MessageContent =
            serde_json::from_str(r#"[{"type":"text","data":{"text":"hi"}}]"#)
                .expect("数组形态反序列化失败");
        assert!(array.is_array());
        assert_eq!(array.segments()[0], Segment::text("hi"));

        assert_eq!(
            serde_json::to_string(&text).expect("序列化失败"),
            "\"hi[CQ:at,qq=1]\""
        );
    }
}
