//! 腾讯云 COS 自签直传（oss_type = 4）。
//!
//! 对齐 Go `imagehosting/cos.go`：
//! - `PUT https://<bucket>.cos.<region>.myqcloud.com/gensokyo/<ts>/<filename>`，原始字节直传；
//! - `Authorization` 用 [`super::qcloud`] 的 `q-sign-algorithm=sha1` 自签（不依赖 COS SDK）；
//! - `Content-Type` 按图片嗅探结果，未知时 `image/jpeg`；
//! - 只接受 HTTP 200，返回 `<domain 或 https://host>/<key>`。
//!
//! 与 Go 的差异：Go 直接把入参 `filename` 拼进对象 key（可带 `../` 或绝对路径），这里用
//! [`multipart::ensured_filename`] 去掉目录部分并按真实格式修正扩展名——key 会随后续
//! 上传复用，不该由客户端的本地路径决定。

use std::sync::Arc;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::hosting::multipart;
use crate::hosting::qcloud;
use crate::hosting::transport::{HostingTransport, UploadBody, UploadMethod, UploadRequest};
use crate::hosting::{DEFAULT_UPLOAD_TIMEOUT, HostingBackend, check_url, unix_now};
use crate::sniff::{self, ImageKind};

/// 对象 key 的固定前缀（Go `gensokyo/<ts>/<filename>`）。
const KEY_PREFIX: &str = "gensokyo";
/// 形态未知时的 MIME 兜底（对齐 Go `detectMIME` 的 `image/jpeg`）。
const FALLBACK_MIME: &str = "image/jpeg";

/// COS 凭据与目标桶（对齐 Go `structs.ImageHostingCOS`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CosConfig {
    /// `secret_id`。
    pub secret_id: String,
    /// `secret_key`。
    pub secret_key: String,
    /// 地域，如 `ap-guangzhou`。
    pub region: String,
    /// 存储桶名（含 APPID 后缀），如 `bucket-1250000000`。
    pub bucket: String,
    /// 自定义访问域名（留空用 `https://<host>`）。
    pub domain: String,
}

impl CosConfig {
    /// Go `tryCOS` 开头的必填项检查。
    fn validate(&self) -> Result<()> {
        if self.secret_id.is_empty()
            || self.secret_key.is_empty()
            || self.region.is_empty()
            || self.bucket.is_empty()
        {
            return Err(Error::HostingConfig(
                "COS 未配置（请填写 secret_id / secret_key / region / bucket）".to_owned(),
            ));
        }
        Ok(())
    }
}

/// 一次 COS 上传的纯数据描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CosPlan {
    /// 完整上传 URL。
    pub url: String,
    /// `Host` 头（签名覆盖 `host`）。
    pub host: String,
    /// 对象 key。
    pub key: String,
    /// 图片 MIME。
    pub content_type: &'static str,
    /// `Authorization` 头。
    pub authorization: String,
    /// 上传成功后的访问 URL。
    pub result_url: String,
}

/// 构造上传计划；`timestamp` 由调用方给（Unix 秒），便于断言签名。
pub fn plan(config: &CosConfig, filename: &str, data: &[u8], timestamp: i64) -> Result<CosPlan> {
    config.validate()?;
    let key = format!(
        "{KEY_PREFIX}/{timestamp}/{}",
        multipart::ensured_filename(filename, data)
    );
    let host = qcloud::object_host(&config.bucket, &config.region);
    let content_type = sniff::sniff_image(data).map_or(FALLBACK_MIME, ImageKind::mime);
    let domain = if config.domain.is_empty() {
        format!("https://{host}")
    } else {
        config.domain.clone()
    };
    Ok(CosPlan {
        url: format!("https://{host}/{key}"),
        host: host.clone(),
        content_type,
        authorization: qcloud::authorization(
            &config.secret_id,
            &config.secret_key,
            &key,
            &host,
            timestamp,
        ),
        result_url: format!("{domain}/{key}"),
        key,
    })
}

/// COS 上传后端。
#[derive(Debug)]
pub struct CosBackend {
    transport: Arc<dyn HostingTransport>,
    config: CosConfig,
    timeout: Duration,
}

