//! 百度云 BOS 直传（`oss_type` = 2），对齐 Go `oss/baidu.go` 的 `uploadImageBOS`。
//!
//! Go 用 `bce-sdk-go` 的 `PutObjectFromFile`；这里不引 SDK，按 BCE `bce-auth-v1` 规范自签：
//!
//! ```text
//! authStringPrefix = "bce-auth-v1/<AK>/<ISO8601>/1800"
//! signingKey       = hex(hmac_sha256(SK, authStringPrefix))
//! canonicalRequest = "PUT\n/<bucket>/<key>\n\nhost:<host>\nx-bce-date:<date>\n"
//! signature        = hex(hmac_sha256(signingKey, canonicalRequest))
//! Authorization    = authStringPrefix + "/host;x-bce-date/" + signature
//! ```
//!
//! 对象桶名是 **AppID**（Go `clientBos.PutObjectFromFile(appid, …)`），key 是
//! `qqbot-upload-<md5>-<纳秒>.jpg`（Go 用 `os.CreateTemp` 的随机后缀），访问 URL 与上传 URL
//! 同形：`https://<b_BOS_BUCKETNAME>/<appid>/<key>`（BOS 的 path-style 寻址）。
//!
//! 与 Go 的差异：`b_audit`（0 仅上传 / 1 BOS+审核 / 2 本地上传+审核）只支持 0；1 与 2 都依赖
//! Go 的审核客户端或本地 `/uploadpic` 端点，未实现时在发请求前返回 [`Error::HostingConfig`]。

use std::sync::Arc;

use crate::error::{Error, Result};
use crate::hosting::digest;
use crate::hosting::transport::HostingTransport;
use crate::hosting::unix_now;
use crate::oss::{host_of, iso8601, put};

/// 签名有效期（秒），对齐 `bce-sdk-go` 的默认 1800。
const EXPIRATION_SECONDS: u32 = 1800;

/// 百度云 BOS 凭据与目标桶（Go `b_BOS_BUCKETNAME` / `b_BCE_AK` / `b_BCE_SK` / `b_audit`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BaiduBosConfig {
    /// BOS 访问域名（`b_BOS_BUCKETNAME`，如 `bj.bcebos.com`）：Go 里既当 SDK endpoint，
    /// 又当结果 URL 的前缀。
    pub domain: String,
    /// `b_BCE_AK`。
    pub access_key: String,
    /// `b_BCE_SK`。
    pub secret_key: String,
    /// 对象桶名：Go 用 AppID（`config.GetAppIDStr()`）。
    pub bucket: String,
    /// `b_audit`：0 仅上传；1 / 2 带审核（未实现）。
    pub audit: i64,
}

impl BaiduBosConfig {
    /// 发请求前的必填项与模式检查（Go 只打日志，SDK 调用才失败）。
    fn validate(&self) -> Result<()> {
        if self.domain.is_empty() {
            return Err(Error::HostingConfig(
                "百度云 BOS 未配置（请填 b_BOS_BUCKETNAME）".to_owned(),
            ));
        }
        if self.access_key.is_empty() || self.secret_key.is_empty() {
            return Err(Error::HostingConfig(
                "百度云 BOS 未配置（请填 b_BCE_AK / b_BCE_SK）".to_owned(),
            ));
        }
        if self.bucket.is_empty() {
            return Err(Error::HostingConfig(
                "百度云 BOS 未取到 AppID：对象桶名就是 AppID".to_owned(),
            ));
        }
        if self.audit != 0 {
            return Err(Error::HostingConfig(format!(
                "百度云 BOS 审核模式 b_audit={} 尚未实现（仅支持 0，仅上传），本次上传跳过",
                self.audit
            )));
        }
        Ok(())
    }
}

/// 一次 BOS 上传的纯数据描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaiduPlan {
    /// 完整上传 URL（`https://<domain>/<bucket>/<key>`，path-style）。
    pub url: String,
    /// `Host` 头（签名覆盖 `host`）。
    pub host: String,
    /// 桶内对象 key（`qqbot-upload-<md5>-<纳秒>.jpg`）。
    pub key: String,
    /// `x-bce-date` 头（签名覆盖它）。
    pub date: String,
    /// `Authorization` 头。
    pub authorization: String,
    /// 上传成功后的访问 URL（与 [`Self::url`] 相同）。
    pub result_url: String,
}

/// 构造上传计划；`timestamp` / `suffix` 由调用方给，便于断言签名与文件名。
pub fn plan(
    config: &BaiduBosConfig,
    data: &[u8],
    timestamp: i64,
    suffix: u64,
) -> Result<BaiduPlan> {
    config.validate()?;
    let host = host_of(&config.domain);
    if host.is_empty() {
        return Err(Error::HostingConfig(
            "百度云 BOS 的 b_BOS_BUCKETNAME 只有协议头，没有域名".to_owned(),
        ));
    }
    let key = format!("qqbot-upload-{}-{suffix}.jpg", digest::md5_hex(data));
    // path-style：第一个路径段是桶名（AppID），与 Go 的 PutObjectFromFile(appid, key) 一致
    let uri = format!("/{}/{key}", config.bucket);
    let date = iso8601(timestamp);
    let authorization = authorization(config, host, &uri, &date);
    Ok(BaiduPlan {
        url: format!("https://{host}{uri}"),
        result_url: format!("https://{host}{uri}"),
        host: host.to_owned(),
        key,
        date,
        authorization,
    })
}

