//! CQ 码：转义、反转义、分词解析、生成与码表。
//!
//! 真源是 `handlers/cqcode.go`（出站消费口径与正则）与
//! `internal/domain/message/parse_string.go`（解析骨架），`handlers/message_parser.go`
//! 的 `foundItems` 决定哪些码会落到出站计划里。
//!
//! # 转义口径（M0 已钉死）
//! 出站只转 `,` `]` `&`（[`escape`]），**不转 `[`**：OneBot v11 规范要求转 `[`，但
//! `handlers/cqcode.go` 不转；为保住与 Go 输出字节一致，这里跟随 Go。入站 [`unescape`]
//! 仍接受 `&#91;`，因此对端按规范转义的内容也能还原。
//! 注意 `internal/domain/message/cq.go` 的 `EscapeCQ` 是转 `[` 的版本（新架构内部自洽），
//! 两处原本就不一致；协议层对外只保留 Go 出站口径一种。
//!
//! # 解析规则
//! - `[CQ:` 之前的普通文本不做反转义，原样成为 `text` 段（与 `parse_string.go` 一致）。
//! - 码体以**第一个** `]` 结束；没有 `]` 时剩余部分整体按文本处理。
//! - 参数按 `,` 切分，但 `&#44;` 视为值的一部分；无 `=` 的片段再按首个 `:` 切一次
//!   （`[CQ:stream,type:start,qq:123]`），两边都没有的裸片段丢弃。
//! - 与 `cq_parse_mode:new`（`handlers/cqparse/tokenize.go`）有两处口径差异：那边**只**对
//!   `stream` 认 `:`（`splitKV`），且无分隔符的裸片段会追加到前一参数的值上
//!   （`params[lastKey] + "," + t`）；本实现按 legacy 口径丢弃裸片段（`handlers/cqcode.go`
//!   的 `SplitN(part,"=",2)` 只认 `=`，stream 另由专用正则认 `:`），只把 `:` 放宽成通用兜底。
//! - 未知码类型原样保留为消息段（`[CQ:cardboard,xx=1]` 往返不变）。
//!
//! 与 `handlers/cqcode.go` 的 markdown 正则（贪婪 `\{.*\}`）相比，本实现对载荷里未转义的
//! `]` 不做特殊照顾——生成侧会把 `]` 转成 `&#93;`，因此往返仍无损。

use std::fmt;

use gensokyo_core::{Error, Result};

use crate::segment::Segment;

/// 转义 CQ 码参数值：`,` → `&#44;`、`]` → `&#93;`、`&` → `&amp;`。
///
/// **刻意不转 `[`**：见模块文档。base64 中的 `+` `/` `=` 同样不转义。
#[must_use]
pub fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            ']' => out.push_str("&#93;"),
            ',' => out.push_str("&#44;"),
            _ => out.push(ch),
        }
    }
    out
}

/// CQ 码起始标记。
const CODE_PREFIX: &[u8] = b"[CQ:";

/// CQ 实体（`&amp;` / `&#91;` / `&#93;` / `&#44;`）的统一长度。
const ENTITY_LEN: usize = 5;

/// 字宽单字节查找（`memchr` 语义）：返回 `haystack[from..]` 里第一个 `byte` 的绝对下标。
///
/// 整字比较 SWAR：一个 `usize` 一次排掉 8 个字节，只有命中字才逐字节定位，因此不依赖端序
/// （掩码里的"最低命中字节"在小端下才对应最低地址）。CQ 文本里 `[` `]` `,` 稀疏，
/// 逐字节比较会白白多跑几倍。
fn memchr_byte(haystack: &[u8], from: usize, byte: u8) -> Option<usize> {
    const LO: usize = usize::MAX / 0xFF;
    const HI: usize = LO << 7;
    const WIDTH: usize = std::mem::size_of::<usize>();

    let wide = LO * usize::from(byte);
    let mut at = from;
    while at + WIDTH <= haystack.len() {
        let mut word = [0_u8; WIDTH];
        word.copy_from_slice(&haystack[at..at + WIDTH]);
        let diff = usize::from_ne_bytes(word) ^ wide;
        // 字节为 0 时该字节高位置 1：借位减 + `!diff` 掩掉跨字节借位
        let hit = diff.wrapping_sub(LO) & !diff & HI;
        if hit != 0 {
            return haystack[at..at + WIDTH]
                .iter()
                .position(|&b| b == byte)
                .map(|idx| at + idx);
        }
        at += WIDTH;
    }
    haystack[at..]
        .iter()
        .position(|&b| b == byte)
        .map(|idx| at + idx)
}

