//! deferred 媒体链路：`MediaRef` → 字节 → QQ `files` 端点 → `file_info`。
//!
//! # 边界
//!
//! - **下载**走 [`Fetcher`]：URL / `base64://` / `data:` / `file://` / 裸路径统一落成字节，
//!   上限 [`MAX_MEDIA_BYTES`]、超时 [`gensokyo_media::DEFAULT_TIMEOUT`]。下载器可注入
//!   （[`MediaRuntime::with_transport`]），单测用假传输，全程不碰网络。
//! - **上传**交给 [`gensokyo_actions::QqApi`] 的 `upload_group_file` / `upload_c2c_file`：
//!   本模块不认群 / 私聊，也不发消息，只给出 `file_data` + 建议文件名，端点选择留在
//!   [`crate::deferred`]。
//! - **临时文件**：URL / base64 取回时 `gensokyo-media` 会落一个临时文件并约定调用方删除；
//!   本模块读完字节立刻删。`file://` / 裸路径复用调用方原文件，不删也不复制。
//!
//! # 限制
//!
//! - 下载上限 [`MAX_UPLOAD_BYTES`]（200MB，对齐 Go `getHardLimit`）：超过即请求侧错误
//!   （`retcode 1400`）。超过 [`MAX_MEDIA_BYTES`]（5MB，Go `file_data` 直传上限）的媒体由
//!   [`crate::deferred`] 改走分片上传（`upload_prepare` → `upload_part_finish` → `files`）。
//! - 图片压缩 / 语音转码只在 [`MediaRuntime::prepare_image`] / [`MediaRuntime::prepare_record`]
//!   里做，条件由调用方按 Go 的口径判定（`image_sizelimit` / 本地或 base64 来源）。
//! - SSRF / 私网拦截由 `gensokyo-media` 的下载出口（`HttpTransport`）逐跳执行：
//!   私网 / 回环 / 链路本地 / 未指定 / ULA 与解析失败一律拒绝（口径与 Go 差异见该 crate
//!   文档的「P56」节）。

use std::sync::Arc;
use std::sync::OnceLock;

use gensokyo_core::{Error, Result};
use gensokyo_media::compress::{CompressOptions, compress_image};
use gensokyo_media::silk::{DEFAULT_SAMPLE_RATE, SilkCodec};
use gensokyo_media::{
    BoxFuture, FetchRequest, Fetcher, HttpTransport, MediaSource, MediaTransport, TransportResponse,
};
use gensokyo_protocol::MediaRef;

/// `file_data` 直传上限：5MB（**十进制 5_000_000 字节**，与 Go `maxDirectFileDataBytes`
/// 同口径）。超过它由 [`crate::deferred`] 改走分片上传，不再是请求侧错误。
pub const MAX_MEDIA_BYTES: u64 = 5_000_000;

/// 下载硬上限：200MB（Go `getHardLimit`），超过直接报错。
pub const MAX_UPLOAD_BYTES: u64 = 200 * 1024 * 1024;

/// 取回的一份媒体。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedMedia {
    /// 原始字节。
    pub bytes: Vec<u8>,
    /// 文件头嗅探出的媒体类型（嗅不出时 `application/octet-stream`）。
    pub content_type: String,
    /// 上传时建议使用的文件名：本地路径保留原名，其余按媒体类型给扩展名。
    pub file_name: String,
}

/// deferred 的媒体下载器 + 发送前处理（压缩 / 转码）。
#[derive(Debug)]
pub struct MediaRuntime {
    fetcher: Fetcher,
    silk: SilkCodec,
}

impl Default for MediaRuntime {
    /// 生产默认：HTTP 传输延迟到第一次下载才建客户端。
    ///
    /// [`crate::AppState`] 的构造不能失败（配置缺失也要能启动到 WebUI 排障），因此这里
    /// 不在构造期建 `reqwest::Client`；rustls provider 由二进制启动时安装
    /// （`gensokyo_bin::startup::install_crypto_provider`），未安装时错误留在单次请求上。
    fn default() -> Self {
        Self::with_transport(Arc::new(LazyHttpTransport::default()))
    }
}

impl MediaRuntime {
    /// 用给定传输创建（上限 [`MAX_UPLOAD_BYTES`]、超时取 `gensokyo_media` 默认值）。
    #[must_use]
    pub fn with_transport(transport: Arc<dyn MediaTransport>) -> Self {
        Self {
            fetcher: Fetcher::from_shared(transport).with_max_bytes(MAX_UPLOAD_BYTES),
            silk: SilkCodec::new(),
        }
    }

