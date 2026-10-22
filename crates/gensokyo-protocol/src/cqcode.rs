//! CQ 码实体转义、反转义与单码构造。
//!
//! M0 只做转义/反转义与构造；完整码表（at / reply / image / record / video / file / face /
//! markdown / card / keyboard / stream / input_notify / qqmusic / active / wakeup / node）
//! 与分词解析在 M1 按 Go `handlers/cqcode.go` 逐案例对齐。
//!
//! **转义口径刻意跟随 Go 而非 OneBot v11 规范**：规范要求 `[` 也转义，但 Go 实现不转；
//! 为保住 M1 黄金样本的字节级一致，本项目出站只转 `,` `]` `&`（见 [`escape`]）。

use std::fmt;

use gensokyo_core::{Error, Result};

/// 转义 CQ 码参数值：`,` → `&#44;`、`]` → `&#93;`、`&` → `&amp;`。
///
/// **刻意不转`[`**：OneBot v11 规范要求转义 `[`，Go 实现（`handlers/cqcode.go`）只转上述
/// 三个字符；为保证与 Go 输出字节一致，这里跟随 Go。入站 [`unescape`] 仍接受 `&#91;`，
/// 因此对端按规范发来的内容也能还原。base64 中的 `+` `/` `=` 同样不转义。
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

/// 反转义 CQ 码参数值。
///
/// 与 Go `handlers/cqcode.go` 的 `ReplaceAll` 容错语义一致：只还原已知实体，**未知 `&`
/// 实体按字面保留**（裸 `&` 不报错、不吞字符）；顺序不可颠倒——先还原实体，再还原 `&amp;`。
/// 比 Go 多还原 `&#91;`，用于兼容对端按规范转义的 `[`（出站 [`escape`] 不产出它）。
#[must_use]
pub fn unescape(value: &str) -> String {
    value
        .replace("&#44;", ",")
        .replace("&#93;", "]")
        .replace("&#91;", "[")
        .replace("&amp;", "&")
}

/// 严格反转义：未知 `&` 实体返回 [`Error::Protocol`]。
///
/// 内部校验入口（不属于对外 API 承诺）：运行路径必须用宽容的 [`unescape`]，以免用户文本
/// 里的 `&` 被当成协议错误。
#[doc(hidden)]
pub fn unescape_strict(value: &str) -> Result<String> {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(idx) = rest.find('&') {
        out.push_str(&rest[..idx]);
        let tail = &rest[idx..];
        let (replacement, consumed) = if tail.starts_with("&amp;") {
            ("&", 5)
        } else if tail.starts_with("&#91;") {
            ("[", 5)
        } else if tail.starts_with("&#93;") {
            ("]", 5)
        } else if tail.starts_with("&#44;") {
            (",", 5)
        } else {
            let preview: String = tail.chars().take(5).collect();
            return Err(Error::Protocol(format!("非法 CQ 实体: {preview}")));
        };
        out.push_str(replacement);
        rest = &tail[consumed..];
    }
    out.push_str(rest);
    Ok(out)
}

/// 一个 CQ 码：`[CQ:type,key=value,...]`。
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

#[cfg(test)]
mod tests {
    use super::{CqCode, escape, unescape, unescape_strict};

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
}
