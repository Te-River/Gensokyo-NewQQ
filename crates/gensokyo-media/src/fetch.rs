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
//!
//! # SSRF 守卫（P56）
//! - 下载出口 [`HttpTransport`] 关闭 `reqwest` 自动重定向，**手动逐跳跟随**：每跳请求前
//!   （含首跳）复用 [`crate::redirect`] 的逐跳判定——协议白名单 http(s)、IP 字面量直接判、
//!   域名先解析再逐个 IP 判（任一命中即拒、解析失败也拒）。IP 组合复用
//!   `gensokyo_core::net`（RFC1918 + 回环 / 链路本地 / 未指定 / ULA，IPv4-mapped 先归一），
//!   守卫只有这一处实现。
//! - 重定向跳数上限与 [`crate::redirect`] 共用（5 跳）；整链总超时仍按 [`FetchRequest::timeout`]
//!   （默认 30s）。大小上限 / 只认 200 / 落盘语义不变。
//! - 上传出口（hosting 的 [`crate::hosting::HostingTransport`]）复用另一个客户端，
//!   保持原有重定向语义；守卫只管下载。
//!
//! # 与 Go 的差异（P56）
//! - **跳数收紧**：Go 通用下载最多跟随 10 跳，这里 5 跳（对齐本 crate redirect 口径）。
//! - **逐跳复检收紧**：Go `handlers/reply_helpers.go:124-138` 的裸 `client.Get` 无逐跳复检；
//!   这里所有下载路径一律逐跳复检（有意安全差异，lead 裁决）。
//! - **拦截面扩大**：Go `isPrivateOrLoopback` 之外另拦 `0.0.0.0` / `::` / IPv6 ULA。
//! - **无 DNS pinning**：建连时 `reqwest` 会再解析一次 DNS，存在 rebinding 窗口
//!   （与 Go 一致的已知边界）。

use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::Url;
use rustls::crypto::CryptoProvider;
use rustls::{ClientConfig, RootCertStore};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use crate::error::{Error, Result};
use crate::redirect::{self, LazyHttpProbe, RedirectProbe};
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
///
/// 持有两个客户端：下载出口关闭自动重定向、手动逐跳跟随并逐跳复检 SSRF（P56）；
/// 上传出口（hosting 经 [`Self::client`] 复用）保持 reqwest 默认重定向语义，互不影响。
#[derive(Debug, Clone)]
pub struct HttpTransport {
    /// 上传出口（hosting）。
    client: reqwest::Client,
    /// 下载出口：不自动跟随重定向。
    download_client: reqwest::Client,
    /// SSRF 守卫的 DNS 解析出口（生产 [`LazyHttpProbe`]，测试注入假判定）。
    probe: Arc<dyn RedirectProbe>,
}

impl HttpTransport {
    /// 使用进程级 rustls provider 构建客户端。
    ///
    /// 二进制需先安装 provider，例如（`rustls` 启用 `ring` feature 后）：
    /// `rustls::crypto::ring::default_provider().install_default().ok();`
    pub fn new() -> Result<Self> {
        Self::with_provider(require_provider()?)
    }

    /// 使用调用方注入的 rustls provider 构建客户端（`rustls-no-provider` 语义）。
    pub fn with_provider(provider: Arc<CryptoProvider>) -> Result<Self> {
        Self::with_client_config(rustls_config(provider)?)
    }

    /// 使用已构建好的 rustls 配置（便于复用平台校验器）。
    pub fn with_client_config(config: ClientConfig) -> Result<Self> {
        Ok(Self {
            client: build_client(config.clone(), true)?,
            download_client: build_client(config, false)?,
            probe: Arc::new(LazyHttpProbe),
        })
    }

    /// 不跟随重定向的客户端：短签名直链解析要逐跳手动跟随才能逐跳做 SSRF 校验，
    /// 重定向不能交给 `reqwest` 自动处理。provider 口径与 [`Self::new`] 相同。
    pub(crate) fn no_redirect_client() -> Result<reqwest::Client> {
        build_client(rustls_config(require_provider()?)?, false)
    }

