//! OneBot V11 消息段（`array_value` 数组形态）与 `message` 字段的两种形态。
//!
//! 对照 Go：`internal/domain/message/segment.go` 的 `Segment{Type, Data}` 把 `data` 存成
//! `map[string]string`（键序丢失）；这里用 `Vec<(String, String)>` 保留参数顺序，CQ 码生成
//! 才能做到与 Go 输出逐字节一致（见 [`crate::cqcode`]）。
//!
//! 参数值一律存**未转义**的原文；转义只发生在生成 CQ 文本时，与解析侧互为逆操作。
//! `data` 里的嵌套对象 / 数组同样落成字符串（紧凑 JSON），因此 `params` 仍是键序稳定的
//! `Vec<(String, String)>`——与 Go `Segment.Data` 的 `map[string]string` 同构，而嵌套形态
//! 由 `foundItems` 契约（base64 / JSON 串）承载。唯一例外是 ark 段的 `fields`（P52）：
//! Go `arkSegmentData` 出真 JSON 对象，经 [`Segment::param_json`] 标记，序列化时按解析后的
//! JSON 值输出。
//!
//! JSON 序列化的键序（P53）对齐 Go `json.Marshal` 的 map 排序：外层 `data` 先于 `type`，
//! 内层 data 键按字母序；`Vec` 插入序只服务 CQ 文本路径。

use std::borrow::Cow;
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
/// JSON 形态为 `{"data":{"qq":"123"},"type":"at"}`（键序同 Go `json.Marshal`，P53）；`data` 里的数字/布尔值反序列化时按
/// Go `stringify` 的语义转成字符串（QQ 侧两种写法都出现过）。
#[derive(Debug, Clone)]
pub struct Segment {
    /// 段类型（`text` / `at` / `image` …）。
    pub kind: String,
    /// 参数，保持插入（线上）顺序。
    pub params: Vec<(String, String)>,
    /// raw JSON 参数键（P52）：序列化时这些键的值按解析后的 JSON 值输出
    /// （ark 的 `fields`，Go `arkSegmentData` 的 `map[string]interface{}` 契约），解析失败回退字符串。
    pub(crate) json_keys: Vec<String>,
    /// 透传原文切片：`cqcode::parse` 命中码段时记录的**整段原始字节**（含 `[CQ:` 与收尾 `]`）。
    /// 透传臂经 [`Segment::raw_or_cq`] 输出它，保证用户输入不被规范化改写（Major 修复）。
    /// 协议层自产段（builder 构造 / JSON 反序列化）为 `None`，透传输出回退到规范化序列化，
    /// 两路都不改字节。
    pub(crate) raw: Option<String>,
}

/// 语义相等只看 `kind` + `params`：`raw` 是字节保真载体、`json_keys` 是序列化形态标记，
/// 都不参与比较——同一段码「parse 而来」与「代码构造」必须判等（码表往返与计划比对的既有口径）。
/// params 语义上是 map：P53 之后键序只是序列化细节，判等与顺序无关（P54）；
/// 重复键按多重集比较——每个 (key, value) 的重数须一致，不做 last-wins 去重。
impl PartialEq for Segment {
    fn eq(&self, other: &Self) -> bool {
        if self.kind != other.kind || self.params.len() != other.params.len() {
            return false;
        }
        let mut lhs: Vec<&(String, String)> = self.params.iter().collect();
        let mut rhs: Vec<&(String, String)> = other.params.iter().collect();
        lhs.sort_unstable();
        rhs.sort_unstable();
        lhs == rhs
    }
}

impl Eq for Segment {}

impl Segment {
    /// 新建一个只有类型、无参数的消息段。
    #[must_use]
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            params: Vec::new(),
            json_keys: Vec::new(),
            raw: None,
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

