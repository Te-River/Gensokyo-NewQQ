//! 星野签名图床（oss_type = 9）。
//!
//! 对齐 Go `imagehosting/signed.go` 的 `signedUpload(…, "xingye")` 分支，两步走：
//! 1. 取签名（与 Ukaka 同一服务、同一请求，见 [`super::signed`]）；
//! 2. `PUT` 签名给的地址，原始字节直传；`Content-Type` 取签名响应的 `header.Content-Type`，
//!    缺失或为空时用图片嗅探结果（Go 同款）；
//! 3. 上传响应 `status < 300` 视为成功，返回签名响应里的 `resourceUrl`。

use std::sync::Arc;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::hosting::signed::{self, SignedPlan};
use crate::hosting::transport::{
    HostingTransport, USER_AGENT, USER_AGENT_HEADER, UploadBody, UploadMethod, UploadRequest,
};
use crate::hosting::{DEFAULT_UPLOAD_TIMEOUT, HostingBackend, check_url};

pub use crate::hosting::signed::{SIGN_ORIGIN, SIGN_URL};

/// 签名服务要求的模块名（Go `tryXingye` 传的 `"xingye"`）。
pub const MODULE: &str = "xingye";

/// 构造签名计划。
#[must_use]
pub fn plan(filename: &str, data: &[u8]) -> SignedPlan {
    signed::plan(MODULE, filename, data)
}

/// 星野上传后端。
#[derive(Debug)]
pub struct XingyeBackend {
    transport: Arc<dyn HostingTransport>,
    timeout: Duration,
}

impl XingyeBackend {
    /// 用给定传输与默认超时（30s）构造。
    #[must_use]
    pub fn new(transport: Arc<dyn HostingTransport>) -> Self {
        Self {
            transport,
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

impl HostingBackend for XingyeBackend {
    async fn upload(&self, data: &[u8], filename: &str) -> Result<String> {
        let plan = plan(filename, data);
        let target = signed::request(&*self.transport, &plan, self.timeout).await?;

        // Go：`ct := signData.Header["Content-Type"]; if ct == "" { ct = mime }`
        let content_type = target
            .header("Content-Type")
            .filter(|value| !value.is_empty())
            .unwrap_or(plan.mime)
            .to_owned();

        let response = self
            .transport
            .send(UploadRequest {
                method: UploadMethod::Put,
                url: target.upload_url.clone(),
                headers: vec![(USER_AGENT_HEADER.to_owned(), USER_AGENT.to_owned())],
                body: Some(UploadBody {
                    content_type,
                    bytes: data.to_vec(),
                }),
                timeout: self.timeout,
            })
            .await?;
        // Go：`resp.StatusCode >= 300` 才算失败
        if response.status >= 300 {
            return Err(Error::HttpStatus(response.status));
        }
        check_url(&target.resource_url, "星野")?;
        Ok(target.resource_url)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{MODULE, SIGN_URL, XingyeBackend, plan};
    use crate::error::Error;
    use crate::hosting::HostingBackend;
    use crate::hosting::UploadMethod;
    use crate::hosting::fixtures::FakeTransport;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0DIHDR\x00\x00\x02\x80\x00\x00\x01\xE0\x08\x06\x00\x00\x00";

    fn sign_body(content_type: &str) -> String {
        format!(
            r#"{{"url":"https://upload.example/xingye","resourceUrl":"https://cdn.example/x.png",
                "header":{{"Content-Type":"{content_type}"}},"body":{{}}}}"#
        )
    }

    #[test]
    fn plan_uses_xingye_module() {
        let plan = plan("a.jpg", PNG);
        assert_eq!(plan.module, MODULE);
        assert_eq!(plan.filename, "a.png");
        assert_eq!(
            plan.sign_url,
            format!("{SIGN_URL}?filename=a.png&mimeType=image%2Fpng&module=xingye")
        );
    }

    #[tokio::test]
    async fn signs_then_puts_raw_bytes() {
        let transport = Arc::new(
            FakeTransport::new()
                .respond(200, &sign_body("image/png"))
                .respond(201, ""),
        );
        let url = XingyeBackend::new(transport.clone())
            .upload(PNG, "a.png")
            .await
            .expect("上传成功");
        assert_eq!(url, "https://cdn.example/x.png");

        let calls = transport.calls();
        assert_eq!(calls.len(), 2, "先签名再 PUT");
        assert_eq!(calls[0].method, UploadMethod::Get);
        let upload = &calls[1];
        assert_eq!(
            upload.method,
            UploadMethod::Put,
            "星野是 PUT 直传，不是 multipart"
        );
        assert_eq!(upload.url, "https://upload.example/xingye");
        assert!(
            upload
                .headers
                .iter()
                .any(|(name, value)| name == "User-Agent" && value.starts_with("Mozilla/5.0"))
        );
        let body = upload.body.as_ref().expect("有请求体");
        assert_eq!(body.content_type, "image/png", "取自签名响应的 header");
        assert_eq!(body.bytes, PNG);
    }

    #[tokio::test]
    async fn falls_back_to_sniffed_mime_when_header_missing() {
        let transport = Arc::new(
            FakeTransport::new()
                .respond(200, r#"{"url":"https://upload.example/x","resourceUrl":"https://cdn.example/x.png"}"#)
                .respond(200, ""),
        );
        XingyeBackend::new(transport.clone())
            .upload(PNG, "a.png")
            .await
            .expect("上传成功");
        let calls = transport.calls();
        assert_eq!(
            calls[1].body.as_ref().expect("有请求体").content_type,
            "image/png"
        );
    }

    #[tokio::test]
    async fn maps_upload_failures() {
        let rejected = FakeTransport::new()
            .respond(200, &sign_body("image/png"))
            .respond(500, "boom");
        let error = XingyeBackend::new(Arc::new(rejected))
            .upload(PNG, "a.png")
            .await
            .unwrap_err();
        assert!(matches!(error, Error::HttpStatus(500)), "{error}");

        // 签名成功但 resourceUrl 不是 http(s)：不回传坏 URL
        let broken = FakeTransport::new()
            .respond(
                200,
                r#"{"url":"https://upload","resourceUrl":"/relative.png"}"#,
            )
            .respond(200, "");
        let error = XingyeBackend::new(Arc::new(broken))
            .upload(PNG, "a.png")
            .await
            .unwrap_err();
        assert!(matches!(error, Error::HostingResponse(_)), "{error}");
    }
}
