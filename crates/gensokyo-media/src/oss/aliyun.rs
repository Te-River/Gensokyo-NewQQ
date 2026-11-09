//! 阿里云 OSS 直传（`oss_type` = 3），对齐 Go `oss/aliyun.go` 的 `UploadAndAuditImageA`。
//!
//! Go 用 `aliyun-oss-go-sdk` 的 `PutObjectFromFile`；这里不引 SDK，按 OSS V1 签名自签：
//!
//! ```text
//! stringToSign = "PUT\n<Content-MD5>\n<Content-Type>\n<Date>\n/<bucket>/<key>"
//! Authorization = "OSS <AccessKeyId>:" + base64(hmac_sha1(AccessKeySecret, stringToSign))
//! ```
//!
//! - `Date` 用 RFC 1123 GMT；
//! - `Content-MD5` 是 `base64(md5(字节))`，与请求头同值；
//! - 虚拟主机寻址：`https://<bucket>.<a_OSS_EndPoint>/<key>`，key 是 `md5(字节).jpg`。
//!
//! 与 Go 的差异：
//! - Go 的结果 URL 是 `https://<bucket>.oss-<region>.aliyuncs.com/<key>`（region 从 endpoint
//!   里截出来）；这里直接用「bucket + endpoint」拼，标准 endpoint 下两者完全等价，
//!   非标准 endpoint（自定义域名）下这里才是对的；
//! - `a_audit` 打开时按 Go `green.CreateImageSyncScanRequest` 的形态审核（见
//!   [`crate::oss::audit`]）；`oss://<bucket>/<key>` 里的 bucket 用 `a_OSS_BucketName`
//!   （Go 用 `GetAppIDStr()`，指向一个不存在的桶）。

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::error::{Error, Result};
use crate::hosting::digest;
use crate::hosting::transport::HostingTransport;
use crate::hosting::unix_now;
use crate::oss::{IMAGE_CONTENT_TYPE, host_of, http_date, put};

/// 阿里云 OSS 凭据与目标桶（Go `a_OSS_EndPoint` / `a_OSS_BucketName` /
/// `a_OSS_AccessKeyId` / `a_OSS_AccessKeySecret` / `a_audit`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AliyunOssConfig {
    /// `a_OSS_EndPoint`，如 `https://oss-cn-hangzhou.aliyuncs.com`。
    pub endpoint: String,
    /// `a_OSS_BucketName`。
    pub bucket: String,
    /// `a_OSS_AccessKeyId`。
    pub access_key_id: String,
    /// `a_OSS_AccessKeySecret`。
    pub access_key_secret: String,
    /// `a_audit`：Go 上传后调内容安全审核，不合规直接报错。
    pub audit: bool,
}

impl AliyunOssConfig {
    /// 发请求前的必填项与开关检查（Go `oss.New` 失败时被忽略，错误留到 Bucket 调用）。
    fn validate(&self) -> Result<()> {
        if self.endpoint.is_empty() || self.bucket.is_empty() {
            return Err(Error::HostingConfig(
                "阿里云 OSS 未配置（请填 a_OSS_EndPoint / a_OSS_BucketName）".to_owned(),
            ));
        }
        if self.access_key_id.is_empty() || self.access_key_secret.is_empty() {
            return Err(Error::HostingConfig(
                "阿里云 OSS 未配置（请填 a_OSS_AccessKeyId / a_OSS_AccessKeySecret）".to_owned(),
            ));
        }
        Ok(())
    }
}

/// 一次 OSS 上传的纯数据描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliyunPlan {
    /// 完整上传 URL（`https://<bucket>.<endpoint>/<key>`）。
    pub url: String,
    /// `Host` 头（虚拟主机风格）。
    pub host: String,
    /// 对象 key（`<md5>.jpg`）。
    pub key: String,
    /// `Date` 头（RFC 1123 GMT，签名覆盖它）。
    pub date: String,
    /// `Content-MD5` 头（`base64(md5(字节))`，签名覆盖它）。
    pub content_md5: String,
    /// `Authorization` 头。
    pub authorization: String,
    /// 上传成功后的访问 URL（与 [`Self::url`] 相同）。
    pub result_url: String,
}

/// 构造上传计划；`timestamp` 由调用方给（Unix 秒），便于断言签名。
pub fn plan(config: &AliyunOssConfig, data: &[u8], timestamp: i64) -> Result<AliyunPlan> {
    config.validate()?;
    let host = format!("{}.{}", config.bucket, host_of(&config.endpoint));
    if host.ends_with('.') {
        return Err(Error::HostingConfig(
            "阿里云 OSS 的 a_OSS_EndPoint 只有协议头，没有域名".to_owned(),
        ));
    }
    let key = format!("{}.jpg", digest::md5_hex(data));
    let date = http_date(timestamp);
    let content_md5 = BASE64.encode(digest::md5_digest(data));
    let authorization = authorization(config, &key, &content_md5, &date);
    Ok(AliyunPlan {
        url: format!("https://{host}/{key}"),
        result_url: format!("https://{host}/{key}"),
        host,
        key,
        date,
        content_md5,
        authorization,
    })
}

