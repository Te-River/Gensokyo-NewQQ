//! 图片尺寸探测：只读文件头，不解码像素。
//!
//! 覆盖 PNG / JPEG / GIF / WebP（VP8 / VP8L / VP8X）与 BMP，对齐 Go `images/format.go`
//! 的 `GetImageDimensions`。与 Go 的差异：
//! - Go 先按 URL 后缀分派、失败后再逐个方法重试，每次都重新下载同一张图；本模块只处理
//!   已在内存里的字节，分派完全依据文件头，扩展名不参与判断。
//! - Go 的 JPEG 分支逐字节扫描 `FF`，遇到非标记字节也能继续；本模块要求标记严格相邻，
//!   损坏数据宁可不给尺寸（避免把像素数据里的 `FF C0` 当帧头）。
//! - Go 的 `getJpegDimensions` 返回值写反了（`return int(height), int(width)`），本模块
//!   统一返回 `(宽, 高)`。
//! - WebP / BMP 是 Go 侧没有的格式（Go 只做 PNG / JPEG / GIF），本模块补齐。

use crate::sniff::{self, ImageKind};

/// 从文件头读出 `(宽, 高)`；格式不支持或数据被截断时返回 `None`。
#[must_use]
pub fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    match sniff::sniff_image(bytes)? {
        ImageKind::Png => png_size(bytes),
        ImageKind::Jpeg => jpeg_size(bytes),
        ImageKind::Gif => gif_size(bytes),
        ImageKind::WebP => webp_size(bytes),
        ImageKind::Bmp => bmp_size(bytes),
    }
}

/// PNG：签名(8) + IHDR 长度(4) + `"IHDR"`(4) + 宽(4) + 高(4)，宽高都是大端 `u32`。
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    // IHDR 必须是第一个块；不是就当不是合法 PNG
    if bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    let width = u32_be(bytes, 16)?;
    let height = u32_be(bytes, 20)?;
    (width > 0 && height > 0).then_some((width, height))
}

/// JPEG：跳过各段长度字段，在 SOF0/2（以及其它 SOFn）里取高宽。
///
/// SOFn 的 payload 依次是：精度(1) / 高(2) / 宽(2) / 分量数(1) / 分量描述(N)，均为大端。
fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut index = 2; // 跳过 SOI（FF D8）
    while index < bytes.len() {
        if bytes[index] != 0xFF {
            return None;
        }
        // 标记前允许任意多个 FF 填充字节（ISO/IEC 10918-1 B.1.1.2）
        let mut marker_index = index + 1;
        while bytes.get(marker_index) == Some(&0xFF) {
            marker_index += 1;
        }
        let marker = *bytes.get(marker_index)?;
        match marker {
            // 无长度字段的独立标记：TEM / RSTn / SOI / EOI
            0x01 | 0xD0..=0xD7 | 0xD8 | 0xD9 => {
                index = marker_index + 1;
                continue;
            }
            // 扫描数据从 SOS 开始；SOF 一定在它之前，走到这里说明没有帧头
            0xDA => return None,
            _ => {}
        }
        // 段长度包含长度字段自身，所以最小是 2
        let length = usize::from(u16_be(bytes, marker_index + 1)?);
        if length < 2 {
            return None;
        }
        // SOF0..SOF15 里 C4(DHT) / C8(JPG) / CC(DAC) 不是帧头
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let payload = marker_index + 3;
            let height = u32::from(u16_be(bytes, payload + 1)?);
            let width = u32::from(u16_be(bytes, payload + 3)?);
            return (width > 0 && height > 0).then_some((width, height));
        }
        index = marker_index + 1 + length;
    }
    None
}

/// GIF：逻辑屏幕宽高是小端 `u16`，紧跟 6 字节签名。
fn gif_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let width = u32::from(u16_le(bytes, 6)?);
    let height = u32::from(u16_le(bytes, 8)?);
    (width > 0 && height > 0).then_some((width, height))
}

