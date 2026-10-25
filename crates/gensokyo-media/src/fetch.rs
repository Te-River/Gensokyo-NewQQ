//! 媒体下载与落盘。
//!
//! # 分层
//! - [`MediaTransport`]：唯一的网络出口。生产实现 [`HttpTransport`]（`reqwest` + `rustls`），
//!   测试实现只要记录 [`FetchRequest`] 就能断言 URL / 超时 / 大小上限，无需外网。
//! - [`Fetcher`]：来源分发（URL / 本地文件 / base64）、大小复核、临时文件创建与失败清理。
//!
//! # 落盘约定
//! - `http(s)://` 与内联 base64 一律写到临时文件后返回路径（对齐 Go `FetchToTempFile`）。
//! - `file://` 指向的文件已经在盘上，直接复用原路径，不复制。
//! - 任何在临时文件创建后的失败都会删除半成品；超限检查在写盘前完成。
//! - 成功返回的临时文件**归调用方所有**：本 crate 不删、不注册清理，调用方用完自行删除。

use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rustls::crypto::CryptoProvider;
use rustls::{ClientConfig, RootCertStore};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use crate::error::{Error, Result};
use crate::sniff;
use crate::source::MediaSource;

/// 默认大小上限：20MB（M2a 设计约定）。
pub const DEFAULT_MAX_BYTES: u64 = 20 * 1024 * 1024;
/// 默认总超时：30s（对齐 Go `media.NewFetcher` 的默认 `Timeout`）。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// 临时文件名前缀（对齐 Go `os.CreateTemp("", "gensokyo-media-*")`）。
const TEMP_FILE_PREFIX: &str = "gensokyo-media-";
/// Content-Type 嗅探读取的头部字节数（Go `http.DetectContentType` 取 512）。
const CONTENT_TYPE_PROBE_LEN: u64 = 512;
/// 连接超时（含 DNS / TCP / TLS 握手），与 botgo 的 `TLSHandshakeTimeout` 对齐。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// 一次下载请求；传输实现只读。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchRequest {
    /// 完整 URL。
    pub url: String,
    /// 单次请求总超时（含响应体读取）。
    pub timeout: Duration,
    /// 允许读取的最大字节数；`0` 表示不限制（对齐 Go 只在 `> 0` 时限流）。
    pub max_bytes: u64,
}

/// 传输层返回的完整响应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportResponse {
    /// HTTP 状态码。
    pub status: u16,
    /// 响应头 `Content-Type`（原样，未归一化）。
    pub content_type: Option<String>,
    /// 响应体；实现必须保证不超过请求里的 `max_bytes`（`max_bytes == 0` 时不做上限）。
    pub body: Vec<u8>,
}

/// 装箱的异步结果。
///
/// trait 方法返回 boxed future 而不是 `impl Future`：保持 [`MediaTransport`] 的 dyn 兼容，
/// runtime 可以持有 `Arc<dyn MediaTransport>` 而不必把整条装配链泛型化。一次请求一次装箱，
/// 相对网络开销可以忽略。
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 网络出口抽象。
///
/// 显式 `Send` 保证 [`Fetcher::fetch`] 的 future 可以被 `tokio::spawn`。
pub trait MediaTransport: Send + Sync {
    /// 下载 `request.url`。
    fn get(&self, request: FetchRequest) -> BoxFuture<'_, Result<TransportResponse>>;
}

/// 生产传输：`reqwest` + `rustls`。
#[derive(Debug, Clone)]
pub struct HttpTransport {
    client: reqwest::Client,
}

impl HttpTransport {
    /// 使用进程级 rustls provider 构建客户端。
    ///
    /// 二进制需先安装 provider，例如（`rustls` 启用 `ring` feature 后）：
    /// `rustls::crypto::ring::default_provider().install_default().ok();`
    pub fn new() -> Result<Self> {
        let provider = CryptoProvider::get_default().ok_or_else(|| {
            Error::Transport(
                "未安装 rustls CryptoProvider；二进制需先调用 \
                 rustls::crypto::<provider>::default_provider().install_default()"
                    .to_owned(),
            )
        })?;
        Self::with_provider(Arc::clone(provider))
    }

