//! 最小 base64（STANDARD alphabet + `=` 填充）编解码。
//!
//! markdown / keyboard 的 `foundItems` 契约是 base64（`handlers/message_parser.go:1058`、
//! `handlers/cqcode.go:125`），但本 crate 不引入 `base64` 依赖：workspace 声明的版本尚未被
//! 任何成员使用，加依赖会让 `--locked` / `--offline` 构建失效。协议层只需要一种固定 alphabet，
//! 这里的实现只为契约服务，不追求通用性。

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 编码（Go `base64.StdEncoding` 同款：标准 alphabet + `=` 填充）。
pub(crate) fn encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).map_or(0, |byte| u32::from(*byte));
        let b2 = chunk.get(2).map_or(0, |byte| u32::from(*byte));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(char::from(ALPHABET[(triple >> 18) as usize & 63]));
        out.push(char::from(ALPHABET[(triple >> 12) as usize & 63]));
        out.push(if chunk.len() > 1 {
            char::from(ALPHABET[(triple >> 6) as usize & 63])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(ALPHABET[triple as usize & 63])
        } else {
            '='
        });
    }
    out
}

/// 严格解码：长度非 4 的倍数、含非 alphabet 字符或填充位置非法时返回 `None`。
pub(crate) fn decode(input: &str) -> Option<Vec<u8>> {
    let bytes = input.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let a = value_of(chunk[0])?;
        let b = value_of(chunk[1])?;
        let c = match chunk[2] {
            b'=' => None,
            byte => Some(value_of(byte)?),
        };
        let d = match chunk[3] {
            b'=' => None,
            byte => Some(value_of(byte)?),
        };
        out.push((a << 2) | (b >> 4));
        if let Some(c) = c {
            out.push(((b & 0x0f) << 4) | (c >> 2));
            if let Some(d) = d {
                out.push(((c & 0x03) << 6) | d);
            }
        }
    }
    Some(out)
}

fn value_of(byte: u8) -> Option<u8> {
    ALPHABET
        .iter()
        .position(|candidate| *candidate == byte)
        .map(|index| index as u8)
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    #[test]
    fn encodes_with_standard_padding() {
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"a"), "YQ==");
        assert_eq!(encode(b"ab"), "YWI=");
        assert_eq!(encode(b"abc"), "YWJj");
        assert_eq!(encode(br#"{"content":"hi"}"#), "eyJjb250ZW50IjoiaGkifQ==");
    }

    #[test]
    fn decodes_back_to_original_bytes() {
        for raw in [
            &b""[..],
            b"a",
            b"ab",
            b"abc",
            br#"{"content":"hi"}"#,
            &[0_u8, 255, 128, 1, 2],
        ] {
            let encoded = encode(raw);
            assert_eq!(decode(&encoded).as_deref(), Some(raw), "往返: {encoded}");
        }
    }

    #[test]
    fn rejects_malformed_input() {
        assert_eq!(decode("YQ="), None, "长度非 4 的倍数");
        assert_eq!(decode("YQ!="), None, "非 alphabet 字符");
    }
}