/// 识别 `bytes[at..]` 处的 CQ 实体，返回替换文本；未知实体或长度不足返回 `None`。
fn entity_at(bytes: &[u8], at: usize) -> Option<&'static str> {
    if at + ENTITY_LEN > bytes.len() {
        return None;
    }
    let tail = &bytes[at..at + ENTITY_LEN];
    if tail.starts_with(b"&amp;") {
        Some("&")
    } else if tail.starts_with(b"&#91;") {
        Some("[")
    } else if tail.starts_with(b"&#93;") {
        Some("]")
    } else if tail.starts_with(b"&#44;") {
        Some(",")
    } else {
        None
    }
}

/// 反转义 CQ 码参数值。
///
/// 与 Go `handlers/cqcode.go` 的 `ReplaceAll` 容错语义一致：只还原已知实体，**未知 `&`
/// 实体按字面保留**（裸 `&` 不报错、不吞字符）；顺序不可颠倒——先还原实体，再还原 `&amp;`。
/// 比 Go 多还原 `&#91;`，用于兼容对端按规范转义的 `[`（出站 [`escape`] 不产出它）。
#[must_use]
pub fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    push_unescaped(&mut out, value);
    out
}

/// 单趟反转义，结果追加到 `out`。
///
/// 四个实体等长、互不为前缀，且替换产物都是单字符（拼不出新实体），所以逐位置匹配与 Go
/// 的四次 `ReplaceAll` 结果逐字节一致，却只需一次遍历、一次分配（旧实现每次 `replace`
/// 都新分配一个 `String`，无实体的值也要付四次）。
fn push_unescaped(out: &mut String, value: &str) {
    let bytes = value.as_bytes();
    let mut copied = 0;
    let mut search = 0;
    while let Some(at) = memchr_byte(bytes, search, b'&') {
        let Some(replacement) = entity_at(bytes, at) else {
            search = at + 1;
            continue;
        };
        out.push_str(&value[copied..at]);
        out.push_str(replacement);
        copied = at + ENTITY_LEN;
        search = copied;
    }
    out.push_str(&value[copied..]);
}

/// 严格反转义：未知 `&` 实体返回 [`Error::Protocol`]。
///
/// 内部校验入口（不属于对外 API 承诺）：运行路径必须用宽容的 [`unescape`]，以免用户文本
/// 里的 `&` 被当成协议错误。
#[doc(hidden)]
pub fn unescape_strict(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(value.len());
    let mut copied = 0;
    let mut search = 0;
    while let Some(at) = memchr_byte(bytes, search, b'&') {
        let Some(replacement) = entity_at(bytes, at) else {
            let preview: String = value[at..].chars().take(5).collect();
            return Err(Error::Protocol(format!("非法 CQ 实体: {preview}")));
        };
        out.push_str(&value[copied..at]);
        out.push_str(replacement);
        copied = at + ENTITY_LEN;
        search = copied;
    }
    out.push_str(&value[copied..]);
    Ok(out)
}

/// 解析一段消息文本为消息段序列。
///
/// 非 CQ 部分成为 `text` 段；相邻文本不会重复分段（与 `parse_string.go` 的 flush 语义一致）。
///
/// 实现要点：整趟只做字宽查找（`[CQ:` 起点与结束括号各扫一次），`copied` 游标标记"尚未
/// 成为段"的文本起点。码段类型非法时不推进 `copied`，因此待落的文本永远是一段连续切片
/// （`input[copied..start]`），可以直接原地落成 `Segment`，中间不留临时 `String`。
#[must_use]
pub fn parse(input: &str) -> Vec<Segment> {
    let bytes = input.as_bytes();
    // 段数上界：码段数 ≤ `[` 个数，文本段数 ≤ 码段数 + 1
    let mut segments = Vec::with_capacity(count_byte(bytes, b'[') * 2 + 1);
    let mut copied = 0;
    let mut cursor = 0;

    while let Some(start) = find_code_start(bytes, cursor) {
        let body_start = start + CODE_PREFIX.len();
        let Some(end) = memchr_byte(bytes, body_start, b']') else {
            // 无结束括号：剩余全部按文本处理（Go `parse_string.go` 的 malformed 分支）
            break;
        };
        // 类型非法（含空类型）时整段当文本：不推进 `copied`，收尾的文本段会一并带上，
        // 保证输出可逆
        if let Some(segment) = parse_body(&input[body_start..end]) {
            if start > copied {
                segments.push(Segment::text(&input[copied..start]));
            }
            segments.push(segment);
            copied = end + 1;
        }
        cursor = end + 1;
    }
    if copied < input.len() {
        segments.push(Segment::text(&input[copied..]));
    }
    segments
}