impl CosBackend {
    /// 用给定传输、凭据与默认超时（30s）构造。
    #[must_use]
    pub fn new(transport: Arc<dyn HostingTransport>, config: CosConfig) -> Self {
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

impl HostingBackend for CosBackend {
    async fn upload(&self, data: &[u8], filename: &str) -> Result<String> {
        let (timestamp, _) = unix_now();
        let plan = plan(&self.config, filename, data, timestamp)?;
        let response = self
            .transport
            .send(UploadRequest {
                method: UploadMethod::Put,
                url: plan.url,
                headers: vec![
                    ("Host".to_owned(), plan.host),
                    ("Authorization".to_owned(), plan.authorization),
                ],
                body: Some(UploadBody {
                    content_type: plan.content_type.to_owned(),
                    bytes: data.to_vec(),
                }),
                timeout: self.timeout,
            })
            .await?;
        // Go：`resp.StatusCode != 200` 即失败（COS 成功只回 200，不解析响应体）
        if response.status != 200 {
            return Err(Error::HttpStatus(response.status));
        }
        check_url(&plan.result_url, "COS")?;
        Ok(plan.result_url)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{CosBackend, CosConfig, plan};
    use crate::error::Error;
    use crate::hosting::HostingBackend;
    use crate::hosting::UploadMethod;
    use crate::hosting::fixtures::FakeTransport;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0DIHDR\x00\x00\x02\x80\x00\x00\x01\xE0\x08\x06\x00\x00\x00";

    fn config() -> CosConfig {
        CosConfig {
            secret_id: "AKIDtest".to_owned(),
            secret_key: "secret-key".to_owned(),
            region: "ap-guangzhou".to_owned(),
            bucket: "bucket-125".to_owned(),
            domain: String::new(),
        }
    }

    #[test]
    fn plan_builds_key_host_and_signature() {
        let built = plan(&config(), "C:\\tmp\\photo.jpg", PNG, 1_700_000_000).expect("已配置");
        assert_eq!(
            built.key, "gensokyo/1700000000/photo.png",
            "目录被剥离、扩展名按嗅探修正"
        );
        assert_eq!(built.host, "bucket-125.cos.ap-guangzhou.myqcloud.com");
        assert_eq!(
            built.url,
            "https://bucket-125.cos.ap-guangzhou.myqcloud.com/gensokyo/1700000000/photo.png"
        );
        assert_eq!(built.result_url, built.url, "未配 domain 时用 https://host");
        assert_eq!(built.content_type, "image/png");
        assert!(
            built
                .authorization
                .starts_with("q-sign-algorithm=sha1&q-ak=AKIDtest&")
        );
        assert!(
            built
                .authorization
                .contains("q-header-list=host&q-url-param-list=&q-signature=")
        );

        // domain 覆盖访问域名，但不影响上传地址与签名
        let custom = CosConfig {
            domain: "https://cdn.example".to_owned(),
            ..config()
        };
        let custom_plan = plan(&custom, "C:\\tmp\\photo.jpg", PNG, 1_700_000_000).expect("已配置");
        assert_eq!(
            custom_plan.result_url,
            "https://cdn.example/gensokyo/1700000000/photo.png"
        );
        assert_eq!(custom_plan.url, built.url);
        assert_eq!(custom_plan.authorization, built.authorization);
    }

    #[test]
    fn plan_rejects_incomplete_configuration() {
        for broken in [
            CosConfig {
                secret_id: String::new(),
                ..config()
            },
            CosConfig {
                secret_key: String::new(),
                ..config()
            },
            CosConfig {
                region: String::new(),
                ..config()
            },
            CosConfig {
                bucket: String::new(),
                ..config()
            },
        ] {
            let error = plan(&broken, "a.png", PNG, 0).unwrap_err();
            assert!(
                matches!(&error, Error::HostingConfig(reason) if reason.contains("COS 未配置")),
                "{broken:?} → {error}"
            );
        }
    }

    #[tokio::test]
    async fn uploads_with_raw_body_and_returns_public_url() {
        let transport = Arc::new(FakeTransport::new().respond(200, ""));
        let url = CosBackend::new(transport.clone(), config())
            .upload(PNG, "photo.png")
            .await
            .expect("上传成功");
        let calls = transport.calls();
        assert_eq!(calls.len(), 1);
        let request = &calls[0];
        assert_eq!(request.method, UploadMethod::Put);
        assert!(
            request
                .url
                .starts_with("https://bucket-125.cos.ap-guangzhou.myqcloud.com/gensokyo/")
        );
        assert!(request.url.ends_with("/photo.png"));
        assert_eq!(url, request.url, "未配 domain 时访问 URL 就是上传 URL");
        assert!(
            request
                .headers
                .iter()
                .any(|(name, value)| name == "Authorization"
                    && value.starts_with("q-sign-algorithm=sha1")),
            "{:?}",
            request.headers
        );
        assert!(
            request.headers.iter().any(|(name, value)| name == "Host"
                && value == "bucket-125.cos.ap-guangzhou.myqcloud.com")
        );
        let body = request.body.as_ref().expect("有请求体");
        assert_eq!(body.content_type, "image/png");
        assert_eq!(body.bytes, PNG, "COS 直传原始字节，不重新编码");
    }

    #[tokio::test]
    async fn maps_status_and_missing_config() {
        let rejected = FakeTransport::new().respond(403, "AccessDenied");
        let error = CosBackend::new(Arc::new(rejected), config())
            .upload(PNG, "a.png")
            .await
            .unwrap_err();
        assert!(matches!(error, Error::HttpStatus(403)), "{error}");

        let quiet = FakeTransport::new();
        let probe = quiet.clone();
        let error = CosBackend::new(Arc::new(quiet), CosConfig::default())
            .upload(PNG, "a.png")
            .await
            .unwrap_err();
        assert!(matches!(error, Error::HostingConfig(_)), "{error}");
        assert!(probe.calls().is_empty(), "未配置时不应发出请求");
    }
}
