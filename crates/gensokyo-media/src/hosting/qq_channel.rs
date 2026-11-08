//! QQ频道图床（oss_type = 6）：向频道发一条图片消息，再按图片 MD5 拼出 `gchat.qpic.cn` 地址。
//!
//! 对齐 Go `imagehosting/qq_channel.go`：
//! - `POST https://api.bot.qq.com/channels/<channel_id>/messages?msg_id=1`；
//! - `multipart/form-data`，图片放在 `file_image` 字段；
//! - `Authorization` 直接填配置里的值（`QQBot xxx.yyy`），留空则不发这个头；
//! - 返回 `https://gchat.qpic.cn/qmeetpic/0/0-0-<大写 MD5>/0`——URL 不来自响应体，而是
//!   图片内容的 MD5（频道把图落到 CDN 后按内容寻址）。
//!
//! 与 Go 的差异：
//! - Go 完全不看这一步的状态码，即使上传失败也返回那条 MD5 URL（失败被吞掉）。这里要求
//!   `2xx`，否则返回 [`Error::HttpStatus`]——宁可报错也不回一个必然 404 的地址。
//! - 文件名用 [`multipart::ensured_filename`]（剥目录 + 修正扩展名）；Go 原样使用入参。

use std::sync::Arc;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::hosting::digest;
use crate::hosting::multipart::{self, MultipartBody};
use crate::hosting::transport::{HostingTransport, UploadBody, UploadMethod, UploadRequest};
use crate::hosting::{DEFAULT_UPLOAD_TIMEOUT, HostingBackend, check_url};

/// 消息端点前缀。
pub const ENDPOINT_PREFIX: &str = "https://api.bot.qq.com/channels";
/// CDN 地址前缀（后面接 `<大写 MD5>/0`）。
pub const CDN_PREFIX: &str = "https://gchat.qpic.cn/qmeetpic/0/0-0-";
/// 表单里的文件字段名（Go `CreateFormFile("file_image", …)`）。
const FILE_FIELD: &str = "file_image";

/// QQ频道配置（对齐 Go `structs.ImageHostingQQChannel`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QqChannelConfig {
    /// 目标子频道 ID。
    pub channel_id: String,
    /// `Authorization` 的完整值（如 `QQBot xxx.yyy`），留空则不发。
    pub token: String,
}

impl QqChannelConfig {
    /// Go `tryQQChannel` 开头的必填项检查。
    fn validate(&self) -> Result<()> {
        if self.channel_id.is_empty() {
            return Err(Error::HostingConfig(
                "QQ频道 未配置（请填写 channel_id）".to_owned(),
            ));
        }
        Ok(())
    }
}

/// 一次 QQ频道上传的纯数据描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QqChannelPlan {
    /// 完整上传 URL。
    pub url: String,
    /// 规整后的文件名。
    pub filename: String,
    /// `Authorization` 头（`None` 表示配置里没填）。
    pub authorization: Option<String>,
    /// 上传成功后返回的 CDN 地址。
    pub result_url: String,
}

/// 构造上传计划。
pub fn plan(config: &QqChannelConfig, filename: &str, data: &[u8]) -> Result<QqChannelPlan> {
    config.validate()?;
    Ok(QqChannelPlan {
        url: format!("{ENDPOINT_PREFIX}/{}/messages?msg_id=1", config.channel_id),
        filename: multipart::ensured_filename(filename, data),
        authorization: (!config.token.is_empty()).then(|| config.token.clone()),
        result_url: format!("{CDN_PREFIX}{}/0", digest::md5_hex_upper(data)),
    })
}

/// QQ频道上传后端。
#[derive(Debug)]
pub struct QqChannelBackend {
    transport: Arc<dyn HostingTransport>,
    config: QqChannelConfig,
    timeout: Duration,
}

impl QqChannelBackend {
    /// 用给定传输、频道配置与默认超时（30s）构造。
    #[must_use]
    pub fn new(transport: Arc<dyn HostingTransport>, config: QqChannelConfig) -> Self {
        Self {
            transport,
            config,
            timeout: DEFAULT_UPLOAD_TIMEOUT,
        }
    }