    /// 使用调用方注入的 rustls provider 构建客户端（`rustls-no-provider` 语义）。
    pub fn with_provider(provider: Arc<CryptoProvider>) -> Result<Self> {
        Self::with_client_config(rustls_config(provider)?)
    }

    /// 使用已构建好的 rustls 配置（便于复用平台校验器）。
    pub fn with_client_config(config: ClientConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .tls_backend_preconfigured(config)
            .connect_timeout(CONNECT_TIMEOUT)
            // 总超时按请求走：不同媒体大小差异大，统一用 reqwest 客户端超时会互相拖累
            .user_agent(concat!("gensokyo-media/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|err| Error::Transport(format!("构建 reqwest 客户端失败: {err}")))?;
        Ok(Self { client })
    }

    /// 复用外部已配置好的 `reqwest::Client`（TLS / 代理由调用方决定）。
    #[must_use]
    pub fn from_client(client: reqwest::Client) -> Self {
        Self { client }
    }

    /// 复用底层 `reqwest::Client`：图床上传出口（[`crate::hosting::HostingTransport`]）
    /// 与下载出口共享同一连接池 / TLS 配置，不重复建客户端。
    #[must_use]
    pub(crate) fn client(&self) -> &reqwest::Client {
        &self.client
    }
}

impl MediaTransport for HttpTransport {
    fn get(&self, request: FetchRequest) -> BoxFuture<'_, Result<TransportResponse>> {
        let client = self.client.clone();
        let FetchRequest {
            url,
            timeout,
            max_bytes,
        } = request;

        Box::pin(async move {
            let operation = async {
                let response = client
                    .get(&url)
                    .send()
                    .await
                    .map_err(|err| Error::Transport(err.to_string()))?;
                let status = response.status().as_u16();
                // 与 Go `checkResponse` 一致：只接受 200
                if status != 200 {
                    return Err(Error::HttpStatus(status));
                }
                // 有 Content-Length 时先拒绝，避免为一个注定超限的响应建临时文件；
                // max_bytes == 0 表示不限制（Go 只在 > 0 时限流）
                if max_bytes > 0
                    && let Some(length) = response.content_length()
                    && length > max_bytes
                {
                    return Err(Error::TooLarge {
                        limit: max_bytes,
                        actual: Some(length),
                    });
                }
                let content_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned);

                let mut response = response;
                let mut body = Vec::new();
                while let Some(chunk) = response
                    .chunk()
                    .await
                    .map_err(|err| Error::Transport(err.to_string()))?
                {
                    if max_bytes > 0 && body.len() as u64 + chunk.len() as u64 > max_bytes {
                        // 流式读满上限即停，不把超大响应读完
                        return Err(Error::TooLarge {
                            limit: max_bytes,
                            actual: None,
                        });
                    }
                    body.extend_from_slice(&chunk);
                }
                Ok(TransportResponse {
                    status,
                    content_type,
                    body,
                })
            };

            match tokio::time::timeout(timeout, operation).await {
                Ok(result) => result,
                Err(_elapsed) => Err(Error::Timeout(timeout)),
            }
        })
    }
}

/// 取回并落盘的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// 内容所在路径：URL / base64 是新建的临时文件（**调用方负责删除**），`file://` 是原路径
    /// （不复制、也不删除）。
    pub path: PathBuf,
    /// 字节数。
    pub bytes: u64,
    /// 媒体类型：以文件头嗅探为准；嗅不出（`application/octet-stream`）时才采用响应头 /
    /// `data:` URI 声明，仍缺省则兜底 `application/octet-stream`。
    pub content_type: String,
}

/// 下载器：默认上限 [`DEFAULT_MAX_BYTES`]、超时 [`DEFAULT_TIMEOUT`]。
pub struct Fetcher {
    transport: Arc<dyn MediaTransport>,
    max_bytes: u64,
    timeout: Duration,
    temp_dir: Option<PathBuf>,
}

impl Fetcher {
    /// 用给定传输创建下载器（默认上限 20MB、超时 30s）。
    pub fn new<T>(transport: T) -> Self
    where
        T: MediaTransport + 'static,
    {
        Self::from_shared(Arc::new(transport))
    }