    /// 追加 raw JSON 参数：值按紧凑 JSON 文本存储，序列化时按解析后的 JSON 值输出（P52）。
    ///
    /// 供 ark 段 `fields` 这类 Go `map[string]interface{}` 契约
    /// （`handlers/message_parser.go:2445-2447`）；解析失败回退字符串输出，保证不丢内容。
    #[must_use]
    pub fn param_json(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let key = key.into();
        self.params.push((key.clone(), value.into()));
        self.json_keys.push(key);
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

    /// 挂上透传原文切片（仅 `cqcode::parse` / `parse_one` 内部使用，`raw` 为含括号整段字节）。
    #[must_use]
    pub(crate) fn with_raw(mut self, raw: &str) -> Self {
        self.raw = Some(raw.to_owned());
        self
    }

    /// 透传输出：parse 而来的段逐字节返回原文切片；自产段（`raw = None`）回退规范化 [`Self::to_cq`]。
    ///
    /// 消费点：`deferred::render_plain` 的非 at 臂、`outbound::from_segments` 的 at 臂——
    /// 凡是**不消费参数、只把段转写回文本**的臂都必须走这里，不得走 `to_cq()`（会改写用户字节）。
    #[must_use]
    pub fn raw_or_cq(&self) -> Cow<'_, str> {
        match &self.raw {
            Some(raw) => Cow::Borrowed(raw),
            None => Cow::Owned(self.to_cq()),
        }
    }

    /// 生成该段的 CQ 文本：文本段原样输出，其余按 `[CQ:type,k=v,...]` 输出并转义参数。
    ///
    /// 这是**规范化**输出（供真正消费参数后重排的场景）；透传用户输入请用 [`Self::raw_or_cq`]。
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
        // Go json.Marshal 对 map 按键字母排序：data < type（P53）
        map.serialize_entry(
            "data",
            &ParamsRef {
                params: &self.params,
                json_keys: &self.json_keys,
            },
        )?;
        map.serialize_entry("type", &self.kind)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for Segment {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = SegmentWire::deserialize(deserializer)?;
        let (params, json_keys) = wire.data.map(|data| (data.0, data.1)).unwrap_or_default();
        // 只有 ark 段的 `fields` 在 Go 里是真对象（`arkSegmentData`）；其余段的嵌套值按
        // `map[string]string` 契约保持字符串化（markdown / keyboard / node 往返不变）
        let json_keys = if wire.kind == "ark" {
            json_keys.into_iter().filter(|key| key == "fields").collect()
        } else {
            Vec::new()
        };
        Ok(Self {
            kind: wire.kind,
            params,
            json_keys,
            // JSON 数组形态没有 CQ 原文：按自产段处理（透传输出 = 规范化序列化）
            raw: None,
        })
    }
}

/// 以 `data` 对象形态序列化参数（键按字母排序，对齐 Go `json.Marshal` 的 map 键序，P53）；
/// `json_keys` 里的键按解析后的 JSON 值输出。
struct ParamsRef<'a> {
    params: &'a [(String, String)],
    json_keys: &'a [String],
}

impl Serialize for ParamsRef<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Go json.Marshal 对 map 按键字母排序（P53）；稳定排序保留重复键的插入序（现状行为）
        let mut entries: Vec<&(String, String)> = self.params.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut map = serializer.serialize_map(Some(entries.len()))?;
        for (key, value) in entries {
            if self.json_keys.contains(key) {
                // 解析失败回退字符串输出，保证不丢内容
                match serde_json::from_str::<serde_json::Value>(value) {
                    Ok(parsed) => map.serialize_entry(key, &parsed)?,
                    Err(_) => map.serialize_entry(key, value)?,
                }
            } else {
                map.serialize_entry(key, value)?;
            }
        }
        map.end()
    }
}

/// `data` 反序列化结果（顺序由反序列化器给出；JSON 对象按书写顺序）；
/// 第二元是值为对象的键（raw JSON 候选，P52；数组不标记，P53；是否生效由 `Segment::deserialize` 按段类型收口）。
#[derive(Debug)]
struct Params(Vec<(String, String)>, Vec<String>);

impl<'de> Deserialize<'de> for Params {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer
            .deserialize_map(ParamsVisitor)
            .map(|(params, json_keys)| Self(params, json_keys))
    }
}

struct ParamsVisitor;

