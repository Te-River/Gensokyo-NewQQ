//! Bilibili 图床（oss_type = 5）。
//!
//! 对齐 Go `imagehosting/bilibili.go`：
//! - `POST https://api.bilibili.com/x/upload/web/image?bucket=<bucket>&csrf=<csrf>`；
//! - `multipart/form-data`，图片放在 `file` 字段（文件部件是八位流，与 Go `CreateFormFile` 一致）；
//! - 登录态走 `Cookie: SESSDATA=<sessdata>; bili_jct=<csrf>`，`bucket` 缺省 `openplatform`；
//! - 只接受 HTTP 200，响应 `{"code":0,"message":"","data":{"location":"<url>"}}`；
//! - `code != 0` 是业务错误，`location` 为空也是错误；`http://` 会被改写成 `https://`（Go 同款）。

use std::sync::Arc;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::hosting::json;
use crate::hosting::multipart::{self, MultipartBody};
use crate::hosting::transport::{
    HostingTransport, USER_AGENT, USER_AGENT_HEADER, UploadBody, UploadMethod, UploadRequest,
};
use crate::hosting::{DEFAULT_UPLOAD_TIMEOUT, HostingBackend, check_url};

/// 上传端点。
pub const ENDPOINT: &str = "https://api.bilibili.com/x/upload/web/image";
/// `bucket` 缺省值（Go `bucket = "openplatform"`）。
pub const DEFAULT_BUCKET: &str = "openplatform";
/// 表单里的文件字段名（Go `CreateFormFile("file", …)`）。
const FILE_FIELD: &str = "file";

/// B 站登录态（对齐 Go `structs.ImageHostingBilibili`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BilibiliConfig {
    /// `csrf_token`（也就是 `bili_jct` cookie）。
    pub csrf_token: String,
    /// `sessdata`。
    pub sessdata: String,
    /// 上传 bucket，留空用 [`DEFAULT_BUCKET`]。
    pub bucket: String,
}

impl BilibiliConfig {
    /// Go `tryBilibili` 开头的必填项检查。
    fn validate(&self) -> Result<()> {
        if self.sessdata.is_empty() || self.csrf_token.is_empty() {
            return Err(Error::HostingConfig(
                "Bilibili 未配置（请填写 csrf_token 和 sessdata）".to_owned(),
            ));
        }
        Ok(())
    }

    /// 实际使用的 bucket。
    fn bucket(&self) -> &str {
        if self.bucket.is_empty() {
            DEFAULT_BUCKET
        } else {
            &self.bucket
        }
    }
}

/// 一次 Bilibili 上传的纯数据描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BilibiliPlan {
    /// 完整上传 URL（含 `bucket` / `csrf` 查询参数）。
    pub url: String,
    /// 规整后的文件名（Go `ensureExt`）。
    pub filename: String,
    /// `Cookie` 头。
    pub cookie: String,
}

/// 构造上传计划。
pub fn plan(config: &BilibiliConfig, filename: &str, data: &[u8]) -> Result<BilibiliPlan> {
    config.validate()?;
    Ok(BilibiliPlan {
        // 查询参数按 key 字典序（Go `url.Values.Encode()`）：bucket < csrf
        url: format!(
            "{ENDPOINT}?bucket={}&csrf={}",
            multipart::encode_query_value(config.bucket()),
            multipart::encode_query_value(&config.csrf_token),
        ),
        filename: multipart::ensured_filename(filename, data),
        cookie: format!(
            "SESSDATA={}; bili_jct={}",
            config.sessdata, config.csrf_token
        ),
    })
}

/// Bilibili 上传后端。
#[derive(Debug)]
pub struct BilibiliBackend {
    transport: Arc<dyn HostingTransport>,
    config: BilibiliConfig,
    timeout: Duration,
}

