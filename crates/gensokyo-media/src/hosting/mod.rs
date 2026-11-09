//! 图床上传：`oss_type` → 后端分派。
//!
//! 对齐 Go `imagehosting/hosting.go` 的 `UploadProvider`：配置里只有一个 `oss_type`，
//! 由它决定用哪个后端（不再同时启用多个图床）。
//!
//! # 覆盖范围（P7）
//! - [`HostingKind`]：`oss_type` 4~10 的枚举，[`HostingKind::from_oss_type`] 负责映射；
//!   0（未启用）与 1~3（阿里云 / 百度 / 腾讯云内置 OSS）返回 `None`——那几档走 `oss/`，
//!   不在本 crate。
//! - [`HostingBackend`]：`async fn upload(&self, data, filename) -> Result<String>`，
//!   返回可直接访问的 URL；**一次 `upload` 就是一次尝试**。
//! - 各后端与 Go 文件一一对应：
//!
//!   | oss_type | 后端 | Go 文件 | 形态 |
//!   |---|---|---|---|
//!   | 4 | [`CosBackend`] | `cos.go` | 需配置；q-sign sha1 自签 + PUT 原始字节 |
//!   | 5 | [`BilibiliBackend`] | `bilibili.go` | 需登录态；multipart POST + `location` |
//!   | 6 | [`QqChannelBackend`] | `qq_channel.go` | 需 channel_id；频道发图 + 内容 MD5 拼 CDN |
//!   | 7 | [`ChatGlmBackend`] | `chatglm.go` | 免配置；multipart 直传 + `result.file_url` |
//!   | 8 | [`UkakaBackend`] | `signed.go`（ukaka） | 免配置；取签名 + multipart POST |
//!   | 9 | [`XingyeBackend`] | `signed.go`（xingye） | 免配置；取签名 + PUT 直传 |
//!   | 10 | [`NatureBackend`] | `nature.go` | 免配置（内置密钥）；q-sign sha1 + PUT |
//!
//! - [`HostingConfig`]：4/5/6 需要的凭据与目标桶。Go 从全局 `config` 读，本 crate 由调用方
//!   注入（`gensokyo-config` 的 `ImageHosting*` 结构体在这里有同名镜像类型，接入层负责搬运）。
//!   缺配置时返回 [`Error::HostingConfig`]，**不发请求**。
//! - [`upload`] / [`upload_with_retry`]：唯一的分派与重试入口（见下）。
//! - [`HostingTransport`]：上传专用的网络出口；生产实现复用 [`crate::fetch::HttpTransport`]
//!   的同一个 `reqwest::Client`，测试实现只记录请求，断言 URL / 头 / 字段。
//!
//! # 重试放在这一层
//! Go 的 `retryUpload` 包在 `UploadProvider` 外面，各 provider 函数本身只做一次尝试。这里
//! 保持一致：后端 `upload` 单次生效，[`upload`] 才是「3 次 / 1s / 2s」的调用侧。分类见
//! [`retry::is_retryable`]：网络失败、超时、5xx 重试；4xx 与响应格式错误不重试。
//!
//! # 与 Go 的差异
//! - **不重试的错误集合**：Go 任何错误都重试 3 次；这里只重试可恢复错误。
//! - **状态码**：ChatGLM/COS/Nature/Bilibili 只认 200（等同 Go）；Ukaka/星野 认 `< 300`
//!   （等同 Go）；签名请求 Go 不查状态码，这里要求 200；QQ频道 Go 完全不查，这里要求 2xx。
//! - **URL 校验**：Go 只判空；本 crate 还要求返回的是 `http(s)` 绝对地址。
//! - **短签名外链**：ChatGLM 上传成功后若结果命中 `chatglm.cn`（短签名域），先用
//!   [`crate::redirect`] 解析 302 末跳直链；解析失败保留原短链并告警，不阻断发送。
//!   Go 在同一位置解析失败时回退本地公网 URL 上传（`localUploadBehavior`），本 crate
//!   这一层没有本地上传出口，降级语义交给调用方。
//! - **文件名**：与 Go `ensureExt` 一致按嗅探结果修正扩展名，并额外去掉目录部分与控制字符
//!   （避免把客户端本地路径写进对象 key / `Content-Disposition`）。
//! - **COS / Nature 的签名**：与 Go 逐字节一致（[`qcloud`] 一份实现，Go 在两个文件里各抄一份）。

pub mod bilibili;
pub mod chatglm;
pub mod cos;
pub(crate) mod digest;
#[cfg(test)]
pub(crate) mod fixtures;
pub(crate) mod json;
pub mod multipart;
pub mod nature;
pub(crate) mod qcloud;
pub mod qq_channel;
pub mod retry;
pub mod signed;
pub mod transport;
pub mod ukaka;
pub mod xingye;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};

