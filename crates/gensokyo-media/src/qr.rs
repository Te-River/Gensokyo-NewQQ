//! QR 码生成（P56-T2）。
//!
//! Go `handlers/qrcode_real.go:13-25` `generateQRCode` 的等价物：
//! `qrcode.New(url, qrcode.High)` + `DisableBorder = true` + `PNG(qr_size)` +
//! `base64.StdEncoding`。注意 go-qrcode 的 `High` 常量是 **ISO Q 级（25%）**，
//! 其 `Highest` 才是 ISO H 级（version.go:19-31 注释明示），故这里用 `EcLevel::Q`。
//! 模块矩阵与 go-qrcode（主仓 go.mod 同版本）金样一致
//! （`tests/qr_golden.json`）；PNG 字节不跨库逐字节对齐（lead 裁决接受矩阵级 parity）。

use image::imageops::FilterType;
use qrcode::{EcLevel, QrCode};

use crate::data_uri::encode_base64;
use crate::error::{Error, Result};

/// 生成 `url` 的二维码 PNG，返回标准 base64（带 `=` 填充）。
///
/// 容错级 Q（Go `qrcode.High` 即 ISO Q 级）、无静区（Go `DisableBorder`）。
/// `size` 语义逐字对齐 go-qrcode `Image(size)`（qrcode.go:286-332）：
/// - 负数：每模块 `|size|` 像素，边长 `|size| × 模块数`；
/// - `0` 或小于模块数：静默放大到模块数；
/// - 其余：恰好 `size × size`（最近邻重采样）。
///
/// # Errors
///
/// 内容超出 version 40 Q 级编码容量、或 `size` 超出可表示范围时返回
/// [`Error::QrEncode`]；PNG 编码失败返回 [`Error::ImageEncode`]。
pub fn qr_png_base64(url: &str, size: i64) -> Result<String> {
    let code = QrCode::with_error_correction_level(url.as_bytes(), EcLevel::Q)
        .map_err(|e| Error::QrEncode(e.to_string()))?;
    let modules = code.width();
    let side = if size < 0 {
        size.checked_neg()
            .and_then(|s| s.checked_mul(modules as i64))
            .ok_or_else(|| Error::QrEncode(format!("size 超出范围: {size}")))?
    } else {
        size.max(modules as i64)
    };
    let side =
        u32::try_from(side).map_err(|_| Error::QrEncode(format!("size 超出范围: {size}")))?;

    let img = code
        .render::<image::Luma<u8>>()
        .quiet_zone(false)
        .module_dimensions(1, 1)
        .build();
    let img = if side == modules as u32 {
        img
    } else {
        image::imageops::resize(&img, side, side, FilterType::Nearest)
    };

    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| Error::ImageEncode(e.to_string()))?;
    Ok(encode_base64(&png))
}

#[cfg(test)]
mod tests {
    use qrcode::{Color, EcLevel, QrCode};

    use super::qr_png_base64;
    use crate::data_uri::decode_base64;
    use crate::error::Error;

    const URL: &str = "https://example.com/some/path?q=1";

    fn modules(url: &str) -> u32 {
        QrCode::with_error_correction_level(url.as_bytes(), EcLevel::Q)
            .unwrap()
            .width() as u32
    }

    /// base64 → PNG 魔数 → 解码，返回边长（QR 恒为正方形）。
    fn png_side(b64: &str) -> u32 {
        let bytes = decode_base64(b64).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        let img = image::load_from_memory(&bytes).unwrap();
        assert_eq!(img.width(), img.height());
        img.width()
    }

    #[test]
    fn negative_size_scales_per_module() {
        let m = modules(URL);
        assert_eq!(png_side(&qr_png_base64(URL, -5).unwrap()), 5 * m);
        assert_eq!(png_side(&qr_png_base64(URL, -1).unwrap()), m);
    }

    #[test]
    fn zero_and_small_size_silently_grow_to_modules() {
        let m = modules(URL);
        for size in [0, 1, 5, i64::from(m) - 1] {
            assert_eq!(png_side(&qr_png_base64(URL, size).unwrap()), m, "size={size}");
        }
    }

    #[test]
    fn exact_size_is_honored() {
        let m = modules(URL);
        for size in [i64::from(m), 200, 512] {
            assert_eq!(
                png_side(&qr_png_base64(URL, size).unwrap()),
                size as u32,
                "size={size}"
            );
        }
    }

