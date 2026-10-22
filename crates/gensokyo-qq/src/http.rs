//! HTTP 传输 seam：所有 OpenAPI 调用都经过 [`HttpTransport`]。
//!
//! 生产实现 [`ReqwestTransport`] 基于 `reqwest 0.13` + `rustls`；测试实现只需实现
//! trait，即可在无网络环境下断言 URL / 方法 / 请求头 / 请求体。
//!
//! # TLS provider
//! 本 crate 只启用 `rustls-no-provider`：provider（`ring` / `aws-lc-rs`）由二进制侧选择，
//! 通过 [`ReqwestTransport::with_provider`] 注入；这样 workspace 不会被迫引入 C 工具链。
//! 已安装进程级 provider 时可直接用 [`ReqwestTransport::new`]。

use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use rustls::crypto::CryptoProvider;
use rustls::{ClientConfig, RootCertStore};

use crate::error::{Error, Result};

/// 连接超时（含 DNS / TCP / TLS 握手），与 botgo 的 `TLSHandshakeTimeout` 对齐。
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 单次请求总超时。botgo 默认 3s，本 crate 取 10s 以容纳媒体上传，可通过
/// [`ReqwestTransport::from_client`] 换用自定义 `reqwest::Client`。
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// HTTP 方法（本 crate 只用到 GET / POST）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HttpMethod {
    /// GET。
    Get,
    /// POST。
    Post,
}

impl HttpMethod {
    /// 线上的方法名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

impl fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 与传输实现无关的请求描述。
#[derive(Clone)]
pub struct HttpRequest {
    /// 方法。
    pub method: HttpMethod,
    /// 完整 URL。
    pub url: String,
    /// 请求头，按插入顺序保留（便于测试与审计）。
    pub headers: Vec<(String, String)>,
    /// 原始请求体；空表示无 body。
    pub body: Vec<u8>,
}

impl HttpRequest {
    /// 构造一个无 body 的请求。
    #[must_use]
    pub fn new(method: HttpMethod, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// 构造 GET 请求。
    #[must_use]
    pub fn get(url: impl Into<String>) -> Self {
        Self::new(HttpMethod::Get, url)
    }

    /// 构造 POST 请求。
    #[must_use]
    pub fn post(url: impl Into<String>) -> Self {
        Self::new(HttpMethod::Post, url)
    }

    /// 追加请求头。
    #[must_use]
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// 设置请求体。
    #[must_use]
    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    /// 读取同名请求头（大小写不敏感）。
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 绝不把 Authorization（含 access_token）或请求体写进 Debug 输出。
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(name, value)| {
                if name.eq_ignore_ascii_case("authorization") {
                    (name.as_str(), "<redacted>")
                } else {
                    (name.as_str(), value.as_str())
                }
            })
            .collect();
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &headers)
            .field("body_len", &self.body.len())
            .finish()
    }
}

/// 与传输实现无关的响应描述。
#[derive(Clone)]
pub struct HttpResponse {
    /// HTTP 状态码。
    pub status: u16,
    /// 响应头。
    pub headers: Vec<(String, String)>,
    /// 原始响应体。
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// 构造一个空响应。
    #[must_use]
    pub fn new(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// 追加响应头。
    #[must_use]
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// 设置响应体。
    #[must_use]
    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    /// 读取同名响应头（大小写不敏感）。
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

impl fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .field("body_len", &self.body.len())
            .finish()
    }
}

/// HTTP 传输抽象。
///
/// 声明为 `fn request(..) -> impl Future + Send`（等价于 `async fn request`）：显式
/// `Send` 约束保证客户端 future 可以被 `tokio::spawn`，同时避免 `async_fn_in_trait`
/// 对公开 trait 的告警。
pub trait HttpTransport: Send + Sync {
    /// 发起一次请求。
    fn request(&self, req: HttpRequest) -> impl Future<Output = Result<HttpResponse>> + Send;
}

