//! 图床上传：`oss_type` → 后端分派。
//!
//! 对齐 Go `imagehosting/hosting.go` 的 `UploadProvider`：配置里只有一个 `oss_type`，
//! 由它决定用哪个后端（不再同时启用多个图床）。
//!
//! # 覆盖范围（M2b）
//! - [`HostingKind`]：`oss_type` 4~10 的枚举，[`HostingKind::from_oss_type`] 负责映射；
//!   0（未启用）与 1~3（阿里云/百度云/腾讯云内置 OSS）返回 `None`——那几档走 `oss/`，
//!   不在本 crate。
//! - [`HostingBackend`]：`async fn upload(&self, data, filename) -> Result<String>`，
//!   返回可直接访问的 URL。
//! - [`ChatGlmBackend`]（oss_type 7）与 [`UkakaBackend`]（oss_type 8）：单步 multipart
//!   直传 / 先取签名再 multipart，URL、字段名、成功判定都照 Go。
//! - [`upload_plan`]：纯函数，把「用哪个后端 + 文件名 + 字节」变成可断言的
//!   [`UploadPlan`]（端点、规整后的文件名、MIME、签名 URL），不发任何请求。
//! - [`HostingTransport`]：上传专用的网络出口；生产实现由 `fetch::HttpTransport`
//!   复用同一个 `reqwest::Client`，测试实现只要记录请求即可断言 multipart 字段。
//!
//! # 与 Go 的差异
//! - **重试**：Go `retryUpload` 最多 3 次、失败间隔 1s/2s。本 crate **不在库内重试**：
//!   退避属于调度策略（谁调用谁决定），一次 `upload` 就是一次尝试。
//! - **状态码**：ChatGLM 与 Go 一致只认 200；Ukaka 与 Go 一致认 `< 300`。签名 GET 在 Go
//!   里不检查状态码，本 crate 要求 200（否则会把错误页当 JSON 解析）。
//! - **URL 校验**：Go 只判空；本 crate 还要求返回的是 `http(s)` 绝对地址。
//! - **文件部分 MIME**：与 Go `CreateFormFile` 一致恒为 `application/octet-stream`，
//!   真实图片类型由 ChatGLM 的 `X-File-Mime` 头声明。
//! - **文件名**：与 Go `ensureExt` 一致按嗅探结果修正扩展名；额外去掉目录部分与控制字符。
//!
//! # 未做项（见 crate 级文档）
//! COS 自签上传（`imagehosting/cos.go`）、Bilibili 登录态与分片、星野的 PUT 直传、
//! Nature 的内置 COS 密钥、QQ频道（oss_type 6）上传。

pub mod chatglm;
#[cfg(test)]
mod fixtures;
mod json;
pub mod multipart;
pub mod transport;
pub mod ukaka;

use std::sync::Arc;
use std::time::Duration;

use crate::error::{Error, Result};

pub use chatglm::ChatGlmBackend;
pub use multipart::MultipartBody;
pub use transport::{
    HostingTransport, MAX_RESPONSE_BYTES, UploadBody, UploadMethod, UploadRequest, UploadResponse,
};
pub use ukaka::UkakaBackend;

/// 默认单次上传总超时（对齐 Go `providerHTTPClient` 的 30s）。
pub const DEFAULT_UPLOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// 图床后端种类，取值与 Go `config.OssTypeXXX` 一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostingKind {
    /// 腾讯云 COS 自签（oss_type 4）。
    Cos,
    /// Bilibili 相册（oss_type 5，需登录态）。
    Bilibili,
    /// QQ频道文件（oss_type 6）。
    QqChannel,
    /// ChatGLM 免费图床（oss_type 7）。
    ChatGlm,
    /// Ukaka 签名图床（oss_type 8）。
    Ukaka,
    /// 星野签名图床（oss_type 9）。
    Xingye,
    /// Nature 内置 COS 直传（oss_type 10）。
    Nature,
}

impl HostingKind {
    /// `oss_type` → 图床种类。0（未启用）与 1~3（内置 OSS 后端）不是图床，返回 `None`。
    #[must_use]
    pub const fn from_oss_type(oss_type: i64) -> Option<Self> {
        match oss_type {
            4 => Some(Self::Cos),
            5 => Some(Self::Bilibili),
            6 => Some(Self::QqChannel),
            7 => Some(Self::ChatGlm),
            8 => Some(Self::Ukaka),
            9 => Some(Self::Xingye),
            10 => Some(Self::Nature),
            _ => None,
        }
    }

    /// 反向映射，便于日志与测试对拍 [`Self::from_oss_type`]。
    #[must_use]
    pub const fn oss_type(self) -> i64 {
        match self {
            Self::Cos => 4,
            Self::Bilibili => 5,
            Self::QqChannel => 6,
            Self::ChatGlm => 7,
            Self::Ukaka => 8,
            Self::Xingye => 9,
            Self::Nature => 10,
        }
    }

