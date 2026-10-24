//! OneBot `file` 字段 → [`MediaSource`]。
//!
//! 对齐 Go `internal/domain/message/media.go` 的 `MediaSourceFromFile` 与
//! `handlers/message_parser.go` 的 `safeLocalPath`：`base64://`、`http(s)://`、`file://`
//! 与裸路径。本 crate 额外支持 `data:` URI 与裸 base64（M2a 交付要求）。

use std::path::PathBuf;

use crate::data_uri;
use crate::error::{Error, Result};
use crate::sniff;

/// `base64://` 前缀（OneBot / CQ 码约定）。
pub const BASE64_PREFIX: &str = "base64://";
/// `file://` 前缀。
pub const FILE_PREFIX: &str = "file://";

/// 媒体来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaSource {
    /// http(s) 远程地址（原样保留，不在解析阶段校验）。
    Url(String),
    /// 本地路径；`file://` 前缀已剥离，Windows 盘符前的 `/` 已去掉，并已转绝对路径。
    File(PathBuf),
    /// 内联 base64；`mime` 仅在 `data:` URI 时已知。
    Base64 {
        /// `data:` URI 声明的媒体类型。
        mime: Option<String>,
        /// 解码后的原始字节。
        data: Vec<u8>,
    },
}

impl MediaSource {
    /// 按 OneBot `file` 字段值推断来源。
    ///
    /// 判定顺序：`base64://` → `data:` → `http(s)://` → `file://` → 其他 `scheme://`（拒绝）
    /// → 裸 base64 → 普通路径。scheme 比较大小写不敏感。
    pub fn parse(raw: &str) -> Result<Self> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(Error::InvalidSource("空字符串".to_owned()));
        }

        if let Some(payload) = strip_scheme(raw, BASE64_PREFIX) {
            let payload = payload.trim();
            if payload.is_empty() {
                return Err(Error::InvalidSource("base64:// 后没有负载".to_owned()));
            }
            return Ok(Self::Base64 {
                mime: None,
                data: data_uri::decode_base64(payload)?,
            });
        }
        if data_uri::is_data_uri(raw) {
            let uri = data_uri::parse(raw)?;
            return Ok(Self::Base64 {
                mime: uri.mime,
                data: uri.data,
            });
        }
        if strip_scheme(raw, "http://").is_some() || strip_scheme(raw, "https://").is_some() {
            return Ok(Self::Url(raw.to_owned()));
        }
        if let Some(rest) = strip_scheme(raw, FILE_PREFIX) {
            return Ok(Self::File(parse_file_path(rest)?));
        }
        if let Some(scheme_end) = raw.find("://") {
            return Err(Error::UnsupportedScheme(raw[..scheme_end].to_owned()));
        }
        if let Some(data) = decode_bare_base64(raw) {
            return Ok(Self::Base64 { mime: None, data });
        }
        Ok(Self::File(PathBuf::from(raw)))
    }

    /// 已知的媒体类型（只有 `data:` URI 会带）。
    #[must_use]
    pub fn declared_mime(&self) -> Option<&str> {
        match self {
            Self::Base64 { mime, .. } => mime.as_deref(),
            Self::Url(_) | Self::File(_) => None,
        }
    }
}

/// 大小写不敏感的 scheme 前缀剥离（`HTTP://` 与 `http://` 等价）。
fn strip_scheme<'a>(raw: &'a str, prefix: &str) -> Option<&'a str> {
    let head = raw.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then_some(&raw[prefix.len()..])
}