/// 查找下一个 `[CQ:` 的起始下标。
fn find_code_start(bytes: &[u8], from: usize) -> Option<usize> {
    let mut at = from;
    loop {
        let found = memchr_byte(bytes, at, b'[')?;
        if bytes[found..].starts_with(CODE_PREFIX) {
            return Some(found);
        }
        at = found + 1;
    }
}

/// 统计 `byte` 在 `bytes` 里出现的次数。
///
/// 用于估段数上界：逐字节 `filter().count()` 要付一次均匀扫描，这里同一个字宽查找循环
/// 只在命中处停一下，稀疏时几乎只按字宽走完全程。
fn count_byte(bytes: &[u8], byte: u8) -> usize {
    let mut count = 0;
    let mut at = 0;
    while let Some(found) = memchr_byte(bytes, at, byte) {
        count += 1;
        at = found + 1;
    }
    count
}

/// 解析单个完整 CQ 码（`[CQ:type,...]`）。
///
/// # Errors
/// 缺少 `[CQ:` / `]` 包裹，或类型不是小写字母/数字/下划线时返回 [`Error::Protocol`]。
pub fn parse_one(code: &str) -> Result<Segment> {
    let trimmed = code.trim();
    let body = trimmed
        .strip_prefix("[CQ:")
        .and_then(|rest| rest.strip_suffix(']'))
        .ok_or_else(|| Error::Protocol(format!("不是完整的 CQ 码: {code}")))?;
    parse_body(body).ok_or_else(|| Error::Protocol(format!("CQ 码类型非法: {code}")))
}

/// 生成单个消息段的 CQ 文本。
#[must_use]
pub fn generate(segment: &Segment) -> String {
    segment.to_cq()
}

/// 把消息段序列拼成 CQ 文本（文本段原样拼接）。
#[must_use]
pub fn to_cq_string(segments: &[Segment]) -> String {
    let mut out = String::new();
    for segment in segments {
        out.push_str(&segment.to_cq());
    }
    out
}

fn parse_body(body: &str) -> Option<Segment> {
    let (kind, params_part) = match memchr_byte(body.as_bytes(), 0, b',') {
        Some(idx) => (&body[..idx], &body[idx + 1..]),
        None => (body, ""),
    };
    // Go 正则是 `[a-z_]+`；这里放宽到数字，仍拒绝空类型与大写（大写会与 `[CQ:` 前缀判定打架）
    if kind.is_empty()
        || !kind
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return None;
    }
    let mut params = Vec::new();
    scan_params(&mut params, params_part);
    Some(Segment {
        kind: kind.to_owned(),
        params,
    })
}

/// 参数片段 → `(key, 未反转义的原值)`。
///
/// 先按首个 `=` 切；没有 `=` 时按首个 `:` 切（`[CQ:stream,type:start,qq:123]`，
/// `handlers/cqcode.go:41` 的 stream 正则就是这个写法）。两者都没有的裸片段丢弃。
fn split_kv(chunk: &str) -> Option<(&str, &str)> {
    if let Some(idx) = memchr_byte(chunk.as_bytes(), 0, b'=') {
        return Some((&chunk[..idx], &chunk[idx + 1..]));
    }
    let idx = memchr_byte(chunk.as_bytes(), 0, b':')?;
    // 空 key 丢弃；无 `=` 的 URL 型裸值（`https://x`）会被切成 `https` + `//x`，
    // 这是"无分隔符"本身有歧义的代价——CQ 参数键不含 `:`，值必须显式带分隔符。
    if idx == 0 {
        return None;
    }
    Some((&chunk[..idx], &chunk[idx + 1..]))
}