/// BCE `bce-auth-v1` 签名（只签 `host` 与 `x-bce-date`，BOS 要求 `host` 必签）。
fn authorization(config: &BaiduBosConfig, host: &str, uri: &str, date: &str) -> String {
    let prefix = format!(
        "bce-auth-v1/{}/{date}/{EXPIRATION_SECONDS}",
        config.access_key
    );
    let signing_key = digest::hmac_sha256_hex(config.secret_key.as_bytes(), prefix.as_bytes());
    let canonical_headers = format!("host:{host}\nx-bce-date:{date}\n");
    let canonical_request = format!("PUT\n{uri}\n\n{canonical_headers}");
    let signature = digest::hmac_sha256_hex(signing_key.as_bytes(), canonical_request.as_bytes());
    format!("{prefix}/host;x-bce-date/{signature}")
}

/// 上传一份图片字节，返回 `https://<b_BOS_BUCKETNAME>/<appid>/<key>`。
pub async fn upload(
    transport: Arc<dyn HostingTransport>,
    config: &BaiduBosConfig,
    data: &[u8],
) -> Result<String> {
    let (timestamp, nanos) = unix_now();
    let plan = plan(config, data, timestamp, nanos)?;
    put(
        transport,
        plan.url,
        vec![
            ("Host".to_owned(), plan.host),
            ("x-bce-date".to_owned(), plan.date),
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

    use super::{BaiduBosConfig, plan};
    use crate::error::Error;
    use crate::hosting::UploadMethod;
    use crate::hosting::fixtures::FakeTransport;
    use crate::oss::upload as upload_oss;
    use crate::oss::{OssConfig, OssKind};

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0DIHDR";

    fn bos() -> BaiduBosConfig {
        BaiduBosConfig {
            domain: "bj.bcebos.com".to_owned(),
            access_key: "bce-ak".to_owned(),
            secret_key: "bce-sk".to_owned(),
            bucket: "10001".to_owned(),
            audit: 0,
        }
    }

    fn config() -> OssConfig {
        OssConfig {
            baidu: bos(),
            ..OssConfig::default()
        }
    }

    #[test]
    fn plan_uses_appid_bucket_and_bce_auth() {
        let built = plan(&bos(), PNG, 1_794_132_000, 42).expect("已配置");
        assert_eq!(built.host, "bj.bcebos.com");
        assert!(built.key.ends_with("-42.jpg"), "{}", built.key);
        assert!(built.key.starts_with("qqbot-upload-"));
        assert_eq!(built.url, format!("https://bj.bcebos.com/10001/{}", built.key));
        assert_eq!(built.result_url, built.url);
        assert_eq!(built.date, "2026-11-08T10:00:00Z");
        let expected_prefix = "bce-auth-v1/bce-ak/2026-11-08T10:00:00Z/1800/host;x-bce-date/";
        assert!(
            built.authorization.starts_with(expected_prefix),
            "{}",
            built.authorization
        );
        assert_eq!(
            built.authorization.len(),
            expected_prefix.len() + 64,
            "签名是 64 位十六进制"
        );
    }

    #[test]
    fn plan_strips_scheme_from_domain_and_rejects_bad_config() {
        let with_scheme = BaiduBosConfig {
            domain: "https://bj.bcebos.com/".to_owned(),
            ..bos()
        };
        let built = plan(&with_scheme, PNG, 0, 1).expect("已配置");
        assert_eq!(built.host, "bj.bcebos.com");

        for broken in [
            BaiduBosConfig {
                domain: String::new(),
                ..bos()
            },
            BaiduBosConfig {
                access_key: String::new(),
                ..bos()
            },
            BaiduBosConfig {
                secret_key: String::new(),
                ..bos()
            },
            BaiduBosConfig {
                bucket: String::new(),
                ..bos()
            },
            BaiduBosConfig {
                audit: 1,
                ..bos()
            },
            BaiduBosConfig {
                audit: 2,
                ..bos()
            },
        ] {
            let error = plan(&broken, PNG, 0, 1).unwrap_err();
            assert!(
                matches!(error, Error::HostingConfig(_)),
                "{broken:?} → {error}"
            );
        }
    }

    #[tokio::test]
    async fn uploads_put_with_bce_headers() {
        let transport = Arc::new(FakeTransport::new().respond(200, ""));
        let url = upload_oss(OssKind::BaiduBos, transport.clone(), &config(), PNG)
            .await
            .expect("上传成功");

        let calls = transport.calls();
        assert_eq!(calls.len(), 1, "单次尝试，不重试");
        let request = &calls[0];
        assert_eq!(request.method, UploadMethod::Put);
        assert_eq!(request.url, url);
        assert!(request.url.starts_with("https://bj.bcebos.com/10001/qqbot-upload-"));
        for header in ["Host", "x-bce-date", "Authorization"] {
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
        let transport = Arc::new(FakeTransport::new().respond(403, "AccessDenied"));
        let error = upload_oss(OssKind::BaiduBos, transport, &config(), PNG)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::HttpStatus(403)), "{error}");
    }
}