    /// 直接使用外部构造好的下载器（测试按需覆盖上限 / 超时 / 临时目录）。
    #[must_use]
    pub fn with_fetcher(fetcher: Fetcher) -> Self {
        Self {
            fetcher,
            silk: SilkCodec::new(),
        }
    }

    /// 替换 silk 编解码器的进程出口（单测注入假执行器；生产用默认 `ffmpeg` / `pts`）。
    #[must_use]
    pub fn with_silk_codec(mut self, silk: SilkCodec) -> Self {
        self.silk = silk;
        self
    }

    /// 当前大小上限（`0` 表示不限制）。
    #[must_use]
    pub const fn max_bytes(&self) -> u64 {
        self.fetcher.max_bytes()
    }

    /// 取回一份媒体引用对应的字节。
    ///
    /// # Errors
    /// 来源无法解析、超过 [`MAX_MEDIA_BYTES`]、本地文件不可读（[`Error::Protocol`]，请求侧
    /// 问题），或下载 / 写盘失败（[`Error::Network`]，服务端 / 上游问题）。
    pub async fn fetch(&self, reference: &MediaRef) -> Result<FetchedMedia> {
        // `to_wire()` 把四种 MediaRef 还原成 OneBot `file` 字段形态，交给 media 的解析器：
        // Local → `file:///…`、Url → 原 URL、Base64 → `base64://…`。
        let wire = reference.to_wire();
        let source = MediaSource::parse(&wire)
            .map_err(|err| Error::Protocol(format!("媒体来源非法: {err}")))?;
        let local = matches!(source, MediaSource::File(_));

        let fetched = self
            .fetcher
            .fetch(&source)
            .await
            .map_err(|err| map_fetch_error(err, local))?;
        // 先读字节再删临时文件：读失败要保留原始错误
        let bytes = tokio::fs::read(&fetched.path).await.map_err(|err| {
            Error::Protocol(format!(
                "读取媒体文件失败（{}）: {err}",
                fetched.path.display()
            ))
        });
        if !local && let Err(err) = tokio::fs::remove_file(&fetched.path).await {
            // 临时文件装着用户媒体，删不掉要留线索，但不能盖掉主流程
            tracing::warn!(
                path = %fetched.path.display(),
                error = %err,
                "deferred 媒体临时文件清理失败"
            );
        }
        let bytes = bytes?;
        if bytes.is_empty() {
            return Err(Error::Protocol("媒体内容为空".to_owned()));
        }
        Ok(FetchedMedia {
            file_name: suggested_name(reference, &fetched.content_type),
            content_type: fetched.content_type,
            bytes,
        })
    }

    /// 图片压缩（Go `images.CompressSingleImage`）：`threshold_kb == 0` 时原样返回。
    ///
    /// 参数按 Go 口径：不缩放（`max_side = 0`）、质量阶梯 [`gensokyo_media::compress::GO_QUALITY_LADDER`]、
    /// 目标体积 `threshold_kb * 1024` 字节。格式差异（PNG 保留 PNG、GIF 不支持）见
    /// [`gensokyo_media::compress`] 的模块文档。
    ///
    /// # Errors
    /// 解码 / 编码失败（含 GIF / BMP）返回 [`Error::Protocol`]，与 Go「压缩图片失败」同样中止本条消息。
    pub async fn prepare_image(&self, bytes: Vec<u8>, threshold_kb: u64) -> Result<Vec<u8>> {
        if threshold_kb == 0 {
            return Ok(bytes);
        }
        let target = usize::try_from(threshold_kb.saturating_mul(1024)).unwrap_or(usize::MAX);
        let options = CompressOptions {
            max_side: 0,
            target_bytes: Some(target),
            ..CompressOptions::default()
        };
        tokio::task::spawn_blocking(move || compress_image(&bytes, &options))
            .await
            .map_err(|err| Error::Upstream {
                code: 0,
                message: format!("图片压缩任务异常终止: {err}"),
            })?
            .map_err(|err| Error::Protocol(format!("图片压缩失败（retcode 1400）: {err}")))
    }

