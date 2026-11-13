//! 内置云 OSS 三后端：`oss_type` 1~3（腾讯云 COS / 百度云 BOS / 阿里云 OSS）。
//!
//! 对齐 Go `oss/{tencent,baidu,aliyun}.go`——`images.UploadBase64ImageToServer` 里 0~3 走
//! 的是内置通道，4~10 才走 [`crate::hosting`]（图床）。两条链互斥，上传失败都保留原引用。
//!
//! # 共同口径
//!
//! - 对象 key 一律是「图片字节的 MD5（十六进制）+ `.jpg`」：腾讯/阿里写死 `.jpg`，百度是
//!   `qqbot-upload-<md5>-<纳秒>.jpg`（Go 用 `os.CreateTemp` 的随机后缀），与 Go 逐字一致；
//! - `Content-Type` 一律 `image/jpeg`：Go 先写进 `.jpg` 临时文件再由 SDK 按后缀给类型；
//! - **单次尝试**：Go 的 `oss` 包没有重试（重试只存在于 `imagehosting`），这里保持一致；
//! - 成功判定是 2xx（三家 SDK 同口径），响应体不解析——URL 由配置与 key 拼出来。
//!
//! # 内容审核（`t_audit` / `b_audit` / `a_audit`）
//!
//! 开关打开时**先上传、再审核**（与 Go 同序），审核在 [`audit`] 里；不通过返回
//! [`Error::AuditRejected`]，调用方据此保留原引用。三家请求形态逐字段对齐 Go 所用 SDK；
//! Go 在几处会静默放行、这里统一报错，逐条列在 [`audit`] 的模块文档里。
//! `b_audit=2`（先传本机 `/uploadpic` 再审核）依赖 runtime 的本地上传出口，本 crate 没有，
//! 仍在发请求前返回 [`Error::HostingConfig`]（fail-closed）。
//!
//! # 与 Go 的差异
//!
//! **语音上传不在这里**：Go 的 `oss.UploadAndAuditRecord*`（`.amr`）只服务
//! `images.UploadBase64RecordToServer`，而那条函数在 Go 里**没有调用方**——`POST /uploadrecord`
//! 的 handler（`server/uploadpic.go:186`）只把字节落到 `./channel_temp/<md5>.silk` 再拼静态 URL，
//! 完全不看 `oss_type`。Rust 的 `/uploadrecord` 与 Go handler 逐行一致（见
//! `gensokyo-runtime::upload_http`），因此这里不提供无消费者的语音上传 API。

pub mod aliyun;
mod audit;
pub mod baidu;
pub mod tencent;

use std::sync::Arc;

use time::OffsetDateTime;
use time::format_description::FormatItem;
use time::macros::format_description;

use crate::error::{Error, Result};
use crate::hosting::DEFAULT_UPLOAD_TIMEOUT;
use crate::hosting::transport::{HostingTransport, UploadBody, UploadMethod, UploadRequest};

pub use aliyun::{AliyunOssConfig, AliyunPlan};
pub use baidu::{BaiduBosConfig, BaiduPlan};
pub use tencent::{TencentCosConfig, TencentCosPlan};

/// 三个后端共用的对象类型：Go 的临时文件后缀写死 `.jpg`，SDK 按后缀给 `Content-Type`。
pub(crate) const IMAGE_CONTENT_TYPE: &str = "image/jpeg";

/// 内置云 OSS 后端种类，取值与 Go `config.OssTypeTencent/Baidu/Aliyun` 一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OssKind {
    /// 腾讯云 COS（oss_type 1，`oss/tencent.go`）。
    TencentCos,
    /// 百度云 BOS（oss_type 2，`oss/baidu.go`）。
    BaiduBos,
    /// 阿里云 OSS（oss_type 3，`oss/aliyun.go`）。
    AliyunOss,
}

impl OssKind {
    /// `oss_type` → 内置云 OSS 后端；0（本地服务器）与 4 以上（图床）都不是它。
    #[must_use]
    pub const fn from_oss_type(oss_type: i64) -> Option<Self> {
        match oss_type {
            1 => Some(Self::TencentCos),
            2 => Some(Self::BaiduBos),
            3 => Some(Self::AliyunOss),
            _ => None,
        }
    }

    /// 反向映射，便于日志与测试对拍 [`Self::from_oss_type`]。
    #[must_use]
    pub const fn oss_type(self) -> i64 {
        match self {
            Self::TencentCos => 1,
            Self::BaiduBos => 2,
            Self::AliyunOss => 3,
        }
    }

    /// Go `config.GetOssTypeName` 的名字（日志用）。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::TencentCos => "tencent_cos",
            Self::BaiduBos => "baidu_bos",
            Self::AliyunOss => "aliyun_oss",
        }
    }
}