/// WebP：`RIFF....WEBP` 之后是第一个块，按块类型分三种容器。
fn webp_size(bytes: &[u8]) -> Option<(u32, u32)> {
    match bytes.get(12..16)? {
        // 有损：3 字节帧标签 + 起始码 9D 01 2A + 14 位宽 + 14 位高（小端，低位对齐）
        b"VP8 " => {
            if bytes.get(23..26)? != b"\x9D\x01\x2A" {
                return None;
            }
            let width = u32::from(u16_le(bytes, 26)? & 0x3FFF);
            let height = u32::from(u16_le(bytes, 28)? & 0x3FFF);
            (width > 0 && height > 0).then_some((width, height))
        }
        // 无损：`0x2F` 签名 + 4 字节位域 (宽-1):14 | (高-1):14
        b"VP8L" => {
            if bytes.get(20) != Some(&0x2F) {
                return None;
            }
            let bits = u32_le(bytes, 21)?;
            Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
        }
        // 扩展：4 字节标志位之后是 24 位画布宽-1 / 高-1（小端）
        b"VP8X" => {
            let width = u24_le(bytes, 24)? + 1;
            let height = u24_le(bytes, 27)? + 1;
            Some((width, height))
        }
        _ => None,
    }
}

/// BMP：DIB 头有 12 字节（BITMAPCOREHEADER，宽高 `u16`）与 ≥40 字节（`i32`，高度可为负）。
fn bmp_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let header_size = u32_le(bytes, 14)?;
    if header_size == 12 {
        let width = u32::from(u16_le(bytes, 18)?);
        let height = u32::from(u16_le(bytes, 20)?);
        return (width > 0 && height > 0).then_some((width, height));
    }
    let width = i32::from_le_bytes(bytes.get(18..22)?.try_into().ok()?);
    let height = i32::from_le_bytes(bytes.get(22..26)?.try_into().ok()?);
    // 自顶向下位图的高度为负数
    let (width, height) = (width.unsigned_abs(), height.unsigned_abs());
    (width > 0 && height > 0).then_some((width, height))
}

fn u16_be(bytes: &[u8], offset: usize) -> Option<u16> {
    bytes
        .get(offset..offset + 2)?
        .try_into()
        .ok()
        .map(u16::from_be_bytes)
}

fn u16_le(bytes: &[u8], offset: usize) -> Option<u16> {
    bytes
        .get(offset..offset + 2)?
        .try_into()
        .ok()
        .map(u16::from_le_bytes)
}

fn u32_be(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes
        .get(offset..offset + 4)?
        .try_into()
        .ok()
        .map(u32::from_be_bytes)
}

fn u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes
        .get(offset..offset + 4)?
        .try_into()
        .ok()
        .map(u32::from_le_bytes)
}

fn u24_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let bytes = bytes.get(offset..offset + 3)?;
    Some(u32::from(bytes[0]) | (u32::from(bytes[1]) << 8) | (u32::from(bytes[2]) << 16))
}

#[cfg(test)]
mod tests {
    use super::image_size;
    use crate::data_uri::decode_base64;

    /// 640x480 的 PNG 头（签名 + IHDR + 位深/颜色类型/压缩/滤波/隔行）。
    const PNG_640_480: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0DIHDR\x00\x00\x02\x80\x00\x00\x01\xE0\x08\x06\x00\x00\x00";

    /// 1x1 PNG（真实文件，含 CRC 的 IHDR）——证明真实产物也能解析。
    const PNG_1_1_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==";

    /// SOI + APP0(JFIF) + SOF0(高 2 / 宽 3) + EOI。
    const JPEG_BASELINE_3_2: &[u8] = b"\xFF\xD8\
        \xFF\xE0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00\
        \xFF\xC0\x00\x11\x08\x00\x02\x00\x03\x03\x01\x22\x00\x02\x11\x01\x03\x11\x01\
        \xFF\xD9";

    /// 同上，但帧头是 SOF2（渐进式）；SOF 之前没有 APP 段。
    const JPEG_PROGRESSIVE_3_2: &[u8] = b"\xFF\xD8\
        \xFF\xC2\x00\x11\x08\x00\x02\x00\x03\x03\x01\x22\x00\x02\x11\x01\x03\x11\x01\
        \xFF\xD9";

    /// GIF89a，逻辑屏幕 320x240（小端）。
    const GIF_320_240: &[u8] = b"GIF89a\x40\x01\xF0\x00\x00\x00\x00";

    /// WebP 有损：RIFF/WEBP + `VP8 `，起始码 + 400x300。
    const WEBP_VP8_400_300: &[u8] = b"RIFF\x26\x00\x00\x00WEBPVP8 \x1A\x00\x00\x00\
        \x30\x01\x00\x9D\x01\x2A\x90\x01\x2C\x01";

