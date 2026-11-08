//! 签名服务：Ukaka（oss_type 8）与星野（oss_type 9）共用。
//!
//! 对齐 Go `imagehosting/signed.go` 的第一步——`GET <SIGN_URL>?filename=&mimeType=&module=`，
//! 响应形如：
//!
//! ```json
//! {"url":"<上传地址>","resourceUrl":"<最终 URL>","header":{…},"body":{…}}
//! ```
//!
//! 两个后端只在第二步分叉：Ukaka 用 `body` 里的字段做 `multipart` POST，星野直接 PUT 原始字节。
//! 因此签名请求与解析只此一份，第二步留给 [`super::ukaka`] / [`super::xingye`]。
//!
//! 与 Go 的差异：Go 不检查签名请求的状态码，错误页会被当 JSON 解析；这里要求 200。

use std::time::Duration;

use crate::error::{Error, Result};
use crate::hosting::json;
use crate::hosting::multipart;
use crate::hosting::transport::{
    HostingTransport, USER_AGENT, USER_AGENT_HEADER, UploadMethod, UploadRequest,
};
use crate::sniff::{self, ImageKind};

/// 签名服务端点（Go `_signURL`）。
pub const SIGN_URL: &str = "https://bed-sign.vercel.0013107.xyz/sign";
/// 签名服务要求的 `Origin` / `Referer` 源（Go `_signOrigin`）。
pub const SIGN_ORIGIN: &str = "https://bed.vercel.0013107.xyz";
/// 形态未知时的 MIME 兜底（对齐 Go `detectMIME` 的 `image/jpeg`）。
const FALLBACK_MIME: &str = "image/jpeg";

/// 一次签名上传的公共部分：签名 URL 与待上传文件名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedPlan {
    /// 签名服务要求的模块名（`ukaka` / `xingye`）。
    pub module: &'static str,
    /// 完整签名 URL（查询参数已编码）。
    pub sign_url: String,
    /// 规整后的文件名。
    pub filename: String,
    /// 图片 MIME。
    pub mime: &'static str,
}

/// 构造签名计划（纯函数，签名 URL 可直接断言）。
#[must_use]
pub fn plan(module: &'static str, filename: &str, data: &[u8]) -> SignedPlan {
    let mime = sniff::sniff_image(data).map_or(FALLBACK_MIME, ImageKind::mime);
    let filename = multipart::ensured_filename(filename, data);
    SignedPlan {
        module,
        // 查询参数按 key 字典序，与 Go `url.Values.Encode()` 的输出一致
        sign_url: format!(
            "{SIGN_URL}?filename={}&mimeType={}&module={module}",
            multipart::encode_query_value(&filename),
            multipart::encode_query_value(mime),
        ),
        filename,
        mime,
    }
}

/// 签名服务给出的上传目标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedTarget {
    /// 第二步的上传地址。
    pub upload_url: String,
    /// 上传成功后对外可访问的 URL。
    pub resource_url: String,
    /// 签名服务要求的附加请求头（`header`）。
    pub headers: Vec<(String, String)>,
    /// 签名服务要求的附加表单字段（`body`；`file` 键由后端跳过）。
    pub fields: Vec<(String, String)>,
}

impl SignedTarget {
    /// 按名字取签名响应里的头（大小写敏感，与 Go 的 map 取值一致）。
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// 取签名并解析出上传目标。
pub async fn request(
    transport: &dyn HostingTransport,
    plan: &SignedPlan,
    timeout: Duration,
) -> Result<SignedTarget> {
    let response = transport
        .send(UploadRequest {
            method: UploadMethod::Get,
            url: plan.sign_url.clone(),
            headers: vec![
                ("Accept".to_owned(), "*/*".to_owned()),
                ("Origin".to_owned(), SIGN_ORIGIN.to_owned()),
                ("Referer".to_owned(), format!("{SIGN_ORIGIN}/")),
                (USER_AGENT_HEADER.to_owned(), USER_AGENT.to_owned()),
            ],
            body: None,
            timeout,
        })
        .await?;
    // Go 不检查这一步；不检查会把错误页当 JSON 解析，这里收紧到 200
    if response.status != 200 {
        return Err(Error::HttpStatus(response.status));
    }
    let root = json::parse(&response.body)
        .ok_or_else(|| Error::HostingResponse("签名响应不是合法 JSON".to_owned()))?;
    let upload_url = root
        .path_str(&["url"])
        .ok_or_else(|| Error::HostingResponse("签名响应缺少 url".to_owned()))?
        .to_owned();
    let resource_url = root
        .path_str(&["resourceUrl"])
        .ok_or_else(|| Error::HostingResponse("签名响应缺少 resourceUrl".to_owned()))?
        .to_owned();
    Ok(SignedTarget {
        upload_url,
        resource_url,
        headers: root.string_entries("header"),
        fields: root.string_entries("body"),
    })
}

#[cfg(test)]
mod tests {
    use super::{SIGN_ORIGIN, SIGN_URL, plan, request};
    use crate::error::Error;
    use crate::hosting::UploadMethod;
    use crate::hosting::fixtures::FakeTransport;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0DIHDR\x00\x00\x02\x80\x00\x00\x01\xE0\x08\x06\x00\x00\x00";
    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