/// 按 `,` 切分参数，并直接落成 `(key, 反转义后的值)`。
///
/// 分隔符是裸 `,`；`&#44;` 里没有裸 `,`（实体本身已是转义形态），所以直接按字节切分与
/// Go `splitCQParams` 逐字节一致，不需要为实体单独跳过。旧实现先收集 `Vec<&str>` 再逐个
/// 处理，这里一趟切分直接落成参数。
fn scan_params(params: &mut Vec<(String, String)>, input: &str) {
    let bytes = input.as_bytes();
    let mut start = 0;
    while let Some(at) = memchr_byte(bytes, start, b',') {
        push_param(params, &input[start..at]);
        start = at + 1;
    }
    push_param(params, &input[start..]);
}

/// 单个参数片段落成参数；无 `=` 也无 `:` 的裸片段丢弃（legacy 口径）。
fn push_param(params: &mut Vec<(String, String)>, chunk: &str) {
    let Some((key, value)) = split_kv(chunk) else {
        return;
    };
    params.push((key.to_owned(), unescape(value)));
}

/// 一个 CQ 码：`[CQ:type,key=value,...]`。
///
/// `params` 存**已转义**的值（与 [`escape`] 配套），因此 `Display` 直接拼接即可；
/// 需要未转义的模型表示时用 [`Segment`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CqCode {
    /// 码类型（`at` / `image` …）。
    pub kind: String,
    /// 参数，保持插入顺序（与 Go 输出一致）。
    pub params: Vec<(String, String)>,
}

impl CqCode {
    /// 新建一个只有 type 的 CQ 码。
    #[must_use]
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            params: Vec::new(),
        }
    }

    /// 追加参数；值做实体转义。
    #[must_use]
    pub fn param(mut self, key: impl Into<String>, value: impl AsRef<str>) -> Self {
        self.params.push((key.into(), escape(value.as_ref())));
        self
    }

    /// 追加已转义好的参数（base64 等无需二次转义的场景）。
    #[must_use]
    pub fn raw_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.push((key.into(), value.into()));
        self
    }

    /// 由消息段构造（文本段没有 CQ 形态，返回 `None`）。
    #[must_use]
    pub fn from_segment(segment: &Segment) -> Option<Self> {
        if segment.is_text() {
            return None;
        }
        Some(Self {
            kind: segment.kind.clone(),
            params: segment
                .params
                .iter()
                .map(|(key, value)| (key.clone(), escape(value)))
                .collect(),
        })
    }

    /// 还原为未转义的消息段。
    #[must_use]
    pub fn into_segment(self) -> Segment {
        Segment {
            kind: self.kind,
            params: self
                .params
                .into_iter()
                .map(|(key, value)| (key, unescape(&value)))
                .collect(),
        }
    }
}

impl fmt::Display for CqCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[CQ:{}", self.kind)?;
        for (key, value) in &self.params {
            write!(f, ",{key}={value}")?;
        }
        f.write_str("]")
    }
}

/// 一个 CQ 码的静态说明（码表条目）。
///
/// `params` 是 Gensokyo 实际使用的参数顺序；`inbound` / `outbound` 标记该码在
/// `Processor/`（入站上报）与 `handlers/`（出站解析）里是否有消费点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeSpec {
    /// 码类型。
    pub name: &'static str,
    /// 参数名（顺序即 Go 生成顺序）。
    pub params: &'static [&'static str],
    /// 入站上报会产出该码。
    pub inbound: bool,
    /// 出站解析会消费该码。
    pub outbound: bool,
}