/// 剥离前缀后的 `file://` 路径：百分号解码 → 拒绝 `..` → 转绝对路径。
///
/// 对齐 Go `handlers/message_parser.go` 的 `safeLocalPath`：`..` 必须在 `Clean` 之前拒绝，
/// 否则 `/../` 会被折叠掉、绕过校验；随后统一转绝对路径，避免相对路径逃出工作目录。
fn parse_file_path(rest: &str) -> Result<PathBuf> {
    let decoded = String::from_utf8_lossy(&data_uri::percent_decode(rest)).into_owned();
    if decoded.is_empty() {
        return Err(Error::InvalidSource("file:// 后路径为空".to_owned()));
    }
    if decoded.contains("..") {
        return Err(Error::InvalidSource(format!(
            "file:// 路径包含非法字符 `..`: {decoded}"
        )));
    }
    // Windows 写法 `file:///C:/x` 剥前缀后是 `/C:/x`，盘符前的 `/` 要去掉
    let path = match decoded.strip_prefix('/') {
        Some(stripped) if is_drive_path(stripped) => stripped,
        _ => decoded.as_str(),
    };
    std::path::absolute(path)
        .map_err(|err| Error::InvalidSource(format!("file:// 路径无法转为绝对路径: {err}")))
}

fn is_drive_path(candidate: &str) -> bool {
    let bytes = candidate.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// 裸 base64 的保守判定：避免把普通文件名（尤其纯数字名）当内联数据。
///
/// 只接受两类：
/// 1. 解码后能嗅出媒体头（`\x89PNG` 等）——M2a 真正关心的场景；
/// 2. 长度 ≥ [`BARE_BASE64_MIN_LEN`] 且带 `=` / `+` 的标准串——非图片媒体走这条。
///
/// 字母表仍然刻意不含 `/`：`dir/file` 这类相对路径同样满足字母表，误判会把真实文件
/// 当内联数据。需要 `/` 的 base64 请用 `base64://` 前缀或 `data:` URI。
fn decode_bare_base64(raw: &str) -> Option<Vec<u8>> {
    const MIN_LEN: usize = 8;
    const BARE_BASE64_MIN_LEN: usize = 32;

    if raw.len() < MIN_LEN || !raw.len().is_multiple_of(4) {
        return None;
    }
    if !raw
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'='))
    {
        return None;
    }
    let data = data_uri::decode_base64(raw).ok()?;
    if sniff::sniff_image(&data).is_some() {
        return Some(data);
    }
    let has_marker = raw.contains('=') || raw.contains('+');
    (raw.len() >= BARE_BASE64_MIN_LEN && has_marker).then_some(data)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::MediaSource;
    use crate::error::Error;

    const PNG_HEADER: &[u8] = b"\x89PNG\r\n\x1a\n";

    #[test]
    fn parses_base64_prefix() {
        assert_eq!(
            MediaSource::parse("base64://Zm9v").unwrap(),
            MediaSource::Base64 {
                mime: None,
                data: b"foo".to_vec(),
            }
        );
    }

    #[test]
    fn parses_data_uri_with_mime() {
        assert_eq!(
            MediaSource::parse("data:image/jpeg;base64,/9j/").unwrap(),
            MediaSource::Base64 {
                mime: Some("image/jpeg".to_owned()),
                data: vec![0xFF, 0xD8, 0xFF],
            }
        );
    }

    #[test]
    fn parses_http_and_https() {
        assert_eq!(
            MediaSource::parse("https://example.com/a.png").unwrap(),
            MediaSource::Url("https://example.com/a.png".to_owned())
        );
        assert_eq!(
            MediaSource::parse("http://example.com/a.png").unwrap(),
            MediaSource::Url("http://example.com/a.png".to_owned())
        );
    }

    #[test]
    fn accepts_uppercase_schemes() {
        assert_eq!(
            MediaSource::parse("HTTP://example.com/a.png").unwrap(),
            MediaSource::Url("HTTP://example.com/a.png".to_owned())
        );
        assert_eq!(
            MediaSource::parse("HtTpS://example.com/a.png").unwrap(),
            MediaSource::Url("HtTpS://example.com/a.png".to_owned())
        );
        assert_eq!(
            MediaSource::parse("BASE64://Zm9v").unwrap(),
            MediaSource::Base64 {
                mime: None,
                data: b"foo".to_vec(),
            }
        );
        // `file://` 同样大小写不敏感：剥掉前缀后仍走安全化路径
        assert!(matches!(
            MediaSource::parse("FILE:///tmp/a.png"),
            Ok(MediaSource::File(_))
        ));
    }

    #[test]
    fn parses_file_uri_and_decodes_percent_escapes() {
        let parsed = MediaSource::parse("file:///tmp/a%20b.png").unwrap();
        let MediaSource::File(path) = parsed else {
            panic!("应解析为本地文件: {parsed:?}");
        };
        assert!(path.is_absolute(), "file:// 必须转绝对路径: {path:?}");
        assert!(path.ends_with(Path::new("tmp").join("a b.png")), "{path:?}");
    }

    #[test]
    fn strips_drive_slash_on_windows_style_file_uri() {
        let expected = std::path::absolute("C:/media/a.png").expect("转绝对路径");
        assert_eq!(
            MediaSource::parse("file:///C:/media/a.png").unwrap(),
            MediaSource::File(expected)
        );
    }

    #[test]
    fn rejects_file_uri_with_parent_components() {
        // 对齐 Go `safeLocalPath`：百分号解码后仍含 `..` 即拒绝
        for raw in [
            "file://../../etc/passwd",
            "file:///tmp/%2e%2e/secret",
            "file:///C:/a/../b.png",
        ] {
            assert!(
                matches!(MediaSource::parse(raw), Err(Error::InvalidSource(_))),
                "{raw} 必须被拒绝"
            );
        }
    }

    #[test]
    fn treats_plain_and_relative_paths_as_files() {
        assert_eq!(
            MediaSource::parse("C:/media/a.png").unwrap(),
            MediaSource::File(PathBuf::from("C:/media/a.png"))
        );
        assert_eq!(
            MediaSource::parse("dir/file").unwrap(),
            MediaSource::File(PathBuf::from("dir/file"))
        );
        assert_eq!(
            MediaSource::parse("./a.png").unwrap(),
            MediaSource::File(PathBuf::from("./a.png"))
        );
    }

    #[test]
    fn parses_bare_base64_with_media_header() {
        assert_eq!(
            MediaSource::parse("iVBORw0KGgo=").unwrap(),
            MediaSource::Base64 {
                mime: None,
                data: PNG_HEADER.to_vec(),
            }
        );
    }

    #[test]
    fn parses_long_bare_base64_with_padding_or_plus() {
        // 非图片（40 字节 0xAA）嗅不出媒体头，但够长且带 `=`，仍按内联数据处理
        let encoded = crate::data_uri::encode_base64(&[0xAA; 40]);
        assert!(encoded.contains('='), "{encoded}");
        assert_eq!(
            MediaSource::parse(&encoded).unwrap(),
            MediaSource::Base64 {
                mime: None,
                data: vec![0xAA; 40],
            }
        );
    }

    #[test]
    fn keeps_plain_filenames_out_of_bare_base64() {
        // 纯数字名、md5 样式的长十六进制名：字母表合法但既嗅不出媒体头也没有 `=`/`+`
        for raw in [
            "20240101",
            "12345678",
            "d41d8cd98f00b204e9800998ecf8427e",
            "deadbeef",
        ] {
            assert_eq!(
                MediaSource::parse(raw).unwrap(),
                MediaSource::File(PathBuf::from(raw)),
                "{raw} 应按路径处理"
            );
        }
    }

    #[test]
    fn rejects_empty_and_unknown_scheme() {
        assert!(matches!(
            MediaSource::parse("  "),
            Err(Error::InvalidSource(_))
        ));
        assert!(matches!(
            MediaSource::parse("ftp://example.com/a.png"),
            Err(Error::UnsupportedScheme(scheme)) if scheme == "ftp"
        ));
        assert!(matches!(
            MediaSource::parse("file://"),
            Err(Error::InvalidSource(_))
        ));
    }

    #[test]
    fn declared_mime_only_for_base64() {
        let data_uri = MediaSource::parse("data:image/webp;base64,AAAA").unwrap();
        assert_eq!(data_uri.declared_mime(), Some("image/webp"));
        assert_eq!(
            MediaSource::parse("https://example.com/a")
                .unwrap()
                .declared_mime(),
            None
        );
    }
}