    /// 语音转码（Go `silk.EncoderSilk` 路径）：已是 AMR / SILK 的原样返回。
    ///
    /// 非音频内容（图片 / 纯文本）返回 [`Error::Protocol`]（Go `voice type error`）；
    /// 外部 `ffmpeg` / `pts` 缺失或失败返回 [`Error::Upstream`]。`sample_rate = 0` 回退
    /// [`DEFAULT_SAMPLE_RATE`]（Go 直接拿 0 去调编码器会失败，这里取模板默认值）。
    pub async fn prepare_record(&self, bytes: Vec<u8>, sample_rate: u32) -> Result<Vec<u8>> {
        if gensokyo_media::silk::is_amr_or_silk(&bytes) {
            return Ok(bytes);
        }
        if !gensokyo_media::silk::check_audio(&bytes) {
            return Err(Error::Protocol(
                "语音格式不支持：既不是 AMR / SILK，也识别不出音频类型（Go voice type error）"
                    .to_owned(),
            ));
        }
        let codec = self.silk.clone();
        let rate = if sample_rate == 0 {
            DEFAULT_SAMPLE_RATE
        } else {
            sample_rate
        };
        tokio::task::spawn_blocking(move || codec.encode_bytes_to_silk(&bytes, rate))
            .await
            .map_err(|err| Error::Upstream {
                code: 0,
                message: format!("语音转码任务异常终止: {err}"),
            })?
            .map_err(|err| Error::Upstream {
                code: 0,
                message: format!("语音转码失败，请检查 ffmpeg / pts 是否安装: {err}"),
            })
    }
}

/// `gensokyo_media::Error` → 统一错误：来源 / 大小 / 格式是请求侧问题（`1400`），
/// 传输与写盘是服务端 / 上游问题（`1500`）。
fn map_fetch_error(err: gensokyo_media::Error, local: bool) -> Error {
    use gensokyo_media::Error as MediaError;
    match err {
        MediaError::InvalidSource(_)
        | MediaError::Base64(_)
        | MediaError::DataUri(_)
        | MediaError::UnsupportedScheme(_)
        | MediaError::SsrfBlocked(_)
        | MediaError::TooLarge { .. }
        | MediaError::UnsupportedImageFormat(_)
        | MediaError::InvalidCompressOptions(_) => Error::Protocol(format!("媒体不可用: {err}")),
        // 本地路径读不了是调用方给的路径问题；URL / base64 的 IO 只可能发生在写临时文件时
        MediaError::Io(_) if local => Error::Protocol(format!("本地媒体不可读: {err}")),
        other => Error::Network(format!("媒体获取失败: {other}")),
    }
}

/// 上传用的文件名：本地路径保留原名（群文件靠它展示），URL / base64 按嗅探出的类型给扩展名。
fn suggested_name(reference: &MediaRef, content_type: &str) -> String {
    if let MediaRef::Local(path) = reference
        && let Some(name) = path.file_name().and_then(|name| name.to_str())
    {
        return name.to_owned();
    }
    format!("media.{}", extension_for(content_type))
}

/// 常见媒体类型 → 扩展名；认不出时给 `bin`。
fn extension_for(content_type: &str) -> &'static str {
    match content_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/mpeg" => "mp3",
        "video/mp4" => "mp4",
        _ => "bin",
    }
}

/// 延迟构造的 HTTP 下载出口（`OnceLock`：第一次下载才建客户端）。
#[derive(Debug, Default)]
struct LazyHttpTransport {
    inner: OnceLock<std::result::Result<HttpTransport, String>>,
}