/// CQ 码表（M1f 覆盖范围，按 `handlers/cqcode.go` + `handlers/message_parser.go` 汇总）。
pub const CODES: &[CodeSpec] = &[
    CodeSpec {
        name: "text",
        params: &["text"],
        inbound: true,
        outbound: true,
    },
    CodeSpec {
        name: "at",
        params: &["qq"],
        inbound: true,
        outbound: true,
    },
    CodeSpec {
        name: "reply",
        params: &["id", "msg_idx"],
        inbound: true,
        outbound: true,
    },
    CodeSpec {
        name: "image",
        params: &["file", "subType", "url"],
        inbound: true,
        outbound: true,
    },
    CodeSpec {
        name: "record",
        params: &["file", "asr"],
        inbound: true,
        outbound: true,
    },
    CodeSpec {
        name: "video",
        params: &["file"],
        inbound: true,
        outbound: true,
    },
    CodeSpec {
        name: "file",
        params: &["file", "file_name", "name"],
        inbound: true,
        outbound: true,
    },
    CodeSpec {
        name: "face",
        params: &["id"],
        inbound: true,
        outbound: true,
    },
    CodeSpec {
        name: "markdown",
        params: &["data"],
        inbound: true,
        outbound: true,
    },
    CodeSpec {
        // 出站 `[CQ:card,title=…,desc=…,pic=…,url=…]`（handlers/cqcode_pipeline.go:114）；
        // `data=` 是段形态的 JSON 串写法（handlers/message_parser.go:988）
        name: "card",
        params: &["title", "desc", "pic", "url", "data"],
        inbound: true,
        outbound: true,
    },
    CodeSpec {
        name: "keyboard",
        params: &["data"],
        inbound: true,
        outbound: true,
    },
    CodeSpec {
        name: "stream",
        params: &["type", "qq"],
        inbound: true,
        outbound: true,
    },
    CodeSpec {
        name: "input_notify",
        params: &["type", "second"],
        inbound: true,
        outbound: true,
    },
    CodeSpec {
        // foundItems 的 key 叫 qqmusic，CQ 类型沿用 OneBot 的 music,type=qq
        name: "music",
        params: &["type", "id"],
        inbound: true,
        outbound: true,
    },
    CodeSpec {
        name: "active",
        params: &["type", "sub_type"],
        inbound: true,
        outbound: true,
    },
    CodeSpec {
        name: "wakeup",
        params: &["userid"],
        inbound: true,
        outbound: true,
    },
    CodeSpec {
        name: "node",
        params: &["id", "user_id", "nickname", "content"],
        inbound: true,
        outbound: false,
    },
    CodeSpec {
        name: "set_group",
        params: &[
            "action",
            "group_id",
            "user_id",
            "user_ids",
            "duration",
            "enable",
            "flag",
            "approve",
            "reason",
            "strategy_id",
            "execute",
            "add_blacklist",
            "add_to_member_blacklist",
        ],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        name: "remove",
        params: &["user_id", "msg_id"],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        name: "poke",
        params: &["qq"],
        inbound: true,
        outbound: true,
    },
    CodeSpec {
        name: "member",
        params: &["type", "group_id", "user_id"],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        name: "group_info",
        params: &["field", "group_id", "fallback"],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        name: "avatar",
        params: &["qq"],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        name: "voice",
        params: &["file"],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        // 入站上报会产出 ark 卡片，出站无消费点（ProcessCQInboundOnlyCodes 会剥离）
        name: "ark",
        params: &["data"],
        inbound: true,
        outbound: false,
    },
    CodeSpec {
        name: "json",
        params: &["data"],
        inbound: true,
        outbound: false,
    },
];

/// 码类型别名：`qqmusic` 是 `foundItems` 的 key 名，线上 CQ 类型是 `music,type=qq`。
pub const CODE_ALIASES: &[(&str, &str)] = &[("qqmusic", "music")];

/// 按类型查码表（支持 [`CODE_ALIASES`]）。
#[must_use]
pub fn code_spec(kind: &str) -> Option<&'static CodeSpec> {
    let canonical = CODE_ALIASES
        .iter()
        .find(|(alias, _)| *alias == kind)
        .map_or(kind, |(_, target)| *target);
    CODES.iter().find(|spec| spec.name == canonical)
}

#[cfg(test)]
mod tests {
    use super::{
        CODES, CqCode, code_spec, escape, parse, parse_one, to_cq_string, unescape, unescape_strict,
    };
    use crate::segment::Segment;

    #[test]
    fn bare_ampersand_is_kept_verbatim() {
        // Go: ReplaceAll 容错，未知实体不报错
        assert_eq!(unescape("a&b"), "a&b");
        assert_eq!(unescape("100% & more"), "100% & more");
        assert_eq!(unescape("&unknown;"), "&unknown;");
    }

