//! `data:` URI 解析与 CQ 用 base64 生成。
//!
//! 对应 Go 侧把 `data:image/png;base64,...` 拆成「媒体类型 + 字节」的零散逻辑，
//! 这里收敛成 [`parse`]、[`to_data_uri`]、[`cq_base64`] 三个入口。
//!
//! base64 编解码用 workspace 的 `base64` crate（`GeneralPurpose` 引擎），配置见 [`ENGINE`]：
//! 标准字母表、填充模式 `Indifferent`（有无 `=` 都接受）、允许尾随位，解码前剥离 ASCII 空白。
//! 与 Go `base64.StdEncoding` 的差异只在这些放宽项：OneBot 客户端经常省略填充或把长负载
//! 换行，严格模式会误判为非法。

use base64::engine::DecodePaddingMode;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::{Engine as _, alphabet};

use crate::error::{Error, Result};
use crate::source::BASE64_PREFIX;

/// `data:` 前缀。
pub const SCHEME: &str = "data:";

/// 解码引擎：标准字母表 + 宽松填充（缺失/多余填充均可）。
const ENGINE: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        // Go 的非严格模式同样不校验尾随位，保持逐字节兼容
        .with_decode_allow_trailing_bits(true),
);

/// 解析后的 `data:` URI。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataUri {
    /// 媒体类型（`data:` 与第一个 `;` 之间），缺省为 `None`。
    pub mime: Option<String>,
    /// 解码后的字节。
    pub data: Vec<u8>,
    /// 原始负载是否声明了 `;base64`（否则按百分号转义解码）。
    pub base64: bool,
}

/// 判断字符串是否是 `data:` URI（scheme 大小写不敏感）。
#[must_use]
pub fn is_data_uri(raw: &str) -> bool {
    raw.len() >= SCHEME.len() && raw.as_bytes()[..SCHEME.len()].eq_ignore_ascii_case(b"data:")
}

/// 解析 `data:[<mime>][;base64],<payload>`。
///
/// - `;base64` 负载按标准 base64 解码；
/// - 未声明 `;base64` 时按百分号转义解码（`+` 不当作空格，与 URI 语义一致）；
/// - 其他参数（如 `charset=utf-8`）忽略：M2a 不消费文本字符集。
pub fn parse(raw: &str) -> Result<DataUri> {
    if !is_data_uri(raw) {
        return Err(Error::DataUri("前缀不是 `data:`".to_owned()));
    }
    let (header, payload) = raw[SCHEME.len()..]
        .split_once(',')
        .ok_or_else(|| Error::DataUri("缺少 `,` 分隔符".to_owned()))?;

    let mut parts = header.split(';');
    let mime = parts
        .next()
        .map(str::trim)
        .filter(|mime| !mime.is_empty())
        .map(str::to_owned);
    let mut is_base64 = false;
    for flag in parts {
        if flag.trim().eq_ignore_ascii_case("base64") {
            is_base64 = true;
        }
    }

    let data = if is_base64 {
        decode_base64(payload)?
    } else {
        percent_decode(payload)
    };
    Ok(DataUri {
        mime,
        data,
        base64: is_base64,
    })
}

/// 生成 `data:<mime>;base64,<payload>`。
#[must_use]
pub fn to_data_uri(mime: &str, data: &[u8]) -> String {
    format!("data:{mime};base64,{}", encode_base64(data))
}

/// 生成 OneBot / CQ 码里的 `base64://<payload>`。
#[must_use]
pub fn cq_base64(data: &[u8]) -> String {
    format!("{BASE64_PREFIX}{}", encode_base64(data))
}

/// 标准 base64 编码（带 `=` 填充，供 runtime / actions 复用）。
#[must_use]
pub fn encode_base64(data: &[u8]) -> String {
    ENGINE.encode(data)
}

/// 标准 base64 解码（容忍 ASCII 空白与缺失填充，供 runtime / actions 复用）。
pub fn decode_base64(input: &str) -> Result<Vec<u8>> {
    let compact: String = input
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect();
    ENGINE
        .decode(compact.as_bytes())
        .map_err(|err| Error::Base64(err.to_string()))
}

/// `%XX` 解码；非法转义原样保留（Go `url.PathUnescape` 的宽松版）。
pub(crate) fn percent_decode(raw: &str) -> Vec<u8> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = char::from(bytes[index + 1]).to_digit(16);
            let low = char::from(bytes[index + 2]).to_digit(16);
            if let (Some(high), Some(low)) = (high, low) {
                out.push((high * 16 + low) as u8);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        DataUri, cq_base64, decode_base64, encode_base64, is_data_uri, parse, to_data_uri,
    };

    #[test]
    fn parses_base64_png_data_uri() {
        let uri = parse("data:image/png;base64,iVBORw0KGgo=").unwrap();
        assert_eq!(
            uri,
            DataUri {
                mime: Some("image/png".to_owned()),
                data: b"\x89PNG\r\n\x1a\n".to_vec(),
                base64: true,
            }
        );
    }

    #[test]
    fn parses_default_mime_and_extra_parameters() {
        let uri = parse("data:;charset=utf-8;base64,Zm9v").unwrap();
        assert_eq!(uri.mime, None);
        assert_eq!(uri.data, b"foo");
        assert!(uri.base64);
    }

    #[test]
    fn decodes_percent_escaped_payload_without_base64_flag() {
        let uri = parse("data:text/plain,a%20b%2Fc").unwrap();
        assert_eq!(uri.mime.as_deref(), Some("text/plain"));
        assert_eq!(uri.data, b"a b/c");
        assert!(!uri.base64);
    }

    #[test]
    fn rejects_non_data_uri_and_missing_comma() {
        assert!(parse("https://example.com/a.png").is_err());
        assert!(parse("data:image/png;base64").is_err());
    }

    #[test]
    fn generates_data_uri_and_cq_base64() {
        assert_eq!(
            to_data_uri("image/png", b"foo"),
            "data:image/png;base64,Zm9v"
        );
        assert_eq!(cq_base64(b"foo"), "base64://Zm9v");
        assert_eq!(decode_base64("Zm9v").unwrap(), b"foo");
    }

    #[test]
    fn scheme_check_is_case_insensitive() {
        assert!(is_data_uri("DATA:image/png;base64,Zm9v"));
        assert!(!is_data_uri("metadata:foo"));
    }

    #[test]
    fn matches_go_std_encoding_vectors() {
        // RFC 4648 §10 的向量，与 Go `base64.StdEncoding` 逐字节一致
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        assert_eq!(encode_base64(b"foob"), "Zm9vYg==");
        assert_eq!(encode_base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode_base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn round_trips_binary() {
        let payload: Vec<u8> = (0..=255).collect();
        assert_eq!(decode_base64(&encode_base64(&payload)).unwrap(), payload);
    }

    #[test]
    fn decode_ignores_whitespace_and_missing_padding() {
        assert_eq!(decode_base64(" Zm9v\nYmFy\t").unwrap(), b"foobar");
        assert_eq!(decode_base64("Zg").unwrap(), b"f");
        assert_eq!(decode_base64("Zm8").unwrap(), b"fo");
    }

    #[test]
    fn decode_rejects_invalid_input() {
        assert!(decode_base64("Zm9vY").is_err(), "长度 mod 4 == 1 非法");
        assert!(decode_base64("Zm=v").is_err(), "填充符不在末尾");
        assert!(decode_base64("Zm9v!").is_err(), "非字母表字符");
        assert!(decode_base64("Zg===").is_err(), "填充符超过 2 个");
    }
}