impl MediaTransport for LazyHttpTransport {
    fn get(
        &self,
        request: FetchRequest,
    ) -> BoxFuture<'_, gensokyo_media::Result<TransportResponse>> {
        match self
            .inner
            .get_or_init(|| HttpTransport::new().map_err(|err| err.to_string()))
        {
            Ok(transport) => transport.get(request),
            Err(message) => {
                let message = message.clone();
                Box::pin(async move { Err(gensokyo_media::Error::Transport(message)) })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use gensokyo_media::silk::{ProcessOutput, ProcessRunner};

    #[tokio::test]
    async fn download_limit_is_the_two_hundred_megabyte_hard_limit() {
        assert_eq!(
            MAX_MEDIA_BYTES, 5_000_000,
            "官方 5MB 按十进制口径：5 * 1024 * 1024 会少放行 24_288 字节"
        );
        assert_eq!(MAX_UPLOAD_BYTES, 200 * 1024 * 1024, "对齐 Go getHardLimit");
        let runtime = MediaRuntime::default();
        assert_eq!(runtime.max_bytes(), MAX_UPLOAD_BYTES);
    }

    #[derive(Debug, Clone, Copy)]
    struct EmptyTransport;

    impl MediaTransport for EmptyTransport {
        fn get(
            &self,
            _request: FetchRequest,
        ) -> BoxFuture<'_, gensokyo_media::Result<TransportResponse>> {
            Box::pin(async {
                Ok(TransportResponse {
                    status: 200,
                    content_type: None,
                    body: Vec::new(),
                })
            })
        }
    }

    #[tokio::test]
    async fn url_media_pointing_at_private_network_is_rejected_at_entry() {
        // 测试进程先装 provider（生产由二进制 startup 装）：否则客户端构建失败，断言方向反
        let _ = rustls::crypto::ring::default_provider().install_default();
        let runtime = MediaRuntime::default();
        for url in [
            "http://127.0.0.1/a.png",
            "http://10.0.0.1/a.png",
            "http://192.168.1.1/a.png",
            "http://[::1]/a.png",
        ] {
            let err = runtime
                .fetch(&MediaRef::Url(url.to_owned()))
                .await
                .expect_err("私网 URL 必须在下载前拒绝");
            assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
        }
    }

    #[tokio::test]
    async fn local_file_at_limit_passes_and_one_byte_over_is_rejected() {
        // 上限收到 4 字节：本地文件读路径不经过传输，只验大小闸门
        let runtime = MediaRuntime::with_fetcher(Fetcher::new(EmptyTransport).with_max_bytes(4));
        let dir = tempfile::tempdir().expect("临时目录");

        let exact = dir.path().join("exact.bin");
        std::fs::write(&exact, [1, 2, 3, 4]).expect("写 4 字节失败");
        let fetched = runtime
            .fetch(&MediaRef::Local(exact))
            .await
            .expect("恰好 4 字节应放行");
        assert_eq!(fetched.bytes.len(), 4);

        let over = dir.path().join("over.bin");
        std::fs::write(&over, [1, 2, 3, 4, 5]).expect("写 5 字节失败");
        let err = runtime
            .fetch(&MediaRef::Local(over))
            .await
            .expect_err("超限必须拒绝");
        assert!(matches!(err, Error::Protocol(_)), "超限应映射 1400: {err}");
    }

    /// 质量 95 编码的 JPEG：压缩阶梯从 75 起步，重编码后字节必然不同。
    fn jpeg_fixture() -> Vec<u8> {
        let mut image = image::RgbImage::new(64, 64);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = image::Rgb([x as u8 * 3, y as u8 * 3, (x + y) as u8]);
        }
        let mut out = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95);
        encoder.encode_image(&image).expect("编码测试 JPEG");
        out
    }

    #[tokio::test]
    async fn prepare_image_compresses_only_when_threshold_is_set() {
        let source = jpeg_fixture();
        let runtime = MediaRuntime::default();

        let unchanged = runtime
            .prepare_image(source.clone(), 0)
            .await
            .expect("阈值 0 原样返回");
        assert_eq!(unchanged, source, "阈值 0 必须逐字节不变");

        let compressed = runtime
            .prepare_image(source.clone(), 1)
            .await
            .expect("阈值 1KB 应压缩成功");
        assert_ne!(compressed, source, "命中阈值必须重编码");
        assert!(compressed.starts_with(&[0xFF, 0xD8]), "JPEG 应保持 JPEG");
    }

    #[tokio::test]
    async fn prepare_record_passes_through_amr_and_silk() {
        let runtime = MediaRuntime::default();
        for already in [b"\x02#!SILK_V3abc".to_vec(), b"#!AMR\x0a".to_vec()] {
            let out = runtime
                .prepare_record(already.clone(), 24_000)
                .await
                .expect("已是 AMR/SILK 应原样返回");
            assert_eq!(out, already, "不满足转码条件时必须零变化");
        }
    }

    #[tokio::test]
    async fn prepare_record_rejects_non_audio() {
        let runtime = MediaRuntime::default();
        let err = runtime
            .prepare_record(b"hello, not audio".to_vec(), 24_000)
            .await
            .expect_err("文本必须拒绝");
        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
    }

    /// 假进程出口：ffmpeg 写 PCM，pts 写 Silk；都不启动真实进程。
    #[derive(Debug, Clone, Copy)]
    struct FakeRunner;

    impl ProcessRunner for FakeRunner {
        fn run(&self, program: &str, args: &[String]) -> gensokyo_media::Result<ProcessOutput> {
            match program {
                "ffmpeg" => {
                    std::fs::write(Path::new(args.last().expect("末参是 PCM 路径")), b"pcm")
                        .expect("写 PCM");
                }
                "pts" => {
                    let index = args
                        .iter()
                        .position(|arg| arg.as_str() == "-o")
                        .expect("pts 参数含 -o");
                    std::fs::write(Path::new(&args[index + 1]), b"\x02#!SILK_V3-runtime")
                        .expect("写 Silk");
                }
                other => panic!("意外的程序: {other}"),
            }
            Ok(ProcessOutput {
                status: Some(0),
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn prepare_record_encodes_non_silk_audio() {
        let runtime = MediaRuntime::default().with_silk_codec(SilkCodec::with_runner(FakeRunner));
        let out = runtime
            .prepare_record(b"ID3\x04not-silk".to_vec(), 24_000)
            .await
            .expect("非 Silk 音频应转码");
        assert_eq!(out, b"\x02#!SILK_V3-runtime");
    }
}