/// OSS V1 签名：`OSS <AccessKeyId>:<base64(hmac_sha1)>`。
fn authorization(
    config: &AliyunOssConfig,
    key: &str,
    content_md5: &str,
    date: &str,
) -> String {
    let string_to_sign = format!(
        "PUT\n{content_md5}\n{IMAGE_CONTENT_TYPE}\n{date}\n/{}/{key}",
        config.bucket
    );
    let signature = BASE64.encode(digest::hmac_sha1(
        config.access_key_secret.as_bytes(),
        string_to_sign.as_bytes(),
    ));
    format!("OSS {}:{signature}", config.access_key_id)
}

/// 上传一份图片字节，返回 `https://<bucket>.<endpoint>/<key>`。
///
/// `a_audit` 打开时先 PUT 再审核（Go 同序）：审核不通过返回 [`Error::AuditRejected`]。
pub async fn upload(
    transport: Arc<dyn HostingTransport>,
    config: &AliyunOssConfig,
    data: &[u8],
) -> Result<String> {
    let (timestamp, _) = unix_now();
    let plan = plan(config, data, timestamp)?;
    put(
        Arc::clone(&transport),
        plan.url,
        vec![
            ("Date".to_owned(), plan.date),
            ("Content-MD5".to_owned(), plan.content_md5),
            ("Authorization".to_owned(), plan.authorization),
        ],
        data,
    )
    .await?;
    if config.audit {
        super::audit::aliyun(transport, config, &plan.key).await?;
    }
    Ok(plan.result_url)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{AliyunOssConfig, plan};
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use crate::error::Error;
    use crate::hosting::UploadMethod;
    use crate::hosting::digest;
    use crate::hosting::fixtures::FakeTransport;
    use crate::oss::upload as upload_oss;
    use crate::oss::{OssConfig, OssKind};

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0DIHDR";

    fn oss() -> AliyunOssConfig {
        AliyunOssConfig {
            endpoint: "https://oss-cn-hangzhou.aliyuncs.com".to_owned(),
            bucket: "a-bucket".to_owned(),
            access_key_id: "a-id".to_owned(),
            access_key_secret: "a-secret".to_owned(),
            audit: false,
        }
    }

    fn config() -> OssConfig {
        OssConfig {
            aliyun: oss(),
            ..OssConfig::default()
        }
    }

    #[test]
    fn plan_uses_virtual_host_and_v1_signature() {
        let built = plan(&oss(), PNG, 1_794_132_000).expect("已配置");
        assert_eq!(built.host, "a-bucket.oss-cn-hangzhou.aliyuncs.com");
        assert_eq!(built.date, "Sun, 08 Nov 2026 10:00:00 GMT");
        assert_eq!(built.key.len(), 36, "32 位 md5 + `.jpg`");
        assert_eq!(built.content_md5.len(), 24, "16 字节 md5 的 base64");
        assert_ne!(
            built.content_md5,
            plan(&oss(), b"other", 1_794_132_000)
                .expect("已配置")
                .content_md5,
            "Content-MD5 随内容变化"
        );
        assert_eq!(
            built.url,
            format!("https://a-bucket.oss-cn-hangzhou.aliyuncs.com/{}", built.key)
        );
        assert_eq!(built.result_url, built.url);
        assert!(built.authorization.starts_with("OSS a-id:"));
        assert_eq!(built.authorization.len(), "OSS a-id:".len() + 28, "base64 的 20 字节 hmac");
    }

    #[test]
    fn plan_strips_scheme_and_rejects_bad_config() {
        let with_scheme = AliyunOssConfig {
            endpoint: "oss-cn-hangzhou.aliyuncs.com".to_owned(),
            ..oss()
        };
        assert_eq!(
            plan(&with_scheme, PNG, 0).expect("已配置").host,
            "a-bucket.oss-cn-hangzhou.aliyuncs.com"
        );

        for broken in [
            AliyunOssConfig {
                endpoint: String::new(),
                ..oss()
            },
            AliyunOssConfig {
                bucket: String::new(),
                ..oss()
            },
            AliyunOssConfig {
                access_key_id: String::new(),
                ..oss()
            },
            AliyunOssConfig {
                access_key_secret: String::new(),
                ..oss()
            },
        ] {
            let error = plan(&broken, PNG, 0).unwrap_err();
            assert!(
                matches!(error, Error::HostingConfig(_)),
                "{broken:?} → {error}"
            );
        }
    }

    #[tokio::test]
    async fn uploads_put_with_oss_headers() {
        let transport = Arc::new(FakeTransport::new().respond(200, ""));
        let url = upload_oss(OssKind::AliyunOss, transport.clone(), &config(), PNG)
            .await
            .expect("上传成功");

        let calls = transport.calls();
        assert_eq!(calls.len(), 1, "单次尝试，不重试");
        let request = &calls[0];
        assert_eq!(request.method, UploadMethod::Put);
        assert_eq!(request.url, url);
        for header in ["Date", "Content-MD5", "Authorization"] {
            assert!(
                request.headers.iter().any(|(name, _)| name == header),
                "缺 {header}：{:?}",
                request.headers
            );
        }
        assert_eq!(request.body.as_ref().expect("有请求体").bytes, PNG);
    }

    #[tokio::test]
    async fn maps_status_to_error() {
        let transport = Arc::new(FakeTransport::new().respond(500, "InternalError"));
        let error = upload_oss(OssKind::AliyunOss, transport, &config(), PNG)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::HttpStatus(500)), "{error}");
    }

    fn audited() -> OssConfig {
        OssConfig {
            aliyun: AliyunOssConfig {
                audit: true,
                ..oss()
            },
            ..OssConfig::default()
        }
    }

    #[tokio::test]
    async fn audits_after_upload_with_image_sync_scan() {
        let transport = Arc::new(
            FakeTransport::new().respond(200, "").respond(
                200,
                r#"{"code":200,"requestId":"r","data":[{"results":[{"suggestion":"pass","label":"normal"}]}]}"#,
            ),
        );
        let url = upload_oss(OssKind::AliyunOss, transport.clone(), &audited(), PNG)
            .await
            .expect("审核通过");

        let calls = transport.calls();
        assert_eq!(calls.len(), 2, "PUT 之后再来一次审核");
        let audit = &calls[1];
        assert_eq!(audit.method, UploadMethod::Post);
        assert_eq!(audit.url, "https://green.aliyuncs.com/green/image/scan");
        let header = |name: &str| {
            audit
                .headers
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
                .unwrap_or_else(|| panic!("缺 {name}：{:?}", audit.headers))
        };
        assert_eq!(header("Accept"), "application/json");
        assert_eq!(header("x-acs-action"), "ImageSyncScan");
        assert_eq!(header("x-acs-version"), "2018-05-09");
        assert_eq!(header("x-acs-signature-method"), "HMAC-SHA1");
        assert!(header("Authorization").starts_with("acs a-id:"));
        let body = audit.body.as_ref().expect("审核有 JSON 请求体");
        assert_eq!(body.content_type, "application/octet-stream");
        let text = String::from_utf8_lossy(&body.bytes);
        assert!(text.contains(r#""scenes":["porn"]"#), "{text}");
        assert!(
            text.contains(&format!(r#""url":"oss://a-bucket/{}""#, url.rsplit('/').next().unwrap())),
            "审核的是刚上传的对象：{text}"
        );
        assert_eq!(
            header("Content-MD5"),
            BASE64.encode(digest::md5_digest(&body.bytes)),
            "Content-MD5 覆盖 body"
        );
        assert_eq!(url, calls[0].url);
    }

    #[tokio::test]
    async fn rejects_images_with_non_pass_suggestion() {
        let transport = Arc::new(FakeTransport::new().respond(200, "").respond(
            200,
            r#"{"code":200,"data":[{"results":[{"suggestion":"block"}]}]}"#,
        ));
        let error = upload_oss(OssKind::AliyunOss, transport.clone(), &audited(), PNG)
            .await
            .unwrap_err();
        assert!(
            matches!(&error, Error::AuditRejected(reason) if reason.contains("block")),
            "{error}"
        );
    }

    #[tokio::test]
    async fn audit_errors_when_green_service_is_not_enabled() {
        let transport = Arc::new(
            FakeTransport::new()
                .respond(200, "")
                .respond(200, r#"{"code":596,"msg":"service not opened"}"#),
        );
        let error = upload_oss(OssKind::AliyunOss, transport.clone(), &audited(), PNG)
            .await
            .unwrap_err();
        assert!(
            matches!(&error, Error::HostingResponse(reason) if reason.contains("596")),
            "{error}"
        );
    }

    #[tokio::test]
    async fn audit_switch_off_sends_no_audit_request() {
        let transport = Arc::new(FakeTransport::new().respond(200, ""));
        upload_oss(OssKind::AliyunOss, transport.clone(), &config(), PNG)
            .await
            .expect("上传成功");
        assert_eq!(transport.calls().len(), 1, "a_audit=false 不审核");
    }

    #[tokio::test]
    async fn upload_failure_skips_audit() {
        let transport = Arc::new(FakeTransport::new().respond(403, "AccessDenied"));
        let error = upload_oss(OssKind::AliyunOss, transport.clone(), &audited(), PNG)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::HttpStatus(403)), "{error}");
        assert_eq!(transport.calls().len(), 1, "上传失败不发审核请求");
    }
}
