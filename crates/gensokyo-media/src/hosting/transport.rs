//! 上传请求的传输出口。
//!
//! 与 [`crate::fetch::MediaTransport`] 同一模式（dyn 兼容的装箱 future + 测试替身），但
//! 语义是「写」：需要 POST / GET、请求头与 multipart body，而下载出口只有 GET。
//! 生产实现直接复用 [`crate::fetch::HttpTransport`] 的 `reqwest::Client`（连接池、TLS、
//! 代理全共享），测试实现只需记录 [`UploadRequest`] 即可断言 URL / 头 / 字段，全程不用外网。

use std::time::Duration;

use crate::error::{Error, Result};
use crate::fetch::{BoxFuture, HttpTransport};

/// 图床响应体上限（对齐 Go `imagehosting.maxProviderResponseBytes` 的 4MiB）。
pub const MAX_RESPONSE_BYTES: usize = 4 << 20;

/// 图床请求统一 UA（对齐 Go `imagehosting` 各 provider 的 `User-Agent`）。
pub(crate) const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36";
/// `User-Agent` 头名。
pub(crate) const USER_AGENT_HEADER: &str = "User-Agent";

/// 上传请求的 HTTP 方法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UploadMethod {
    /// 取签名 / 查询。
    Get,
    /// multipart 上传。
    Post,
    /// 原始字节直传（签名 URL 场景，M2b 未实现）。
    Put,
}

impl UploadMethod {
    /// 方法名（用于日志与测试断言）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
        }
    }

    fn to_reqwest(self) -> reqwest::Method {
        match self {
            Self::Get => reqwest::Method::GET,
            Self::Post => reqwest::Method::POST,
            Self::Put => reqwest::Method::PUT,
        }
    }
}

/// 请求体：已经编码好的字节 + 它的 `Content-Type`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadBody {
    /// 完整 `Content-Type`（multipart 时含 boundary）。
    pub content_type: String,
    /// 请求体字节。
    pub bytes: Vec<u8>,
}

/// 一次上传相关的 HTTP 请求（已经过编码，传输实现只负责发出）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadRequest {
    /// HTTP 方法。
    pub method: UploadMethod,
    /// 完整 URL（含已编码的查询串）。
    pub url: String,
    /// 附加请求头；同名头以先出现的为准（reqwest 追加语义）。
    pub headers: Vec<(String, String)>,
    /// 请求体；`None` 表示无 body（GET）。
    pub body: Option<UploadBody>,
    /// 单次请求总超时（含响应体读取）。
    pub timeout: Duration,
}

/// 传输层返回的原始响应。
///
/// 非 2xx **不在这里变成错误**：各图床的成功判定不同（ChatGLM 只认 200，Ukaka 认 `< 300`），
/// 由后端自己决定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadResponse {
    /// HTTP 状态码。
    pub status: u16,
    /// 响应体（不超过 [`MAX_RESPONSE_BYTES`]）。
    pub body: Vec<u8>,
}

/// 上传网络出口。
///
/// 显式 `Debug` 约束让持有 `Arc<dyn HostingTransport>` 的后端也能 `derive(Debug)`；
/// 显式 `Send + Sync` 保证后端的 future 可以被 `tokio::spawn`。
pub trait HostingTransport: Send + Sync + std::fmt::Debug {
    /// 发出请求并读回完整响应体。
    fn send(&self, request: UploadRequest) -> BoxFuture<'_, Result<UploadResponse>>;
}

impl HostingTransport for HttpTransport {
    fn send(&self, request: UploadRequest) -> BoxFuture<'_, Result<UploadResponse>> {
        let client = self.client().clone();
        Box::pin(async move {
            let UploadRequest {
                method,
                url,
                headers,
                body,
                timeout,
            } = request;
            let operation = async {
                let mut builder = client.request(method.to_reqwest(), &url);
                for (name, value) in &headers {
                    builder = builder.header(name.as_str(), value.as_str());
                }
                if let Some(body) = body {
                    builder = builder
                        .header(reqwest::header::CONTENT_TYPE, body.content_type)
                        .body(body.bytes);
                }
                let response = builder
                    .send()
                    .await
                    .map_err(|err| Error::Transport(err.to_string()))?;
                let status = response.status().as_u16();
                // 先看 Content-Length：超限就不必把响应读进内存
                if let Some(length) = response.content_length()
                    && length > MAX_RESPONSE_BYTES as u64
                {
                    return Err(Error::TooLarge {
                        limit: MAX_RESPONSE_BYTES as u64,
                        actual: Some(length),
                    });
                }
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|err| Error::Transport(err.to_string()))?;
                if bytes.len() > MAX_RESPONSE_BYTES {
                    return Err(Error::TooLarge {
                        limit: MAX_RESPONSE_BYTES as u64,
                        actual: Some(bytes.len() as u64),
                    });
                }
                Ok(UploadResponse {
                    status,
                    body: bytes.to_vec(),
                })
            };
            tokio::time::timeout(timeout, operation)
                .await
                .map_err(|_| Error::Timeout(timeout))?
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{HostingTransport, MAX_RESPONSE_BYTES, UploadMethod};
    use crate::fetch::HttpTransport;

    #[test]
    fn method_names_are_stable() {
        assert_eq!(UploadMethod::Get.as_str(), "GET");
        assert_eq!(UploadMethod::Post.as_str(), "POST");
        assert_eq!(UploadMethod::Put.as_str(), "PUT");
    }

    #[test]
    fn http_transport_serves_the_upload_seam() {
        // 编译期断言：生产下载传输同时是上传出口的默认实现（复用同一 reqwest::Client）
        fn assert_transport<T: HostingTransport>() {}
        assert_transport::<HttpTransport>();
        assert_eq!(MAX_RESPONSE_BYTES, 4 * 1024 * 1024);
    }
}
