//! 图片压缩：解码 → 等比缩放 → 按质量阶梯编码。
//!
//! 对齐 Go `images/easycompress.go` + `images/Compress.go`：
//! - 质量阶梯取自 Go 的 `defaultMaxQuality`(75) / `defaultQualityStep`(10) / `defaultMinQuality`(25)，
//!   即 [`GO_QUALITY_LADDER`]；从最高质量往下试，**第一个**满足目标体积的结果直接返回，
//!   全都超限时返回体积最小的那个（等同于 Go 在 `quality <= MinQuality` 时收尾）。
//! - 目标体积对应 Go 的 `thresholdKB`，但以字节表达（[`CompressOptions::target_bytes`]）。
//! - PNG → JPEG 时的透明通道压到白底，对齐 Go `compressPNG` 的 `draw.Over` 白底处理。
//!
//! 与 Go 的差异：
//! - **尺寸上限**：Go 只按质量阶梯重编码，从不缩放；本模块先按 [`DEFAULT_MAX_SIDE`] 等比缩小长边，
//!   再进阶梯。`max_side = 0` 表示不限制（等价 Go 行为）。
//! - **PNG 默认保持 PNG**：Go 的 `compressPNG` 一律转白底 JPEG（透明丢失）；本模块 PNG 输入默认输出
//!   PNG 以保留透明通道，只有显式 [`CompressOptions::format`]` = Some(ImageFormat::Jpeg)` 才转 JPEG。
//!   PNG 没有质量参数，阶梯对它无效（只编码一次）。
//! - **阈值为 0 的短路**：Go `thresholdKB == 0` 时原样返回入参字节；本模块不内置这条短路，
//!   `target_bytes = None` 表示「不设体积目标」，仍会缩放 + 按阶梯首档编码。需要原样返回的调用方
//!   自行判断（见 crate 文档的「与 Go 的差异（M2c）」）。
//! - **GIF / BMP**：Go 对 GIF 逐帧压缩，M2c 不解码（返回 [`Error::UnsupportedImageFormat`]）。
//! - **编码器参数**：Go 用标准库默认 JPEG/PNG 参数，这里用 `image` crate 的默认参数（JPEG 采样
//!   4:2:0、PNG default 压缩），体积与像素不会与 Go 逐字节一致。

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ExtendedColorType, ImageEncoder, Rgb, RgbImage};

use crate::error::{Error, Result};
use crate::sniff::{self, ImageKind};

/// Go `images/easycompress.go` 的默认质量阶梯：75 起步、步长 10、下限 25。
pub const GO_QUALITY_LADDER: &[u8] = &[75, 65, 55, 45, 35, 25];

/// 默认长边上限（像素）。Go 没有对应概念，M2c 新增。
pub const DEFAULT_MAX_SIDE: u32 = 1024;

/// 压缩输出格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    /// JPEG：采用质量阶梯，透明通道压到白底。
    Jpeg,
    /// PNG：无质量参数，保留透明通道，不做阶梯。
    Png,
}

/// 压缩参数。
#[derive(Debug, Clone, Copy)]
pub struct CompressOptions<'a> {
    /// 长边上限（像素）；`0` 表示不缩放。
    pub max_side: u32,
    /// 质量阶梯，按给定顺序尝试（仅对 JPEG 输出生效）；不能为空。
    pub quality_ladder: &'a [u8],
    /// 输出格式；`None` 表示沿用源格式（PNG → PNG，JPEG / WebP → JPEG）。
    pub format: Option<ImageFormat>,
    /// 目标体积（字节）；`None` 表示不设体积目标（直接取阶梯首档）。
    pub target_bytes: Option<usize>,
}

impl Default for CompressOptions<'_> {
    fn default() -> Self {
        Self {
            max_side: DEFAULT_MAX_SIDE,
            quality_ladder: GO_QUALITY_LADDER,
            format: None,
            target_bytes: None,
        }
    }
}