    /// 复用底层 `reqwest::Client`：图床上传出口（[`crate::hosting::HostingTransport`]）
    /// 与下载出口共享同一连接池 / TLS 配置，不重复建客户端。
    #[must_use]
    pub(crate) fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// 测试注入：自定义客户端（须关闭自动重定向）与 DNS 判定出口。
    #[cfg(test)]
    fn with_test_parts(client: reqwest::Client, probe: Arc<dyn RedirectProbe>) -> Self {
        Self {
            client: client.clone(),
            download_client: client,
            probe,
        }
    }
}

impl MediaTransport for HttpTransport {
    fn get(&self, request: FetchRequest) -> BoxFuture<'_, Result<TransportResponse>> {
        let client = self.download_client.clone();
        let probe = Arc::clone(&self.probe);
        let FetchRequest {
            url,
            timeout,
            max_bytes,
        } = request;

        Box::pin(async move {
            let operation = async {
                let mut current = Url::parse(&url)
                    .map_err(|err| Error::InvalidSource(format!("URL 解析失败: {err}")))?;
                // 手动逐跳跟随：每跳请求前（含首跳）复检 SSRF，最多 MAX_REDIRECTS 次重定向。
                // 不能交给 reqwest 自动跟随——那样重定向目标就绕过了守卫。
                for _ in 0..=redirect::MAX_REDIRECTS {
                    if !redirect::target_allowed(probe.as_ref(), &current).await {
                        return Err(Error::SsrfBlocked(current.to_string()));
                    }
                    let response = client
                        .get(current.clone())
                        .send()
                        .await
                        .map_err(|err| Error::Transport(err.to_string()))?;
                    let status = response.status().as_u16();
                    match status {
                        // 与 Go `checkResponse` 一致：只接受 200
                        200 => return read_body(response, max_bytes).await,
                        301 | 302 | 303 | 307 | 308 => {
                            let location = response
                                .headers()
                                .get(reqwest::header::LOCATION)
                                .and_then(|value| value.to_str().ok())
                                .ok_or_else(|| {
                                    Error::Transport(format!("重定向缺少 Location: {current}"))
                                })?;
                            current = current.join(location).map_err(|err| {
                                Error::Transport(format!("重定向 Location 非法: {err}"))
                            })?;
                        }
                        _ => return Err(Error::HttpStatus(status)),
                    }
                }
                Err(Error::TooManyRedirects(redirect::MAX_REDIRECTS))
            };

            match tokio::time::timeout(timeout, operation).await {
                Ok(result) => result,
                Err(_elapsed) => Err(Error::Timeout(timeout)),
            }
        })
    }
}

/// 读取 200 响应体：Content-Length 已知超限时先拒绝，流式读满上限即停。
async fn read_body(response: reqwest::Response, max_bytes: u64) -> Result<TransportResponse> {
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
        status: 200,
        content_type,
        body,
    })
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

/// 进程级 rustls provider（由二进制安装）。
fn require_provider() -> Result<Arc<CryptoProvider>> {
    CryptoProvider::get_default().cloned().ok_or_else(|| {
        Error::Transport(
            "未安装 rustls CryptoProvider；二进制需先调用 \
             rustls::crypto::<provider>::default_provider().install_default()"
                .to_owned(),
        )
    })
}

/// 构建 `reqwest` 客户端；`follow_redirects = false` 时由调用方逐跳驱动重定向。
fn build_client(config: ClientConfig, follow_redirects: bool) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .tls_backend_preconfigured(config)
        .connect_timeout(CONNECT_TIMEOUT)
        // 总超时按请求走：不同媒体大小差异大，统一用 reqwest 客户端超时会互相拖累
        .user_agent(concat!("gensokyo-media/", env!("CARGO_PKG_VERSION")));
    if !follow_redirects {
        builder = builder.redirect(reqwest::redirect::Policy::none());
    }
    builder
        .build()
        .map_err(|err| Error::Transport(format!("构建 reqwest 客户端失败: {err}")))
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

#[cfg(test)]
mod ssrf_tests {
    use std::io::{Read as _, Write as _};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::{FetchRequest, Fetcher, HttpTransport, MediaTransport, TransportResponse};
    use crate::error::Error;
    use crate::redirect::MAX_REDIRECTS;
    use crate::redirect::fixtures::FakeProbe;
    use crate::source::MediaSource;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

    fn request(url: &str) -> FetchRequest {
        FetchRequest {
            url: url.to_owned(),
            timeout: Duration::from_secs(5),
            max_bytes: 1024,
        }
    }