    /// Go `config.GetOssTypeName` 的 provider 名（`imagehosting.UploadProvider` 的分派键）。
    #[must_use]
    pub const fn provider_name(self) -> &'static str {
        match self {
            Self::Cos => "cos",
            Self::Bilibili => "bilibili",
            Self::QqChannel => "qq_channel",
            Self::ChatGlm => "chatglm",
            Self::Ukaka => "ukaka",
            Self::Xingye => "xingye",
            Self::Nature => "nature",
        }
    }

    /// M2b 是否提供了实现。[`ChatGlmBackend`] / [`UkakaBackend`] 之外的种类只保留枚举值。
    #[must_use]
    pub const fn is_implemented(self) -> bool {
        matches!(self, Self::ChatGlm | Self::Ukaka)
    }

    /// 未实现时给 [`Error::HostingUnsupported`] 用的说明。
    fn unsupported_reason(self) -> String {
        format!(
            "{} (oss_type={}, provider={}) 未在 M2b 实现",
            self.provider_name(),
            self.oss_type(),
            self.provider_name()
        )
    }
}

/// 图床上传后端。
///
/// 只有具体后端（`ChatGlmBackend` / `UkakaBackend`）需要被调用，因此 trait 用 `async fn`
/// 而不是装箱 future；需要 dyn 分派的通道是 [`HostingTransport`]。
#[allow(async_fn_in_trait, reason = "不做 dyn 分派；调用方持有具体后端类型")]
pub trait HostingBackend: Send + Sync {
    /// 上传 `data`，返回可直接访问的 URL。
    ///
    /// 入参非法、传输失败、非预期状态码、响应里没有 URL 都会返回 [`Error`]；
    /// 不会静默返回空字符串（Go 的空 URL 检查在这里是错误）。
    async fn upload(&self, data: &[u8], filename: &str) -> Result<String>;
}

/// 单步 `multipart/form-data` 直传计划（ChatGLM）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectMultipartPlan {
    /// 上传端点。
    pub url: &'static str,
    /// 规整后的文件名（扩展名已按真实格式修正）。
    pub filename: String,
    /// 图片 MIME（嗅探结果，未知时兜底 `image/jpeg`）。
    pub mime: &'static str,
    /// 需要随请求附带的头。
    pub headers: Vec<(&'static str, &'static str)>,
}

/// 先取签名再上传的计划（Ukaka）。
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

/// 一次上传的纯数据描述，便于在不发请求的前提下断言端点与字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadPlan {
    /// 单步 multipart 直传。
    DirectMultipart(DirectMultipartPlan),
    /// 先 GET 签名，再按签名结果上传。
    Signed(SignedPlan),
}

/// 按 `oss_type` 对应的图床构造上传计划；未实现的种类返回
/// [`Error::HostingUnsupported`]。
pub fn upload_plan(kind: HostingKind, filename: &str, data: &[u8]) -> Result<UploadPlan> {
    match kind {
        HostingKind::ChatGlm => Ok(UploadPlan::DirectMultipart(chatglm::plan(filename, data))),
        HostingKind::Ukaka => Ok(UploadPlan::Signed(ukaka::plan(filename, data))),
        other => Err(Error::HostingUnsupported(other.unsupported_reason())),
    }
}

/// 按 `oss_type` 对应的图床上传，等价 Go `UploadProvider` 的分派。
///
/// `transport` 由调用方注入（生产用 `Arc<HttpTransport>`，测试用 Fake）。
pub async fn upload(
    kind: HostingKind,
    transport: Arc<dyn HostingTransport>,
    data: &[u8],
    filename: &str,
) -> Result<String> {
    match kind {
        HostingKind::ChatGlm => ChatGlmBackend::new(transport).upload(data, filename).await,
        HostingKind::Ukaka => UkakaBackend::new(transport).upload(data, filename).await,
        other => Err(Error::HostingUnsupported(other.unsupported_reason())),
    }
}

/// 图床返回的 URL 必须是 `http(s)` 绝对地址（Go 只判空）。
pub(crate) fn check_url(url: &str, provider: &str) -> Result<()> {
    if url.is_empty() {
        return Err(Error::HostingResponse(format!("{provider} 返回空 URL")));
    }
    if !is_http_url(url) {
        return Err(Error::HostingResponse(format!(
            "{provider} 返回的 URL 不是 http(s): {url}"
        )));
    }
    Ok(())
}