pub use bilibili::{BilibiliBackend, BilibiliConfig, BilibiliPlan};
pub use chatglm::{ChatGlmBackend, ChatGlmPlan};
pub use cos::{CosBackend, CosConfig, CosPlan};
pub use multipart::MultipartBody;
pub use nature::{NatureBackend, NaturePlan};
pub use qq_channel::{QqChannelBackend, QqChannelConfig, QqChannelPlan};
pub use retry::{RetryPolicy, is_retryable};
pub use signed::SignedPlan;
pub use transport::{
    HostingTransport, MAX_RESPONSE_BYTES, UploadBody, UploadMethod, UploadRequest, UploadResponse,
};
pub use ukaka::UkakaBackend;
pub use xingye::XingyeBackend;

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
    /// `oss_type` → 图床种类。0（本地上传，Go `localUploadBehavior`）与 1~3（内置云 OSS，
    /// 见 [`crate::oss`]）不是图床，返回 `None`。
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
}

/// 图床凭据与目标桶（对齐 Go `structs.ImageHosting{COS,Bilibili,QQChannel}`）。
///
/// 免配置的后端（7/8/9/10）忽略它；4/5/6 缺字段时返回 [`Error::HostingConfig`]。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostingConfig {
    /// 腾讯云 COS（oss_type 4）。
    pub cos: CosConfig,
    /// B 站登录态（oss_type 5）。
    pub bilibili: BilibiliConfig,
    /// QQ频道（oss_type 6）。
    pub qq_channel: QqChannelConfig,
}

/// 图床上传后端。
///
/// 只有具体后端需要被调用，因此 trait 用 `async fn` 而不是装箱 future；需要 dyn 分派的
/// 通道是 [`HostingTransport`]。
#[allow(async_fn_in_trait, reason = "不做 dyn 分派；调用方持有具体后端类型")]
pub trait HostingBackend: Send + Sync {
    /// 上传 `data`，返回可直接访问的 URL。
    ///
    /// 入参非法、缺配置、传输失败、非预期状态码、响应里没有 URL 都会返回 [`Error`]；
    /// 不会静默返回空字符串。**单次尝试**：重试由 [`upload`] 负责。
    async fn upload(&self, data: &[u8], filename: &str) -> Result<String>;
}

/// 按 `oss_type` 上传，等价 Go `UploadProvider`。
///
/// 走默认重试策略（3 次尝试、1s/2s 退避，见 [`RetryPolicy::DEFAULT`]）。
pub async fn upload(
    kind: HostingKind,
    transport: Arc<dyn HostingTransport>,
    config: &HostingConfig,
    data: &[u8],
    filename: &str,
) -> Result<String> {
    upload_with_retry(
        kind,
        transport,
        config,
        RetryPolicy::DEFAULT,
        data,
        filename,
    )
    .await
}

/// 按 `oss_type` 上传，重试参数由调用方给（测试用零退避的策略）。
pub async fn upload_with_retry(
    kind: HostingKind,
    transport: Arc<dyn HostingTransport>,
    config: &HostingConfig,
    policy: RetryPolicy,
    data: &[u8],
    filename: &str,
) -> Result<String> {
    let attempts = policy.max_attempts.max(1);
    let mut round: u32 = 0;
    loop {
        if round > 0 {
            tokio::time::sleep(policy.delay_before(round)).await;
        }
        match dispatch(kind, Arc::clone(&transport), config, data, filename).await {
            Ok(url) => return Ok(url),
            Err(error) => {
                round += 1;
                if round >= attempts || !is_retryable(&error) {
                    return Err(error);
                }
            }
        }
    }
}

/// 把一种图床种类落到具体后端（单次尝试）。
async fn dispatch(
    kind: HostingKind,
    transport: Arc<dyn HostingTransport>,
    config: &HostingConfig,
    data: &[u8],
    filename: &str,
) -> Result<String> {
    match kind {
        HostingKind::Cos => {
            CosBackend::new(transport, config.cos.clone())
                .upload(data, filename)
                .await
        }
        HostingKind::Bilibili => {
            BilibiliBackend::new(transport, config.bilibili.clone())
                .upload(data, filename)
                .await
        }
        HostingKind::QqChannel => {
            QqChannelBackend::new(transport, config.qq_channel.clone())
                .upload(data, filename)
                .await
        }
        HostingKind::ChatGlm => ChatGlmBackend::new(transport).upload(data, filename).await,
        HostingKind::Ukaka => UkakaBackend::new(transport).upload(data, filename).await,
        HostingKind::Xingye => XingyeBackend::new(transport).upload(data, filename).await,
        HostingKind::Nature => NatureBackend::new(transport).upload(data, filename).await,
    }
}

