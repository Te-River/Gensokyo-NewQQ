//! ChatGLM 免费图床（oss_type = 7）。
//!
//! 对齐 Go `imagehosting/chatglm.go`：
//! - 端点 `https://chatglm.cn/chatglm/backend-api/assistant/file_upload`；
//! - 单次 `multipart/form-data` POST，图片放在 `file` 字段；
//! - `X-File-Mime` 头声明真实图片类型（服务端靠它识别格式）；
//! - 只接受 HTTP 200，响应 `{"result":{"file_url":"https://…"}}`。

use std::sync::Arc;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::hosting::json;
use crate::hosting::multipart::{self, MultipartBody};
use crate::hosting::transport::{
    HostingTransport, USER_AGENT, USER_AGENT_HEADER, UploadBody, UploadMethod, UploadRequest,
};
use crate::hosting::{DEFAULT_UPLOAD_TIMEOUT, DirectMultipartPlan, HostingBackend, check_url};
use crate::sniff::{self, ImageKind};

/// 上传端点。
pub const ENDPOINT: &str = "https://chatglm.cn/chatglm/backend-api/assistant/file_upload";
/// 声明真实图片类型的头（Go 同名）。
pub const FILE_MIME_HEADER: &str = "X-File-Mime";
/// 形态未知时的 MIME 兜底（对齐 Go `detectMIME` 的 `image/jpeg`）。
const FALLBACK_MIME: &str = "image/jpeg";
/// 表单里的文件字段名（Go `CreateFormFile("file", …)`）。
const FILE_FIELD: &str = "file";

/// 构造上传计划（纯函数，便于断言端点 / 文件名 / MIME）。
#[must_use]
pub fn plan(filename: &str, data: &[u8]) -> DirectMultipartPlan {
    let mime = sniff::sniff_image(data).map_or(FALLBACK_MIME, ImageKind::mime);
    DirectMultipartPlan {
        url: ENDPOINT,
        filename: multipart::ensured_filename(filename, data),
        mime,
        headers: vec![(FILE_MIME_HEADER, mime)],
    }
}

/// ChatGLM 上传后端。
#[derive(Debug)]
pub struct ChatGlmBackend {
    transport: Arc<dyn HostingTransport>,
    timeout: Duration,
}

impl ChatGlmBackend {
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

impl HostingBackend for ChatGlmBackend {
    async fn upload(&self, data: &[u8], filename: &str) -> Result<String> {
        let plan = plan(filename, data);
        let mut body = MultipartBody::new(multipart::generate_boundary());
        body.file(FILE_FIELD, &plan.filename, data);
        let (content_type, bytes) = body.finish();

        let mut headers = vec![(USER_AGENT_HEADER.to_owned(), USER_AGENT.to_owned())];
        headers.extend(
            plan.headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
        );

        let response = self
            .transport
            .send(UploadRequest {
                method: UploadMethod::Post,
                url: plan.url.to_owned(),
                headers,
                body: Some(UploadBody {
                    content_type,
                    bytes,
                }),
                timeout: self.timeout,
            })
            .await?;

        // Go：`resp.StatusCode != 200` 即失败
        if response.status != 200 {
            return Err(Error::HttpStatus(response.status));
        }
        let root = json::parse(&response.body)
            .ok_or_else(|| Error::HostingResponse("ChatGLM 响应不是合法 JSON".to_owned()))?;
        let file_url = root
            .path_str(&["result", "file_url"])
            .ok_or_else(|| Error::HostingResponse("ChatGLM 响应缺少 result.file_url".to_owned()))?;
        check_url(file_url, "ChatGLM")?;
        Ok(file_url.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{ChatGlmBackend, ENDPOINT, FILE_MIME_HEADER, plan};
    use crate::error::Error;
    use crate::hosting::HostingBackend;
    use crate::hosting::fixtures::{FakeTransport, parse_multipart};

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0DIHDR\x00\x00\x02\x80\x00\x00\x01\xE0\x08\x06\x00\x00\x00";
    const OK: &str = r#"{"code":0,"result":{"file_url":"https://chatglm.cn/files/abc.png"}}"#;

    #[test]
    fn plan_declares_endpoint_mime_and_extension() {
        let direct = plan("shot", PNG);
        assert_eq!(direct.url, ENDPOINT);
        assert_eq!(direct.filename, "shot.png");
        assert_eq!(direct.mime, "image/png");
        assert_eq!(direct.headers, vec![(FILE_MIME_HEADER, "image/png")]);
        // 非图片字节按 Go 兜底 image/jpeg
        assert_eq!(plan("x", b"hello world, not an image").mime, "image/jpeg");
    }

    #[tokio::test]
    async fn uploads_file_as_multipart_and_returns_file_url() {
        let transport = Arc::new(FakeTransport::new().respond(200, OK));
        let url = ChatGlmBackend::new(transport.clone())
            .upload(PNG, "shot.jpg")
            .await
            .expect("上传成功");
        assert_eq!(url, "https://chatglm.cn/files/abc.png");

        let calls = transport.calls();
        assert_eq!(calls.len(), 1);
        let request = &calls[0];
        assert_eq!(request.method, crate::hosting::UploadMethod::Post);
        assert_eq!(request.url, ENDPOINT);
        // 图片 MIME 走 header，body 里的文件部件是八位流（Go 行为）
        assert!(
            request
                .headers
                .iter()
                .any(|(name, value)| name == FILE_MIME_HEADER && value == "image/png"),
            "{:?}",
            request.headers
        );
        assert!(
            request
                .headers
                .iter()
                .any(|(name, value)| name == "User-Agent" && value.starts_with("Mozilla/5.0"))
        );
        let body = request.body.as_ref().expect("有请求体");
        assert!(
            body.content_type
                .starts_with("multipart/form-data; boundary=gensokyo-"),
            "{}",
            body.content_type
        );

        let parts = parse_multipart(&body.content_type, &body.bytes);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].name, "file");
        assert_eq!(parts[0].filename.as_deref(), Some("shot.png"));
        assert_eq!(parts[0].data, PNG);
    }

    #[tokio::test]
    async fn rejects_non_200_and_transport_failures() {
        let rejected = FakeTransport::new().respond(500, r#"{"msg":"boom"}"#);
        let error = ChatGlmBackend::new(Arc::new(rejected))
            .upload(PNG, "a.png")
            .await
            .unwrap_err();
        assert!(matches!(error, Error::HttpStatus(500)), "{error}");

        let offline = FakeTransport::new().fail("连接被拒绝");
        let error = ChatGlmBackend::new(Arc::new(offline))
            .upload(PNG, "a.png")
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Transport(_)), "{error}");
    }

    #[tokio::test]
    async fn rejects_unusable_payloads() {
        for (body, expected) in [
            ("<html>oops</html>", "不是合法 JSON"),
            (r#"{"result":{}}"#, "缺少 result.file_url"),
            (r#"{"result":{"file_url":""}}"#, "空 URL"),
            (r#"{"result":{"file_url":"/relative.png"}}"#, "不是 http(s)"),
        ] {
            let transport = FakeTransport::new().respond(200, body);
            let error = ChatGlmBackend::new(Arc::new(transport))
                .upload(PNG, "a.png")
                .await
                .unwrap_err();
            assert!(
                matches!(&error, Error::HostingResponse(reason) if reason.contains(expected)),
                "{body} → {error}"
            );
        }
    }
}
