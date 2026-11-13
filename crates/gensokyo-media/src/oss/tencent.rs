//! 腾讯云 COS 直传（`oss_type` = 1），对齐 Go `oss/tencent.go`。
//!
//! Go 用 `cos-go-sdk-v5` 的 `AuthorizationTransport` 给每个请求自签；Rust 侧不引 SDK，
//! 复用 oss_type=4（[`crate::hosting::cos`]）已验证的 [`crate::hosting::qcloud`] 签名
//! （`q-sign-algorithm=sha1`，上传签 `host`，审核再签 `ci-process`）——同一套 COS 签名协议。
//!
//! 与 Go 的差异：
//! - 对象 key 是 `md5(字节).jpg`（与 Go 一致：Go 也不按真实格式改扩展名）；
//! - `t_audit` 打开时按 Go `CI.ImageRecognition` 的形态再发一次审核请求（见
//!   [`crate::oss::audit`]）；Go 审核不通过返回 `("", nil)`，这里返回
//!   [`Error::AuditRejected`]，调用方保留原引用。

use std::sync::Arc;

use crate::error::{Error, Result};
use crate::hosting::digest;
use crate::hosting::qcloud;
use crate::hosting::transport::HostingTransport;
use crate::hosting::unix_now;
use crate::oss::put;

/// 腾讯云 COS 凭据与目标桶（Go `t_COS_BUCKETNAME` / `t_COS_REGION` / `t_COS_SECRETID` /
/// `t_COS_SECRETKEY` / `t_audit`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TencentCosConfig {
    /// 存储桶名（`t_COS_BUCKETNAME`）。
    pub bucket: String,
    /// 地域，如 `ap-shanghai`（`t_COS_REGION`）。
    pub region: String,
    /// `t_COS_SECRETID`。
    pub secret_id: String,
    /// `t_COS_SECRETKEY`。
    pub secret_key: String,
    /// `t_audit`：Go 上传后调 COS 内容审核（`CI.ImageRecognition`），未通过的图片不返回 URL。
    pub audit: bool,
}

impl TencentCosConfig {
    /// 发请求前的必填项检查（Go `initClient` 拿到空配置只会在 SDK 里报错）。
    fn validate(&self) -> Result<()> {
        if self.bucket.is_empty() || self.region.is_empty() {
            return Err(Error::HostingConfig(
                "腾讯云 COS 未配置（请填 t_COS_BUCKETNAME / t_COS_REGION）".to_owned(),
            ));
        }
        if self.secret_id.is_empty() || self.secret_key.is_empty() {
            return Err(Error::HostingConfig(
                "腾讯云 COS 未配置（请填 t_COS_SECRETID / t_COS_SECRETKEY）".to_owned(),
            ));
        }
        Ok(())
    }
}

/// 一次 COS 上传的纯数据描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TencentCosPlan {
    /// 完整上传 URL。
    pub url: String,
    /// `Host` 头（签名覆盖 `host`）。
    pub host: String,
    /// 对象 key（`<md5>.jpg`）。
    pub key: String,
    /// `Authorization` 头。
    pub authorization: String,
    /// 上传成功后的访问 URL（Go：`<bucketURL>/<key>`）。
    pub result_url: String,
}

/// 构造上传计划；`timestamp` 由调用方给（Unix 秒），便于断言签名。
pub fn plan(config: &TencentCosConfig, data: &[u8], timestamp: i64) -> Result<TencentCosPlan> {
    config.validate()?;
    let key = format!("{}.jpg", digest::md5_hex(data));
    let host = qcloud::object_host(&config.bucket, &config.region);
    Ok(TencentCosPlan {
        url: format!("https://{host}/{key}"),
        authorization: qcloud::authorization(
            &config.secret_id,
            &config.secret_key,
            &key,
            &host,
            timestamp,
        ),
        result_url: format!("https://{host}/{key}"),
        host,
        key,
    })
}