impl BilibiliBackend {
    /// 用给定传输、登录态与默认超时（30s）构造。
    #[must_use]
    pub fn new(transport: Arc<dyn HostingTransport>, config: BilibiliConfig) -> Self {
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

impl HostingBackend for BilibiliBackend {
    async fn upload(&self, data: &[u8], filename: &str) -> Result<String> {
        let plan = plan(&self.config, filename, data)?;
        let mut body = MultipartBody::new(multipart::generate_boundary());
        body.file(FILE_FIELD, &plan.filename, data);
        let (content_type, bytes) = body.finish();

        let response = self
            .transport
            .send(UploadRequest {
                method: UploadMethod::Post,
                url: plan.url,
                headers: vec![
                    (USER_AGENT_HEADER.to_owned(), USER_AGENT.to_owned()),
                    ("Cookie".to_owned(), plan.cookie),
                ],
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
            .ok_or_else(|| Error::HostingResponse("Bilibili 响应不是合法 JSON".to_owned()))?;
        let code = root
            .path_i64(&["code"])
            .ok_or_else(|| Error::HostingResponse("Bilibili 响应缺少 code".to_owned()))?;
        if code != 0 {
            let message = root.path_str(&["message"]).unwrap_or_default();
            return Err(Error::HostingResponse(format!(
                "Bilibili 业务错误: code={code} msg={message}"
            )));
        }
        let location = root.path_str(&["data", "location"]).ok_or_else(|| {
            Error::HostingResponse("Bilibili 返回成功但 location 为空".to_owned())
        })?;
        if location.is_empty() {
            return Err(Error::HostingResponse(
                "Bilibili 返回成功但 location 为空".to_owned(),
            ));
        }
        // Go：`http:` 前缀改写成 `https:`
        let url = location
            .strip_prefix("http:")
            .map_or_else(|| location.to_owned(), |rest| format!("https:{rest}"));
        check_url(&url, "Bilibili")?;
        Ok(url)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{BilibiliBackend, BilibiliConfig, DEFAULT_BUCKET, ENDPOINT, plan};
    use crate::error::Error;
    use crate::hosting::HostingBackend;
    use crate::hosting::UploadMethod;
    use crate::hosting::fixtures::{FakeTransport, parse_multipart};
    use crate::hosting::transport::USER_AGENT_HEADER;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0DIHDR\x00\x00\x02\x80\x00\x00\x01\xE0\x08\x06\x00\x00\x00";

    fn config() -> BilibiliConfig {
        BilibiliConfig {
            csrf_token: "csrf-token".to_owned(),
            sessdata: "sess-data".to_owned(),
            bucket: String::new(),
        }
    }

    #[test]
    fn plan_builds_query_cookie_and_filename() {
        let built = plan(&config(), "photo.jpg", PNG).expect("已配置");
        assert_eq!(
            built.url,
            format!("{ENDPOINT}?bucket={DEFAULT_BUCKET}&csrf=csrf-token")
        );
        assert_eq!(built.filename, "photo.png", "Go ensureExt 修正扩展名");
        assert_eq!(built.cookie, "SESSDATA=sess-data; bili_jct=csrf-token");

        // 自定义 bucket 会被百分号编码
        let custom = BilibiliConfig {
            bucket: "my bucket".to_owned(),
            ..config()
        };
        assert_eq!(
            plan(&custom, "a.png", PNG).expect("已配置").url,
            format!("{ENDPOINT}?bucket=my%20bucket&csrf=csrf-token")
        );

        // 未配置：与 Go 同样的提示，且不发请求
        for broken in [
            BilibiliConfig {
                sessdata: String::new(),
                ..config()
            },
            BilibiliConfig {
                csrf_token: String::new(),
                ..config()
            },
        ] {
            let error = plan(&broken, "a.png", PNG).unwrap_err();
            assert!(
                matches!(&error, Error::HostingConfig(reason) if reason.contains("Bilibili 未配置")),
                "{error}"
            );
        }
    }

    #[tokio::test]
    async fn uploads_multipart_and_rewrites_insecure_location() {
        let transport = Arc::new(FakeTransport::new().respond(
            200,
            r#"{"code":0,"message":"0","data":{"location":"http:\/\/i0.hdslb.com\/bfs\/a.png"}}"#,
        ));
        let url = BilibiliBackend::new(transport.clone(), config())
            .upload(PNG, "photo.jpg")
            .await
            .expect("上传成功");
        assert_eq!(url, "https://i0.hdslb.com/bfs/a.png");

        let calls = transport.calls();
        assert_eq!(calls.len(), 1);
        let request = &calls[0];
        assert_eq!(request.method, UploadMethod::Post);
        assert_eq!(
            request.url,
            format!("{ENDPOINT}?bucket={DEFAULT_BUCKET}&csrf=csrf-token")
        );
        assert!(
            request.headers.iter().any(|(name, value)| name == "Cookie"
                && value == "SESSDATA=sess-data; bili_jct=csrf-token")
        );
        assert!(
            request
                .headers
                .iter()
                .any(|(name, value)| name == USER_AGENT_HEADER && value.starts_with("Mozilla/5.0"))
        );
        let body = request.body.as_ref().expect("有请求体");
        let parts = parse_multipart(&body.content_type, &body.bytes);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].name, "file");
        assert_eq!(parts[0].filename.as_deref(), Some("photo.png"));
        assert_eq!(parts[0].data, PNG);
    }

    #[tokio::test]
    async fn maps_status_business_error_and_missing_location() {
        let rejected = FakeTransport::new().respond(500, "oops");
        let error = BilibiliBackend::new(Arc::new(rejected), config())
            .upload(PNG, "a.png")
            .await
            .unwrap_err();
        assert!(matches!(error, Error::HttpStatus(500)), "{error}");

        for (body, expected) in [
            (
                r#"{"code":-101,"message":"账号未登录"}"#,
                "code=-101 msg=账号未登录",
            ),
            (r#"{"code":0,"data":{}}"#, "location 为空"),
            (r#"{"code":0,"data":{"location":""}}"#, "location 为空"),
            ("<html>oops</html>", "不是合法 JSON"),
            (r#"{"data":{"location":"https://x/a.png"}}"#, "缺少 code"),
        ] {
            let transport = FakeTransport::new().respond(200, body);
            let error = BilibiliBackend::new(Arc::new(transport), config())
                .upload(PNG, "a.png")
                .await
                .unwrap_err();
            assert!(
                matches!(&error, Error::HostingResponse(reason) if reason.contains(expected)),
                "{body} → {error}"
            );
        }

        // 未配置时一个请求都不发
        let quiet = FakeTransport::new();
        let probe = quiet.clone();
        let error = BilibiliBackend::new(Arc::new(quiet), BilibiliConfig::default())
            .upload(PNG, "a.png")
            .await
            .unwrap_err();
        assert!(matches!(error, Error::HostingConfig(_)), "{error}");
        assert!(probe.calls().is_empty());
    }
}