    #[test]
    fn known_entities_are_restored_in_go_order() {
        assert_eq!(unescape("&#44;"), ",");
        assert_eq!(unescape("&#93;"), "]");
        assert_eq!(unescape("&#91;"), "[");
        assert_eq!(unescape("&amp;"), "&");
        // 顺序：先实体后 &amp;，避免 amp; 二次解析
        assert_eq!(unescape("a&amp;&#44;b"), "a&,b");
    }

    #[test]
    fn escape_follows_go_and_leaves_brackets_alone() {
        // Go 只转 , ] &；不转 [（OneBot 规范要求转，本项目刻意跟随 Go）
        assert_eq!(escape("[CQ:at]"), "[CQ:at&#93;");
        assert_eq!(escape("a,b]c&d"), "a&#44;b&#93;c&amp;d");
        assert_eq!(escape("[unclosed"), "[unclosed");
    }

    #[test]
    fn unescape_is_order_sensitive_like_go() {
        // &amp; 最后还原：先还原 &#44; &#93; &#91;，避免把 &amp; 拆出的 & 再当实体解析
        assert_eq!(unescape("&amp;,&#91;,&#93;,&#44;"), "&,[,],,");
        // 顺序保证：`&amp;#91;` 只还原成字面 `&#91;`，不会被二次解码成 `[`
        assert_eq!(unescape("&amp;#91;"), "&#91;");
    }

    #[test]
    fn escape_unescape_round_trip() {
        let raw = "a&b,[c],d";
        assert_eq!(unescape(&escape(raw)), raw);
    }

    #[test]
    fn strict_variant_still_reports_unknown_entities() {
        assert!(unescape_strict("a&b").is_err());
        assert_eq!(unescape_strict("a&amp;b").expect("应为合法实体"), "a&b");
    }

    #[test]
    fn cq_code_display_escapes_params() {
        let code = CqCode::new("at").param("qq", "10001").param("name", "a,b");
        assert_eq!(code.to_string(), "[CQ:at,qq=10001,name=a&#44;b]");
    }

    #[test]
    fn parse_splits_text_and_codes_in_order() {
        let segments = parse("你好[CQ:at,qq=123]看图[CQ:image,file=http://x.com/a.png]");
        assert_eq!(
            segments,
            vec![
                Segment::text("你好"),
                Segment::new("at").param("qq", "123"),
                Segment::text("看图"),
                Segment::new("image").param("file", "http://x.com/a.png"),
            ]
        );
        assert_eq!(
            to_cq_string(&segments),
            "你好[CQ:at,qq=123]看图[CQ:image,file=http://x.com/a.png]"
        );
    }

    #[test]
    fn params_keep_wire_order_and_unescape_values() {
        let segment = parse_one("[CQ:file,file=https://x.com/a&#44;b.txt,file_name=a&#44;b.txt]")
            .expect("解析失败");
        assert_eq!(
            segment.params,
            vec![
                ("file".to_owned(), "https://x.com/a,b.txt".to_owned()),
                ("file_name".to_owned(), "a,b.txt".to_owned()),
            ]
        );
        assert_eq!(
            segment.to_cq(),
            "[CQ:file,file=https://x.com/a&#44;b.txt,file_name=a&#44;b.txt]"
        );
    }

    #[test]
    fn unescaped_comma_in_value_is_split_like_go() {
        // Go `splitCQParams`：`b.txt` 没有 `=`，被跳过；file 只到第一个裸逗号
        let segment = parse_one("[CQ:file,file=https://x.com/a,b.txt]").expect("解析失败");
        assert_eq!(segment.get("file"), Some("https://x.com/a"));
        assert_eq!(segment.params.len(), 1);
    }