/// 解码 → 等比缩放 → 按质量阶梯编码；失败一律返回 [`Error`]，不 panic。
///
/// 支持 JPEG / PNG / WebP 解码；输出 JPEG（走质量阶梯）或 PNG（保留透明）。
pub fn compress_image(bytes: &[u8], options: &CompressOptions<'_>) -> Result<Vec<u8>> {
    if options.quality_ladder.is_empty() {
        return Err(Error::InvalidCompressOptions(
            "quality_ladder 不能为空".to_string(),
        ));
    }

    let kind = sniff::sniff_image(bytes).ok_or_else(|| {
        Error::UnsupportedImageFormat("无法识别的图片头（只支持 JPEG / PNG / WebP）".to_string())
    })?;
    let source = match kind {
        ImageKind::Png => SourceFormat::Png,
        ImageKind::Jpeg => SourceFormat::Jpeg,
        ImageKind::WebP => SourceFormat::WebP,
        ImageKind::Gif | ImageKind::Bmp => {
            return Err(Error::UnsupportedImageFormat(format!(
                "{kind:?} 不在 M2c 解码范围（只支持 JPEG / PNG / WebP）"
            )));
        }
    };

    let image =
        image::load_from_memory(bytes).map_err(|err| Error::ImageDecode(err.to_string()))?;

    let (width, height) = scaled_size(image.width(), image.height(), options.max_side);
    let image = if (width, height) == (image.width(), image.height()) {
        image
    } else {
        image.resize_exact(width, height, FilterType::Lanczos3)
    };

    match options.format.unwrap_or(source.default_output()) {
        ImageFormat::Jpeg => {
            encode_jpeg_ladder(&image, options.quality_ladder, options.target_bytes)
        }
        ImageFormat::Png => encode_png(&image),
    }
}

/// 源格式：只用于推导默认输出格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceFormat {
    Jpeg,
    Png,
    WebP,
}

impl SourceFormat {
    /// PNG 保持 PNG（保留透明）；JPEG / WebP 统一输出 JPEG。
    fn default_output(self) -> ImageFormat {
        match self {
            Self::Png => ImageFormat::Png,
            Self::Jpeg | Self::WebP => ImageFormat::Jpeg,
        }
    }
}

/// 长边缩到 `max_side` 以内的等比尺寸；不超标或 `max_side == 0` 时原样返回。
fn scaled_size(width: u32, height: u32, max_side: u32) -> (u32, u32) {
    if max_side == 0 || (width <= max_side && height <= max_side) {
        return (width, height);
    }
    let ratio = f64::from(max_side) / f64::from(width.max(height));
    let scaled_width = ((f64::from(width) * ratio).round() as u32).clamp(1, max_side);
    let scaled_height = ((f64::from(height) * ratio).round() as u32).clamp(1, max_side);
    (scaled_width, scaled_height)
}

/// 按阶梯逐档编码：第一个不超过 `target` 的返回；都不满足取最小；`target` 为 `None` 只取首档。
fn encode_jpeg_ladder(
    image: &DynamicImage,
    ladder: &[u8],
    target: Option<usize>,
) -> Result<Vec<u8>> {
    let rgb = flatten_for_jpeg(image);
    let mut smallest: Option<Vec<u8>> = None;
    for &quality in ladder {
        let encoded = encode_jpeg(&rgb, quality)?;
        if target.is_some_and(|limit| encoded.len() <= limit) {
            return Ok(encoded);
        }
        if smallest
            .as_ref()
            .is_none_or(|best| encoded.len() < best.len())
        {
            smallest = Some(encoded);
        }
        if target.is_none() {
            break;
        }
    }
    smallest.ok_or_else(|| Error::InvalidCompressOptions("quality_ladder 不能为空".to_string()))
}

/// 编码单档 JPEG；`quality` 夹到 1..=100（`image` 不接受 0 / >100）。
fn encode_jpeg(rgb: &RgbImage, quality: u8) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
        .write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            ExtendedColorType::Rgb8,
        )
        .map_err(|err| Error::ImageEncode(err.to_string()))?;
    Ok(out)
}

/// PNG 编码：保留解码后的颜色类型（含 alpha），不做质量阶梯。
fn encode_png(image: &DynamicImage) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(
            image.as_bytes(),
            image.width(),
            image.height(),
            image.color().into(),
        )
        .map_err(|err| Error::ImageEncode(err.to_string()))?;
    Ok(out)
}

/// JPEG 只吃 RGB：有 alpha 时按 Go `compressPNG` 的做法压到白底。
fn flatten_for_jpeg(image: &DynamicImage) -> RgbImage {
    if !image.color().has_alpha() {
        return image.to_rgb8();
    }
    let rgba = image.to_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        Rgb([over_white(r, a), over_white(g, a), over_white(b, a)])
    })
}