    /// 测试进程装 ring provider（生产由二进制 startup 装）：reqwest 构建客户端时
    /// 要求默认 provider 已安装，纯 HTTP 也不例外。幂等。
    fn install_provider() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }

    /// 不预置任何 DNS 映射的客户端：守卫若漏放，连接会失败 / 超时，与 SsrfBlocked 可区分。
    fn offline_client() -> reqwest::Client {
        install_provider();
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .expect("构建测试客户端")
    }

    /// 测试客户端：关闭自动重定向（逐跳由 HttpTransport 驱动），把指定主机名钉到 stub
    /// 地址（reqwest 的 resolve 只覆盖 IP，端口以 URL 为准）；守卫的 DNS 判定另由 FakeProbe 注入。
    fn test_client(addr: SocketAddr, hosts: &[&str]) -> reqwest::Client {
        install_provider();
        // no_proxy：假主机名必须直连 stub，不能被系统代理劫持（代理侧解析不到 *.test）
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy();
        for host in hosts {
            builder = builder.resolve(host, addr);
        }
        builder.build().expect("构建测试客户端")
    }

    fn transport(client: reqwest::Client, probe: FakeProbe) -> HttpTransport {
        HttpTransport::with_test_parts(client, Arc::new(probe))
    }

    /// 连接层错误判定：reqwest 发送失败（含火绒 10053 中止）的 Display 文案。
    /// 其余 Transport 错误（缺 Location / Location 非法 / DNS 任务失败）不在此列。
    fn is_conn_abort(err: &Error) -> bool {
        matches!(err, Error::Transport(message) if message.contains("error sending request"))
    }

    /// 连接层重试（环境缓解，非产品行为）：本机火绒网络过滤驱动会间歇性中止已建立的
    /// 环回 TCP 连接（os error 10053）。仅对连接层错误重试至多 3 次，其余错误
    /// （SSRF 拒绝 / 非 200 / 跳数上限 / 解析失败）原样上抛；每次尝试前清空 stub 已记录
    /// 的目标，重试不会在精确序列断言里留下重复/残缺条目（同 storage retry_seq 口径）。
    async fn get_with_retry(
        transport: &HttpTransport,
        stub: &Stub,
        url: &str,
    ) -> Result<TransportResponse, Error> {
        let mut attempt = 0;
        loop {
            stub.clear_targets();
            match transport.get(request(url)).await {
                Ok(response) => return Ok(response),
                Err(err) if attempt < 3 && is_conn_abort(&err) => attempt += 1,
                Err(err) => return Err(err),
            }
        }
    }

    /// 手写 HTTP stub（同 storage http_remote 测试口径）：专职线程 + std TcpListener，
    /// keep-alive 复用连接缩小火绒连接中止暴露面；端口取专用段（20000–32767），
    /// 避开本机自定义动态端口范围（1024–15000）与出站源端口游标。
    /// responder 按请求 target 决定 (status, Location, body)。
    struct Stub {
        addr: SocketAddr,
        requests: Arc<Mutex<Vec<String>>>,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl Stub {
        fn spawn(
            responder: impl Fn(&str) -> (u16, Option<String>, Vec<u8>) + Send + 'static,
        ) -> Self {
            static NEXT: AtomicU16 = AtomicU16::new(20000);
            let listener = loop {
                let port = NEXT.fetch_add(1, Ordering::Relaxed);
                assert!(port < 32768, "测试端口段耗尽");
                if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
                    break listener;
                }
            };
            let addr = listener.local_addr().expect("本地地址");
            listener.set_nonblocking(true).expect("设置 nonblocking");
            let requests = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let recorded = Arc::clone(&requests);
            let stopped = Arc::clone(&stop);
            let handle = std::thread::spawn(move || {
                while !stopped.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => serve_conn(stream, &recorded, &responder),
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                addr,
                requests,
                stop,
                handle: Some(handle),
            }
        }

        fn targets(&self) -> Vec<String> {
            self.requests.lock().expect("requests 锁").clone()
        }

        fn clear_targets(&self) {
            self.requests.lock().expect("requests 锁").clear();
        }
    }

    impl Drop for Stub {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn serve_conn(
        mut stream: TcpStream,
        requests: &Mutex<Vec<String>>,
        responder: &impl Fn(&str) -> (u16, Option<String>, Vec<u8>),
    ) {
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        // keep-alive：同一连接顺序处理多个请求（同 storage stub 口径）
        loop {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            // GET 无 body：读到请求头结束即可
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => return,
                    Ok(read) => {
                        buf.extend_from_slice(&chunk[..read]);
                        if buf.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => return,
                }
            }
            let head = String::from_utf8_lossy(&buf);
            let Some(target) = head
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
            else {
                return;
            };
            requests.lock().expect("requests 锁").push(target.to_owned());
            let (status, location, body) = responder(target);
            let mut response =
                format!("HTTP/1.1 {status} Status\r\nContent-Length: {}\r\n", body.len());
            if let Some(location) = location {
                response.push_str(&format!("Location: {location}\r\n"));
            }
            response.push_str("\r\n");
            if stream.write_all(response.as_bytes()).is_err() {
                return;
            }
            if stream.write_all(&body).is_err() {
                return;
            }
            let _ = stream.flush();
        }
    }

    #[tokio::test]
    async fn private_literal_targets_are_rejected_before_any_request() {
        let transport = transport(offline_client(), FakeProbe::new());
        for url in [
            "http://127.0.0.1/a.png",
            "http://10.1.2.3/a.png",
            "http://172.16.0.1/a.png",
            "http://192.168.1.1/a.png",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]/a.png",
            "http://0.0.0.0/a.png",
            "http://[::]/a.png",
            "http://[::ffff:127.0.0.1]/a.png",
            "http://[::ffff:192.168.1.1]/a.png",
            "http://[fc00::1]/a.png",
        ] {
            let err = transport
                .get(request(url))
                .await
                .expect_err("私网目标必须拒绝");
            assert!(matches!(err, Error::SsrfBlocked(_)), "{url}: {err:?}");
        }
    }

    #[tokio::test]
    async fn hostname_resolving_to_private_is_rejected() {
        let probe = FakeProbe::new().resolve_to("intranet.test", &["10.0.0.5"]);
        let transport = transport(offline_client(), probe);
        let err = transport
            .get(request("http://intranet.test/a.png"))
            .await
            .expect_err("解析到私网必须拒绝");
        assert!(matches!(err, Error::SsrfBlocked(_)), "{err:?}");
    }

    #[tokio::test]
    async fn resolution_failure_is_rejected() {
        let probe = FakeProbe::new().fail_resolve("nx.test");
        let transport = transport(offline_client(), probe);
        let err = transport
            .get(request("http://nx.test/a.png"))
            .await
            .expect_err("解析失败必须拒绝");
        assert!(matches!(err, Error::SsrfBlocked(_)), "{err:?}");
    }

    #[tokio::test]
    async fn non_http_scheme_and_unparsable_url_are_rejected() {
        let transport = transport(offline_client(), FakeProbe::new());
        let err = transport
            .get(request("ftp://example.com/a.png"))
            .await
            .expect_err("非 http(s) 必须拒绝");
        assert!(matches!(err, Error::SsrfBlocked(_)), "{err:?}");
        let err = transport
            .get(request("http://"))
            .await
            .expect_err("解析失败必须拒绝");
        assert!(matches!(err, Error::InvalidSource(_)), "{err:?}");
    }

    #[tokio::test]
    async fn redirect_to_private_literal_is_rejected_per_hop() {
        let stub = Stub::spawn(|target| {
            if target == "/start" {
                (302, Some("http://127.0.0.1:9/a.png".to_owned()), Vec::new())
            } else {
                (200, None, PNG.to_vec())
            }
        });
        let client = test_client(stub.addr, &["start.test"]);
        let transport = transport(client, FakeProbe::new());
        let url = format!("http://start.test:{}/start", stub.addr.port());

        let err = get_with_retry(&transport, &stub, &url)
            .await
            .expect_err("重定向到私网必须拒绝");
        assert!(matches!(err, Error::SsrfBlocked(_)), "{err:?}");
        assert_eq!(
            stub.targets(),
            vec!["/start".to_owned()],
            "私网那一跳不得发出请求"
        );
    }

    #[tokio::test]
    async fn redirect_to_private_resolving_host_is_rejected_per_hop() {
        let stub = Stub::spawn(|target| {
            if target == "/start" {
                (302, Some("http://intranet.test/a.png".to_owned()), Vec::new())
            } else {
                (200, None, PNG.to_vec())
            }
        });
        let client = test_client(stub.addr, &["start.test"]);
        let probe = FakeProbe::new().resolve_to("intranet.test", &["192.168.0.1"]);
        let transport = transport(client, probe);
        let url = format!("http://start.test:{}/start", stub.addr.port());

        let err = get_with_retry(&transport, &stub, &url)
            .await
            .expect_err("重定向到私网解析域名必须拒绝");
        assert!(matches!(err, Error::SsrfBlocked(_)), "{err:?}");
        assert_eq!(stub.targets(), vec!["/start".to_owned()]);
    }

    #[tokio::test]
    async fn redirect_chain_is_followed_to_final_body() {
        let stub = Stub::spawn(|target| match target {
            "/start" => (302, Some("/ok".to_owned()), Vec::new()),
            "/ok" => (200, None, PNG.to_vec()),
            _ => (404, None, Vec::new()),
        });
        let client = test_client(stub.addr, &["chain.test"]);
        let transport = transport(client, FakeProbe::new());
        let url = format!("http://chain.test:{}/start", stub.addr.port());

        let response = get_with_retry(&transport, &stub, &url)
            .await
            .expect("下载成功");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, PNG);
        assert_eq!(stub.targets(), vec!["/start".to_owned(), "/ok".to_owned()]);
    }

    #[tokio::test]
    async fn hop_limit_is_enforced() {
        let stub = Stub::spawn(|target| {
            let hop: u32 = target.trim_start_matches("/h").parse().expect("跳数");
            (302, Some(format!("/h{}", hop + 1)), Vec::new())
        });
        let client = test_client(stub.addr, &["hop.test"]);
        let transport = transport(client, FakeProbe::new());
        let url = format!("http://hop.test:{}/h0", stub.addr.port());

        let err = get_with_retry(&transport, &stub, &url)
            .await
            .expect_err("跳数上限必须拒绝");
        assert!(matches!(err, Error::TooManyRedirects(MAX_REDIRECTS)), "{err:?}");
        assert_eq!(
            stub.targets().len(),
            MAX_REDIRECTS + 1,
            "最多 6 次请求 = 首跳 + 5 次重定向"
        );
    }

    #[tokio::test]
    async fn redirect_without_location_is_transport_error() {
        let stub = Stub::spawn(|_| (302, None, Vec::new()));
        let client = test_client(stub.addr, &["noloc.test"]);
        let transport = transport(client, FakeProbe::new());
        let url = format!("http://noloc.test:{}/start", stub.addr.port());

        let err = get_with_retry(&transport, &stub, &url)
            .await
            .expect_err("缺少 Location 必须报错");
        assert!(matches!(err, Error::Transport(_)), "{err:?}");
    }

    #[tokio::test]
    async fn final_non_200_and_oversize_body_are_rejected() {
        let stub = Stub::spawn(|target| match target {
            "/big" => (200, None, vec![0u8; 2048]),
            _ => (404, None, Vec::new()),
        });
        let client = test_client(stub.addr, &["gate.test"]);
        let transport = transport(client, FakeProbe::new());
        let base = format!("http://gate.test:{}", stub.addr.port());

        let err = get_with_retry(&transport, &stub, &format!("{base}/missing"))
            .await
            .expect_err("非 200 必须拒绝");
        assert!(matches!(err, Error::HttpStatus(404)), "{err:?}");

        let err = get_with_retry(&transport, &stub, &format!("{base}/big"))
            .await
            .expect_err("超限必须拒绝");
        assert!(matches!(err, Error::TooLarge { limit: 1024, .. }), "{err:?}");
    }

    #[tokio::test]
    async fn fetcher_url_source_is_guarded_before_download() {
        let transport = transport(offline_client(), FakeProbe::new());
        let fetcher = Fetcher::new(transport);
        let err = fetcher
            .fetch(&MediaSource::Url("http://192.168.1.1/a.png".to_owned()))
            .await
            .expect_err("私网 URL 必须拒绝");
        assert!(matches!(err, Error::SsrfBlocked(_)), "{err:?}");
    }
}