    /// WebP 无损：`VP8L`，位域 (5-1) | (6-1)<<14。
    const WEBP_VP8L_5_6: &[u8] =
        b"RIFF\x1A\x00\x00\x00WEBPVP8L\x12\x00\x00\x00\x2F\x04\x40\x01\x00";

    /// WebP 扩展：`VP8X`，画布 800x600（宽-1 / 高-1 均为 24 位小端）。
    const WEBP_VP8X_800_600: &[u8] =
        b"RIFF\x2A\x00\x00\x00WEBPVP8X\x00\x00\x00\x00\x00\x00\x00\x00\x1F\x03\x00\x57\x02\x00";

    #[test]
    fn parses_png_ihdr() {
        assert_eq!(image_size(PNG_640_480), Some((640, 480)));
        let real = decode_base64(PNG_1_1_BASE64).expect("样例 base64 可解码");
        assert_eq!(image_size(&real), Some((1, 1)));
    }

    #[test]
    fn parses_jpeg_sof0_and_sof2() {
        assert_eq!(image_size(JPEG_BASELINE_3_2), Some((3, 2)));
        assert_eq!(image_size(JPEG_PROGRESSIVE_3_2), Some((3, 2)));
    }

    #[test]
    fn parses_gif_logical_screen() {
        assert_eq!(image_size(GIF_320_240), Some((320, 240)));
        assert_eq!(
            image_size(b"GIF87a\x01\x00\x02\x00\x00\x00\x00"),
            Some((1, 2))
        );
    }

    #[test]
    fn parses_all_webp_containers() {
        assert_eq!(image_size(WEBP_VP8_400_300), Some((400, 300)));
        assert_eq!(image_size(WEBP_VP8L_5_6), Some((5, 6)));
        assert_eq!(image_size(WEBP_VP8X_800_600), Some((800, 600)));
    }

    #[test]
    fn parses_bmp_both_header_sizes() {
        // BITMAPINFOHEADER：320x240，高度为正（自底向上）
        let info = b"BM\x36\x00\x00\x00\x00\x00\x00\x00\x36\x00\x00\x00\
            \x28\x00\x00\x00\x40\x01\x00\x00\xF0\x00\x00\x00\x01\x00\x18\x00";
        assert_eq!(image_size(info), Some((320, 240)));
        // 同一张图自顶向下（高度为负）
        let top_down = b"BM\x36\x00\x00\x00\x00\x00\x00\x00\x36\x00\x00\x00\
            \x28\x00\x00\x00\x40\x01\x00\x00\x10\xFF\xFF\xFF\x01\x00\x18\x00";
        assert_eq!(image_size(top_down), Some((320, 240)));
        // BITMAPCOREHEADER：宽高是 u16
        let core = b"BM\x36\x00\x00\x00\x00\x00\x00\x00\x36\x00\x00\x00\
            \x0C\x00\x00\x00\x40\x01\xF0\x00\x01\x00\x18\x00";
        assert_eq!(image_size(core), Some((320, 240)));
    }

    #[test]
    fn rejects_unknown_and_truncated_input() {
        assert_eq!(image_size(b""), None);
        assert_eq!(image_size(b"not an image"), None);
        // 每种格式都在尺寸字段之前截断
        assert_eq!(image_size(&PNG_640_480[..20]), None, "PNG 缺高");
        assert_eq!(image_size(&JPEG_BASELINE_3_2[..15]), None, "JPEG 缺 SOF");
        assert_eq!(image_size(b"GIF89a\x40\x01"), None, "GIF 缺高");
        assert_eq!(image_size(&WEBP_VP8_400_300[..25]), None, "WebP 缺宽高");
        // 只在 SOS 之后才有 FF C0 的像素数据不算帧头
        assert_eq!(
            image_size(b"\xFF\xD8\xFF\xDA\x00\x04\x00\x00\xFF\xC0\x00\x11\x08\x00\x02\x00\x03"),
            None
        );
    }

    #[test]
    fn rejects_zero_dimensions() {
        assert_eq!(
            image_size(b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0DIHDR\x00\x00\x00\x00\x00\x00\x00\x00\x08\x06\x00\x00\x00"),
            None
        );
        assert_eq!(image_size(b"GIF89a\x00\x00\x00\x00\x00\x00\x00"), None);
    }
}