/// 三个内置后端的凭据与目标桶（Go `structs.Settings` 的 `t_COS_*` / `b_*` / `a_OSS_*`）。
///
/// 每个后端只读自己那一段；缺字段时在**发请求前**返回 [`Error::HostingConfig`]。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OssConfig {
    /// 腾讯云 COS（oss_type 1）。
    pub tencent: TencentCosConfig,
    /// 百度云 BOS（oss_type 2）。
    pub baidu: BaiduBosConfig,
    /// 阿里云 OSS（oss_type 3）。
    pub aliyun: AliyunOssConfig,
}

/// 按 `oss_type` 上传一份图片字节，返回可公开访问的 URL。
///
/// **单次尝试**（Go `oss` 包无重试）：失败原样返回 [`Error`]，由调用方决定保留原引用。
pub async fn upload(
    kind: OssKind,
    transport: Arc<dyn HostingTransport>,
    config: &OssConfig,
    data: &[u8],
) -> Result<String> {
    match kind {
        OssKind::TencentCos => tencent::upload(transport, &config.tencent, data).await,
        OssKind::BaiduBos => baidu::upload(transport, &config.baidu, data).await,
        OssKind::AliyunOss => aliyun::upload(transport, &config.aliyun, data).await,
    }
}

/// 三个后端共用的 PUT：原始字节直传，2xx 视为成功，不解析响应体。
pub(crate) async fn put(
    transport: Arc<dyn HostingTransport>,
    url: String,
    headers: Vec<(String, String)>,
    data: &[u8],
) -> Result<()> {
    let response = transport
        .send(UploadRequest {
            method: UploadMethod::Put,
            url,
            headers,
            body: Some(UploadBody {
                content_type: IMAGE_CONTENT_TYPE.to_owned(),
                bytes: data.to_vec(),
            }),
            timeout: DEFAULT_UPLOAD_TIMEOUT,
        })
        .await?;
    if !(200..300).contains(&response.status) {
        return Err(Error::HttpStatus(response.status));
    }
    Ok(())
}

/// `Date` 头（RFC 1123 / GMT，阿里云 OSS 的签名时间）：`Sun, 08 Nov 2026 10:00:00 GMT`。
pub(crate) fn http_date(timestamp: i64) -> String {
    const FORMAT: &[FormatItem<'static>] = format_description!(
        "[weekday repr:short], [day] [month repr:short] [year] [hour]:[minute]:[second] GMT"
    );
    // 格式描述是静态常量，`OffsetDateTime` 也由合法时间戳构造，格式化不会失败。
    moment(timestamp).format(FORMAT).unwrap_or_default()
}

/// `x-bce-date` 头（ISO 8601 UTC，百度云 BOS 的签名时间）：`2026-11-08T10:00:00Z`。
pub(crate) fn iso8601(timestamp: i64) -> String {
    const FORMAT: &[FormatItem<'static>] =
        format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");
    moment(timestamp).format(FORMAT).unwrap_or_default()
}

fn moment(timestamp: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(timestamp).unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

/// 配置里的 endpoint / 域名 → 纯主机名：去掉 `http(s)://` 与尾部 `/`。
///
/// Go 把配置值直接拼进 URL（`fmt.Sprintf("https://%s/%s/%s", bucketDomain, …)`），
/// 用户多打一个协议头就会拼出 `https://https://…`；这里统一剥掉，语义只多不少。
pub(crate) fn host_of(endpoint: &str) -> &str {
    let trimmed = endpoint.trim();
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    without_scheme.trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use super::{OssKind, http_date, iso8601};

    #[test]
    fn maps_oss_type_one_to_three() {
        let expected = [
            (1, OssKind::TencentCos, "tencent_cos"),
            (2, OssKind::BaiduBos, "baidu_bos"),
            (3, OssKind::AliyunOss, "aliyun_oss"),
        ];
        for (oss_type, kind, name) in expected {
            assert_eq!(OssKind::from_oss_type(oss_type), Some(kind), "{oss_type}");
            assert_eq!(kind.oss_type(), oss_type);
            assert_eq!(kind.name(), name);
        }
        for other in [-1, 0, 4, 7, 10, 11] {
            assert_eq!(OssKind::from_oss_type(other), None, "{other}");
        }
    }

    #[test]
    fn formats_signature_dates() {
        // 2026-11-08T10:00:00Z
        let timestamp = 1_794_132_000;
        assert_eq!(http_date(timestamp), "Sun, 08 Nov 2026 10:00:00 GMT");
        assert_eq!(iso8601(timestamp), "2026-11-08T10:00:00Z");
        assert_eq!(http_date(0), "Thu, 01 Jan 1970 00:00:00 GMT");
    }
}