    #[test]
    fn plan_sorts_and_encodes_query() {
        let plan = plan("ukaka", "图 1.jpg", PNG);
        assert_eq!(plan.module, "ukaka");
        assert_eq!(plan.filename, "图 1.png", "扩展名按真实格式修正");
        assert_eq!(
            plan.sign_url,
            format!("{SIGN_URL}?filename=%E5%9B%BE%201.png&mimeType=image%2Fpng&module=ukaka")
        );
        assert_eq!(plan.mime, "image/png");
    }

    #[tokio::test]
    async fn request_sends_signed_headers_and_parses_target() {
        let transport = FakeTransport::new().respond(
            200,
            r#"{"url":"https://upload.example/put","resourceUrl":"https://cdn.example/a.png",
                "header":{"Content-Type":"image/png"},"body":{"key":"token","file":"ignored"}}"#,
        );
        let plan = plan("xingye", "a.png", PNG);
        let target = request(&transport, &plan, TIMEOUT).await.expect("签名成功");
        assert_eq!(target.upload_url, "https://upload.example/put");
        assert_eq!(target.resource_url, "https://cdn.example/a.png");
        assert_eq!(target.header("Content-Type"), Some("image/png"));
        assert_eq!(
            target.fields,
            vec![
                ("key".to_owned(), "token".to_owned()),
                ("file".to_owned(), "ignored".to_owned()),
            ]
        );

        let calls = transport.calls();
        assert_eq!(calls.len(), 1);
        let sign = &calls[0];
        assert_eq!(sign.method, UploadMethod::Get);
        assert_eq!(sign.url, plan.sign_url);
        assert!(sign.body.is_none());
        for (name, value) in [
            ("Accept", "*/*".to_owned()),
            ("Origin", SIGN_ORIGIN.to_owned()),
            ("Referer", format!("{SIGN_ORIGIN}/")),
        ] {
            assert!(
                sign.headers
                    .iter()
                    .any(|(key, actual)| key == name && *actual == value),
                "{name}: {:?}",
                sign.headers
            );
        }
    }

    #[tokio::test]
    async fn request_rejects_bad_status_and_payloads() {
        let busy = FakeTransport::new().respond(503, "busy");
        let error = request(&busy, &plan("ukaka", "a.png", PNG), TIMEOUT)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::HttpStatus(503)), "{error}");

        for (body, expected) in [
            ("not json", "签名响应不是合法 JSON"),
            (r#"{"resourceUrl":"https://cdn/a.png"}"#, "缺少 url"),
            (r#"{"url":"https://upload"}"#, "缺少 resourceUrl"),
        ] {
            let transport = FakeTransport::new().respond(200, body);
            let error = request(&transport, &plan("ukaka", "a.png", PNG), TIMEOUT)
                .await
                .unwrap_err();
            assert!(
                matches!(&error, Error::HostingResponse(reason) if reason.contains(expected)),
                "{body} → {error}"
            );
        }
    }

    #[tokio::test]
    async fn request_propagates_transport_failure() {
        let offline = FakeTransport::new().fail("连接被拒绝");
        let error = request(&offline, &plan("ukaka", "a.png", PNG), TIMEOUT)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Transport(_)), "{error}");
    }
}