    #[test]
    fn bare_ampersand_and_bracket_survive_round_trip() {
        for raw in [
            "[CQ:at,qq=1,name=a[b]",
            "[CQ:at,qq=1,name=a&#93;b]",
            "[CQ:at,qq=1,name=a&#44;b]",
        ] {
            let segments = parse(raw);
            let round = to_cq_string(&segments);
            assert_eq!(round, raw, "往返应逐字节一致: {raw}");
        }
        // 裸 `&` 解析时按字面保留，生成时按 Go 口径规范化为 `&amp;`；再生成应幂等
        let once = to_cq_string(&parse("[CQ:at,qq=1,name=a&b]"));
        assert_eq!(once, "[CQ:at,qq=1,name=a&amp;b]");
        assert_eq!(to_cq_string(&parse(&once)), once, "生成应幂等");
        assert_eq!(parse(&once), parse("[CQ:at,qq=1,name=a&b]"), "语义往返一致");
        // 裸逗号是参数分隔符（Go 语义），`b` 无 `=` 被丢弃，往返不再逐字节相等
        assert_eq!(
            to_cq_string(&parse("[CQ:at,qq=1,name=a,b]")),
            "[CQ:at,qq=1,name=a]"
        );
    }

    #[test]
    fn malformed_code_stays_text() {
        // 无结束括号：Go 的 malformed 分支
        let segments = parse("[CQ:markdown,data={\"a\":");
        assert_eq!(segments, vec![Segment::text("[CQ:markdown,data={\"a\":")]);
        // 空类型
        assert_eq!(parse("[CQ:,x=1]"), vec![Segment::text("[CQ:,x=1]")]);
        // 大写类型
        assert_eq!(parse("[cq:at,qq=1]"), vec![Segment::text("[cq:at,qq=1]")]);
    }

    #[test]
    fn colon_params_parse_for_stream_and_generic_segments() {
        // Go `streamPattern` 就是 `type:xxx,qq:123`（handlers/cqcode.go:41）
        let segments = parse("[CQ:stream,type:start,qq:123]");
        assert_eq!(
            segments,
            vec![
                Segment::new("stream")
                    .param("type", "start")
                    .param("qq", "123")
            ]
        );
        assert_eq!(to_cq_string(&segments), "[CQ:stream,type=start,qq=123]");

        // 混合写法：`=` 优先，`:` 兜底
        assert_eq!(
            parse("[CQ:stream,type:start,qq=123]"),
            vec![
                Segment::new("stream")
                    .param("type", "start")
                    .param("qq", "123")
            ]
        );

        // 非 stream 的无 `=` 片段同样按首个 `:` 切
        assert_eq!(
            parse("[CQ:custom,action:run]"),
            vec![Segment::new("custom").param("action", "run")]
        );

        // 裸片段（既无 `=` 也无 `:`）按 legacy 口径丢弃
        assert_eq!(
            parse("[CQ:file,file=a.txt,b.txt]"),
            vec![Segment::new("file").param("file", "a.txt")]
        );

        assert_eq!(
            parse_one("[CQ:stream,type:finish,qq:9]")
                .expect("解析失败")
                .get("type"),
            Some("finish")
        );
    }

    #[test]
    fn unknown_code_is_preserved() {
        let segments = parse("[CQ:cardboard,xx=1]");
        assert_eq!(segments, vec![Segment::new("cardboard").param("xx", "1")]);
        assert_eq!(to_cq_string(&segments), "[CQ:cardboard,xx=1]");
    }

    #[test]
    fn commandless_code_parses_with_no_params() {
        assert_eq!(parse("[CQ:at]"), vec![Segment::new("at")]);
        assert_eq!(to_cq_string(&parse("[CQ:at]")), "[CQ:at]");
    }

    #[test]
    fn parse_one_rejects_broken_input() {
        assert!(parse_one("[CQ:at,qq=1").is_err());
        assert!(parse_one("at,qq=1]").is_err());
        assert!(parse_one("[CQ:,x=1]").is_err());
    }

    #[test]
    fn full_code_table_round_trips() {
        for spec in CODES {
            if spec.name == "text" {
                continue; // 文本段没有 CQ 形态，单独覆盖
            }
            let mut segment = Segment::new(spec.name);
            for (index, param) in spec.params.iter().enumerate() {
                segment = segment.param(*param, format!("v{index}"));
            }
            let wire = to_cq_string(std::slice::from_ref(&segment));
            let back = parse(&wire);
            assert_eq!(back, vec![segment], "码表往返失败: {}", spec.name);
        }
    }

