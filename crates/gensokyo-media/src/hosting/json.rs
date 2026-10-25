//! 极简 JSON 读取器：只为图床响应里的几个字符串字段服务。
//!
//! 为什么不用 `serde_json`：M2b 期间 `Cargo.lock` 冻结（`[[package]] gensokyo-media`
//! 的依赖边不允许变动），而本 crate 只需要「按路径取字符串」。因此这里实现一个只读、
//! 零依赖的子集解析器：
//! - 支持 object / array / string / number / `true` / `false` / `null`；
//! - 数字与数组不构造值（跳过即止），图床响应里我们没有消费方；
//! - 严格按 RFC 8259 处理转义，含 `\/` 与 `\uXXXX` 代理对；
//! - 拒绝尾随垃圾，与 Go `json.Unmarshal` 一致。

/// 解析后的 JSON 值：只需要对象容器与字符串，其余标量统一成 [`Value::Other`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Value {
    /// JSON 字符串（已解码转义）。
    String(String),
    /// JSON 对象，保持书写顺序。
    Object(Vec<(String, Value)>),
    /// 数字 / 布尔 / null / 数组：只要能跳过，就不需要值。
    Other,
}

impl Value {
    /// 对象里按 key 取子值（重复 key 取第一个，与 Go `encoding/json` 一致）。
    fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(entries) => entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// 按路径取子值，例如 `["result", "file_url"]`。
    fn path(&self, path: &[&str]) -> Option<&Self> {
        path.iter().try_fold(self, |node, key| node.get(key))
    }