/// 上传一份图片字节，返回 `https://<bucket>.cos.<region>.myqcloud.com/<key>`。
///
/// `t_audit` 打开时先 PUT 再审核（Go 同序）：审核不通过返回 [`Error::AuditRejected`]。
pub async fn upload(
    transport: Arc<dyn HostingTransport>,
    config: &TencentCosConfig,
    data: &[u8],
) -> Result<String> {
    let (timestamp, _) = unix_now();
    let plan = plan(config, data, timestamp)?;
    put(
        Arc::clone(&transport),
        plan.url,
        vec![
            ("Host".to_owned(), plan.host),
            ("Authorization".to_owned(), plan.authorization),
        ],
        data,
    )
    .await?;
    if config.audit {
        super::audit::tencent(transport, config, &plan.key).await?;
    }
    Ok(plan.result_url)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{TencentCosConfig, plan};
    use crate::error::Error;
    use crate::hosting::UploadMethod;
    use crate::hosting::fixtures::FakeTransport;
    use crate::oss::upload as upload_oss;
    use crate::oss::{OssConfig, OssKind};

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0DIHDR";

    fn config() -> OssConfig {
        OssConfig {
            tencent: TencentCosConfig {
                bucket: "bucket-125".to_owned(),
                region: "ap-shanghai".to_owned(),
                secret_id: "AKIDtest".to_owned(),
                secret_key: "secret-key".to_owned(),
                audit: false,
            },
            ..OssConfig::default()
        }
    }

    fn tencent() -> TencentCosConfig {
        config().tencent
    }

    #[test]
    fn plan_uses_md5_key_and_qsign() {
        let built = plan(&tencent(), PNG, 1_700_000_000).expect("已配置");
        assert_eq!(built.host, "bucket-125.cos.ap-shanghai.myqcloud.com");
        assert_eq!(built.key.len(), 36, "32 位 md5 + `.jpg`");
        assert!(built.key.ends_with(".jpg"));
        assert_eq!(built.url, format!("https://{}/{}", built.host, built.key));
        assert_eq!(built.result_url, built.url);
        assert!(
            built
                .authorization
                .starts_with("q-sign-algorithm=sha1&q-ak=AKIDtest&")
        );
        assert!(built.authorization.contains("q-header-list=host"));
    }

    #[test]
    fn plan_rejects_incomplete_configuration() {
        for broken in [
            TencentCosConfig {
                bucket: String::new(),
                ..tencent()
            },
            TencentCosConfig {
                region: String::new(),
                ..tencent()
            },
            TencentCosConfig {
                secret_id: String::new(),
                ..tencent()
            },
            TencentCosConfig {
                secret_key: String::new(),
                ..tencent()
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
    async fn uploads_put_with_raw_bytes() {
        let transport = Arc::new(FakeTransport::new().respond(200, ""));
        let url = upload_oss(OssKind::TencentCos, transport.clone(), &config(), PNG)
            .await
            .expect("上传成功");

        let calls = transport.calls();
        assert_eq!(calls.len(), 1, "单次尝试，不重试");
        let request = &calls[0];
        assert_eq!(request.method, UploadMethod::Put);
        assert_eq!(request.url, url);
        assert!(
            request
                .url
                .starts_with("https://bucket-125.cos.ap-shanghai.myqcloud.com/")
        );
        assert!(request.headers.iter().any(|(name, _)| name == "Host"));
        assert!(
            request
                .headers
                .iter()
                .any(|(name, value)| name == "Authorization"
                    && value.starts_with("q-sign-algorithm=sha1"))
        );
        let body = request.body.as_ref().expect("有请求体");
        assert_eq!(body.content_type, "image/jpeg");
        assert_eq!(body.bytes, PNG);
    }

    #[tokio::test]
    async fn maps_status_to_error_without_retry() {
        let transport = Arc::new(FakeTransport::new().respond(403, "AccessDenied"));
        let error = upload_oss(OssKind::TencentCos, transport.clone(), &config(), PNG)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::HttpStatus(403)), "{error}");
        assert_eq!(transport.calls().len(), 1, "4xx 不重试");
    }

    fn audited() -> OssConfig {
        OssConfig {
            tencent: TencentCosConfig {
                audit: true,
                ..tencent()
            },
            ..OssConfig::default()
        }
    }

    #[tokio::test]
    async fn audits_after_upload_with_ci_recognition_request() {
        let transport = Arc::new(FakeTransport::new().respond(200, "").respond(
            200,
            "<RecognitionResult><JobId>j</JobId><Result>0</Result></RecognitionResult>",
        ));
        let url = upload_oss(OssKind::TencentCos, transport.clone(), &audited(), PNG)
            .await
            .expect("审核通过");

        let calls = transport.calls();
        assert_eq!(calls.len(), 2, "PUT 之后再来一次审核");
        let audit = &calls[1];
        assert_eq!(audit.method, UploadMethod::Get);
        assert!(
            audit
                .url
                .ends_with(".jpg?ci-process=sensitive-content-recognition"),
            "{}",
            audit.url
        );
        let authorization = audit
            .headers
            .iter()
            .find(|(name, _)| name == "Authorization")
            .map(|(_, value)| value.as_str())
            .expect("审核请求要签名");
        assert!(
            authorization.contains("q-url-param-list=ci-process&q-signature="),
            "查询串要进签名：{authorization}"
        );
        assert!(audit.body.is_none(), "审核是 GET，无请求体");
        assert_eq!(url, calls[0].url);
    }

    #[tokio::test]
    async fn rejects_images_that_fail_audit() {
        let transport = Arc::new(FakeTransport::new().respond(200, "").respond(
            200,
            "<RecognitionResult><Result>1</Result></RecognitionResult>",
        ));
        let error = upload_oss(OssKind::TencentCos, transport.clone(), &audited(), PNG)
            .await
            .unwrap_err();
        assert!(
            matches!(&error, Error::AuditRejected(reason) if reason.contains("Result=1")),
            "{error}"
        );
        assert_eq!(transport.calls().len(), 2, "上传已完成，审核才判违规");
    }

    #[tokio::test]
    async fn audit_switch_off_sends_no_audit_request() {
        let transport = Arc::new(FakeTransport::new().respond(200, ""));
        upload_oss(OssKind::TencentCos, transport.clone(), &config(), PNG)
            .await
            .expect("上传成功");
        assert_eq!(transport.calls().len(), 1, "t_audit=false 不审核");
    }

    #[tokio::test]
    async fn upload_failure_skips_audit() {
        let transport = Arc::new(FakeTransport::new().respond(403, "AccessDenied"));
        let error = upload_oss(OssKind::TencentCos, transport.clone(), &audited(), PNG)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::HttpStatus(403)), "{error}");
        assert_eq!(transport.calls().len(), 1, "上传失败不发审核请求");
    }
}