impl<'de> Visitor<'de> for ParamsVisitor {
    type Value = (Vec<(String, String)>, Vec<String>);

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("消息段 data 对象")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut out = Vec::with_capacity(map.size_hint().unwrap_or(0));
        let mut json_keys = Vec::new();
        while let Some((key, value)) = map.next_entry::<String, Option<Scalar>>()? {
            match value {
                Some(Scalar::Json(value)) => {
                    let text = value.to_string();
                    // 只标对象形态（Go map 契约）；数组保持字符串化（P53）
                    if text.starts_with('{') {
                        json_keys.push(key.clone());
                    }
                    out.push((key, text));
                }
                other => out.push((key, other.map(Scalar::into_string).unwrap_or_default())),
            }
        }
        Ok((out, json_keys))
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
        // data 是 map[string]string（Go `Segment.Data` 同构），数字被 stringify 成 "0"；
        // JSON 键序同 Go json.Marshal：外层 data 先、内层字母序（P53）
        assert_eq!(
            back,
            r#"{"data":{"file":"https://x.com/a.png","subType":"0","url":"https://y.com/b.png"},"type":"image"}"#
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
            r#"{"data":{"data":"{\"content\":\"hi\",\"extra\":[\"a\",1]}"},"type":"markdown"}"#,
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

    #[test]
    fn ark_fields_param_json_serializes_as_object() {
        let segment = Segment::new("ark")
            .param("ark_type", "tuwen")
            .param("ark_name", "")
            .param("prompt", "")
            .param_json("fields", r#"{"tag":"t","title":"T"}"#);
        assert_eq!(
            serde_json::to_string(&segment).expect("序列化失败"),
            r#"{"data":{"ark_name":"","ark_type":"tuwen","fields":{"tag":"t","title":"T"},"prompt":""},"type":"ark"}"#
        );

        let empty = Segment::new("ark").param_json("fields", "{}");
        assert_eq!(
            serde_json::to_string(&empty).expect("序列化失败"),
            r#"{"data":{"fields":{}},"type":"ark"}"#,
            "空 fields 出对象 {{}} 而非字符串"
        );
    }

    #[test]
    fn ark_fields_json_round_trip_keeps_object() {
        let json = r#"{"type":"ark","data":{"ark_type":"tuwen","ark_name":"","prompt":"","fields":{"title":"T","n":1}}}"#;
        let segment: Segment = serde_json::from_str(json).expect("反序列化失败");
        assert_eq!(
            segment.get("fields"),
            Some(r#"{"n":1,"title":"T"}"#),
            "存储仍是紧凑 JSON 文本（键序同 Go json.Marshal）"
        );
        let back = serde_json::to_string(&segment).expect("序列化失败");
        assert_eq!(
            back,
            r#"{"data":{"ark_name":"","ark_type":"tuwen","fields":{"n":1,"title":"T"},"prompt":""},"type":"ark"}"#,
            "fields 对象进 → 对象出"
        );
        assert_eq!(
            segment,
            serde_json::from_str::<Segment>(&back).expect("往返反序列化失败")
        );
    }

    #[test]
    fn param_json_falls_back_to_string_on_invalid_json() {
        let segment = Segment::new("ark").param_json("fields", "not json");
        assert_eq!(
            serde_json::to_string(&segment).expect("序列化失败"),
            r#"{"data":{"fields":"not json"},"type":"ark"}"#,
            "解析失败回退字符串，不丢内容"
        );
    }

    #[test]
    fn only_ark_fields_is_marked_on_deserialize() {
        // P52 只标记 ark 的 fields：ark 的其它键与其它段的嵌套值仍按 map[string]string 字符串化
        let segment: Segment = serde_json::from_str(
            r#"{"type":"ark","data":{"fields":{"title":"T"},"extra":{"a":1}}}"#,
        )
        .expect("反序列化失败");
        assert_eq!(
            serde_json::to_string(&segment).expect("序列化失败"),
            r#"{"data":{"extra":"{\"a\":1}","fields":{"title":"T"}},"type":"ark"}"#
        );
    }

    #[test]
    fn plain_segments_keep_string_params() {
        // 回归锁（P52）：未标记段的参数值恒为字符串
        for (segment, expected) in [
            (
                Segment::new("at").param("qq", "7"),
                r#"{"data":{"qq":"7"},"type":"at"}"#,
            ),
            (
                Segment::text("hi"),
                r#"{"data":{"text":"hi"},"type":"text"}"#,
            ),
            (
                Segment::new("image")
                    .param("file", "a.image")
                    .param("subType", "0"),
                r#"{"data":{"file":"a.image","subType":"0"},"type":"image"}"#,
            ),
        ] {
            assert_eq!(
                serde_json::to_string(&segment).expect("序列化失败"),
                expected
            );
        }
    }

    #[test]
    fn json_key_order_matches_go_marshal() {
        // P53：Go json.Marshal 对 map 按键字母排序——外层 data < type，内层键也排序。
        // 刻意按非字母插入序构造：JSON 排序输出，Vec 序不变（CQ 路径不受影响）
        let ark = Segment::new("ark")
            .param("ark_type", "tuwen")
            .param("prompt", "点我")
            .param_json("fields", r#"{"title":"T","tag":"tuwen"}"#)
            .param("ark_name", "图文 H5");
        assert_eq!(
            serde_json::to_string(&ark).expect("序列化失败"),
            r#"{"data":{"ark_name":"图文 H5","ark_type":"tuwen","fields":{"tag":"tuwen","title":"T"},"prompt":"点我"},"type":"ark"}"#,
            "ark 四键字母序，fields 保持真对象（内层键也排序）"
        );

        let record = Segment::new("record")
            .param("url", "https://x/a.silk")
            .param("file", "https://x/a.wav")
            .param("asr_refer_text", "你好");
        assert_eq!(
            serde_json::to_string(&record).expect("序列化失败"),
            r#"{"data":{"asr_refer_text":"你好","file":"https://x/a.wav","url":"https://x/a.silk"},"type":"record"}"#
        );

        let image = Segment::new("image")
            .param("url", "https://y/b.png")
            .param("subType", "0")
            .param("file", "https://x/a.png");
        assert_eq!(
            serde_json::to_string(&image).expect("序列化失败"),
            r#"{"data":{"file":"https://x/a.png","subType":"0","url":"https://y/b.png"},"type":"image"}"#
        );

        let dup = Segment::new("x")
            .param("b", "1")
            .param("a", "2")
            .param("b", "3");
        assert_eq!(
            serde_json::to_string(&dup).expect("序列化失败"),
            r#"{"data":{"a":"2","b":"1","b":"3"},"type":"x"}"#,
            "稳定排序：重复键保持插入序（现状行为）"
        );
    }

    #[test]
    fn segment_eq_is_param_order_independent() {
        // P54：P53 排序序列化后，往返两次 parse 的 params Vec 序不同，
        // 判等须按 map 语义忽略顺序；重复键按多重集比较（重数须一致）
        let a = Segment::new("ark")
            .param("ark_type", "tuwen")
            .param("prompt", "p")
            .param("ark_name", "");
        let b = Segment::new("ark")
            .param("ark_name", "")
            .param("ark_type", "tuwen")
            .param("prompt", "p");
        assert_eq!(a, b, "键值集合相同、顺序不同必须判等");
        assert_ne!(
            a,
            Segment::new("ark2")
                .param("ark_name", "")
                .param("ark_type", "tuwen")
                .param("prompt", "p"),
            "kind 仍须参与比较"
        );

        let dup = Segment::new("x").param("b", "1").param("b", "3");
        assert_eq!(dup, Segment::new("x").param("b", "3").param("b", "1"));
        assert_ne!(dup, Segment::new("x").param("b", "3"), "重复键重数须一致");
    }

    #[test]
    fn ark_fields_array_shape_is_not_marked_as_object() {
        // P53：Go fields 是 map 契约——数组形态不标 raw JSON，输出保持字符串化
        let segment: Segment =
            serde_json::from_str(r#"{"type":"ark","data":{"fields":["a",1]}}"#)
                .expect("反序列化失败");
        assert_eq!(
            serde_json::to_string(&segment).expect("序列化失败"),
            r#"{"data":{"fields":"[\"a\",1]"},"type":"ark"}"#
        );
    }
}