    /// 复用共享传输（runtime 只建一次客户端时用）。
    #[must_use]
    pub fn from_shared(transport: Arc<dyn MediaTransport>) -> Self {
        Self {
            transport,
            max_bytes: DEFAULT_MAX_BYTES,
            timeout: DEFAULT_TIMEOUT,
            temp_dir: None,
        }
    }

    /// 覆盖大小上限；`0` 表示不限制（对齐 Go 只在 `> 0` 时限流）。
    #[must_use]
    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    /// 覆盖单次请求总超时。
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// 指定临时文件目录（默认 `std::env::temp_dir()`）。
    #[must_use]
    pub fn with_temp_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.temp_dir = Some(dir.into());
        self
    }

    /// 当前大小上限（`0` 表示不限制）。
    #[must_use]
    pub const fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    /// 当前总超时。
    #[must_use]
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }

    /// 超限判定：`max_bytes == 0` 表示不限制（对齐 Go 只在 `> 0` 时限流）。
    const fn exceeds_limit(&self, actual: u64) -> bool {
        self.max_bytes > 0 && actual > self.max_bytes
    }

    /// 取回媒体并落到本地文件。
    pub async fn fetch(&self, source: &MediaSource) -> Result<Fetched> {
        match source {
            MediaSource::Url(url) => {
                let response = self
                    .transport
                    .get(FetchRequest {
                        url: url.clone(),
                        timeout: self.timeout,
                        max_bytes: self.max_bytes,
                    })
                    .await?;
                if response.status != 200 {
                    return Err(Error::HttpStatus(response.status));
                }
                let actual = response.body.len() as u64;
                if self.exceeds_limit(actual) {
                    return Err(Error::TooLarge {
                        limit: self.max_bytes,
                        actual: Some(actual),
                    });
                }
                let content_type =
                    normalize_content_type(response.content_type.as_deref(), &response.body);
                let path = self.write_temp(&response.body).await?;
                Ok(Fetched {
                    path,
                    bytes: actual,
                    content_type,
                })
            }
            MediaSource::Base64 { mime, data } => {
                let actual = data.len() as u64;
                if self.exceeds_limit(actual) {
                    return Err(Error::TooLarge {
                        limit: self.max_bytes,
                        actual: Some(actual),
                    });
                }
                let content_type = normalize_content_type(mime.as_deref(), data);
                let path = self.write_temp(data).await?;
                Ok(Fetched {
                    path,
                    bytes: actual,
                    content_type,
                })
            }
            MediaSource::File(path) => self.fetch_local(path).await,
        }
    }

    /// `file://`：校验大小后复用原路径，只读头部做格式嗅探。
    async fn fetch_local(&self, path: &Path) -> Result<Fetched> {
        let metadata = tokio::fs::metadata(path).await?;
        if !metadata.is_file() {
            return Err(Error::InvalidSource(format!(
                "不是常规文件: {}",
                path.display()
            )));
        }
        let size = metadata.len();
        if self.exceeds_limit(size) {
            return Err(Error::TooLarge {
                limit: self.max_bytes,
                actual: Some(size),
            });
        }

        let file = tokio::fs::File::open(path).await?;
        let mut head = Vec::new();
        file.take(CONTENT_TYPE_PROBE_LEN)
            .read_to_end(&mut head)
            .await?;
        Ok(Fetched {
            path: path.to_path_buf(),
            bytes: size,
            content_type: sniff::sniff_content_type(&head).to_owned(),
        })
    }

    /// 写入唯一命名的临时文件；写失败时删掉半成品。
    async fn write_temp(&self, bytes: &[u8]) -> Result<PathBuf> {
        let path = self.unique_temp_path();
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        // 临时文件装着用户媒体，可能含隐私内容：Unix 下收窄到 0600。
        // Windows 走默认 ACL（无等价的 mode 位），保持系统默认。
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&path).await?;
        if let Err(err) = write_all_and_flush(&mut file, bytes).await {
            drop(file);
            if let Err(cleanup_error) = tokio::fs::remove_file(&path).await {
                // 清理失败不能覆盖原始错误，但也不能静默：残留的半成品要能在日志里找到
                tracing::warn!(
                    path = %path.display(),
                    error = %cleanup_error,
                    "临时文件写入失败且清理失败，可能残留半成品"
                );
            }
            return Err(err);
        }
        Ok(path)
    }

    fn unique_temp_path(&self) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = self.temp_dir.clone().unwrap_or_else(std::env::temp_dir);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
        dir.join(format!(
            "{TEMP_FILE_PREFIX}{}-{nanos}-{sequence}.tmp",
            std::process::id()
        ))
    }
}