/// 生产传输：`reqwest` + `rustls`。
#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    /// 使用进程级 rustls provider 构建客户端。
    ///
    /// 二进制需先安装 provider，例如（`rustls` 启用 `ring` feature 后）：
    /// `rustls::crypto::ring::default_provider().install_default().ok();`
    pub fn new() -> Result<Self> {
        let provider = CryptoProvider::get_default().ok_or_else(|| {
            Error::Tls(
                "未安装 rustls CryptoProvider；二进制需先调用 \
                 rustls::crypto::<provider>::default_provider().install_default()"
                    .to_owned(),
            )
        })?;
        Self::with_provider(Arc::clone(provider))
    }

    /// 使用指定的 rustls provider 构建客户端。
    pub fn with_provider(provider: Arc<CryptoProvider>) -> Result<Self> {
        Self::with_client_config(rustls_config(provider)?)
    }

    /// 使用已构建好的 rustls 配置构建客户端（便于复用平台校验器）。
    pub fn with_client_config(config: ClientConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .tls_backend_preconfigured(config)
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
            .timeout(DEFAULT_REQUEST_TIMEOUT)
            .user_agent(concat!("gensokyo-qq/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|err| Error::Transport(format!("构建 reqwest 客户端失败: {err}")))?;
        Ok(Self { client })
    }

    /// 复用外部已配置好的 `reqwest::Client`（超时、代理、TLS 均由调用方决定）。
    #[must_use]
    pub fn from_client(client: reqwest::Client) -> Self {
        Self { client }
    }
}

impl HttpTransport for ReqwestTransport {
    fn request(&self, req: HttpRequest) -> impl Future<Output = Result<HttpResponse>> + Send {
        let client = self.client.clone();

        async move {
            let method = match req.method {
                HttpMethod::Get => reqwest::Method::GET,
                HttpMethod::Post => reqwest::Method::POST,
            };
            let mut builder = client.request(method, &req.url);
            for (name, value) in &req.headers {
                builder = builder.header(name.as_str(), value.as_str());
            }
            if !req.body.is_empty() {
                builder = builder.body(req.body);
            }

            let response = builder
                .send()
                .await
                .map_err(|err| Error::Transport(err.to_string()))?;
            let status = response.status().as_u16();
            let headers = response
                .headers()
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_owned(),
                        value.to_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect();
            let body = response
                .bytes()
                .await
                .map_err(|err| Error::Transport(err.to_string()))?
                .to_vec();

            Ok(HttpResponse {
                status,
                headers,
                body,
            })
        }
    }
}

/// 用 `webpki-root-certs` 的内置根证书构建 rustls 配置。
fn rustls_config(provider: Arc<CryptoProvider>) -> Result<ClientConfig> {
    let mut roots = RootCertStore::empty();
    let (added, _ignored) =
        roots.add_parsable_certificates(webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().cloned());
    if added == 0 {
        return Err(Error::Tls("内置根证书全部解析失败".to_owned()));
    }

    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|err| Error::Tls(format!("rustls 协议版本配置失败: {err}")))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(config)
}

/// RFC 3986 path segment 编码：保留 `A-Z a-z 0-9 - . _ ~`，其余按 UTF-8 字节转 `%XX`。
///
/// QQ 的 openid 通常只含字母数字，但拼接进 URL 前统一编码，避免 openid 中出现 `/`、`?`
/// 等字符时改变路径语义。
pub(crate) fn encode_path_segment(raw: &str) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::encode_path_segment;

    #[test]
    fn keeps_unreserved_characters() {
        assert_eq!(encode_path_segment("ABC-123_.~"), "ABC-123_.~");
    }

    #[test]
    fn encodes_path_metacharacters() {
        assert_eq!(encode_path_segment("a/b c?d"), "a%2Fb%20c%3Fd");
    }

    #[test]
    fn encodes_utf8_by_byte() {
        assert_eq!(encode_path_segment("群"), "%E7%BE%A4");
    }

    #[test]
    fn debug_redacts_authorization_header() {
        let request = super::HttpRequest::post("https://example.com")
            .with_header("Authorization", "QQBot secret-token")
            .with_body(b"secret-body".to_vec());
        let debug = format!("{request:?}");
        assert!(!debug.contains("secret-token"), "{debug}");
        assert!(!debug.contains("secret-body"), "{debug}");
        assert!(debug.contains("<redacted>"), "{debug}");
    }
}
