//! Nature 免费图床（oss_type = 10）：腾讯 COS 直传，密钥内置。
//!
//! 对齐 Go `imagehosting/nature.go`：
//! - 硬编码一组 COS 密钥（源码里以 base64 存放，运行时解码），指向
//!   `sgame-data-service-1252931805`（ap-nanjing）；
//! - 对象路径 `SnsShare/SocialProfile/<ts>_<8 位十六进制>.<ext>`，随机后缀取纳秒低 32 位；
//! - `Authorization` 与 oss_type 4 同一套 `q-sign-algorithm=sha1` 自签（[`super::qcloud`]）；
//! - GIF 的 `Content-Type` 特殊处理成 `image/jpeg`（Go 原样），只接受 HTTP 200；
//! - 返回 `https://download.nature.qq.com/<path>`。
//!
//! 与 Go 的差异：无。入参 `filename` 不参与对象路径（Go 也忽略它）。

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::data_uri;
use crate::error::{Error, Result};
use crate::hosting::qcloud;
use crate::hosting::transport::{HostingTransport, UploadBody, UploadMethod, UploadRequest};
use crate::hosting::{DEFAULT_UPLOAD_TIMEOUT, HostingBackend, check_url, unix_now};
use crate::sniff::{self, ImageKind};

/// 内置凭据（base64），与 Go `_natureSecretID` / `_natureSecretKey` 逐字符相同。
const SECRET_ID_BASE64: &str = "QUtJRHJiOFRiZlhBWnJ5cVRzMnlnQlNWSkdzSFRROGR0d21O";
/// 内置凭据（base64）。
const SECRET_KEY_BASE64: &str = "UFphTnhLV2ZjTHAzNHJQanJ1dGtXRnlaQ2N5REdCMGQ=";
/// 目标存储桶。
const BUCKET: &str = "sgame-data-service-1252931805";
/// 目标地域。
const REGION: &str = "ap-nanjing";
/// 对外 CDN 前缀。
const CDN: &str = "https://download.nature.qq.com";
/// 对象路径前缀。
const PREFIX: &str = "SnsShare/SocialProfile";

static CREDENTIALS: OnceLock<(String, String)> = OnceLock::new();

/// 解码内置凭据（Go `mustB64` 在 init 时解码；这里首次使用时解码一次）。
fn credentials() -> &'static (String, String) {
    CREDENTIALS.get_or_init(|| {
        let decode = |encoded: &str| {
            data_uri::decode_base64(encoded)
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .expect("Nature 内置 base64 凭据必须可解码")
        };
        (decode(SECRET_ID_BASE64), decode(SECRET_KEY_BASE64))
    })
}

/// 一次 Nature 上传的纯数据描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NaturePlan {
    /// 完整上传 URL。
    pub url: String,
    /// `Host` 头（签名覆盖 `host`）。
    pub host: String,
    /// 对象路径。
    pub path: String,
    /// 图片 MIME（GIF 归一到 `image/jpeg`）。
    pub content_type: &'static str,
    /// `Authorization` 头。
    pub authorization: String,
    /// 上传成功后的访问 URL。
    pub result_url: String,
}

/// 构造上传计划；`timestamp`（Unix 秒）与 `nonce`（纳秒）由调用方给，便于断言路径。
pub fn plan(data: &[u8], timestamp: i64, nonce: u64) -> NaturePlan {
    let kind = sniff::sniff_image(data);
    let extension = extension(kind);
    let content_type = content_type(kind);
    // Go `naturePathSuffix`：`%08x` 取纳秒低 32 位，天然 8 位
    let suffix = nonce % 0x1_0000_0000;
    let path = format!("{PREFIX}/{timestamp}_{suffix:08x}.{extension}");
    let host = qcloud::object_host(BUCKET, REGION);
    let (secret_id, secret_key) = credentials();
    NaturePlan {
        url: format!("https://{host}/{path}"),
        host: host.clone(),
        content_type,
        authorization: qcloud::authorization(secret_id, secret_key, &path, &host, timestamp),
        result_url: format!("{CDN}/{path}"),
        path,
    }
}

/// 对象扩展名：对齐 Go `detectExt`（未识别的格式一律 `jpg`）。
fn extension(kind: Option<ImageKind>) -> &'static str {
    match kind {
        Some(ImageKind::Png) => "png",
        Some(ImageKind::Gif) => "gif",
        Some(ImageKind::WebP) => "webp",
        _ => "jpg",
    }
}

/// `Content-Type`：对齐 Go `detectMIME` + `tryNature` 的 GIF 归一。
fn content_type(kind: Option<ImageKind>) -> &'static str {
    match kind {
        Some(ImageKind::Png) => "image/png",
        Some(ImageKind::WebP) => "image/webp",
        // Go：`if mime == "image/gif" { mime = "image/jpeg" }`；其余落到默认 image/jpeg
        _ => "image/jpeg",
    }
}

/// Nature 上传后端。
#[derive(Debug)]
pub struct NatureBackend {
    transport: Arc<dyn HostingTransport>,
    timeout: Duration,
}

