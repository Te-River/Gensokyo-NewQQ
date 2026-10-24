//! 图片格式识别：只看文件头，不解码像素。
//!
//! 对齐 Go `images/format.go`（PNG / JPEG / GIF 的签名判断），并补上 WebP / BMP。
//! 与 `format.go` 的差异：GIF 收紧到 `GIF87a` / `GIF89a`（Go 只查 3 字节 `GIF` 前缀），
//! 避免把任意 `GIF` 开头的文本当成图片。

/// 识别到的图片容器格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageKind {
    /// PNG（`\x89PNG\r\n\x1a\n`）。
    Png,
    /// JPEG（`FF D8 FF`）。
    Jpeg,
    /// GIF（`GIF87a` / `GIF89a`）。
    Gif,
    /// WebP（`RIFF....WEBP`）。
    WebP,
    /// BMP（`BM`）。
    Bmp,
}

impl ImageKind {
    /// 对应的 `Content-Type`。
    #[must_use]
    pub const fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::WebP => "image/webp",
            Self::Bmp => "image/bmp",
        }
    }

    /// 常用扩展名（不带点）。
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Gif => "gif",
            Self::WebP => "webp",
            Self::Bmp => "bmp",
        }
    }
}

/// PNG 文件头。
const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

/// 按文件头识别图片格式；只可能是图片容器，其他内容返回 `None`。
#[must_use]
pub fn sniff_image(bytes: &[u8]) -> Option<ImageKind> {
    if bytes.starts_with(PNG_MAGIC) {
        return Some(ImageKind::Png);
    }
    if bytes.starts_with(b"\xFF\xD8\xFF") {
        return Some(ImageKind::Jpeg);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(ImageKind::Gif);
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(ImageKind::WebP);
    }
    if bytes.starts_with(b"BM") {
        return Some(ImageKind::Bmp);
    }
    None
}

/// 嗅探失败时的兜底 `Content-Type`（与 Go `http.DetectContentType` 一致）。
pub const FALLBACK_CONTENT_TYPE: &str = "application/octet-stream";

/// `Content-Type`：能识别的图片取 [`ImageKind::mime`]，否则 [`FALLBACK_CONTENT_TYPE`]。
///
/// 等价 Go 的 `http.DetectContentType` 在媒体场景下的期望值（只覆盖 M2a 需要的格式）。
#[must_use]
pub fn sniff_content_type(bytes: &[u8]) -> &'static str {
    sniff_image(bytes).map_or(FALLBACK_CONTENT_TYPE, ImageKind::mime)
}

#[cfg(test)]
mod tests {
    use super::{ImageKind, sniff_content_type, sniff_image};

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    const JPEG: &[u8] = b"\xFF\xD8\xFF\xE0\x00\x10JFIF";
    const GIF87: &[u8] = b"GIF87a\x01\x00\x01\x00";
    const GIF89: &[u8] = b"GIF89a\x01\x00\x01\x00";
    const WEBP: &[u8] = b"RIFF\x24\x00\x00\x00WEBPVP8 ";

    #[test]
    fn recognizes_minimal_headers() {
        assert_eq!(sniff_image(PNG), Some(ImageKind::Png));
        assert_eq!(sniff_image(JPEG), Some(ImageKind::Jpeg));
        assert_eq!(sniff_image(GIF87), Some(ImageKind::Gif));
        assert_eq!(sniff_image(GIF89), Some(ImageKind::Gif));
        assert_eq!(sniff_image(WEBP), Some(ImageKind::WebP));
        assert_eq!(sniff_image(b"BM\x36\x00\x00\x00"), Some(ImageKind::Bmp));
    }

    #[test]
    fn rejects_unknown_and_truncated_input() {
        assert_eq!(sniff_image(b""), None);
        assert_eq!(sniff_image(b"hello world"), None);
        assert_eq!(sniff_image(b"\x89PN"), None, "PNG 头被截断");
        assert_eq!(
            sniff_image(b"RIFF\x24\x00\x00\x00WAVE"),
            None,
            "RIFF 但不是 WEBP"
        );
        assert_eq!(sniff_image(b"GIFT"), None, "只有 GIF 三字节前缀不算");
    }

    #[test]
    fn maps_content_type() {
        assert_eq!(sniff_content_type(PNG), "image/png");
        assert_eq!(sniff_content_type(JPEG), "image/jpeg");
        assert_eq!(sniff_content_type(WEBP), "image/webp");
        assert_eq!(
            sniff_content_type(b"not an image"),
            "application/octet-stream"
        );
    }

    #[test]
    fn extensions_are_stable() {
        assert_eq!(ImageKind::Png.extension(), "png");
        assert_eq!(ImageKind::Jpeg.extension(), "jpg");
        assert_eq!(ImageKind::Gif.extension(), "gif");
        assert_eq!(ImageKind::WebP.extension(), "webp");
        assert_eq!(ImageKind::Bmp.extension(), "bmp");
    }
}