    /// 按路径取字符串；路径不存在或末节点不是字符串时返回 `None`。
    pub(crate) fn path_str(&self, path: &[&str]) -> Option<&str> {
        match self.path(path)? {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    /// 取某个对象字段下的「字符串 → 字符串」映射（非字符串值丢弃），保持书写顺序。
    pub(crate) fn string_entries(&self, key: &str) -> Vec<(String, String)> {
        match self.get(key) {
            Some(Self::Object(entries)) => entries
                .iter()
                .filter_map(|(name, value)| match value {
                    Self::String(value) => Some((name.clone(), value.clone())),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// 解析一段 JSON；非法或带尾随垃圾时返回 `None`。
pub(crate) fn parse(bytes: &[u8]) -> Option<Value> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut parser = Parser {
        bytes: text.as_bytes(),
        index: 0,
    };
    let value = parser.value()?;
    parser.skip_whitespace();
    (parser.index == parser.bytes.len()).then_some(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.index).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.index += 1;
        Some(byte)
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.index += 1;
        }
    }

    fn value(&mut self) -> Option<Value> {
        self.skip_whitespace();
        match self.peek()? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => self.string().map(Value::String),
            b't' => self.literal(b"true"),
            b'f' => self.literal(b"false"),
            b'n' => self.literal(b"null"),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    /// 校验 `true` / `false` / `null` 字面量；值本身不消费。
    fn literal(&mut self, expected: &[u8]) -> Option<Value> {
        let end = self.index.checked_add(expected.len())?;
        if self.bytes.get(self.index..end)? != expected {
            return None;
        }
        self.index = end;
        Some(Value::Other)
    }

    fn object(&mut self) -> Option<Value> {
        self.index += 1; // '{'
        let mut entries = Vec::new();
        self.skip_whitespace();
        if self.peek()? == b'}' {
            self.index += 1;
            return Some(Value::Object(entries));
        }
        loop {
            self.skip_whitespace();
            let key = self.string()?;
            self.skip_whitespace();
            if self.bump()? != b':' {
                return None;
            }
            let value = self.value()?;
            entries.push((key, value));
            self.skip_whitespace();
            match self.bump()? {
                b',' => continue,
                b'}' => return Some(Value::Object(entries)),
                _ => return None,
            }
        }
    }

    /// 数组只做语法校验：逐元素解析后丢弃。
    fn array(&mut self) -> Option<Value> {
        self.index += 1; // '['
        self.skip_whitespace();
        if self.peek()? == b']' {
            self.index += 1;
            return Some(Value::Other);
        }
        loop {
            self.value()?;
            self.skip_whitespace();
            match self.bump()? {
                b',' => continue,
                b']' => return Some(Value::Other),
                _ => return None,
            }
        }
    }

    /// 数字只校验形状，不构造数值。
    fn number(&mut self) -> Option<Value> {
        if self.peek() == Some(b'-') {
            self.index += 1;
        }
        let start = self.index;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.index += 1;
        }
        if self.index == start {
            return None;
        }
        if self.peek() == Some(b'.') {
            self.index += 1;
            let fraction = self.index;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.index += 1;
            }
            if self.index == fraction {
                return None;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.index += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.index += 1;
            }
            let exponent = self.index;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.index += 1;
            }
            if self.index == exponent {
                return None;
            }
        }
        Some(Value::Other)
    }

    fn string(&mut self) -> Option<String> {
        if self.bump()? != b'"' {
            return None;
        }
        let mut out: Vec<u8> = Vec::new();
        loop {
            match self.bump()? {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => self.escape_into(&mut out)?,
                // 未转义的控制字符非法
                byte if byte < 0x20 => return None,
                byte => out.push(byte),
            }
        }
    }

    fn escape_into(&mut self, out: &mut Vec<u8>) -> Option<()> {
        match self.bump()? {
            b'"' => out.push(b'"'),
            b'\\' => out.push(b'\\'),
            b'/' => out.push(b'/'),
            b'b' => out.push(0x08),
            b'f' => out.push(0x0C),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'u' => {
                let first = self.hex4()?;
                let code = if (0xD800..0xDC00).contains(&first) {
                    // 高位代理后面必须跟 `\uDC00`-`\uDFFF`
                    if self.bump()? != b'\\' || self.bump()? != b'u' {
                        return None;
                    }
                    let second = self.hex4()?;
                    if !(0xDC00..0xE000).contains(&second) {
                        return None;
                    }
                    0x1_0000 + ((first - 0xD800) << 10) + (second - 0xDC00)
                } else if (0xDC00..0xE000).contains(&first) {
                    // 孤立的低位代理非法
                    return None;
                } else {
                    first
                };
                let mut buffer = [0u8; 4];
                out.extend_from_slice(char::from_u32(code)?.encode_utf8(&mut buffer).as_bytes());
            }
            _ => return None,
        }
        Some(())
    }

    fn hex4(&mut self) -> Option<u32> {
        let mut value = 0u32;
        for _ in 0..4 {
            value = value * 16 + char::from(self.bump()?).to_digit(16)?;
        }
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{Value, parse};

    #[test]
    fn reads_nested_string_fields() {
        let root =
            parse(br#"{"result":{"file_url":"https://a/b.png"},"code":0}"#).expect("合法 JSON");
        assert_eq!(
            root.path_str(&["result", "file_url"]),
            Some("https://a/b.png")
        );
        assert_eq!(root.path_str(&["code"]), None, "数字不是字符串");
        assert_eq!(root.path_str(&["result", "missing"]), None);
        assert_eq!(root.path_str(&["missing", "file_url"]), None);
    }

    #[test]
    fn decodes_escapes_including_solidus_and_surrogates() {
        let root =
            parse(br#"{"url":"https:\/\/x\/a\u0062.png","emoji":"\ud83d\ude00","n":-1.5e3}"#)
                .expect("合法 JSON");
        assert_eq!(root.path_str(&["url"]), Some("https://x/ab.png"));
        assert_eq!(root.path_str(&["emoji"]), Some("😀"));
    }

    #[test]
    fn collects_string_object_entries_only() {
        let root = parse(br#"{"body":{"key":"v","n":1,"file":""},"other":[]}"#).expect("合法 JSON");
        assert_eq!(
            root.string_entries("body"),
            vec![
                ("key".to_owned(), "v".to_owned()),
                ("file".to_owned(), String::new()),
            ]
        );
        assert!(root.string_entries("missing").is_empty());
        assert!(root.string_entries("other").is_empty());
    }

    #[test]
    fn parses_arrays_and_literals_around_target_field() {
        let root = parse(b"[1,true,null,{\"a\":[{\"b\":\"c\"}]}]\n").expect("数组可解析");
        assert_eq!(root, Value::Other);
        let root = parse(br#"{"list":[1,2],"resourceUrl":"https://r/u.png"}"#).expect("合法 JSON");
        assert_eq!(root.path_str(&["resourceUrl"]), Some("https://r/u.png"));
    }

    #[test]
    fn rejects_malformed_input() {
        assert_eq!(parse(b""), None);
        assert_eq!(parse(b"not json"), None);
        assert_eq!(parse(b"{\"a\":}"), None);
        assert_eq!(parse(b"{\"a\":\"b\"}trailing"), None, "拒绝尾随垃圾");
        assert_eq!(parse(b"{\"a\":\"b}"), None, "缺右引号");
        assert_eq!(parse(b"{\"a\":\"line\nbreak\"}"), None, "未转义换行");
        assert_eq!(parse(b"{\"a\":\"\\q\"}"), None, "非法转义");
        assert_eq!(parse(b"{\"a\":1e}"), None, "指数缺位");
        assert_eq!(parse(&[0xFF, 0xFE]), None, "非 UTF-8");
    }
}
