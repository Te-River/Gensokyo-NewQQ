//! 腾讯云 COS 直传（`oss_type` = 1），对齐 Go `oss/tencent.go`。
//!
//! Go 用 `cos-go-sdk-v5` 的 `AuthorizationTransport` 给每个请求自签；Rust 侧不引 SDK，
//! 复用 oss_type=4（[`crate::hosting::cos`]）已验证的 [`crate::hosting::qcloud`] 签名
//! （`q-sign-algorithm=sha1`，只签 `host`）——同一套 COS 签名协议。
//!
//! 与 Go 的差异：
//! - 对象 key 是 `md5(字节).jpg`（与 Go 一致：Go 也不按真实格式改扩展名）；
//! - `t_audit`（COS 图片审核 `CI.ImageRecognition`）未实现，见 [`TencentCosConfig::validate`]。

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
    /// `t_audit`：Go 上传后调 COS 内容审核，未通过的图片不返回 URL。
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
        if self.audit {
            return Err(Error::HostingConfig(
                "腾讯云 COS 图片审核（t_audit）尚未实现，本次上传跳过以保证不发布未审核图片"
                    .to_owned(),
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
pub async fn upload(
    transport: Arc<dyn HostingTransport>,
    config: &TencentCosConfig,
    data: &[u8],
) -> Result<String> {
    let (timestamp, _) = unix_now();
    let plan = plan(config, data, timestamp)?;
    put(
        transport,
        plan.url,
        vec![
            ("Host".to_owned(), plan.host),
            ("Authorization".to_owned(), plan.authorization),
        ],
        data,
    )
    .await?;
    Ok(plan.result_url)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{TencentCosConfig, plan};
    use crate::error::Error;
    use crate::hosting::UploadMethod;
    use crate::hosting::fixtures::FakeTransport;
    use crate::oss::{OssConfig, OssKind};
    use crate::oss::upload as upload_oss;

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
    fn plan_rejects_incomplete_or_audited_configuration() {
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
            TencentCosConfig {
                audit: true,
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
        assert!(request.url.starts_with("https://bucket-125.cos.ap-shanghai.myqcloud.com/"));
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
}
