//! 上传接线：`oss_type` 1~3（内置云 OSS）与 4~10（图床）的本地图片 → 公网 URL。
//!
//! # 边界
//!
//! - **凭据映射在这里**（[`config_from_settings`]）：`gensokyo-media` 的图床后端由调用方
//!   传 [`HostingConfig`]，不依赖 `gensokyo-config`；映射留在 runtime，依赖方向是
//!   `config → runtime → media`。
//! - **传输出口**与 [`crate::media::MediaRuntime`] 同款惰性：首次上传才建 `reqwest::Client`
//!   （[`gensokyo_media::HttpTransport`]），建不出来只让这一次上传失败，不影响启动。
//! - **进程级共享**（[`shared`]）：`AppState::dispatcher` 是逐请求构造的，客户端不能跟着
//!   Dispatcher 重建。
//!
//! # 与 Go 的差异
//!
//! 重试只覆盖网络 / 超时 / 5xx（`gensokyo_media::hosting::retry` 的口径），Go 对任何错误都会
//! 重打三次；缺凭据时后端在**发请求前**返回 `HostingConfig`，调用方据此保留原引用。

use std::sync::{Arc, LazyLock, OnceLock};

use gensokyo_config::Settings;
use gensokyo_media::{
    AliyunOssConfig, BaiduBosConfig, BilibiliConfig, BoxFuture, CosConfig, HostingConfig,
    HostingKind, HostingTransport, HttpTransport, OssConfig, OssKind, QqChannelConfig,
    TencentCosConfig, UploadRequest, UploadResponse, oss_upload, upload,
};

/// 图床上传运行时：持有一个可注入的传输出口。
#[derive(Debug)]
pub struct HostingRuntime {
    transport: Arc<dyn HostingTransport>,
}

impl Default for HostingRuntime {
    /// 生产默认：客户端延迟到第一次上传才建（rustls provider 由二进制启动时安装，
    /// `gensokyo_bin::startup::install_crypto_provider`；未安装时错误留在单次上传上）。
    fn default() -> Self {
        Self::with_transport(Arc::new(LazyHostingTransport::default()))
    }
}

impl HostingRuntime {
    /// 用给定传输创建（单测注入假传输，全程不碰网络）。
    #[must_use]
    pub fn with_transport(transport: Arc<dyn HostingTransport>) -> Self {
        Self { transport }
    }

    /// 按 `oss_type` 上传一份字节，返回可直接写进 Markdown 的 URL。
    ///
    /// 重试用 [`gensokyo_media::upload`] 的默认策略（3 次尝试、1s/2s 退避，只重试可恢复错误）。
    ///
    /// # Errors
    /// 缺凭据（[`gensokyo_media::Error::HostingConfig`]，不发请求）、传输 / 超时、非预期状态码、
    /// 响应里没有 URL——由调用方决定保留原引用。
    pub async fn upload(
        &self,
        kind: HostingKind,
        config: &HostingConfig,
        data: &[u8],
        filename: &str,
    ) -> gensokyo_media::Result<String> {
        upload(kind, Arc::clone(&self.transport), config, data, filename).await
    }

    /// 按 `oss_type` 1~3 上传一份图片字节，返回可直接写进 Markdown 的 URL。
    ///
    /// 与图床不同，内置云 OSS **不重试**（Go `oss` 包同口径）：失败原样上抛，调用方保留原引用。
    ///
    /// # Errors
    /// 缺凭据 / 审核开关打开（[`gensokyo_media::Error::HostingConfig`]，不发请求）、传输失败、
    /// 非 2xx 状态码——由调用方决定保留原引用。
    pub async fn upload_oss(
        &self,
        kind: OssKind,
        config: &OssConfig,
        data: &[u8],
    ) -> gensokyo_media::Result<String> {
        oss_upload(kind, Arc::clone(&self.transport), config, data).await
    }
}

/// 进程级共享运行时。
///
/// `AppState::dispatcher` 逐请求构造 Dispatcher，客户端若挂在 Dispatcher 上会跟着重建；
/// 这里用一个 `LazyLock` 把「一个进程一个客户端」固定下来。
pub fn shared() -> &'static HostingRuntime {
    static RUNTIME: LazyLock<HostingRuntime> = LazyLock::new(HostingRuntime::default);
    &RUNTIME
}

/// `Settings` 快照 → 各图床凭据。
///
/// 4/5/6 的必填项由对应后端在发请求前校验（缺字段返回
/// [`gensokyo_media::Error::HostingConfig`]）；7/8/9/10 免配置，这里不填任何东西。
#[must_use]
pub fn config_from_settings(settings: &Settings) -> HostingConfig {
    let cos = settings.get_cos();
    let bilibili = settings.get_bilibili();
    let qq_channel = settings.get_qq_channel();
    HostingConfig {
        cos: CosConfig {
            secret_id: cos.secret_id.clone(),
            secret_key: cos.secret_key.clone(),
            region: cos.region.clone(),
            bucket: cos.bucket.clone(),
            domain: cos.domain.clone(),
        },
        bilibili: BilibiliConfig {
            csrf_token: bilibili.csrf_token.clone(),
            sessdata: bilibili.sessdata.clone(),
            bucket: bilibili.bucket.clone(),
        },
        qq_channel: QqChannelConfig {
            channel_id: qq_channel.channel_id.clone(),
            token: qq_channel.token.clone(),
        },
    }
}

