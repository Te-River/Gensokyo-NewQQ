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
//! - [`MAX_MEDIA_BYTES`]（5MB）对齐 Go `handlers/send_group_msg.go` 的 `file_data` 直传上限：
//!   超限返回请求侧错误（`retcode 1400`）。Go 的超限路径走分片上传
//!   （`upload_prepare` / `upload_part_finish`），`gensokyo-qq` 尚未实现，因此这里的超限
//!   直接报错而不是静默降级。
//! - 不压缩、不转码、不抽帧：原字节直传（压缩是 `gensokyo_media::compress` 的独立能力）。
//! - 不做 SSRF / 私网拦截，与 `gensokyo-media` 的既有边界一致。

use std::sync::{Arc, OnceLock};

use gensokyo_core::{Error, Result};
use gensokyo_media::{
    BoxFuture, FetchRequest, Fetcher, HttpTransport, MediaSource, MediaTransport, TransportResponse,
};
use gensokyo_protocol::MediaRef;

/// 单条媒体的大小上限：5MB，对齐 Go `file_data` 直传上限。
pub const MAX_MEDIA_BYTES: u64 = 5 * 1024 * 1024;

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

/// deferred 的媒体下载器（上传由 [`gensokyo_actions::QqApi`] 负责）。
#[derive(Debug)]
pub struct MediaRuntime {
    fetcher: Fetcher,
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
    /// 用给定传输创建（上限 [`MAX_MEDIA_BYTES`]、超时取 `gensokyo_media` 默认值）。
    #[must_use]
    pub fn with_transport(transport: Arc<dyn MediaTransport>) -> Self {
        Self {
            fetcher: Fetcher::from_shared(transport).with_max_bytes(MAX_MEDIA_BYTES),
        }
    }

    /// 直接使用外部构造好的下载器（测试按需覆盖上限 / 超时 / 临时目录）。
    #[must_use]
    pub fn with_fetcher(fetcher: Fetcher) -> Self {
        Self { fetcher }
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