/// scheme 大小写不敏感的 `http(s)://` 判断。
fn is_http_url(url: &str) -> bool {
    ["http://", "https://"].iter().any(|prefix| {
        url.get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        ChatGlmBackend, DirectMultipartPlan, HostingKind, SignedPlan, UploadPlan, chatglm,
        fixtures::FakeTransport, ukaka, upload, upload_plan,
    };
    use crate::error::Error;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0DIHDR\x00\x00\x02\x80\x00\x00\x01\xE0\x08\x06\x00\x00\x00";

    #[test]
    fn maps_oss_type_range() {
        // 0 未启用，1~3 是内置 OSS 后端（阿里云/百度云/腾讯云），都不是图床
        for disabled in [0, 1, 2, 3, -1, 11, 99] {
            assert_eq!(HostingKind::from_oss_type(disabled), None, "{disabled}");
        }
        let expected = [
            (4, HostingKind::Cos, "cos"),
            (5, HostingKind::Bilibili, "bilibili"),
            // Go `imagehosting/hosting.go` 也把 6 当图床分派，这里保留以免 4~10 出现空洞
            (6, HostingKind::QqChannel, "qq_channel"),
            (7, HostingKind::ChatGlm, "chatglm"),
            (8, HostingKind::Ukaka, "ukaka"),
            (9, HostingKind::Xingye, "xingye"),
            (10, HostingKind::Nature, "nature"),
        ];
        for (oss_type, kind, provider) in expected {
            assert_eq!(
                HostingKind::from_oss_type(oss_type),
                Some(kind),
                "{oss_type}"
            );
            assert_eq!(kind.oss_type(), oss_type);
            assert_eq!(kind.provider_name(), provider);
        }
        assert_eq!(
            expected
                .iter()
                .filter(|(_, kind, _)| kind.is_implemented())
                .count(),
            2,
            "M2b 只落地 ChatGLM / Ukaka"
        );
    }

    #[test]
    fn chatglm_plan_is_pure_and_normalized() {
        let plan = upload_plan(HostingKind::ChatGlm, "photo.png", PNG).expect("有实现");
        assert_eq!(
            plan,
            UploadPlan::DirectMultipart(DirectMultipartPlan {
                url: chatglm::ENDPOINT,
                filename: "photo.png".to_owned(),
                mime: "image/png",
                headers: vec![(chatglm::FILE_MIME_HEADER, "image/png")],
            })
        );
        // 扩展名与真实格式不符时按嗅探结果改正（Go `ensureExt`）
        let relabelled = upload_plan(HostingKind::ChatGlm, "photo.jpg", PNG).expect("有实现");
        assert!(matches!(
            relabelled,
            UploadPlan::DirectMultipart(DirectMultipartPlan { ref filename, .. }) if filename == "photo.png"
        ));
        // 同样的输入必须得到同样的计划（便于断言、便于日志对拍）
        assert_eq!(
            relabelled,
            upload_plan(HostingKind::ChatGlm, "photo.jpg", PNG).unwrap()
        );
    }

    #[test]
    fn ukaka_plan_carries_encoded_sign_url() {
        let plan = upload_plan(HostingKind::Ukaka, "图 1.png", PNG).expect("有实现");
        let UploadPlan::Signed(SignedPlan {
            module,
            sign_url,
            filename,
            mime,
        }) = plan
        else {
            panic!("ukaka 应走签名流程");
        };
        assert_eq!(module, ukaka::MODULE);
        assert_eq!(filename, "图 1.png");
        assert_eq!(mime, "image/png");
        assert_eq!(
            sign_url,
            "https://bed-sign.vercel.0013107.xyz/sign\
             ?filename=%E5%9B%BE%201.png&mimeType=image%2Fpng&module=ukaka"
        );
    }

    #[test]
    fn unsupported_kinds_report_reason() {
        for kind in [
            HostingKind::Cos,
            HostingKind::Bilibili,
            HostingKind::QqChannel,
            HostingKind::Xingye,
            HostingKind::Nature,
        ] {
            let error = upload_plan(kind, "a.png", PNG).unwrap_err();
            assert!(
                matches!(&error, Error::HostingUnsupported(reason) if reason.contains(kind.provider_name())),
                "{kind:?}: {error}"
            );
        }
    }

    #[tokio::test]
    async fn upload_dispatches_by_kind() {
        let transport = Arc::new(
            FakeTransport::new().respond(200, r#"{"result":{"file_url":"https://x/1.png"}}"#),
        );
        let url = upload(HostingKind::ChatGlm, transport.clone(), PNG, "a.png")
            .await
            .expect("上传成功");
        assert_eq!(url, "https://x/1.png");
        assert_eq!(transport.calls().len(), 1);

        let unsupported = upload(
            HostingKind::Nature,
            Arc::new(FakeTransport::new()),
            PNG,
            "a.png",
        )
        .await;
        assert!(matches!(unsupported, Err(Error::HostingUnsupported(_))));

        // 未实现的种类不会发出任何请求
        let quiet = FakeTransport::new();
        let quiet_probe = quiet.clone();
        assert!(
            upload(HostingKind::Cos, Arc::new(quiet), PNG, "a.png")
                .await
                .is_err()
        );
        assert!(quiet_probe.calls().is_empty());
    }

    #[test]
    fn backend_types_are_debug_and_constructible() {
        let backend = ChatGlmBackend::new(Arc::new(FakeTransport::new()));
        assert!(format!("{backend:?}").contains("ChatGlmBackend"));
        let backend = super::UkakaBackend::new(Arc::new(FakeTransport::new()));
        assert!(format!("{backend:?}").contains("UkakaBackend"));
    }
}