    #[test]
    fn empty_url_is_accepted_like_go() {
        // Go qrcode.New("") 同样成功（version 1）；parity 保持成功而非报错
        assert_eq!(modules(""), 21);
        assert_eq!(png_side(&qr_png_base64("", 200).unwrap()), 200);
    }

    #[test]
    fn overlong_url_errors() {
        let url = format!("https://example.com/{}", "a".repeat(5000));
        assert!(matches!(qr_png_base64(&url, 200), Err(Error::QrEncode(_))));
    }

    #[test]
    fn absurd_size_errors() {
        assert!(matches!(qr_png_base64(URL, i64::MIN), Err(Error::QrEncode(_))));
        assert!(matches!(qr_png_base64(URL, i64::MAX), Err(Error::QrEncode(_))));
    }

    /// rqrr 解码 PNG 回原文（lead 裁决 (a)：解码载荷等价是语义 parity 的充分证明）。
    fn decode_png(png_bytes: &[u8]) -> String {
        let img = image::load_from_memory(png_bytes).unwrap().to_luma8();
        let mut prepared = rqrr::PreparedImage::prepare(img);
        let grids = prepared.detect_grids();
        assert!(!grids.is_empty(), "应检出 QR 网格");
        let (_meta, content) = grids[0].decode().unwrap();
        content
    }

    /// Go 矩阵金样：go-qrcode v0.0.0-20200617195104（主仓 go.mod 同版本）
    /// `Bitmap()`（DisableBorder=true）+ `PNG(size)` 实际尺寸，一次性 harness 生成。
    /// Go `qrcode.High` = ISO Q 级。模块数一致时矩阵必须逐模块一致（掩码两侧均按
    /// ISO 惩罚规则）；长混合字母数字内容两库分段优化器可能选不同版本（见 lib.rs
    /// P56-T2 差异节），此时以解码载荷等价为准。全部 9 条均断言尺寸语义 + 解码回原文。
    #[test]
    fn go_matrix_and_size_semantics_golden() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/qr_golden.json")).unwrap();
        let entries = fixture.as_array().unwrap();
        assert!(entries.len() >= 8);
        for e in entries {
            let url = e["url"].as_str().unwrap();
            let code = QrCode::with_error_correction_level(url.as_bytes(), EcLevel::Q).unwrap();
            let m = code.width();
            let m_i64 = m as i64;
            let matrix_match = m_i64 == e["modules"].as_i64().unwrap();
            if matrix_match {
                let colors = code.to_colors();
                let bitmap = e["bitmap"].as_array().unwrap();
                assert_eq!(bitmap.len(), m, "矩阵行数: {url}");
                for (y, row) in bitmap.iter().enumerate() {
                    let row = row.as_array().unwrap();
                    assert_eq!(row.len(), m, "矩阵列数: {url}");
                    for (x, v) in row.iter().enumerate() {
                        let dark = colors[y * m + x] == Color::Dark;
                        assert_eq!(dark, v.as_i64().unwrap() == 1, "矩阵差异: {url} ({x},{y})");
                    }
                }
            }

            for s in e["sizes"].as_array().unwrap() {
                let size = s["size"].as_i64().unwrap();
                // 尺寸语义按本库模块数断言（版本分歧条目的模块数与 Go 不同）
                let expected = if size < 0 { -size * m_i64 } else { size.max(m_i64) };
                let b64 = qr_png_base64(url, size).unwrap();
                let bytes = decode_base64(&b64).unwrap();
                let img = image::load_from_memory(&bytes).unwrap();
                assert_eq!(img.width() as i64, expected, "尺寸语义宽: {url} size={size}");
                assert_eq!(img.height() as i64, expected, "尺寸语义高: {url} size={size}");
                if matrix_match {
                    assert_eq!(img.width() as i64, s["w"].as_i64().unwrap(), "Go 宽: {url} size={size}");
                    assert_eq!(img.height() as i64, s["h"].as_i64().unwrap(), "Go 高: {url} size={size}");
                }
                if size == -5 {
                    // 每模块整数倍像素、无重采样畸变，作解码验证样本
                    assert_eq!(decode_png(&bytes), url, "解码载荷: {url}");
                }
            }
        }
    }
}