/// `Settings` 快照 → 内置云 OSS（`oss_type` 1~3）三后端的凭据与目标桶。
///
/// 字段一一对应 Go 的 `t_COS_*` / `b_BCE_*` + `b_BOS_BUCKETNAME` / `a_OSS_*`；百度云的对象桶名
/// 就是 AppID（Go `config.GetAppIDStr()`），这里用 `app_id` 拼出来。缺字段的校验留给各后端
/// 在发请求前做（返回 `HostingConfig`，调用方保留原引用）。
#[must_use]
pub fn oss_config_from_settings(settings: &Settings) -> OssConfig {
    OssConfig {
        tencent: TencentCosConfig {
            bucket: settings.get_tencent_bucket_name().to_owned(),
            region: settings.get_tencent_bucket_region().to_owned(),
            secret_id: settings.get_tencent_cos_secretid().to_owned(),
            secret_key: settings.get_tencent_secret_key().to_owned(),
            audit: settings.get_tencent_audit(),
        },
        baidu: BaiduBosConfig {
            domain: settings.get_baidu_bos_bucket_name().to_owned(),
            access_key: settings.get_baidu_bce_ak().to_owned(),
            secret_key: settings.get_baidu_bce_sk().to_owned(),
            bucket: settings.get_app_id().to_string(),
            audit: settings.get_baidu_audit(),
        },
        aliyun: AliyunOssConfig {
            endpoint: settings.get_aliyun_endpoint().to_owned(),
            bucket: settings.get_aliyun_bucket_name().to_owned(),
            access_key_id: settings.get_aliyun_access_key_id().to_owned(),
            access_key_secret: settings.get_aliyun_access_key_secret().to_owned(),
            audit: settings.get_aliyun_audit(),
        },
    }
}

/// 延迟构造的 HTTP 上传递出（`OnceLock`：第一次上传才建客户端）。
#[derive(Debug, Default)]
struct LazyHostingTransport {
    inner: OnceLock<std::result::Result<HttpTransport, String>>,
}

impl HostingTransport for LazyHostingTransport {
    fn send(
        &self,
        request: UploadRequest,
    ) -> BoxFuture<'_, gensokyo_media::Result<UploadResponse>> {
        match self
            .inner
            .get_or_init(|| HttpTransport::new().map_err(|err| err.to_string()))
        {
            Ok(transport) => transport.send(request),
            Err(message) => {
                let message = message.clone();
                Box::pin(async move { Err(gensokyo_media::Error::Transport(message)) })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use gensokyo_config::parse_str;

    use super::{config_from_settings, oss_config_from_settings};

    #[test]
    fn maps_every_provider_credential_from_settings() {
        let settings = parse_str(
            "version: 1\nsettings:\n  oss_type: 4\n  cos:\n    secret_id: sid\n    secret_key: skey\n    region: ap-shanghai\n    bucket: b1\n    domain: cdn.example\n  bilibili:\n    csrf_token: csrf\n    sessdata: sess\n    bucket: b2\n  qq_channel:\n    channel_id: ch\n    token: QQBot t\n",
        )
        .expect("配置解析失败");

        let config = config_from_settings(&settings);
        assert_eq!(config.cos.secret_id, "sid");
        assert_eq!(config.cos.secret_key, "skey");
        assert_eq!(config.cos.region, "ap-shanghai");
        assert_eq!(config.cos.bucket, "b1");
        assert_eq!(config.cos.domain, "cdn.example");
        assert_eq!(config.bilibili.csrf_token, "csrf");
        assert_eq!(config.bilibili.sessdata, "sess");
        assert_eq!(config.bilibili.bucket, "b2");
        assert_eq!(config.qq_channel.channel_id, "ch");
        assert_eq!(config.qq_channel.token, "QQBot t");
    }

    #[test]
    fn missing_section_yields_empty_credentials() {
        let settings = parse_str("version: 1\nsettings:\n  oss_type: 5\n").expect("配置解析失败");
        let config = config_from_settings(&settings);
        assert!(config.bilibili.sessdata.is_empty());
        assert!(config.cos.secret_id.is_empty());
        assert!(config.qq_channel.channel_id.is_empty());
    }

    #[test]
    fn maps_builtin_oss_credentials_from_settings() {
        let settings = parse_str(
            "version: 1\nsettings:\n  app_id: 10001\n  oss_type: 2\n  t_COS_BUCKETNAME: t-bucket\n  t_COS_REGION: ap-shanghai\n  t_COS_SECRETID: t-id\n  t_COS_SECRETKEY: t-key\n  t_audit: false\n  b_BOS_BUCKETNAME: bj.bcebos.com\n  b_BCE_AK: b-ak\n  b_BCE_SK: b-sk\n  b_audit: 0\n  a_OSS_EndPoint: oss-cn-hangzhou.aliyuncs.com\n  a_OSS_BucketName: a-bucket\n  a_OSS_AccessKeyId: a-id\n  a_OSS_AccessKeySecret: a-secret\n  a_audit: false\n",
        )
        .expect("配置解析失败");

        let config = oss_config_from_settings(&settings);
        assert_eq!(config.tencent.bucket, "t-bucket");
        assert_eq!(config.tencent.region, "ap-shanghai");
        assert_eq!(config.tencent.secret_id, "t-id");
        assert_eq!(config.tencent.secret_key, "t-key");
        assert!(!config.tencent.audit);

        assert_eq!(config.baidu.domain, "bj.bcebos.com");
        assert_eq!(config.baidu.access_key, "b-ak");
        assert_eq!(config.baidu.secret_key, "b-sk");
        assert_eq!(config.baidu.bucket, "10001", "对象桶名就是 AppID");
        assert_eq!(config.baidu.audit, 0);

        assert_eq!(config.aliyun.endpoint, "oss-cn-hangzhou.aliyuncs.com");
        assert_eq!(config.aliyun.bucket, "a-bucket");
        assert_eq!(config.aliyun.access_key_id, "a-id");
        assert_eq!(config.aliyun.access_key_secret, "a-secret");
        assert!(!config.aliyun.audit);
    }
}