    #[test]
    fn code_table_covers_required_types() {
        for name in [
            "at",
            "reply",
            "image",
            "record",
            "video",
            "file",
            "face",
            "markdown",
            "card",
            "keyboard",
            "stream",
            "input_notify",
            "music",
            "active",
            "wakeup",
            "node",
            "set_group",
            "remove",
            "poke",
        ] {
            assert!(code_spec(name).is_some(), "码表缺少 {name}");
        }
        assert!(code_spec("music").expect("music").params.contains(&"id"));
        // foundItems 的 key 叫 qqmusic，码表按线上类型 music 收录，别名可查
        assert_eq!(code_spec("qqmusic").expect("qqmusic 别名").name, "music");
        assert!(!code_spec("ark").expect("ark").outbound);
        assert!(code_spec("set_group").expect("set_group").outbound);
    }

    #[test]
    fn cq_code_from_segment_escapes_on_construction() {
        let segment = Segment::new("at").param("name", "a,b");
        let code = CqCode::from_segment(&segment).expect("非文本段应有 CQ 形态");
        assert_eq!(code.to_string(), "[CQ:at,name=a&#44;b]");
        assert_eq!(code.into_segment(), segment);
        assert!(CqCode::from_segment(&Segment::text("x")).is_none());
    }
}

/// 解析热路径计时（只打印、**不断言耗时**：慢机器或 debug 构建不该让测试变红）。
///
/// 数字要跟 `xtask bench` 对比时用同一个 profile 跑：
/// `cargo test -p gensokyo-protocol --release -- --nocapture parse_`。
#[cfg(test)]
mod perf {
    use std::hint::black_box;
    use std::time::Instant;

    use super::parse;

    /// 计时迭代次数：debug 构建下 2000 次也只有几十毫秒。
    const ITERS: u32 = 2_000;

    /// 预热后计时，返回单次操作耗时（纳秒）。
    fn ns_per_op<T>(iters: u32, mut op: impl FnMut() -> T) -> f64 {
        for _ in 0..100 {
            black_box(op());
        }
        let start = Instant::now();
        for _ in 0..iters {
            black_box(op());
        }
        start.elapsed().as_secs_f64() * 1e9 / f64::from(iters)
    }

    /// 20 段混合 CQ（与 `xtask bench` 的 `cq_sample` 同形，便于两处数字对照）。
    fn mixed_sample() -> String {
        const CODES: [&str; 5] = [
            "[CQ:at,qq=10001]",
            "[CQ:image,file=8f3c.png,url=https://multimedia.nt.qq.com.cn/download?appid=1407]",
            "[CQ:face,id=14]",
            "[CQ:reply,id=1234567890123]",
            "[CQ:record,file=abc.silk]",
        ];
        let mut text = String::new();
        for index in 0..20 {
            text.push('第');
            text.push_str(&index.to_string());
            text.push_str("段 ");
            text.push_str(CODES[index % CODES.len()]);
        }
        text
    }

    #[test]
    fn parse_mixed_20_segments_timing() {
        let text = mixed_sample();
        let segments = parse(&text).len();
        let ns = ns_per_op(ITERS, || black_box(parse(black_box(text.as_str())).len()));
        println!("[perf] parse 20-segment mixed: {ns:.1} ns/op ({segments} segments)");
    }

    #[test]
    fn parse_plain_text_timing() {
        // 无 CQ 码：应当只有一次文本段分配，是整趟扫描的下限
        let text = "纯文本消息，没有 CQ 码，只有中文和 ASCII mixed content。".repeat(8);
        let segments = parse(&text).len();
        let ns = ns_per_op(ITERS, || black_box(parse(black_box(text.as_str())).len()));
        println!(
            "[perf] parse plain text ({} bytes): {ns:.1} ns/op ({segments} segments)",
            text.len()
        );
    }

    #[test]
    fn parse_escaped_values_timing() {
        // 每个值都含实体：走反转义的改写分支（旧实现这里是 4 次 replace/值）
        let text = "[CQ:file,file=a&#44;b.txt&amp;c,file_name=d&#93;e.txt]".repeat(10);
        let segments = parse(&text).len();
        let ns = ns_per_op(ITERS, || black_box(parse(black_box(text.as_str())).len()));
        println!(
            "[perf] parse escaped values ({} bytes): {ns:.1} ns/op ({segments} segments)",
            text.len()
        );
    }
}
