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
//! # 空白契约（两层，各自对偶同源）
//! - **词法切分层**（段名前后到 `,`/`]` 的跳空白、`,` 与 `=` 两侧跳空白、JSON 值起点判定、
//!   `parse_one` 整段外侧）：使用 Go 正则 `\s` 的 ASCII 集合：空格 / 制表 `\t` / 换行 `\n` /
//!   换页 `\f` / 回车 `\r`；**不含**纵向制表 `\v`，不含全角空格（U+3000）等 Unicode 空白。
//!   该集合唯一定义在 [`is_cq_space`]；runtime `at_everyone.rs` 的切分判定（`[CQ:at` 之后到
//!   `,`/`]`、`<qqbot-at-everyone` 到 `/>`）保持同一集合。
//! - **键/值 trim 层**（[`push_param`] 落的 `k=v` 两侧）：使用 Unicode 语义（`str::trim`），
//!   对偶 Go 按键取值路径统一的 `strings.TrimSpace`——`handlers/cqcode.go:85-89`、
//!   `handlers/message_parser.go:3130-3141` 的 `cqCodeParam`、`handlers/cqparse/tokenize.go:157`
//!   的 `buildParams`、`internal/domain/message/cq.go:57-61`。因此
//!   `[CQ:image, file = <全角空格>x]` 两侧都取到 `file=x`；`\v` 与 NBSP 同样被吞。
//!
//! # 解析规则
//! - `[CQ:` 之前的普通文本不做反转义，原样成为 `text` 段（与 `parse_string.go` 一致）。
//!   `[CQ:` 前缀本身**保持大小写敏感**（Go 正则就是字面量 `[CQ:`），放宽的只有段名与参数键。
//! - 码体以**配对**的 `]` 结束：参数值在段首或 `=` 后以 `{` 开头时按花括号配平整段吞掉
//!   （含 JSON 字符串内引号/转义的最小处理），JSON 里的 `,` `]` 不结束段、不收码——
//!   与 `cqparse/tokenize.go` `scanSegments` 的配平口径一致；段名非法、花括号配平失败
//!   或没有闭合 `]` 时整段码按文本处理。
//! - 参数按 `,` 切分，但 `&#44;` 视为值的一部分；`:` 分隔**只对 `stream` 段**生效（冒号语法，
//!   `[CQ:stream,type:start,qq:123]`，对齐 Go `splitKV` 的 `action=="stream"` 判定）；
//!   两者都没有的裸片段并入前一 key 的值、以 `,` 拼接（C3 并集口径，对齐 Go
//!   `cqUnionParams`/`buildParams` 的 `params[lastKey] + "," + t`；无前一键或片段为空则丢弃）。
//! - 参数切分与 `cq_parse_mode:new`（`handlers/cqparse/tokenize.go`）已完全对齐：`:` 仅
//!   `stream` 认、裸片段并入前一键值两处都按并集口径落实现；早期的「按 legacy 口径丢弃裸片段、
//!   `:` 放宽成通用兜底」两处差异已按 Go `cqUnionParams`/`buildParams` 修正收口。
//! - 未知码类型原样保留为消息段（`[CQ:cardboard,xx=1]` 往返不变）。
//! - **段名与参数键大小写不敏感**（与 Go 的最大兼容并集同口径）：`[CQ:Image,File=x]` 识别为
//!   `image` 段，段名与参数键都归一成小写落段（`File` ≡ `file`；Go `buildParams` 就是键小写，
//!   故码表驼峰键 `subType` 在解析/再序列化侧按 `subtype` 走）。段名前后的空白一并跳过。
//!   **值**不参与大小写处理；`escape` 转义口径不变；透传臂（[`Segment::raw_or_cq`]）仍逐字节
//!   吐 `parse` 命中的原文切片，用户字节不被归一改写。
//! - parse 命中的码段携带**整段原文切片**（`Segment::raw`），透传臂用 [`Segment::raw_or_cq`]
//!   逐字节输出；`to_cq` / [`to_cq_string`] 的规范化输出只供真正消费参数后重排的场景。
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
/// 实现要点：`[CQ:` 起点用字宽查找，段尾由 [`find_code_end`] 逐字节做花括号配平扫描
/// （JSON 值内的 `,` `]` 不收码）；`copied` 游标标记"尚未成为段"的文本起点。码段非法
/// （含段名非法、花括号配平失败）时不推进 `copied`，待落的文本永远是一段连续切片，
/// 原样保留。命中码段经 [`Segment::with_raw`] 携带整段原文，透传臂逐字节输出不改写。
#[must_use]
pub fn parse(input: &str) -> Vec<Segment> {
    let bytes = input.as_bytes();
    // 段数上界：码段数 ≤ `[` 个数，文本段数 ≤ 码段数 + 1
    let mut segments = Vec::with_capacity(count_byte(bytes, b'[') * 2 + 1);
    let mut copied = 0;
    let mut cursor = 0;

    while let Some(start) = find_code_start(bytes, cursor) {
        let body_start = start + CODE_PREFIX.len();
        // 先做配平扫描找段尾；失败时退回旧口径「首个裸 `]`」探测候选段尾：
        // 段名非法的码由 `parse_body` 拒掉，花括号配平失败的码由 `parse_body` 的配平检查
        // 拒掉，两者都逐字节留成文本（Go `tryParseCode` 判不成码 → `KindText` 原文），
        // 且游标只走一步（`start + 1`）继续找内嵌的合法码
        let Some(end) =
            find_code_end(input, body_start).or_else(|| memchr_byte(bytes, body_start, b']'))
        else {
            // 无结束括号：剩余全部按文本处理（Go `parse_string.go` 的 malformed 分支）
            break;
        };
        // 类型非法（含空类型）时整段当文本：不推进 `copied`，收尾的文本段会一并带上，
        // 保证输出可逆
        if let Some(segment) = parse_body(&input[body_start..end]) {
            if start > copied {
                segments.push(Segment::text(&input[copied..start]));
            }
            // 透传原文：含 `[CQ:` 与收尾 `]` 的整段字节，透传臂逐字节输出不改写用户输入
            segments.push(segment.with_raw(&input[start..=end]));
            copied = end + 1;
            cursor = end + 1;
        } else {
            // 解析失败（段名非法 / 花括号配平失败）：游标只前进到 `[CQ:` 之后一个字节，
            // 逐字符重扫——Go `Tokenize` 的 `i = start + 1`（`handlers/cqparse/tokenize.go:29`）。
            // 跳过整段（`cursor = end + 1`）会把失败段内后方的合法嵌套码一起吞成文本，
            // 例如 `[CQ:x,data={ [CQ:at,qq=1]` 里的 at 段 Go 能识别、这里也必须能
            cursor = start + 1;
        }
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

/// 为一个码段寻找配平的收尾 `]`（Go `cqparse/tokenize.go` 的 `tryParseCode` + `scanSegments`
/// 同口径：段名前后允许空白，参数里在 `=` 或段首后以 `{` 开头的值做花括号配平整段吞掉，
/// JSON 内的 `,` `]` 不结束段/码）。
///
/// 先确认段名非空 `[a-zA-Z0-9_]+`（大小写都收，与 [`parse_body`] 的归一口径一致）且空白之后
/// 是 `,` 或 `]`；校验不过或配平失败返回 `None`，由调用方（[`parse`]）决定回退策略。
fn find_code_end(input: &str, body_start: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut p = body_start;
    // 段名前的空白同样跳过（`[CQ: at,qq=1]`），集合仍是 ASCII 五元集（[`is_cq_space`]）
    while p < bytes.len() && is_cq_space(bytes[p]) {
        p += 1;
    }
    let name_start = p;
    while p < bytes.len()
        && (bytes[p].is_ascii_alphabetic() || bytes[p].is_ascii_digit() || bytes[p] == b'_')
    {
        p += 1;
    }
    if p == name_start {
        return None;
    }
    // 段名与 `,` / `]` 之间允许空白（统一空白契约，见模块头）
    while p < bytes.len() && is_cq_space(bytes[p]) {
        p += 1;
    }
    if p >= bytes.len() {
        return None;
    }
    if bytes[p] == b']' {
        return Some(p);
    }
    if bytes[p] != b',' {
        return None;
    }
    // 参数区只做配平找段尾，段内容丢弃（参数语义由 parse_body 重新切分）
    scan_regions(&input[p + 1..], p + 1, true, &mut |_| {}).0
}

/// 参数区配平扫描的状态机，字段语义与 Go `scanSegments` 一一对应。
#[derive(Debug)]
struct RegionScan {
    /// 当前值的花括号嵌套深度：> 0 时只关注闭合，段边界语义全部冻结。
    depth: usize,
    in_str: bool,
    esc: bool,
    /// 当前段是否已出现 `=`（其后为值区）。
    seg_has_eq: bool,
    /// 段首 / `=` 之后：下一个 `{` 视为 JSON 值起点。
    at_value_start: bool,
}

impl RegionScan {
    fn new() -> Self {
        Self {
            depth: 0,
            in_str: false,
            esc: false,
            seg_has_eq: false,
            at_value_start: true,
        }
    }

    /// 深度 > 0 时的字符推进：只做 JSON 字符串引号与转义的最小跟踪。
    fn feed_braced(&mut self, c: u8) {
        if self.in_str {
            if self.esc {
                self.esc = false;
            } else if c == b'\\' {
                self.esc = true;
            } else if c == b'"' {
                self.in_str = false;
            }
            return;
        }
        match c {
            b'"' => self.in_str = true,
            b'{' => self.depth += 1,
            b'}' => self.depth -= 1,
            _ => {}
        }
    }

    /// 深度 0 处的字符推进（收尾 `]` 与切分 `,` 由调用方处理，不进这里）。
    fn feed_value(&mut self, c: u8) {
        match c {
            b'{' if self.at_value_start => {
                self.depth = 1;
                self.in_str = false;
                self.esc = false;
                self.at_value_start = false;
            }
            b'=' if !self.seg_has_eq => {
                self.seg_has_eq = true;
                self.at_value_start = true;
            }
            // 空白不重置 JSON 值起点判定：`data = {"k":1}` 与 `data={"k":1}` 同口径
            c if is_cq_space(c) => {}
            _ => self.at_value_start = false,
        }
    }
}

/// Go `scanSegments` 同形配平扫描：逐字节推进 `input`，每遇到段边界把该段交给 `on_chunk`。
///
/// - `terminate_at_bracket == true`：裸 `]` 是整码收尾——收尾段同样交给 `on_chunk`，
///   返回 `(收尾括号的绝对下标, 深度)`（绝对下标以 `base` 为 `input` 在全文中的起点偏移）。
/// - `terminate_at_bracket == false`：`]` 只是值里的普通字节（调用方已处理段尾）——
///   扫到串尾、末段交给 `on_chunk`，返回 `(None, 深度)`。
///
/// 第二个返回值是串尾残留深度：> 0 表示值的 `{` 没闭合（配平失败），调用方应把整码按
/// 非法处理（Go：`scanSegments` 返回 `ok=false` → `KindText` 原文保留）。
fn scan_regions(
    input: &str,
    base: usize,
    terminate_at_bracket: bool,
    on_chunk: &mut dyn FnMut(&str),
) -> (Option<usize>, usize) {
    let bytes = input.as_bytes();
    let mut scan = RegionScan::new();
    let mut start = 0;
    let mut q = 0;
    while q < bytes.len() {
        let c = bytes[q];
        if scan.depth > 0 {
            scan.feed_braced(c);
            q += 1;
            continue;
        }
        match c {
            b']' if terminate_at_bracket => {
                on_chunk(&input[start..q]);
                return (Some(base + q), scan.depth);
            }
            b',' => {
                on_chunk(&input[start..q]);
                start = q + 1;
                scan.seg_has_eq = false;
                scan.at_value_start = true;
            }
            _ => scan.feed_value(c),
        }
        q += 1;
    }
    on_chunk(&input[start..]);
    (None, scan.depth)
}

/// 解析单个完整 CQ 码（`[CQ:type,...]`）。
///
/// # Errors
/// 缺少 `[CQ:` / `]` 包裹，或段名不是字母/数字/下划线（大小写都收）时返回 [`Error::Protocol`]。
pub fn parse_one(code: &str) -> Result<Segment> {
    // 外侧空白按统一空白契约 trim（只含 5 个 ASCII 空白，见 [`is_cq_space`]/模块头）
    let trimmed = trim_ascii(code);
    let body = trimmed
        .strip_prefix("[CQ:")
        .and_then(|rest| rest.strip_suffix(']'))
        .ok_or_else(|| Error::Protocol(format!("不是完整的 CQ 码: {code}")))?;
    // 整段原文（trim 外侧空白之后）同步携带为透传 raw
    parse_body(body)
        .map(|segment| segment.with_raw(trimmed))
        .ok_or_else(|| Error::Protocol(format!("CQ 码类型非法: {code}")))
}

/// 生成单个消息段的 CQ 文本。
#[must_use]
pub fn generate(segment: &Segment) -> String {
    segment.to_cq()
}

/// 把消息段序列拼成 CQ 文本（文本段原样拼接）。
///
/// 输出为**规范化**形态；需要逐字节保留用户字节的透传拼接（如渲染回写）请逐段用
/// [`Segment::raw_or_cq`]，不要用本函数。
#[must_use]
pub fn to_cq_string(segments: &[Segment]) -> String {
    let mut out = String::new();
    for segment in segments {
        out.push_str(&segment.to_cq());
    }
    out
}

/// 词法切分层的空白集合（模块头空白契约）：空格 / 制表 / 换行 / 换页 / 回车——即 Go 正则
/// `\s` = {空格, `\t`, `\n`, `\f`, `\r`}。**不含**纵向制表 `\v`（0x0B）与全角空格（U+3000）
/// 等任何 Unicode 空白。只用于段名前后/参数间/外侧的跳空白；键与值的 trim 走 `str::trim`
/// （Go `strings.TrimSpace`），见 [`push_param`]。
#[inline]
fn is_cq_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\x0C' | b'\r')
}

/// 去掉尾部 CQ 空白（集合见 [`is_cq_space`]，Go 正则 `\s*` 口径）。
fn trim_ascii_end(text: &str) -> &str {
    let bytes = text.as_bytes();
    let mut end = bytes.len();
    while end > 0 && is_cq_space(bytes[end - 1]) {
        end -= 1;
    }
    &text[..end]
}

/// 去掉首尾词法切分层空白（集合见 [`is_cq_space`]）。
/// 只用于**外侧/段名**这类切分位置（`parse_one` 整段外侧、`parse_body` 段名两侧）；
/// 键与值的 trim 不在此列（走 `str::trim`，对偶 Go `strings.TrimSpace`）。
fn trim_ascii(text: &str) -> &str {
    let bytes = text.as_bytes();
    let mut start = 0;
    while start < bytes.len() && is_cq_space(bytes[start]) {
        start += 1;
    }
    trim_ascii_end(&text[start..])
}

fn parse_body(body: &str) -> Option<Segment> {
    // 段名前后都允许空白（`[CQ:image ,file=x]` / `[CQ:at ]` / `[CQ: at,qq=1]`），口径与 at 段
    // 及 Go 正则 `\s*` 一致；空白集合仍是 ASCII 五元集（[`is_cq_space`]）
    let (raw_kind, params_part) = match memchr_byte(body.as_bytes(), 0, b',') {
        Some(idx) => (&body[..idx], &body[idx + 1..]),
        None => (body, ""),
    };
    let kind = trim_ascii(raw_kind);
    // 段名大小写不敏感（与 Go 的最大兼容并集同口径）：ASCII 字母大小写与数字/下划线都收，
    // 落段时统一归一为小写，下游按 kind 匹配的臂（outbound / deferred）无需感知大小写
    if kind.is_empty()
        || !kind
            .bytes()
            .all(|b| b.is_ascii_alphabetic() || b.is_ascii_digit() || b == b'_')
    {
        return None;
    }
    let kind = kind.to_ascii_lowercase();
    let mut params = Vec::new();
    // 冒号语法只对 stream 段生效（Go `splitKV` 的 `action=="stream"` 判定，`cqUnionParams`
    // 的 allowColon 同源）；其余段的 `key:value` 片段按无分隔裸片段并入前一键值
    if !scan_params(&mut params, params_part, kind == "stream") {
        // 值的 `{` 没闭合：整码按非法处理（Go `ok=false` → 文本原文），不产出被截断的参数
        return None;
    }
    Some(Segment {
        kind,
        params,
        json_keys: Vec::new(),
        // raw 由调用方挂载（parse / parse_one 有整段原文）；自产段保持 None，透传回退规范化
        raw: None,
    })
}

/// 参数片段 → `(key, 未反转义的原值)`。
///
/// 先按首个 `=` 切；没有 `=` 时，**仅 stream 段**（`allow_colon`）再按首个 `:` 切一次
/// （`[CQ:stream,type:start,qq:123]`，`handlers/cqparse/tokenize.go:197-207` 的 `splitKV`
/// 就是 `action=="stream"` 才走冒号）。两者都没有返回 `None`，由 [`push_param`] 按裸片段并前键。
fn split_kv(chunk: &str, allow_colon: bool) -> Option<(&str, &str)> {
    if let Some(idx) = memchr_byte(chunk.as_bytes(), 0, b'=') {
        return Some((&chunk[..idx], &chunk[idx + 1..]));
    }
    if !allow_colon {
        return None;
    }
    let idx = memchr_byte(chunk.as_bytes(), 0, b':')?;
    // 空 key 丢弃；stream 里无 `=` 的 URL 型裸值（`https://x`）会被切成 `https` + `//x`，
    // 这是"无分隔符"本身有歧义的代价——CQ 参数键不含 `:`，值必须显式带分隔符。
    if idx == 0 {
        return None;
    }
    Some((&chunk[..idx], &chunk[idx + 1..]))
}

/// 按 `,` 切分参数，并直接落成 `(key, 反转义后的值)`；返回花括号配平是否闭合。
///
/// 分隔符是花括号深度 0 处的裸 `,`：`=` 或段首后以 `{` 开头的值 JSON 整段吞掉（口径同
/// Go `cqparse/tokenize.go` 的 `scanSegments` 配平），data 里的逗号不会被误切、`]` 也不
/// 会截尾。`&#44;` 里没有裸 `,`（实体本身已是转义形态），无花括号时与 Go `splitCQParams`
/// 逐字节一致。`parse` 路径进入这里时 `find_code_end` 已确认段尾，`]` 在本函数里只是值里的
/// 普通字节。第二个返回值残留深度 > 0（配平失败）时本函数返回 `false`，调用方整码拒解析。
fn scan_params(params: &mut Vec<(String, String)>, input: &str, allow_colon: bool) -> bool {
    scan_regions(input, 0, false, &mut |chunk| {
        push_param(params, chunk, allow_colon);
    })
    .1 == 0
}

/// 单个参数片段落成参数。
///
/// 无 `=`、（非 stream 时还）无 `:` 的裸片段**并入前一键值、以 `,` 拼接**（C3 并集口径，
/// 对齐 Go `cqUnionParams`/`buildParams` 的 `params[lastKey] + "," + t`，`user_ids=1,2,3`
/// 三路径等价）；无前一键、前一键为空键或片段本身为空时丢弃（Go 的 `lastKey != "" && t != ""`
/// 守卫）。并入的片段与 `k=v` 的值同走 Unicode trim + 实体解码（`DecodeValue` 对偶）。
fn push_param(params: &mut Vec<(String, String)>, chunk: &str, allow_colon: bool) {
    let Some((key, value)) = split_kv(chunk, allow_colon) else {
        let Some((last_key, last_value)) = params.last_mut() else {
            return;
        };
        let tail = unescape(chunk.trim());
        if last_key.is_empty() || tail.is_empty() {
            return;
        }
        last_value.push(',');
        last_value.push_str(&tail);
        return;
    };
    // 键全 trim；值只 trim 首尾、内部空白保留（`[CQ:file,file=my file.txt]` 值仍为
    // `my file.txt`）。键/值 trim 用 Unicode 语义（`str::trim`），与 Go 按键取值路径的
    // `strings.TrimSpace` 对偶同源（见模块头空白契约）：`\v`、全角空格（U+3000）、NBSP
    // 都参与 trim。段名前后/参数间的跳空白仍是 ASCII 五元集（[`is_cq_space`]），两层各司其职。
    // 键大小写不敏感（并集口径，与 Go `buildParams` 的键小写一致）：trim 后统一小写存储
    // （`File` ≡ `file`、码表驼峰键 `subType` 落为 `subtype`）；值不参与大小写处理。
    params.push((key.trim().to_ascii_lowercase(), unescape(value.trim())));
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
            json_keys: Vec::new(),
            // 转义形态来自 CqCode 构造而非 parse 原文：按自产段处理（透传输出 = 规范化）
            raw: None,
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
/// `inbound` 是行为开关（见 [`CodeSpec::inbound`]）；`params` 与 `outbound` 是**描述性
/// 元数据**，全仓没有生产读取点（`params` 只被本文件的码表往返测试用），写错它们不动
/// 行为，但会误导接手人——填之前先对齐 Go 的真实形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeSpec {
    /// 码类型。
    pub name: &'static str,
    /// 参数名（顺序即 Go 生成顺序）。
    pub params: &'static [&'static str],
    /// `array_value` 段数组上报时，正文里**用户手打**的 `[CQ:x,…]` 是否落成类型化段。
    ///
    /// 唯一消费点是 `gensokyo-runtime` 的 `inbound::reported_message`：真 → 落成类型化段；
    /// 假 → 按原文逐字节折回 text 段（折回臂取码段原文，段名/键名归一只发生在落段臂）。
    ///
    /// 这一位**不是**「Go 是否支持该码」，也不是「入站是否会产出该段」：真实的 image /
    /// record / ark / reply 段由 `inbound` 的 `image_segments` / `message_event_segments`
    /// 按附件与 `msg_elements` 独立产出，与本位无关。Go `ConvertToSegmentedMessage`
    /// （`handlers/message_parser.go:2253-2334`）对正文只做整段切片 + `<@OpenID>` 正则、
    /// 不解析 CQ，所以手打形态一律留 text——除 `text` 与 `at` 外本位全部取假（P72，
    /// `at` 为真的原因见码表该条注释）。
    pub inbound: bool,
    /// 出站解析会消费该码。
    ///
    /// 描述性：出站链路按码名显式分发，没有读这一位的地方。
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
        // `inbound: true` 的唯一例外（结构性，另立 P73 处理）：真实 @ 由 `revert_mentions`
        // 把 `<@OpenID>` 合成成 `[CQ:at,qq=…]`，再靠本位解回类型化段，置假会让真实 mention 全部退化成文本。
        name: "at",
        params: &["qq"],
        inbound: true,
        outbound: true,
    },
    CodeSpec {
        // 手打形态 Go 留 text：`ConvertToSegmentedMessage` 只做整段切片 + `<@OpenID>` 正则，
        // 不解析 CQ（message_parser.go:2253-2334）。reply 段由 message_event_segments
        //（msg_elements 引用）独立产出，不受本位影响。
        name: "reply",
        params: &["id", "msg_idx"],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        // 手打形态同上留 text（message_parser.go:2253-2334）。image 段由 image_segments
        //（图片附件）独立产出，不受本位影响。
        name: "image",
        params: &["file", "subType", "url"],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        // file 为空时兼容 url（Go resolveMediaValue handlers_media.go:51-55）
        // 手打形态同上留 text（message_parser.go:2253-2334）。record 段由
        // message_event_segments（语音附件）独立产出，不受本位影响。
        name: "record",
        params: &["file", "url", "asr"],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        // 同 record/voice 口径的 url 兜底（Go message_parser.go:1218-1222）
        // 手打形态 Go 留 text（message_parser.go:2253-2334）；Go 入站也不产视频段
        //（附件过滤只认 `image/` 前缀，同文件 :2214-2235）。
        name: "video",
        params: &["file", "url"],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        // 手打形态 Go 留 text：无任何入站产出点（message_parser.go:2253-2334 仅切片 + `<@>` 正则）
        name: "file",
        params: &["file", "file_name", "name"],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        // 出站有消费点：Go 新链 `internal/domain/message.ParseOneBotString` 把 `[CQ:face,id=…]`
        // 解析成 face 段（未知媒体回落 `unknown_face`，`internal/domain/message/compat_test.go:38,73`）。
        // Go 入站侧不产出它——`ConvertToSegmentedMessage` 的段类型只有 text/at/image 再加
        // record/ark/reply（`handlers/message_parser.go:2340`），所以手打的 `[CQ:face,…]`
        // 按原文留 text（P72 收位，口径见 [`CodeSpec::inbound`]）。
        name: "face",
        params: &["id"],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        // 手打形态 Go 留 text：markdown 只作出站 msg_type=2，入站无产出点
        //（message_parser.go:2253-2334 仅切片 + `<@>` 正则）
        name: "markdown",
        params: &["data"],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        // 出站 `[CQ:card,title=…,desc=…,pic=…,url=…]`（handlers/cqcode_pipeline.go:114）；
        // `data=` 是段形态的 JSON 串写法（handlers/message_parser.go:988）
        // 手打形态 Go 留 text：card 是出站群消息 msg_type=8，入站无产出点（同上 :2253-2334）
        name: "card",
        params: &["title", "desc", "pic", "url", "data"],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        // 手打形态 Go 留 text：keyboard 是出站按钮消息，入站无产出点（同上 :2253-2334）
        name: "keyboard",
        params: &["data"],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        // 手打形态 Go 留 text：stream 是出站单聊流式消息 msg_type 控制位，入站无产出点（同上 :2253-2334）
        name: "stream",
        params: &["type", "qq"],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        // 手打形态 Go 留 text：input_notify 是出站单聊 msg_type=6 控制位，入站无产出点（同上 :2253-2334）
        name: "input_notify",
        params: &["type", "second"],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        // foundItems 的 key 叫 qqmusic，CQ 类型沿用 OneBot 的 music,type=qq
        // 手打形态 Go 留 text：music 是出站 msg_type=3 卡片，入站卡片走 ark 段（同上 :2253-2334）
        name: "music",
        params: &["type", "id"],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        // 手打形态 Go 留 text：active / active_type / active_sub_type 是出站 foundItems 控制 key，入站不产出
        name: "active",
        params: &["type", "sub_type"],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        // 手打形态 Go 留 text：wakeup 是 `send_private_msg_wakeup` 的出站标记位，入站无产出点
        name: "wakeup",
        params: &["userid"],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        // 手打形态 Go 留 text：node 是转发消息段的形态，入站不产出（message_parser.go:2253-2334）
        name: "node",
        params: &["id", "user_id", "nickname", "content"],
        inbound: false,
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
        // 手打形态 Go 留 text：Go 入站不产出 poke（全仓大小写不敏感零命中，`REACTION_*` 无处理器）
        name: "poke",
        params: &["qq"],
        inbound: false,
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
        // voice 是 record 的同义码（出站 `record | voice` 同臂），参数口径与 record 一致
        name: "voice",
        params: &["file", "url", "asr"],
        inbound: false,
        outbound: true,
    },
    CodeSpec {
        // 入站上报会产出 ark 卡片段——那是 `message_event_segments` 按 `msg_elements` /
        // `message_type=3` 独立产出的，与下面 `inbound` 位无关（P72）；出站无消费点
        //（ProcessCQInboundOnlyCodes 会剥离）。
        // 出站那条路整段删掉、不读任何参数（Go `handlers/cqcode.go:198`+`:203-206`、
        // `handlers/cqparse/handlers_control.go:26` 的 `inboundOnlyHandler`），所以 `params` 记的是
        // **入站** CQ 文本形态的三键（Go `handlers/message_parser.go:2496-2498`）；`fields` 只存在于
        // 段 JSON 的 `data` 里（`arkSegmentData`），不进 CQ 文本。
        // 手打的 `[CQ:ark,…]` 按原文留 text（message_parser.go:2253-2334 仅切片 + `<@>` 正则）。
        name: "ark",
        params: &["ark_type", "ark_name", "prompt"],
        inbound: false,
        outbound: false,
    },
    CodeSpec {
        // 手打形态 Go 留 text：json 段是 OneBot 数组形态的容器，Go 入站无产出点
        //（message_parser.go:2253-2334 仅切片 + `<@>` 正则）
        name: "json",
        params: &["data"],
        inbound: false,
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
    fn unescaped_comma_in_value_is_merged_like_go_union() {
        // Go `cqUnionParams`/`buildParams`：无 `=` 的裸尾段 `b.txt` 并入前一 key `file`，
        // 以 `,` 拼接（C3 并集口径）；两侧同输入同结果，不再"一侧丢弃一侧透传"
        let segment = parse_one("[CQ:file,file=https://x.com/a,b.txt]").expect("解析失败");
        assert_eq!(segment.get("file"), Some("https://x.com/a,b.txt"));
        assert_eq!(segment.params.len(), 1, "并入前一键，不产生新参数");
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
        // 裸逗号是参数分隔符（Go 语义），`b` 无 `=` 按并集口径并入 `name` 值（C3 口径），
        // 规范化输出里以实体回写；raw 透传臂仍逐字节吐原文
        assert_eq!(
            to_cq_string(&parse("[CQ:at,qq=1,name=a,b]")),
            "[CQ:at,qq=1,name=a&#44;b]"
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

    /// 冒号兜底只对 `stream` 生效（Go `splitKV` 的 `action=="stream"` 判定），裸尾段并入前键
    /// （Go `cqUnionParams`/`buildParams` 的 C3 并集口径）——与 Go 逐形态对齐。
    #[test]
    fn colon_params_parse_for_stream_only_and_bare_segments_merge() {
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

        // 非 stream 段不再认 `:`：`action:run` 是无分隔裸片段且无前一键，丢弃（Go 同）
        assert_eq!(
            parse("[CQ:custom,action:run]"),
            vec![Segment::new("custom")]
        );
        // 派发边界用例：music 的 `type:qq` 在 Go 冒号兜底之外，不识别为 `type` 键；Rust 同
        assert_eq!(
            parse("[CQ:music,type:qq,id=123]"),
            vec![Segment::new("music").param("id", "123")]
        );

        // 裸片段并入前一键值、以 `,` 拼接（C3 并集口径，与 Go 同输入同结果）
        assert_eq!(
            parse("[CQ:file,file=a.txt,b.txt]"),
            vec![Segment::new("file").param("file", "a.txt,b.txt")]
        );
        assert_eq!(
            parse("[CQ:reply,id=1,2]"),
            vec![Segment::new("reply").param("id", "1,2")]
        );
        // stream 的裸尾段同样并入前键（Go `buildParams` 对 action 无关，统一并入）
        assert_eq!(
            parse("[CQ:stream,qq:123,456]"),
            vec![Segment::new("stream").param("qq", "123,456")]
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

    /// 与 Go 泳道镜像表同一组用例：所有 CQ 段类型统一容忍段名后、参数间与键值两侧的空白
    #[test]
    fn parse_tolerates_whitespace_like_go_mirror() {
        // 段名后空白 + 键值两侧空白 + 参数间空白
        assert_eq!(
            parse("[CQ:image , file = https://example.com/a.png]"),
            vec![Segment::new("image").param("file", "https://example.com/a.png")]
        );
        // 值只 trim 首尾，内部空白保留
        assert_eq!(
            parse("[CQ:record,file = my voice.amr]"),
            vec![Segment::new("record").param("file", "my voice.amr")]
        );
        // `qq` 不是首参、键带空白也照样取到
        assert_eq!(
            parse("[CQ:at , name = x , qq = 123]"),
            vec![Segment::new("at").param("name", "x").param("qq", "123")]
        );
        // 其余段类型同一口径：face / reply / card
        assert_eq!(
            parse("[CQ:face , id = 1]"),
            vec![Segment::new("face").param("id", "1")]
        );
        assert_eq!(
            parse("[CQ:reply,id = 123]"),
            vec![Segment::new("reply").param("id", "123")]
        );
        assert_eq!(
            parse("[CQ:card , title=A, desc=B]"),
            vec![Segment::new("card").param("title", "A").param("desc", "B")]
        );
        // `=` 与 `:` 两种分隔符混用（stream 的 Go 原生写法）同样容忍空白
        assert_eq!(
            parse("[CQ:stream , type:start , qq=123]"),
            vec![
                Segment::new("stream")
                    .param("type", "start")
                    .param("qq", "123")
            ]
        );
        // 裸段名带尾部空白：识别为无参段（旧口径会整段当文本）
        assert_eq!(parse("[CQ:at ]"), vec![Segment::new("at")]);
        assert_eq!(parse("[CQ:active , ]"), vec![Segment::new("active")]);
        // 统一契约（最大兼容并集）：段名与参数键大小写不敏感、段名前空白同样容忍
        assert_eq!(
            parse("[CQ:Image,file=x]"),
            vec![Segment::new("image").param("file", "x")]
        );
        assert_eq!(
            parse("[CQ: image,file=x]"),
            vec![Segment::new("image").param("file", "x")]
        );
        // 畸形段保持现状：没有 `]` 一律按文本
        assert_eq!(
            parse("[CQ:image , file = x"),
            vec![Segment::text("[CQ:image , file = x")]
        );
        // parse_one 同口径
        assert_eq!(
            parse_one("[CQ:file , file_name = my report.txt ]").expect("应识别"),
            Segment::new("file").param("file_name", "my report.txt")
        );
        // 重新序列化输出为规范化形态：空白被去掉，但值内部空白保留
        assert_eq!(
            to_cq_string(&parse("[CQ:image , file = my pic.png]")),
            "[CQ:image,file=my pic.png]"
        );
    }

    /// 统一契约（与 Go 同口径的最大兼容并集）形态表：段名与参数键大小写不敏感（都归一小写
    /// 存储）、参数任意键序、段名前后空白容忍；`[CQ:` 前缀、值/转义口径与 raw 逐字节透传不动。
    #[test]
    fn case_insensitive_names_and_keys_accept_max_union_shapes() {
        // 段名大写 + 键大写：段名与键都归一为小写落段（`File` ≡ `file`）
        let segments = parse("[CQ:Image,File=x]");
        assert_eq!(segments, vec![Segment::new("image").param("file", "x")]);
        assert_eq!(
            segments[0].get("file"),
            Some("x"),
            "键 `File` 应等价 `file`"
        );
        // 任意键序：按键取值不依赖 `file` 出现的位置
        let segments = parse("[CQ:image,name=x,file=y]");
        assert_eq!(
            segments,
            vec![Segment::new("image").param("name", "x").param("file", "y")]
        );
        assert_eq!(segments[0].get("file"), Some("y"));
        // 段名前空白（at 路径的既有口径不回退）
        assert_eq!(
            parse("[CQ: at,qq=10001]"),
            vec![Segment::new("at").param("qq", "10001")]
        );
        // 大小写 + 空白 + 键序三者组合
        let combo = parse("[CQ: IMAGE , FILE = x , NAME = y ]");
        assert_eq!(
            combo,
            vec![Segment::new("image").param("file", "x").param("name", "y")]
        );
        assert_eq!(combo[0].get("file"), Some("x"));
        assert_eq!(combo[0].get("name"), Some("y"));
        // 未知段名同样归一为小写（仍按未知段保留），透传臂逐字节输出原文
        let unknown = parse("[CQ:Cardboard,XX=1]");
        assert_eq!(unknown, vec![Segment::new("cardboard").param("xx", "1")]);
        assert_eq!(unknown[0].raw_or_cq(), "[CQ:Cardboard,XX=1]");
        // parse_one 同口径；规范化序列化输出小写键形态
        let face = parse_one("[CQ:Face,ID=1]").expect("应识别");
        assert_eq!(face, Segment::new("face").param("id", "1"));
        assert_eq!(face.get("id"), Some("1"));
        assert_eq!(to_cq_string(&parse("[CQ:At,QQ=1]")), "[CQ:at,qq=1]");
        // 边界：值不参与大小写处理（`A B` 与内部空白原样保留）
        let spaced = parse("[CQ:image,File=A B]");
        assert_eq!(spaced, vec![Segment::new("image").param("file", "A B")]);
        assert_eq!(spaced[0].get("file"), Some("A B"));
        // 边界：`[CQ:` 前缀本身仍大小写敏感（Go 正则字面量），整段按文本
        assert_eq!(
            parse("[cq:image,file=x]"),
            vec![Segment::text("[cq:image,file=x]")]
        );
        // 边界：段名前的 `\v` 不属于 ASCII 五元集，照旧按文本
        assert_eq!(
            parse("[CQ:\u{0B}image,file=x]"),
            vec![Segment::text("[CQ:\u{0B}image,file=x]")]
        );
    }

    /// R3 契约形态逐字用例：大写段名 + 切分位空白 + 键大小写 + 任意键序（`asr` 在 `file`
    /// 前）+ 值内部空白五者组合，逐字段断言归一小写键与未改写值（Go record 正则的
    /// `file=…(?:,asr=…)?` 并集口径同形）。
    #[test]
    fn record_contract_shape_case_space_order_and_asr() {
        let segment = parse_one("[CQ:RECORD , ASR = ab , file = my voice.amr]").expect("应识别");
        assert_eq!(segment.kind, "record", "段名归一小写");
        assert_eq!(
            segment.params,
            vec![
                ("asr".to_owned(), "ab".to_owned()),
                ("file".to_owned(), "my voice.amr".to_owned()),
            ],
            "键归一小写、值内部空白保留、键序按线上顺序"
        );
        assert_eq!(segment.get("file"), Some("my voice.amr"));
        assert_eq!(segment.get("asr"), Some("ab"));
    }

    /// Major 回归用例：透传臂输出 parse 原文切片，逐字节不改写（`render_plain` 的 `_` 臂、
    /// `from_segments` 的 at 臂都走 [`Segment::raw_or_cq`]）。
    #[test]
    fn parsed_segment_passthrough_keeps_original_bytes() {
        for raw in [
            "[CQ:image , file = a b.png]",
            "[CQ:keyboard , data = {\"a\":1,2}]",
            "[CQ:at ]",
        ] {
            let segments = parse(raw);
            assert_eq!(segments.len(), 1, "应命中一个码段: {raw}");
            assert_eq!(segments[0].raw_or_cq(), raw, "透传应逐字节一致: {raw}");
        }
        // 自产段（raw=None）：透传输出 = 规范化序列化，与既有行为逐字节相同
        let made = Segment::new("at").param("qq", "123");
        assert_eq!(made.raw_or_cq(), made.to_cq());
        // raw 不参与语义相等：parse 而来与代码构造判等（既有比较口径不回退）
        assert_eq!(parse("[CQ:at , qq = 123]"), vec![made]);
    }

    /// Major 回归用例：值 JSON 按花括号配平整段吞掉，data 内逗号不再截断参数，往返保真。
    #[test]
    fn brace_balanced_json_data_not_truncated() {
        let raw = "[CQ:keyboard , data = {\"a\":1,2}]";
        let segments = parse(raw);
        assert_eq!(segments.len(), 1, "应命中一个码段");
        assert_eq!(segments[0].get("data"), Some(r#"{"a":1,2}"#));
        assert_eq!(segments[0].raw_or_cq(), raw, "透传应逐字节一致");
        // 规范化往返无损：to_cq 把裸逗号转成实体，再 parse 参数一致
        let normalized = segments[0].to_cq();
        assert_eq!(normalized, r#"[CQ:keyboard,data={"a":1&#44;2}]"#);
        let again = parse(&normalized);
        assert_eq!(again[0].get("data"), Some(r#"{"a":1,2}"#));
        assert_eq!(again, segments, "往返后参数语义一致");
    }

    /// Major 回归用例：JSON 里（含字符串内）的 `]` 不再截断码尾。
    #[test]
    fn brace_balanced_end_skips_bracket_inside_json() {
        let raw = r#"[CQ:markdown,data={"content":"a]b , c","x":{"y":1,2}}]"#;
        let segments = parse(raw);
        assert_eq!(segments.len(), 1, "应命中一个码段");
        assert_eq!(
            segments[0].get("data"),
            Some(r#"{"content":"a]b , c","x":{"y":1,2}}"#)
        );
        assert_eq!(segments[0].raw_or_cq(), raw, "透传应逐字节一致");
    }

    /// 配平失败（`{` 没闭合）整码按文本原样保留——Go `scanSegments` `ok=false` 口径，
    /// 不再产出被截断参数的码段；parse_one 同口径报错。
    #[test]
    fn unbalanced_brace_code_stays_text() {
        let raw = "[CQ:keyboard,data={\"a\":1]";
        assert_eq!(parse(raw), vec![Segment::text(raw)]);
        assert!(parse_one(raw).is_err(), "parse_one 同口径：配平失败直接拒");
    }

    /// 配平失败/段名非法**不吞内嵌码**：Go `Tokenize` 的失败分支是 `i = start + 1`
    /// （`handlers/cqparse/tokenize.go:29`）逐字符重扫，失败段后方的合法码照常识别。
    #[test]
    fn failed_code_does_not_swallow_nested_code() {
        let segments = parse("[CQ:x,data={ [CQ:at,qq=1]");
        assert_eq!(
            segments,
            vec![
                Segment::text("[CQ:x,data={ "),
                Segment::new("at").param("qq", "1")
            ],
            "配平失败段留成文本，内嵌 at 码必须与 Go 一样被识别"
        );
        // 文本切片的字节数、以及命中段的透传原文逐字节对齐 Go
        assert_eq!(segments[0].raw_or_cq(), "[CQ:x,data={ ");
        assert_eq!(segments[1].raw_or_cq(), "[CQ:at,qq=1]");
        // 段名大小写不敏感后，`Bad` 归一为未知段名 `bad`（仍识别为段），后方码段照常识别
        assert_eq!(
            parse("[CQ:Bad,x=1][CQ:at,qq=2]"),
            vec![
                Segment::new("bad").param("x", "1"),
                Segment::new("at").param("qq", "2")
            ]
        );
        // 段名真非法（含非法字符）时同样只走一步重扫：后方码段不受影响
        assert_eq!(
            parse("[CQ:Bad!,x=1][CQ:at,qq=2]"),
            vec![
                Segment::text("[CQ:Bad!,x=1]"),
                Segment::new("at").param("qq", "2")
            ]
        );
    }

    /// Go 的 atValueStart 口径：`{` 只在段首或 `=` 之后（跳空白）才算 JSON 值起点。
    #[test]
    fn brace_in_value_without_eq_stays_split_like_go() {
        // 裸值中间的 `{` 不参与配平，仍按裸逗号切分；`c.png}` 属无分隔裸片段，
        // 按并集口径并入 `file` 值（Go `buildParams` 同结果，不再丢弃）
        let segment = parse_one("[CQ:file,file=a{b,c.png}]").expect("应解析");
        assert_eq!(segment.get("file"), Some("a{b,c.png}"));
    }

    /// 空白契约用例（两层）：切分位置用 Go 正则 `\s` 的 ASCII 五元集（`\v`、全角空格不算
    /// 空白）；键/值首尾 trim 用 Unicode（Go `strings.TrimSpace`，`\v`、全角空格被吞）。
    #[test]
    fn whitespace_contract_split_ascii_value_trim_unicode() {
        // `\f`（换页）是空白：段名后、`,` 两侧、键值两侧统一 trim
        assert_eq!(
            parse("[CQ:image\u{0C} ,\u{0C}file\u{0C}=\u{0C}x.png]"),
            vec![Segment::new("image").param("file", "x.png")]
        );
        // ---- 词法切分层：ASCII 五元集（对 Go 正则 `\s*`）----
        // `\v`（纵向制表）不是空白：段名后出现则整段按文本
        assert_eq!(
            parse("[CQ:image\u{0B},file=x]"),
            vec![Segment::text("[CQ:image\u{0B},file=x]")]
        );
        // 全角空格（U+3000）在切分位置不是空白：段名后出现则整段按文本
        assert_eq!(
            parse("[CQ:image\u{3000},file=x]"),
            vec![Segment::text("[CQ:image\u{3000},file=x]")]
        );
        // ---- 键/值 trim 层：Unicode（对 Go `strings.TrimSpace`）----
        // 值首尾的全角空格被 trim（Go 按键取值全部 TrimSpace：`cqCodeParam`/`buildParams`）
        assert_eq!(
            parse("[CQ:image, file = \u{3000}x]"),
            vec![Segment::new("image").param("file", "x")]
        );
        // `\v` 同属 Unicode 空白：值两侧一并吞掉
        assert_eq!(
            parse("[CQ:image, file = \u{0B}x\u{0B}]"),
            vec![Segment::new("image").param("file", "x")]
        );
        // 键两侧同理：U+3000 包裹的键名仍取到 `file`
        assert_eq!(
            parse("[CQ:image,\u{3000}file\u{3000}=\u{3000}x]"),
            vec![Segment::new("image").param("file", "x")]
        );
        // parse_one 外侧属切分层：`\v` 不 trim 直接拒，`\f` 可 trim
        assert!(parse_one("\u{0B}[CQ:at,qq=1]").is_err());
        assert_eq!(
            parse_one("\u{0C}[CQ:at,qq=1]\u{0C}")
                .expect("换页应被 trim")
                .kind,
            "at"
        );
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
            let mut wire_segment = Segment::new(spec.name);
            let mut expected = Segment::new(spec.name);
            for (index, param) in spec.params.iter().enumerate() {
                // wire 由码表原样生成（含驼峰键 `subType`）
                wire_segment = wire_segment.param(*param, format!("v{index}"));
                // 新契约翻转：解析侧参数键统一小写（Go `buildParams` 同口径），期望按小写键比较
                expected = expected.param((*param).to_ascii_lowercase(), format!("v{index}"));
            }
            let wire = to_cq_string(std::slice::from_ref(&wire_segment));
            let back = parse(&wire);
            assert_eq!(back, vec![expected], "码表往返失败: {}", spec.name);
            // raw 保真不受键归一影响：透传臂逐字节吐 parse 命中的原文（大写键形态不改写）
            assert_eq!(
                back[0].raw_or_cq(),
                wire.as_str(),
                "码表透传原文失配: {}",
                spec.name
            );
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

    /// 行为断言（防码表与真实形态再度漂移）：Go 入站构造的 ark CQ 文本
    /// （`handlers/message_parser.go:2496-2498`，含 `&#44;` 转义逗号）在本仓解析后必须拿到
    /// `ark_type` / `ark_name` / `prompt` 三个键，且能原样重新拼回线上形态。
    /// 最后一条把断言钉回码表：`ark.params` 与这三键不同口径就红。
    #[test]
    fn ark_inbound_cq_form_carries_three_keys() {
        let raw = "[CQ:ark,ark_type=picture,ark_name=甲&#44;乙,prompt=点击查看图片]";
        let segments = parse(&format!("hello {raw}"));
        assert_eq!(segments.len(), 2, "正文与卡片应切成两段: {segments:?}");
        assert_eq!(segments[0].text_content(), Some("hello "));
        let card = &segments[1];
        assert_eq!(card.kind, "ark");
        assert_eq!(card.get("ark_type"), Some("picture"));
        assert_eq!(card.get("ark_name"), Some("甲,乙"), "转义逗号应解码");
        assert_eq!(card.get("prompt"), Some("点击查看图片"));
        assert_eq!(
            to_cq_string(std::slice::from_ref(card)),
            raw,
            "ark 的出站重拼必须与 Go 入站形态同口径"
        );
        assert_eq!(
            code_spec("ark").expect("ark").params,
            &["ark_type", "ark_name", "prompt"][..]
        );
    }

    /// 行为断言：face 段的透传字节保真。入站上报把 `[CQ:face,…]` 折回 text 段还是落成类型化段
    /// 由 `CodeSpec::inbound` 决定（消费点在 `gensokyo-runtime::inbound::reported_message`），
    /// 无论哪一路交给客户端的 CQ 文本都不该被规范化改写——这一条守住「改标记只换形态、不丢字节」。
    #[test]
    fn face_cq_text_is_preserved_verbatim() {
        let raw = "[CQ:face , id = 14, name=a&#44;b]";
        let segments = parse(raw);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].kind, "face");
        assert_eq!(
            segments[0].get("name"),
            Some("a,b"),
            "键内空格与转义只在解析结果里被规范化"
        );
        assert_eq!(segments[0].raw_or_cq(), raw, "透传原文必须逐字节一致");
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