/// `(Unix 秒, Unix 纳秒)`：COS 的对象 key / 签名时间与 Nature 的路径后缀都取自它。
pub(crate) fn unix_now() -> (i64, u64) {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => (elapsed.as_secs() as i64, elapsed.as_nanos() as u64),
        Err(_) => (0, 0),
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
    use std::time::Duration;

    use super::{
        BilibiliBackend, ChatGlmBackend, CosBackend, HostingConfig, HostingKind, NatureBackend,
        QqChannelBackend, RetryPolicy, UkakaBackend, XingyeBackend, bilibili, chatglm, cos,
        fixtures::FakeTransport, qq_channel, upload, upload_with_retry,
    };
    use crate::error::Error;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0DIHDR\x00\x00\x02\x80\x00\x00\x01\xE0\x08\x06\x00\x00\x00";
    const CHATGLM_OK: &str = r#"{"code":0,"result":{"file_url":"https://chatglm.cn/files/a.png"}}"#;

    fn transient_policy() -> RetryPolicy {
        RetryPolicy {
            max_attempts: 3,
            backoff: Duration::ZERO,
        }
    }

    fn configured() -> HostingConfig {
        HostingConfig {
            cos: cos::CosConfig {
                secret_id: "AKIDtest".to_owned(),
                secret_key: "secret-key".to_owned(),
                region: "ap-guangzhou".to_owned(),
                bucket: "bucket-125".to_owned(),
                domain: String::new(),
            },
            bilibili: bilibili::BilibiliConfig {
                csrf_token: "csrf".to_owned(),
                sessdata: "sess".to_owned(),
                bucket: String::new(),
            },
            qq_channel: qq_channel::QqChannelConfig {
                channel_id: "123456".to_owned(),
                token: "QQBot a.b".to_owned(),
            },
        }
    }

    #[test]
    fn maps_oss_type_range() {
        // 0 未启用，1~3 是内置 OSS 后端（阿里云 / 百度 / 腾讯云），都不是图床
        for disabled in [0, 1, 2, 3, -1, 11, 99] {
            assert_eq!(HostingKind::from_oss_type(disabled), None, "{disabled}");
        }
        let expected = [
            (4, HostingKind::Cos, "cos"),
            (5, HostingKind::Bilibili, "bilibili"),
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
        // 4~10 没有空洞：全枚举都实现了
        let covered: Vec<i64> = (4..=10).collect();
        assert_eq!(
            covered
                .iter()
                .filter_map(|oss_type| HostingKind::from_oss_type(*oss_type))
                .map(HostingKind::oss_type)
                .collect::<Vec<_>>(),
            covered
        );
    }

    #[tokio::test]
    async fn every_kind_reaches_its_own_endpoint() {
        // (种类, 预设响应, 期望的请求方法, 期望 URL 片段)
        let cases = [
            (
                HostingKind::Cos,
                vec![(200_u16, String::new())],
                super::UploadMethod::Put,
                "cos.ap-guangzhou.myqcloud.com",
            ),
            (
                HostingKind::Bilibili,
                vec![(
                    200,
                    r#"{"code":0,"data":{"location":"https://i0.hdslb.com/a.png"}}"#.to_owned(),
                )],
                super::UploadMethod::Post,
                bilibili::ENDPOINT,
            ),
            (
                HostingKind::QqChannel,
                vec![(200, "{}".to_owned())],
                super::UploadMethod::Post,
                "api.bot.qq.com/channels/123456/messages",
            ),
            (
                HostingKind::ChatGlm,
                vec![(200, CHATGLM_OK.to_owned())],
                super::UploadMethod::Post,
                chatglm::ENDPOINT,
            ),
            (
                HostingKind::Ukaka,
                vec![
                    (
                        200,
                        r#"{"url":"https://upload/ukaka","resourceUrl":"https://cdn/ukaka.png"}"#
                            .to_owned(),
                    ),
                    (204, String::new()),
                ],
                super::UploadMethod::Get,
                super::signed::SIGN_URL,
            ),
            (
                HostingKind::Xingye,
                vec![
                    (
                        200,
                        r#"{"url":"https://upload/xingye","resourceUrl":"https://cdn/xingye.png"}"#
                            .to_owned(),
                    ),
                    (200, String::new()),
                ],
                super::UploadMethod::Get,
                super::signed::SIGN_URL,
            ),
            (
                HostingKind::Nature,
                vec![(200, String::new())],
                super::UploadMethod::Put,
                "sgame-data-service-1252931805.cos.ap-nanjing.myqcloud.com",
            ),
        ];
        for (kind, replies, method, fragment) in cases {
            let mut transport = FakeTransport::new();
            for (status, body) in replies {
                transport = transport.respond(status, &body);
            }
            let transport = Arc::new(transport);
            let url = upload(kind, transport.clone(), &configured(), PNG, "a.png")
                .await
                .unwrap_or_else(|error| panic!("{kind:?} 上传失败: {error}"));
            assert!(url.starts_with("https://"), "{kind:?}: {url}");

            let calls = transport.calls();
            assert!(!calls.is_empty(), "{kind:?} 没有发出请求");
            assert_eq!(calls[0].method, method, "{kind:?}");
            assert!(
                calls[0].url.contains(fragment),
                "{kind:?}: {} 不含 {fragment}",
                calls[0].url
            );
        }
    }

    #[tokio::test]
    async fn retries_transient_failures_then_succeeds() {
        let transport = Arc::new(
            FakeTransport::new()
                .fail("连接被拒绝")
                .fail("连接被拒绝")
                .respond(200, CHATGLM_OK),
        );
        let url = upload_with_retry(
            HostingKind::ChatGlm,
            transport.clone(),
            &HostingConfig::default(),
            transient_policy(),
            PNG,
            "a.png",
        )
        .await
        .expect("第三次成功");
        assert_eq!(url, "https://chatglm.cn/files/a.png");
        assert_eq!(transport.calls().len(), 3, "前两次失败、第三次成功");
    }

    #[tokio::test]
    async fn retries_server_errors_but_gives_up_after_three() {
        let transport = Arc::new(
            FakeTransport::new()
                .respond(500, "boom")
                .respond(503, "busy")
                .respond(502, "bad gateway"),
        );
        let error = upload_with_retry(
            HostingKind::ChatGlm,
            transport.clone(),
            &HostingConfig::default(),
            transient_policy(),
            PNG,
            "a.png",
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, Error::HttpStatus(502)),
            "回报最后一次错误: {error}"
        );
        assert_eq!(transport.calls().len(), 3, "最多 3 次尝试");
    }

    #[tokio::test]
    async fn does_not_retry_client_errors() {
        for status in [400, 401, 403, 404] {
            let transport = Arc::new(FakeTransport::new().respond(status, "denied"));
            let error = upload_with_retry(
                HostingKind::ChatGlm,
                transport.clone(),
                &HostingConfig::default(),
                transient_policy(),
                PNG,
                "a.png",
            )
            .await
            .unwrap_err();
            assert!(matches!(error, Error::HttpStatus(code) if code == status));
            assert_eq!(transport.calls().len(), 1, "{status} 不应重试");
        }
    }

    #[tokio::test]
    async fn does_not_retry_payload_or_configuration_errors() {
        // 200 但响应不可用：重发也不会变好
        let broken = Arc::new(FakeTransport::new().respond(200, "<html>"));
        let error = upload_with_retry(
            HostingKind::ChatGlm,
            broken.clone(),
            &HostingConfig::default(),
            transient_policy(),
            PNG,
            "a.png",
        )
        .await
        .unwrap_err();
        assert!(matches!(error, Error::HostingResponse(_)), "{error}");
        assert_eq!(broken.calls().len(), 1);

        // 缺配置：一个请求都不发
        let quiet = Arc::new(FakeTransport::new());
        let error = upload(
            HostingKind::Cos,
            quiet.clone(),
            &HostingConfig::default(),
            PNG,
            "a.png",
        )
        .await
        .unwrap_err();
        assert!(matches!(error, Error::HostingConfig(_)), "{error}");
        assert!(quiet.calls().is_empty());
    }

    #[test]
    fn backend_types_are_debug_and_constructible() {
        let transport = Arc::new(FakeTransport::new());
        let backends: Vec<(&str, String)> = vec![
            (
                "CosBackend",
                format!(
                    "{:?}",
                    CosBackend::new(transport.clone(), cos::CosConfig::default())
                ),
            ),
            (
                "BilibiliBackend",
                format!(
                    "{:?}",
                    BilibiliBackend::new(transport.clone(), bilibili::BilibiliConfig::default())
                ),
            ),
            (
                "QqChannelBackend",
                format!(
                    "{:?}",
                    QqChannelBackend::new(
                        transport.clone(),
                        qq_channel::QqChannelConfig::default()
                    )
                ),
            ),
            (
                "ChatGlmBackend",
                format!("{:?}", ChatGlmBackend::new(transport.clone())),
            ),
            (
                "UkakaBackend",
                format!("{:?}", UkakaBackend::new(transport.clone())),
            ),
            (
                "XingyeBackend",
                format!("{:?}", XingyeBackend::new(transport.clone())),
            ),
            (
                "NatureBackend",
                format!("{:?}", NatureBackend::new(transport)),
            ),
        ];
        assert_eq!(backends.len(), 7, "4~10 每档一个后端");
        for (name, debug) in backends {
            assert!(debug.contains(name), "{debug}");
        }
    }
}