    /// 覆写单次请求总超时。
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl HostingBackend for QqChannelBackend {
    async fn upload(&self, data: &[u8], filename: &str) -> Result<String> {
        let plan = plan(&self.config, filename, data)?;
        let mut body = MultipartBody::new(multipart::generate_boundary());
        body.file(FILE_FIELD, &plan.filename, data);
        let (content_type, bytes) = body.finish();

        let mut headers = Vec::new();
        if let Some(authorization) = plan.authorization {
            headers.push(("Authorization".to_owned(), authorization));
        }

        let response = self
            .transport
            .send(UploadRequest {
                method: UploadMethod::Post,
                url: plan.url,
                headers,
                body: Some(UploadBody {
                    content_type,
                    bytes,
                }),
                timeout: self.timeout,
            })
            .await?;

        // Go 不看状态码（上传失败也返回 MD5 URL）；这里收紧到 2xx
        if !(200..300).contains(&response.status) {
            return Err(Error::HttpStatus(response.status));
        }
        check_url(&plan.result_url, "QQ频道")?;
        Ok(plan.result_url)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{CDN_PREFIX, ENDPOINT_PREFIX, QqChannelBackend, QqChannelConfig, plan};
    use crate::error::Error;
    use crate::hosting::HostingBackend;
    use crate::hosting::UploadMethod;
    use crate::hosting::fixtures::{FakeTransport, parse_multipart};

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0DIHDR\x00\x00\x02\x80\x00\x00\x01\xE0\x08\x06\x00\x00\x00";

    fn config() -> QqChannelConfig {
        QqChannelConfig {
            channel_id: "123456".to_owned(),
            token: "QQBot abc.def".to_owned(),
        }
    }

    #[test]
    fn plan_builds_endpoint_and_md5_result() {
        let built = plan(&config(), "photo.png", PNG).expect("已配置");
        assert_eq!(
            built.url,
            format!("{ENDPOINT_PREFIX}/123456/messages?msg_id=1")
        );
        assert_eq!(built.filename, "photo.png");
        assert_eq!(built.authorization.as_deref(), Some("QQBot abc.def"));
        // 地址由内容 MD5 决定（RFC 1321 的 "abc" 向量，大写）
        assert_eq!(
            plan(&config(), "a.png", b"abc").expect("已配置").result_url,
            format!("{CDN_PREFIX}900150983CD24FB0D6963F7D28E17F72/0")
        );

        // token 留空：不发 Authorization，但地址照旧
        let anonymous = plan(
            &QqChannelConfig {
                channel_id: "1".to_owned(),
                token: String::new(),
            },
            "a.png",
            b"abc",
        )
        .expect("已配置");
        assert_eq!(anonymous.authorization, None);
        assert_eq!(
            anonymous.result_url,
            format!("{CDN_PREFIX}900150983CD24FB0D6963F7D28E17F72/0")
        );

        // 未配置：不发请求
        let error = plan(&QqChannelConfig::default(), "a.png", PNG).unwrap_err();
        assert!(
            matches!(&error, Error::HostingConfig(reason) if reason.contains("QQ频道 未配置")),
            "{error}"
        );
    }

    #[tokio::test]
    async fn uploads_file_image_and_returns_cdn_url() {
        let transport = Arc::new(FakeTransport::new().respond(200, "{}"));
        let url = QqChannelBackend::new(transport.clone(), config())
            .upload(PNG, "C:\\tmp\\photo.jpg")
            .await
            .expect("上传成功");
        assert!(url.starts_with(CDN_PREFIX));
        assert!(url.ends_with("/0"));

        let calls = transport.calls();
        assert_eq!(calls.len(), 1);
        let request = &calls[0];
        assert_eq!(request.method, UploadMethod::Post);
        assert_eq!(
            request.url,
            format!("{ENDPOINT_PREFIX}/123456/messages?msg_id=1")
        );
        assert!(
            request
                .headers
                .iter()
                .any(|(name, value)| name == "Authorization" && value == "QQBot abc.def")
        );
        let body = request.body.as_ref().expect("有请求体");
        let parts = parse_multipart(&body.content_type, &body.bytes);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].name, "file_image");
        assert_eq!(
            parts[0].filename.as_deref(),
            Some("photo.png"),
            "目录被剥离"
        );
        assert_eq!(parts[0].data, PNG);
    }

    #[tokio::test]
    async fn rejects_non_2xx_and_empty_channel() {
        for status in [400, 403, 500] {
            let transport = FakeTransport::new().respond(status, "denied");
            let error = QqChannelBackend::new(Arc::new(transport), config())
                .upload(PNG, "a.png")
                .await
                .unwrap_err();
            assert!(
                matches!(error, Error::HttpStatus(code) if code == status),
                "{error}"
            );
        }

        let quiet = FakeTransport::new();
        let probe = quiet.clone();
        let error = QqChannelBackend::new(Arc::new(quiet), QqChannelConfig::default())
            .upload(PNG, "a.png")
            .await
            .unwrap_err();
        assert!(matches!(error, Error::HostingConfig(_)), "{error}");
        assert!(probe.calls().is_empty());
    }
}
