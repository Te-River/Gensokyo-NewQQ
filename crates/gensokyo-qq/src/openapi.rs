//! QQ OpenAPI v2 客户端（对齐 `botgo/openapi/v2`）。
//!
//! 所有请求都经过 [`HttpTransport`]，因此可在无网络环境下用 FakeTransport 断言
//! URL / 方法 / 请求头 / 请求体。鉴权头格式为 `Authorization: QQBot <token>`。

use std::fmt;
use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::dto::{FileUpload, MediaResponse, MessageResponse, MessageToCreate};
use crate::error::{ApiError, Error, Result};
use crate::http::{HttpMethod, HttpRequest, HttpResponse, HttpTransport, encode_path_segment};
use crate::token::TokenManager;

/// 正式环境基址（`botgo/openapi/v2/resource.go` 的 `domain`）。
pub const DEFAULT_BASE_URL: &str = "https://api.bot.qq.com";
/// 沙箱基址；官方已把沙箱域名并入 `api.bot.qq.com`，保留常量以对齐 botgo。
pub const SANDBOX_BASE_URL: &str = "https://api.bot.qq.com";

/// 上游链路 ID 响应头。
const TRACE_ID_HEADER: &str = "X-Tps-trace-ID";
/// 鉴权 scheme（botgo `OnBeforeRequest` 硬编码为 `QQBot`）。
const AUTH_SCHEME: &str = "QQBot";

/// QQ OpenAPI v2 客户端。
pub struct OpenApiClient<T: HttpTransport> {
    transport: Arc<T>,
    tokens: Arc<TokenManager<T>>,
    base_url: String,
}

impl<T: HttpTransport> fmt::Debug for OpenApiClient<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenApiClient")
            .field("base_url", &self.base_url)
            .field("tokens", &self.tokens)
            .finish_non_exhaustive()
    }
}

impl<T: HttpTransport> OpenApiClient<T> {
    /// 用共享的传输层与 token 管理器创建客户端。
    #[must_use]
    pub fn new(transport: Arc<T>, tokens: Arc<TokenManager<T>>) -> Self {
        Self {
            transport,
            tokens,
            base_url: DEFAULT_BASE_URL.to_owned(),
        }
    }

    /// 覆盖基址（测试或私有网关用）。
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_owned();
        self
    }

    /// 按沙箱开关选择基址。
    #[must_use]
    pub fn with_sandbox(self, sandbox: bool) -> Self {
        if sandbox {
            self.with_base_url(SANDBOX_BASE_URL)
        } else {
            self.with_base_url(DEFAULT_BASE_URL)
        }
    }

    /// 当前基址。
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// 共享的 token 管理器。
    #[must_use]
    pub fn token_manager(&self) -> &TokenManager<T> {
        &self.tokens
    }

    /// 发送群消息（`POST /v2/groups/{group_openid}/messages`）。
    pub async fn send_group_message(
        &self,
        group_openid: &str,
        message: &MessageToCreate,
    ) -> Result<MessageResponse> {
        message.validate()?;
        let path = format!("/v2/groups/{}/messages", encode_path_segment(group_openid));
        self.post_json(&path, message).await
    }

    /// 发送 C2C 消息（`POST /v2/users/{user_openid}/messages`）。
    pub async fn send_c2c_message(
        &self,
        user_openid: &str,
        message: &MessageToCreate,
    ) -> Result<MessageResponse> {
        message.validate()?;
        let path = format!("/v2/users/{}/messages", encode_path_segment(user_openid));
        self.post_json(&path, message).await
    }

    /// 上传群文件 / 媒体（`POST /v2/groups/{group_openid}/files`）。
    ///
    /// 请求体为 JSON（`file_type` + `url` / `file_data`），与 botgo
    /// `PostGroupMessage(RichMediaMessage)` 一致；botgo 的 multipart 只用于频道消息。
    pub async fn upload_group_file(
        &self,
        group_openid: &str,
        file: &FileUpload,
    ) -> Result<MediaResponse> {
        file.validate()?;
        let path = format!("/v2/groups/{}/files", encode_path_segment(group_openid));
        self.post_json(&path, file).await
    }

    /// 上传 C2C 文件 / 媒体（`POST /v2/users/{user_openid}/files`）。
    pub async fn upload_c2c_file(
        &self,
        user_openid: &str,
        file: &FileUpload,
    ) -> Result<MediaResponse> {
        file.validate()?;
        let path = format!("/v2/users/{}/files", encode_path_segment(user_openid));
        self.post_json(&path, file).await
    }

    /// 发起 GET 并解析 JSON 响应（群管理子集复用）。
    pub(crate) async fn get_json<R: DeserializeOwned>(&self, path: &str) -> Result<R> {
        let response = self.authorized(HttpMethod::Get, path, None).await?;
        decode_json(&response)
    }

    /// 发起 POST 并解析 JSON 响应。
    pub(crate) async fn post_json<B: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<R> {
        let response = self.post_authorized(path, body).await?;
        decode_json(&response)
    }

    /// 发起 POST 并忽略响应体（`204` / 空 body 的写接口）。
    pub(crate) async fn post_json_void<B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<()> {
        self.post_authorized(path, body).await.map(|_| ())
    }

    async fn post_authorized<B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<HttpResponse> {
        let body = serde_json::to_vec(body)?;
        self.authorized(HttpMethod::Post, path, Some(body)).await
    }

    /// 注入 token 与通用请求头，并把非 2xx 响应映射为 [`ApiError`]。
    async fn authorized(
        &self,
        method: HttpMethod,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<HttpResponse> {
        let token = self.tokens.token().await?;
        let mut request = HttpRequest::new(method, format!("{}{path}", self.base_url))
            .with_header("Authorization", format!("{AUTH_SCHEME} {token}"))
            .with_header("X-Union-Appid", self.tokens.config().app_id())
            .with_header("Accept", "application/json")
            .with_header(
                "User-Agent",
                concat!("gensokyo-qq/", env!("CARGO_PKG_VERSION")),
            );
        if body.is_some() {
            request = request.with_header("Content-Type", "application/json");
        }
        if let Some(body) = body {
            request = request.with_body(body);
        }

        let response = self.transport.request(request).await?;
        if !(200..300).contains(&response.status) {
            // token 失效时丢弃缓存，下一次调用会重新获取；本次仍把上游错误原样上抛。
            if response.status == 401 {
                self.tokens.invalidate().await;
            }
            return Err(ApiError::from_response(
                response.status,
                &response.body,
                response.header(TRACE_ID_HEADER),
            )
            .into());
        }
        Ok(response)
    }
}

/// 解析 JSON 响应体，失败时给出状态码与解析错误（不回显 body）。
fn decode_json<R: DeserializeOwned>(response: &HttpResponse) -> Result<R> {
    serde_json::from_slice(&response.body).map_err(|err| {
        Error::InvalidResponse(format!("解析响应失败(status={}): {err}", response.status))
    })
}