impl fmt::Debug for Fetcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 传输实现不要求 Debug，只暴露配置
        f.debug_struct("Fetcher")
            .field("max_bytes", &self.max_bytes)
            .field("timeout", &self.timeout)
            .field("temp_dir", &self.temp_dir)
            .finish_non_exhaustive()
    }
}

/// 文件头嗅探优先，只有嗅不出（`application/octet-stream`）时才采信响应头 / `data:` 声明。
///
/// 对齐 Go `http.DetectContentType` 覆盖响应头的做法：服务端 `Content-Type` 经常缺失或是
/// `application/octet-stream`，前 512 字节的文件头比声明更可靠。
fn normalize_content_type(declared: Option<&str>, bytes: &[u8]) -> String {
    let sniffed = sniff::sniff_content_type(bytes);
    if sniffed != sniff::FALLBACK_CONTENT_TYPE {
        return sniffed.to_owned();
    }
    declared
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map_or_else(|| sniffed.to_owned(), str::to_owned)
}

async fn write_all_and_flush(file: &mut tokio::fs::File, bytes: &[u8]) -> Result<()> {
    file.write_all(bytes).await?;
    file.flush().await?;
    Ok(())
}

/// 用 `webpki-root-certs` 的内置根证书构建 rustls 配置（与 gensokyo-qq 同一套根证书）。
fn rustls_config(provider: Arc<CryptoProvider>) -> Result<ClientConfig> {
    let mut roots = RootCertStore::empty();
    let (added, _ignored) =
        roots.add_parsable_certificates(webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().cloned());
    if added == 0 {
        return Err(Error::Transport("内置根证书全部解析失败".to_owned()));
    }

    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|err| Error::Transport(format!("rustls 协议版本配置失败: {err}")))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(config)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{
        BoxFuture, DEFAULT_MAX_BYTES, DEFAULT_TIMEOUT, FetchRequest, Fetcher, MediaTransport,
        TransportResponse,
    };
    use crate::error::{Error, Result};
    use crate::source::MediaSource;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

    #[derive(Clone)]
    enum FakeReply {
        Response(TransportResponse),
        TooLarge { limit: u64, actual: Option<u64> },
    }

    #[derive(Clone)]
    struct FakeTransport {
        calls: Arc<Mutex<Vec<FetchRequest>>>,
        reply: FakeReply,
    }

    impl Default for FakeTransport {
        fn default() -> Self {
            Self {
                calls: Arc::default(),
                reply: FakeReply::Response(TransportResponse {
                    status: 200,
                    content_type: None,
                    body: Vec::new(),
                }),
            }
        }
    }

    impl FakeTransport {
        fn with_reply(reply: FakeReply) -> Self {
            Self {
                reply,
                ..Self::default()
            }
        }

        fn calls(&self) -> Vec<FetchRequest> {
            self.calls.lock().expect("calls 锁未中毒").clone()
        }
    }

    impl MediaTransport for FakeTransport {
        fn get(&self, request: FetchRequest) -> BoxFuture<'_, Result<TransportResponse>> {
            let calls = Arc::clone(&self.calls);
            let reply = self.reply.clone();
            Box::pin(async move {
                calls.lock().expect("calls 锁未中毒").push(request);
                match reply {
                    FakeReply::Response(response) => Ok(response),
                    FakeReply::TooLarge { limit, actual } => Err(Error::TooLarge { limit, actual }),
                }
            })
        }
    }

    /// 每个测试独占的临时目录，析构时整目录删除。
    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("时钟早于 epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "gensokyo-media-test-{label}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).expect("创建测试目录");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn entries(&self) -> Vec<PathBuf> {
            std::fs::read_dir(&self.0)
                .expect("读取测试目录")
                .map(|entry| entry.expect("目录项").path())
                .collect()
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn defaults_match_design() {
        let fetcher = Fetcher::new(FakeTransport::default());
        assert_eq!(fetcher.max_bytes(), DEFAULT_MAX_BYTES);
        assert_eq!(fetcher.timeout(), DEFAULT_TIMEOUT);
        assert_eq!(fetcher.max_bytes(), 20 * 1024 * 1024);
        assert_eq!(fetcher.timeout(), Duration::from_secs(30));
    }

    #[tokio::test]
    async fn base64_source_is_written_to_temp_file() {
        let dir = TestDir::new("base64");
        let fetcher = Fetcher::new(FakeTransport::default()).with_temp_dir(dir.path());
        let source = MediaSource::Base64 {
            mime: Some("image/png".to_owned()),
            data: PNG.to_vec(),
        };

        let fetched = fetcher.fetch(&source).await.expect("落盘成功");
        assert_eq!(fetched.bytes, PNG.len() as u64);
        assert_eq!(fetched.content_type, "image/png");
        assert!(fetched.path.starts_with(dir.path()));
        assert_eq!(std::fs::read(&fetched.path).expect("读回临时文件"), PNG);
    }

    #[tokio::test]
    async fn base64_without_mime_is_sniffed() {
        let dir = TestDir::new("sniff");
        let fetcher = Fetcher::new(FakeTransport::default()).with_temp_dir(dir.path());
        let source = MediaSource::Base64 {
            mime: None,
            data: PNG.to_vec(),
        };
        let fetched = fetcher.fetch(&source).await.expect("落盘成功");
        assert_eq!(fetched.content_type, "image/png");
    }

    #[tokio::test]
    async fn url_request_carries_configured_url_timeout_and_limit() {
        let dir = TestDir::new("url");
        let transport = FakeTransport::with_reply(FakeReply::Response(TransportResponse {
            status: 200,
            content_type: Some("text/plain".to_owned()),
            body: b"RIFF\x24\x00\x00\x00WEBPVP8 ".to_vec(),
        }));
        let fetcher = Fetcher::new(transport.clone())
            .with_max_bytes(1024)
            .with_timeout(Duration::from_secs(7))
            .with_temp_dir(dir.path());

        let fetched = fetcher
            .fetch(&MediaSource::Url("https://example.com/a.bin".to_owned()))
            .await
            .expect("下载成功");
        let calls = transport.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].url, "https://example.com/a.bin");
        assert_eq!(calls[0].timeout, Duration::from_secs(7));
        assert_eq!(calls[0].max_bytes, 1024);
        // 文件头嗅探优先于响应头声明
        assert_eq!(fetched.content_type, "image/webp");
        assert_eq!(
            std::fs::read(&fetched.path).expect("读回临时文件"),
            b"RIFF\x24\x00\x00\x00WEBPVP8 "
        );
    }

    #[tokio::test]
    async fn declared_content_type_used_only_when_sniff_fails() {
        let dir = TestDir::new("content-type");
        let transport = FakeTransport::with_reply(FakeReply::Response(TransportResponse {
            status: 200,
            content_type: Some("audio/silk".to_owned()),
            body: b"not an image".to_vec(),
        }));
        let fetcher = Fetcher::new(transport).with_temp_dir(dir.path());
        let fetched = fetcher
            .fetch(&MediaSource::Url("https://example.com/a.silk".to_owned()))
            .await
            .expect("下载成功");
        assert_eq!(fetched.content_type, "audio/silk");

        // 无声明且嗅不出：兜底 application/octet-stream
        let empty_dir = TestDir::new("content-type-empty");
        let fetcher = Fetcher::new(FakeTransport::default()).with_temp_dir(empty_dir.path());
        let fetched = fetcher
            .fetch(&MediaSource::Base64 {
                mime: None,
                data: b"not an image".to_vec(),
            })
            .await
            .expect("落盘成功");
        assert_eq!(fetched.content_type, "application/octet-stream");
    }

    #[tokio::test]
    async fn sniffed_content_type_overrides_data_uri_mime() {
        let dir = TestDir::new("data-uri-mime");
        let fetcher = Fetcher::new(FakeTransport::default()).with_temp_dir(dir.path());
        let fetched = fetcher
            .fetch(&MediaSource::Base64 {
                mime: Some("image/jpeg".to_owned()),
                data: PNG.to_vec(),
            })
            .await
            .expect("落盘成功");
        assert_eq!(fetched.content_type, "image/png");
    }

    #[tokio::test]
    async fn zero_max_bytes_means_unlimited() {
        let dir = TestDir::new("zero-limit");
        let body = vec![0_u8; 4096];
        let transport = FakeTransport::with_reply(FakeReply::Response(TransportResponse {
            status: 200,
            content_type: None,
            body: body.clone(),
        }));
        let fetcher = Fetcher::new(transport.clone())
            .with_max_bytes(0)
            .with_temp_dir(dir.path());

        let fetched = fetcher
            .fetch(&MediaSource::Url("https://example.com/a".to_owned()))
            .await
            .expect("0 表示不限制");
        assert_eq!(fetched.bytes, body.len() as u64);
        assert_eq!(transport.calls()[0].max_bytes, 0, "传输层也应收到 0");

        // base64 与本地文件同样不受限
        let fetched = fetcher
            .fetch(&MediaSource::Base64 {
                mime: None,
                data: body.clone(),
            })
            .await
            .expect("base64 不限制");
        assert_eq!(fetched.bytes, body.len() as u64);

        let path = dir.path().join("big.png");
        std::fs::write(&path, &body).expect("写测试文件");
        let fetched = fetcher
            .fetch(&MediaSource::File(path))
            .await
            .expect("本地文件不限制");
        assert_eq!(fetched.bytes, body.len() as u64);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn temp_file_is_owner_only_on_unix() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = TestDir::new("mode");
        let fetcher = Fetcher::new(FakeTransport::default()).with_temp_dir(dir.path());
        let fetched = fetcher
            .fetch(&MediaSource::Base64 {
                mime: None,
                data: PNG.to_vec(),
            })
            .await
            .expect("落盘成功");
        let mode = std::fs::metadata(&fetched.path)
            .expect("读取临时文件元数据")
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "临时文件不得对组/其他人可读: {mode:o}");
    }

    #[tokio::test]
    async fn url_without_content_type_falls_back_to_sniff() {
        let dir = TestDir::new("url-sniff");
        let transport = FakeTransport::with_reply(FakeReply::Response(TransportResponse {
            status: 200,
            content_type: Some("   ".to_owned()),
            body: PNG.to_vec(),
        }));
        let fetcher = Fetcher::new(transport).with_temp_dir(dir.path());
        let fetched = fetcher
            .fetch(&MediaSource::Url("https://example.com/a".to_owned()))
            .await
            .expect("下载成功");
        assert_eq!(fetched.content_type, "image/png");
    }

    #[tokio::test]
    async fn transport_too_large_leaves_no_temp_file() {
        let dir = TestDir::new("transport-limit");
        let fetcher = Fetcher::new(FakeTransport::with_reply(FakeReply::TooLarge {
            limit: 16,
            actual: Some(64),
        }))
        .with_temp_dir(dir.path());

        let err = fetcher
            .fetch(&MediaSource::Url("https://example.com/big".to_owned()))
            .await
            .expect_err("应当超限");
        assert!(
            matches!(
                err,
                Error::TooLarge {
                    limit: 16,
                    actual: Some(64)
                }
            ),
            "unexpected error: {err:?}"
        );
        assert!(dir.entries().is_empty(), "失败不得留下临时文件");
    }

    #[tokio::test]
    async fn body_ignoring_transport_limit_is_rejected() {
        let dir = TestDir::new("body-limit");
        // 传输实现没守 max_bytes：Fetcher 必须自己复核
        let transport = FakeTransport::with_reply(FakeReply::Response(TransportResponse {
            status: 200,
            content_type: None,
            body: vec![0_u8; 32],
        }));
        let fetcher = Fetcher::new(transport)
            .with_max_bytes(16)
            .with_temp_dir(dir.path());

        let err = fetcher
            .fetch(&MediaSource::Url("https://example.com/a".to_owned()))
            .await
            .expect_err("应当超限");
        assert!(matches!(
            err,
            Error::TooLarge {
                limit: 16,
                actual: Some(32)
            }
        ));
        assert!(dir.entries().is_empty(), "失败不得留下临时文件");
    }

    #[tokio::test]
    async fn oversized_base64_is_rejected_before_writing() {
        let dir = TestDir::new("base64-limit");
        let fetcher = Fetcher::new(FakeTransport::default())
            .with_max_bytes(4)
            .with_temp_dir(dir.path());
        let source = MediaSource::Base64 {
            mime: None,
            data: vec![1, 2, 3, 4, 5],
        };

        let err = fetcher.fetch(&source).await.expect_err("应当超限");
        assert!(matches!(
            err,
            Error::TooLarge {
                limit: 4,
                actual: Some(5)
            }
        ));
        assert!(dir.entries().is_empty(), "失败不得留下临时文件");
    }

    #[tokio::test]
    async fn http_status_is_rejected_without_temp_file() {
        let dir = TestDir::new("status");
        let transport = FakeTransport::with_reply(FakeReply::Response(TransportResponse {
            status: 404,
            content_type: None,
            body: b"not found".to_vec(),
        }));
        let fetcher = Fetcher::new(transport).with_temp_dir(dir.path());

        let err = fetcher
            .fetch(&MediaSource::Url("https://example.com/missing".to_owned()))
            .await
            .expect_err("应当拒绝非 200");
        assert!(matches!(err, Error::HttpStatus(404)));
        assert!(dir.entries().is_empty(), "失败不得留下临时文件");
    }

    #[tokio::test]
    async fn local_file_is_reused_and_sniffed() {
        let dir = TestDir::new("local");
        let path = dir.path().join("a.png");
        std::fs::write(&path, PNG).expect("写测试文件");
        let fetcher = Fetcher::new(FakeTransport::default()).with_temp_dir(dir.path());

        let fetched = fetcher
            .fetch(&MediaSource::File(path.clone()))
            .await
            .expect("读取本地文件");
        assert_eq!(fetched.path, path, "file:// 复用原路径");
        assert_eq!(fetched.bytes, PNG.len() as u64);
        assert_eq!(fetched.content_type, "image/png");
    }

    #[tokio::test]
    async fn oversized_local_file_is_rejected() {
        let dir = TestDir::new("local-limit");
        let path = dir.path().join("big.png");
        std::fs::write(&path, vec![0_u8; 32]).expect("写测试文件");
        let fetcher = Fetcher::new(FakeTransport::default())
            .with_max_bytes(16)
            .with_temp_dir(dir.path());

        let err = fetcher
            .fetch(&MediaSource::File(path))
            .await
            .expect_err("应当超限");
        assert!(matches!(
            err,
            Error::TooLarge {
                limit: 16,
                actual: Some(32)
            }
        ));
    }

    #[tokio::test]
    async fn missing_local_file_is_io_error() {
        let dir = TestDir::new("missing");
        let fetcher = Fetcher::new(FakeTransport::default()).with_temp_dir(dir.path());
        let err = fetcher
            .fetch(&MediaSource::File(dir.path().join("nope.png")))
            .await
            .expect_err("文件不存在");
        assert!(matches!(err, Error::Io(_)), "unexpected error: {err:?}");
    }

    #[tokio::test]
    async fn temp_paths_are_unique_per_fetch() {
        let dir = TestDir::new("unique");
        let fetcher = Fetcher::new(FakeTransport::default()).with_temp_dir(dir.path());
        let source = MediaSource::Base64 {
            mime: None,
            data: PNG.to_vec(),
        };
        let first = fetcher.fetch(&source).await.expect("第一次落盘");
        let second = fetcher.fetch(&source).await.expect("第二次落盘");
        assert_ne!(first.path, second.path);
        assert_eq!(dir.entries().len(), 2);
    }
}