impl NatureBackend {
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

impl HostingBackend for NatureBackend {
    async fn upload(&self, data: &[u8], _filename: &str) -> Result<String> {
        let (timestamp, nonce) = unix_now();
        let plan = plan(data, timestamp, nonce);
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
        // Go：`resp.StatusCode != 200` 即失败
        if response.status != 200 {
            return Err(Error::HttpStatus(response.status));
        }
        check_url(&plan.result_url, "Nature")?;
        Ok(plan.result_url)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{NatureBackend, content_type, credentials, extension, plan};
    use crate::error::Error;
    use crate::hosting::HostingBackend;
    use crate::hosting::UploadMethod;
    use crate::hosting::fixtures::FakeTransport;
    use crate::sniff::ImageKind;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0DIHDR\x00\x00\x02\x80\x00\x00\x01\xE0\x08\x06\x00\x00\x00";
    const GIF: &[u8] = b"GIF89a\x01\x00\x01\x00\x00\x00\x00";

    #[test]
    fn built_in_credentials_decode() {
        let (secret_id, secret_key) = credentials();
        assert!(secret_id.starts_with("AKID"), "{secret_id}");
        assert_eq!(secret_id.len(), 36);
        assert_eq!(secret_key.len(), 32);
    }

    #[test]
    fn plan_follows_go_object_path() {
        let built = plan(PNG, 1_700_000_000, 0x1_2345_6789);
        assert_eq!(built.path, "SnsShare/SocialProfile/1700000000_23456789.png");
        assert_eq!(
            built.url,
            "https://sgame-data-service-1252931805.cos.ap-nanjing.myqcloud.com\
             /SnsShare/SocialProfile/1700000000_23456789.png"
        );
        assert_eq!(
            built.result_url,
            "https://download.nature.qq.com/SnsShare/SocialProfile/1700000000_23456789.png"
        );
        assert_eq!(
            built.host,
            "sgame-data-service-1252931805.cos.ap-nanjing.myqcloud.com"
        );
        assert_eq!(built.content_type, "image/png");
        assert!(
            built
                .authorization
                .starts_with("q-sign-algorithm=sha1&q-ak=AKID")
        );

        // 纳秒低 32 位固定 8 位十六进制（Go `%08x`），不足位补前导零
        assert_eq!(
            plan(PNG, 1, 1).path,
            "SnsShare/SocialProfile/1_00000001.png"
        );
        // 超过 32 位的纳秒只取低位
        assert_eq!(
            plan(PNG, 1, 0x1_0000_0001).path,
            "SnsShare/SocialProfile/1_00000001.png"
        );
    }

    #[test]
    fn gif_is_relabelled_as_jpeg_like_go() {
        assert_eq!(
            content_type(Some(ImageKind::Gif)),
            "image/jpeg",
            "Go 的 GIF 特殊处理"
        );
        assert_eq!(extension(Some(ImageKind::Gif)), "gif", "扩展名保持 gif");
        assert_eq!(content_type(Some(ImageKind::Png)), "image/png");
        assert_eq!(content_type(Some(ImageKind::WebP)), "image/webp");
        assert_eq!(content_type(Some(ImageKind::Jpeg)), "image/jpeg");
        assert_eq!(extension(Some(ImageKind::Jpeg)), "jpg");
        assert_eq!(extension(Some(ImageKind::WebP)), "webp");
        // 未识别 / BMP：Go 的默认分支是 jpeg + jpg
        assert_eq!(content_type(None), "image/jpeg");
        assert_eq!(extension(None), "jpg");
        assert_eq!(content_type(Some(ImageKind::Bmp)), "image/jpeg");
        assert_eq!(extension(Some(ImageKind::Bmp)), "jpg");

        let gif_plan = plan(GIF, 9, 0);
        assert_eq!(gif_plan.content_type, "image/jpeg");
        assert!(gif_plan.path.ends_with(".gif"));
    }

    #[tokio::test]
    async fn uploads_raw_bytes_and_returns_cdn_url() {
        let transport = Arc::new(FakeTransport::new().respond(200, ""));
        let url = NatureBackend::new(transport.clone())
            .upload(PNG, "ignored.png")
            .await
            .expect("上传成功");
        assert!(url.starts_with("https://download.nature.qq.com/SnsShare/SocialProfile/"));

        let calls = transport.calls();
        assert_eq!(calls.len(), 1);
        let request = &calls[0];
        assert_eq!(request.method, UploadMethod::Put);
        assert!(request.url.starts_with(
            "https://sgame-data-service-1252931805.cos.ap-nanjing.myqcloud.com/SnsShare/"
        ));
        assert!(
            request
                .headers
                .iter()
                .any(|(name, value)| name == "Authorization"
                    && value.starts_with("q-sign-algorithm=sha1"))
        );
        let body = request.body.as_ref().expect("有请求体");
        assert_eq!(body.content_type, "image/png");
        assert_eq!(body.bytes, PNG);
    }

    #[tokio::test]
    async fn maps_non_200_status() {
        let transport = FakeTransport::new().respond(503, "busy");
        let error = NatureBackend::new(Arc::new(transport))
            .upload(PNG, "a.png")
            .await
            .unwrap_err();
        assert!(matches!(error, Error::HttpStatus(503)), "{error}");
    }
}
