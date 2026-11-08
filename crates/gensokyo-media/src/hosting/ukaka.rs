//! Ukaka 签名图床（oss_type = 8）。
//!
//! 对齐 Go `imagehosting/signed.go` 的 `signedUpload(…, "ukaka")`，两步走：
//! 1. 取签名（见 [`super::signed`]）；
//! 2. `POST` 签名给的地址：`body` 里的非空字段（`file` 除外）作为表单字段，图片放在 `file`；
//! 3. 上传响应 `status < 300` 视为成功，返回签名响应里的 `resourceUrl`。

use std::sync::Arc;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::hosting::multipart::{self, MultipartBody};
use crate::hosting::signed::{self, SignedPlan};
use crate::hosting::transport::{
    HostingTransport, USER_AGENT, USER_AGENT_HEADER, UploadBody, UploadMethod, UploadRequest,
};
use crate::hosting::{DEFAULT_UPLOAD_TIMEOUT, HostingBackend, check_url};

pub use crate::hosting::signed::{SIGN_ORIGIN, SIGN_URL};

/// 模块名（Go `tryUkaka` 传的 `"ukaka"`）。
pub const MODULE: &str = "ukaka";
/// 表单里的文件字段名（签名响应 `body` 里的同名键会被跳过）。
const FILE_FIELD: &str = "file";

/// 构造签名计划。
#[must_use]
pub fn plan(filename: &str, data: &[u8]) -> SignedPlan {
    signed::plan(MODULE, filename, data)
}

/// Ukaka 上传后端。
#[derive(Debug)]
pub struct UkakaBackend {
    transport: Arc<dyn HostingTransport>,
    timeout: Duration,
}

impl UkakaBackend {
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

impl HostingBackend for UkakaBackend {
    async fn upload(&self, data: &[u8], filename: &str) -> Result<String> {
        let plan = plan(filename, data);
        let target = signed::request(&*self.transport, &plan, self.timeout).await?;

        // 签名里的 body 是附加表单字段，`file` 留给真正的文件
        let mut body = MultipartBody::new(multipart::generate_boundary());
        for (key, value) in &target.fields {
            if key != FILE_FIELD && !value.is_empty() {
                body.field(key, value);
            }
        }
        body.file(FILE_FIELD, &plan.filename, data);
        let (content_type, bytes) = body.finish();

        let response = self
            .transport
            .send(UploadRequest {
                method: UploadMethod::Post,
                url: target.upload_url.clone(),
                headers: vec![(USER_AGENT_HEADER.to_owned(), USER_AGENT.to_owned())],
                body: Some(UploadBody {
                    content_type,
                    bytes,
                }),
                timeout: self.timeout,
            })
            .await?;
        // Go：`resp.StatusCode >= 300` 才算失败
        if response.status >= 300 {
            return Err(Error::HttpStatus(response.status));
        }
        check_url(&target.resource_url, "Ukaka")?;
        Ok(target.resource_url)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{MODULE, SIGN_URL, UkakaBackend, plan};
    use crate::error::Error;
    use crate::hosting::HostingBackend;
    use crate::hosting::UploadMethod;
    use crate::hosting::fixtures::{FakeTransport, parse_multipart};

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0DIHDR\x00\x00\x02\x80\x00\x00\x01\xE0\x08\x06\x00\x00\x00";

    fn sign_body() -> &'static str {
        r#"{"url":"https://upload.example/%2Fput","resourceUrl":"https://cdn.example/a.png",
            "header":{"Content-Type":"image/png"},"body":{"key":"token","sign":"sig","file":"ignored","empty":""}}"#
    }

    #[test]
    fn sign_url_sorts_and_encodes_query() {
        let plan = plan("图 1.jpg", PNG);
        assert_eq!(plan.module, MODULE);
        assert_eq!(plan.filename, "图 1.png", "扩展名按真实格式修正");
        assert_eq!(
            plan.sign_url,
            format!("{SIGN_URL}?filename=%E5%9B%BE%201.png&mimeType=image%2Fpng&module=ukaka")
        );
        assert_eq!(plan.mime, "image/png");
    }

    #[tokio::test]
    async fn uploads_with_signed_fields_and_returns_resource_url() {
        let transport = Arc::new(
            FakeTransport::new()
                .respond(200, sign_body())
                .respond(204, ""),
        );
        let url = UkakaBackend::new(transport.clone())
            .upload(PNG, "图 1.png")
            .await
            .expect("上传成功");
        assert_eq!(url, "https://cdn.example/a.png");

        let calls = transport.calls();
        assert_eq!(calls.len(), 2, "先签名再上传");

        let sign = &calls[0];
        assert_eq!(sign.method, UploadMethod::Get);
        assert_eq!(
            sign.url,
            format!("{SIGN_URL}?filename=%E5%9B%BE%201.png&mimeType=image%2Fpng&module=ukaka")
        );
        assert!(sign.body.is_none());

        let upload = &calls[1];
        assert_eq!(upload.method, UploadMethod::Post);
        assert_eq!(upload.url, "https://upload.example/%2Fput");
        let body = upload.body.as_ref().expect("有请求体");
        let parts = parse_multipart(&body.content_type, &body.bytes);
        let names: Vec<&str> = parts.iter().map(|part| part.name.as_str()).collect();
        assert_eq!(names, vec!["key", "sign", "file"], "file/空字段被跳过");
        assert_eq!(parts[0].text(), "token");
        assert_eq!(parts[1].text(), "sig");
        assert_eq!(parts[2].filename.as_deref(), Some("图 1.png"));
        assert_eq!(parts[2].data, PNG);
    }

    #[tokio::test]
    async fn fails_when_signing_fails() {
        let transport = FakeTransport::new().respond(503, "busy");
        let error = UkakaBackend::new(Arc::new(transport))
            .upload(PNG, "a.png")
            .await
            .unwrap_err();
        assert!(matches!(error, Error::HttpStatus(503)), "{error}");
    }

    #[tokio::test]
    async fn fails_on_upload_error_status() {
        let transport = FakeTransport::new()
            .respond(200, sign_body())
            .respond(500, "boom");
        let error = UkakaBackend::new(Arc::new(transport))
            .upload(PNG, "a.png")
            .await
            .unwrap_err();
        assert!(matches!(error, Error::HttpStatus(500)), "{error}");

        // 上传成功但 resourceUrl 不是 http(s)：不回传坏 URL
        let broken = FakeTransport::new()
            .respond(
                200,
                r#"{"url":"https://upload","resourceUrl":"file:///tmp/a.png"}"#,
            )
            .respond(200, "");
        let error = UkakaBackend::new(Arc::new(broken))
            .upload(PNG, "a.png")
            .await
            .unwrap_err();
        assert!(matches!(error, Error::HostingResponse(_)), "{error}");
    }
}