/// 单通道「src over 白底」合成。
fn over_white(channel: u8, alpha: u8) -> u8 {
    let alpha = u16::from(alpha);
    ((u16::from(channel) * alpha + 255 * (255 - alpha)) / 255) as u8
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};

    use super::{
        CompressOptions, DEFAULT_MAX_SIDE, GO_QUALITY_LADDER, ImageFormat, compress_image,
    };
    use crate::error::Error;
    use crate::image_size;

    /// 把 RGBA 图编码成 PNG 字节。
    fn to_png(image: &RgbaImage) -> Vec<u8> {
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(image.clone())
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("测试图 PNG 编码");
        out
    }

    /// 2000x1000 渐变图。
    fn gradient_png(width: u32, height: u32) -> Vec<u8> {
        let image = RgbaImage::from_fn(width, height, |x, y| {
            Rgba([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8, 255])
        });
        to_png(&image)
    }

    /// 确定性的伪噪声图：JPEG 质量对体积影响明显，用来验证质量阶梯。
    fn noise_png(width: u32, height: u32) -> Vec<u8> {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let image = RgbaImage::from_fn(width, height, |_, _| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let value = (state >> 24) as u8;
            Rgba([value, value.wrapping_add(37), value.wrapping_mul(3), 255])
        });
        to_png(&image)
    }

    #[test]
    fn default_options_match_go_ladder() {
        let options = CompressOptions::default();
        assert_eq!(options.quality_ladder, GO_QUALITY_LADDER);
        assert_eq!(options.quality_ladder, &[75, 65, 55, 45, 35, 25]);
        assert_eq!(options.max_side, DEFAULT_MAX_SIDE);
        assert!(options.format.is_none());
        assert!(options.target_bytes.is_none());
    }

    #[test]
    fn compresses_large_gradient_within_max_side() {
        let input = gradient_png(2000, 1000);
        let output = compress_image(&input, &CompressOptions::default()).expect("压缩成功");

        let (width, height) = image_size(&output).expect("输出是可识别的图片");
        assert!(
            width <= DEFAULT_MAX_SIDE && height <= DEFAULT_MAX_SIDE,
            "尺寸超过上限: {width}x{height}"
        );
        // 等比：2000x1000 → 1024x512
        assert_eq!((width, height), (1024, 512));
        assert!(
            output.len() < input.len(),
            "输出没有变小: {} >= {}",
            output.len(),
            input.len()
        );
    }

    #[test]
    fn small_image_is_not_upscaled() {
        let input = gradient_png(100, 50);
        let output = compress_image(&input, &CompressOptions::default()).expect("压缩成功");
        assert_eq!(image_size(&output), Some((100, 50)));
    }

    #[test]
    fn rejects_garbage_and_unsupported_formats() {
        let err = compress_image(b"definitely not an image", &CompressOptions::default())
            .expect_err("坏字节必须报错");
        assert!(matches!(err, Error::UnsupportedImageFormat(_)), "{err:?}");

        // GIF / BMP 能识别文件头但不在 M2c 解码范围
        let gif = b"GIF89a\x40\x01\xF0\x00\x00\x00\x00";
        assert!(matches!(
            compress_image(gif, &CompressOptions::default()),
            Err(Error::UnsupportedImageFormat(_))
        ));

        // 截断的 PNG（有文件头但解码失败）
        let truncated = &gradient_png(32, 32)[..40];
        assert!(matches!(
            compress_image(truncated, &CompressOptions::default()),
            Err(Error::ImageDecode(_))
        ));

        // 空阶梯属于参数错误
        assert!(matches!(
            compress_image(
                &gradient_png(8, 8),
                &CompressOptions {
                    quality_ladder: &[],
                    ..CompressOptions::default()
                }
            ),
            Err(Error::InvalidCompressOptions(_))
        ));
    }

    #[test]
    fn png_keeps_alpha() {
        let mut image = RgbaImage::from_pixel(64, 64, Rgba([10, 20, 30, 0]));
        image.put_pixel(32, 32, Rgba([200, 100, 50, 255]));
        let input = to_png(&image);

        let output = compress_image(&input, &CompressOptions::default()).expect("压缩成功");
        let decoded = image::load_from_memory(&output)
            .expect("输出可解码")
            .to_rgba8();
        assert_eq!(decoded.dimensions(), (64, 64));
        assert_eq!(decoded.get_pixel(0, 0)[3], 0, "透明像素被填实了");
        assert_eq!(decoded.get_pixel(32, 32).0, [200, 100, 50, 255]);
    }

    #[test]
    fn forced_jpeg_flattens_transparency_onto_white() {
        let input = to_png(&RgbaImage::from_pixel(64, 64, Rgba([0, 0, 0, 0])));
        let output = compress_image(
            &input,
            &CompressOptions {
                format: Some(ImageFormat::Jpeg),
                ..CompressOptions::default()
            },
        )
        .expect("压缩成功");
        assert_eq!(image_size(&output), Some((64, 64)));
        let decoded = image::load_from_memory(&output)
            .expect("输出可解码")
            .to_rgb8();
        // 全透明 → 白底；JPEG 有损，留一点余量
        assert!(
            decoded.get_pixel(0, 0)[0] >= 240,
            "透明区域没有压到白底: {:?}",
            decoded.get_pixel(0, 0)
        );
    }

    #[test]
    fn quality_ladder_picks_first_result_within_target() {
        let input = noise_png(400, 400);

        // target = None：只取阶梯首档（75）
        let first = compress_image(
            &input,
            &CompressOptions {
                format: Some(ImageFormat::Jpeg),
                target_bytes: None,
                ..CompressOptions::default()
            },
        )
        .expect("压缩成功");

        // 目标正好等于首档体积：应立即返回首档，不再往下降
        let same_target = compress_image(
            &input,
            &CompressOptions {
                format: Some(ImageFormat::Jpeg),
                target_bytes: Some(first.len()),
                ..CompressOptions::default()
            },
        )
        .expect("压缩成功");
        assert_eq!(same_target.len(), first.len());

        // 目标无法达成：走完阶梯取最小档（25）
        let exhausted = compress_image(
            &input,
            &CompressOptions {
                format: Some(ImageFormat::Jpeg),
                target_bytes: Some(1),
                ..CompressOptions::default()
            },
        )
        .expect("压缩成功");
        assert!(
            exhausted.len() < first.len(),
            "阶梯没有降质量: {} >= {}",
            exhausted.len(),
            first.len()
        );
        assert_eq!(image_size(&exhausted), Some((400, 400)));
    }

    #[test]
    fn decodes_jpeg_and_webp_inputs() {
        let source = RgbaImage::from_fn(48, 32, |x, y| {
            Rgba([(x * 5) as u8, (y * 7) as u8, 128, 255])
        });

        // JPEG 输入 → JPEG 输出
        let mut jpeg = Vec::new();
        image::DynamicImage::ImageRgba8(source.clone())
            .to_rgb8()
            .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut jpeg, 90,
            ))
            .expect("测试图 JPEG 编码");
        let from_jpeg =
            compress_image(&jpeg, &CompressOptions::default()).expect("JPEG 输入压缩成功");
        assert_eq!(
            crate::sniff::sniff_image(&from_jpeg),
            Some(crate::sniff::ImageKind::Jpeg)
        );
        assert_eq!(image_size(&from_jpeg), Some((48, 32)));

        // WebP 输入 → 解出来按 JPEG 输出（WebP 没有编码器）
        let mut webp = Vec::new();
        image::DynamicImage::ImageRgba8(source)
            .write_to(
                &mut std::io::Cursor::new(&mut webp),
                image::ImageFormat::WebP,
            )
            .expect("测试图 WebP 编码");
        let from_webp =
            compress_image(&webp, &CompressOptions::default()).expect("WebP 输入压缩成功");
        assert_eq!(
            crate::sniff::sniff_image(&from_webp),
            Some(crate::sniff::ImageKind::Jpeg)
        );
        assert_eq!(image_size(&from_webp), Some((48, 32)));
    }

    #[test]
    fn scaled_size_is_proportional_and_bounded() {
        assert_eq!(super::scaled_size(2000, 1000, 1024), (1024, 512));
        assert_eq!(super::scaled_size(1000, 2000, 1024), (512, 1024));
        assert_eq!(super::scaled_size(100, 50, 1024), (100, 50), "小图不放大");
        assert_eq!(super::scaled_size(8000, 100, 1024), (1024, 13));
        assert_eq!(super::scaled_size(8000, 100, 0), (8000, 100), "0 = 不限制");
        assert_eq!(super::scaled_size(8000, 1, 8), (8, 1), "短边至少 1 像素");
    }
}
