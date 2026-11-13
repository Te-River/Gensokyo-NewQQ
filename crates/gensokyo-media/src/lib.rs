//! gensokyo-media：下载上传、压缩、图床、silk 编解码、QR。
//!
//! # 覆盖范围
//!
//! ## M2a：媒体基础能力
//! - [`source::MediaSource`]：OneBot `file` 字段 → `Url` / `File` / `Base64`。支持
//!   `base64://`、`data:` URI、`http(s)://`、`file://`（含 `file:///C:/...`）与裸 base64。
//! - [`fetch::Fetcher`]：异步取回并落盘。URL 与 base64 统一写临时文件，`file://` 复用原路径；
//!   默认上限 20MB（`0` 表示不限制）、总超时 30s，均可配置；写盘失败时清理半成品。
//! - [`fetch::MediaTransport`]：唯一下载出口。生产实现 [`fetch::HttpTransport`]
//!   （`reqwest` + `rustls-no-provider`，provider 由调用方注入），测试用 Fake 断言
//!   URL / 超时 / 大小上限，全程不需要外网。
//! - [`sniff`]：PNG / JPEG / GIF / WebP / BMP 文件头识别，不解码像素。
//! - [`data_uri`]：`data:image/png;base64,...` 解析，以及生成 CQ 码用的 `base64://`；
//!   同时提供 [`data_uri::encode_base64`] / [`data_uri::decode_base64`] 两个通用入口。
//!
//! ## M2b：图片尺寸与图床抽象
//! - [`image::image_size`]：从文件头读出 `(宽, 高)`，覆盖 PNG / JPEG（SOF0、SOF2 及同类
//!   SOFn）/ GIF / WebP（VP8、VP8L、VP8X）/ BMP，不做像素解码。
//! - [`hosting::HostingKind`]：`oss_type` 4~10 的图床枚举与
//!   [`hosting::HostingKind::from_oss_type`]（0 = 本地上传，1~3 = 内置云 OSS（见 [`oss`]），均返回 `None`）。
//! - [`oss::OssKind`]：`oss_type` 1~3 的内置云 OSS（腾讯云 COS / 百度云 BOS / 阿里云 OSS），
//!   [`oss::upload`] 单次尝试直传（Go `oss` 包同口径，无重试），对象 key 是 `md5(字节).jpg`，
//!   签名与请求头逐字段照 Go；审核开关（`t_audit` / `b_audit` / `a_audit`）打开时先传后审，
//!   不合规或审核请求不可用返回 [`Error::AuditRejected`] / [`Error::HostingResponse`]，
//!   由调用方保留原引用。
//! - [`hosting::HostingBackend`]：`async fn upload(&self, data, filename) -> Result<String>`；
//!   oss_type 4~10 全部落地（COS / Bilibili / QQ频道 / ChatGLM / Ukaka / 星野 / Nature），
//!   端点、表单字段、签名与成功判定都照 Go `imagehosting/`。
//! - [`hosting::upload`]：按种类分派并重试（3 次、1s/2s 退避；只重试网络 / 超时 / 5xx），
//!   [`hosting::upload_with_retry`] 由调用方自定 [`hosting::RetryPolicy`]。
//! - [`hosting::HostingConfig`]：oss_type 4/5/6 需要的凭据与目标桶（Go 从全局 config 读，
//!   这里由接入层注入）。
//! - [`hosting::HostingTransport`]：上传递出口（POST / PUT / GET），生产实现复用
//!   [`fetch::HttpTransport`] 的 `reqwest::Client`，测试用 Fake 断言 multipart。
//!
//! ## P25：短签名外链直链解析
//! - [`redirect::resolve_redirect`]：把 ChatGLM 等短签名外链的 302 解析为末跳直链
//!   （只读响应头、手动逐跳跟随、**每跳** SSRF 校验、最多 5 跳 / 整链 5s）。
//! - [`redirect::is_short_lived_image_url`]：`chatglm.cn` 及其子域的边界判定（Go
//!   `IsShortLivedImageURL`）。
//! - [`hosting::ChatGlmBackend`] 上传成功后自动解析一次；解析失败保留原短链并告警，
//!   不阻断发送。
//!
//! ## M2c：图片压缩
//! - [`compress::compress_image`]：解码 JPEG / PNG / WebP → 长边超过
//!   [`compress::DEFAULT_MAX_SIDE`]（1024）时等比缩放 → 按质量阶梯编码；有体积目标时取**第一个**
//!   不超过目标的结果，都不满足取体积最小的那个。不支持格式 / 解码失败 / 编码失败一律返回 [`Error`]。
//! - [`compress::CompressOptions`]：`max_side`（0 = 不缩放）、`quality_ladder`
//!   （默认 [`compress::GO_QUALITY_LADDER`] = 75/65/55/45/35/25，对齐 Go 的 max 75 / step 10 /
//!   min 25）、`format`（`None` 沿用源格式）、`target_bytes`（`None` = 不设体积目标）。
//! - 透明通道：PNG 输入默认输出 PNG 并保留 alpha；显式 [`compress::ImageFormat::Jpeg`] 时按 Go
//!   `compressPNG` 的做法压到白底。
//!
//! ## M2d：silk 音频编解码（外部进程版）
//! - [`silk::SilkCodec`]：`encode_to_silk`（ffmpeg → 16-bit 单声道 PCM → `pts` → Silk v3）、
//!   `encode_bytes_to_silk`（同一链路的字节级入口，供出站媒体链在内存里转码）与
//!   `decode_silk`（`pts` → PCM → ffmpeg 转目标容器），参数顺序对齐 Go `silk/silk.go`。
//! - [`silk::is_amr_or_silk`] / [`silk::check_audio`]：Go `IsAMRorSILK` / `CheckAudio` 的
//!   等价判定（转码前短路与非音频初筛）。
//! - [`silk::ProcessRunner`]：唯一的进程出口。生产实现 [`silk::CommandRunner`]
//!   （`std::process::Command`），测试用假执行器断言程序名 / 参数 / 错误映射。
//! - 程序路径可配置：`GENSOKYO_FFMPEG` / `GENSOKYO_PTS`，缺省 `ffmpeg` / `pts`（走 PATH）。
//!   找不到程序返回 [`Error::SilkUnavailable`]，非零退出 / 产物缺失返回 [`Error::SilkCodec`]。
//! - 临时 PCM 落在 `<os temp>/gensokyo-silk/`，成功与失败都由 `Drop` 清理。
//!
//! ## P56-T2：QR 码生成
//! - [`qr::qr_png_base64`]：URL → QR PNG base64（容错级 Q、无静区），Go `generateQRCode`
//!   （`handlers/qrcode_real.go`）的等价物；`size` 语义照 go-qrcode `PNG(size)`：
//!   负数 = 每模块像素数、过小静默放大到模块数、否则恰好 size×size。
//!
//! # P56：下载路径 SSRF 守卫
//! - [`fetch::HttpTransport`] 的下载出口关闭 `reqwest` 自动重定向，**手动逐跳跟随**：
//!   每跳请求前（含首跳）复用 [`redirect`] 的逐跳判定——协议白名单 http(s)、IP 字面量直接判、
//!   域名先解析再逐个 IP 判（任一命中即拒、解析失败也拒）。IP 组合复用
//!   `gensokyo_core::net`（RFC1918 + 回环 / 链路本地 / 未指定 / ULA，IPv4-mapped 先归一），
//!   守卫只有这一处实现。
//! - 重定向跳数上限 5 跳（与 [`redirect::resolve_redirect`] 共用），整链总超时按请求的
//!   `timeout`（默认 30s）；大小上限 / 只认 200 / 落盘语义不变。
//! - 出站 URL 入口（image / record / video / file / QQ 音乐）都经 `Fetcher` → `HttpTransport`，
//!   守卫在发起下载前生效；hosting 上传出口是另一个客户端，语义不变。
//!
//! # 与 Go 的差异（P56）
//! - **跳数口径**：Go 新链 `Fetcher` 默认就是 5 跳（`internal/application/media/fetcher.go:37-38`），
//!   与这里一致；10 跳只出现在 legacy 裸 `client.Get`（`handlers/reply_helpers.go:133-135`，
//!   无 CheckRedirect）。这里所有下载路径统一 5 跳。
//! - **逐跳复检收紧**：Go `handlers/reply_helpers.go:124-138` 的裸 `client.Get` 无逐跳复检；
//!   这里所有下载路径一律逐跳复检（有意安全差异，lead 裁决）。
//! - **拦截面扩大**：在 Go 口径（RFC1918 + 回环 + 链路本地 + ULA，`IsPrivate` 本就含 RFC4193，
//!   `fetcher.go:210` 另有 fc/fd 前缀判）之外另拦 `0.0.0.0` / `::`；反向未拦 Go
//!   `fetcher.go:203` 的链路本地组播（ff02::/16、224.0.0.0/24）——TCP 对组播建不起连接，
//!   实际影响≈0，登记备查。
//! - **无 DNS pinning**：建连时 `reqwest` 会再解析一次 DNS，存在 rebinding 窗口
//!   （与 Go 一致的已知边界）。
//! - **不建 FetchBytes 等价物**：Go `imagehosting/selffetch.go` 的字节自取在 Rust 无消费方
//!   （hosting 上传期已解析直链，`hosting/chatglm.rs`；发送期 URL 媒体一律走守卫 `Fetcher`）。
//! - **删除 `HttpTransport::from_client`**：旧 API 允许注入外部 client，无法保证关闭自动重定向，
//!   是逐跳守卫的绕过缝；全树零调用方，已删。
//!
//! # 与 Go 的差异（M2a）
//! - **SSRF / 重定向 / 私网拦截（下载路径）**：P56 已落地（见上节「P56」）；短签名外链
//!   直链解析（[`redirect`]）与下载路径共用同一套逐跳判定。
//! - **落盘策略**：Go `FetchToTempFile` 在 `threshold` 以下走内存、以上才写临时文件并返回
//!   `path=""`；本 crate 不做阈值分流：URL 与内联 base64 一律写临时文件并返回路径，用
//!   [`Fetched::bytes`] 表达字节数（不把内容复制进返回值），`file://` 直接复用原路径、不复制。
//!   Go 的 `threshold` 语义因此不适用。
//! - **临时文件删除责任**：成功返回后临时文件归**调用方**所有，本 crate 不注册清理、
//!   也不在 [`Fetched`] 上实现 `Drop`；只有写盘过程中失败才会删除半成品（清理失败会
//!   `tracing::warn!` 留下线索）。调用方用完需自行删除，否则会堆积在系统临时目录。
//! - **Content-Type**：优先按文件头嗅探 512 字节（对齐 Go `http.DetectContentType`）；
//!   只有嗅探结果为 `application/octet-stream` 时才采信响应头 / `data:` URI 声明，
//!   仍缺省则兜底 `application/octet-stream`。与 Go 的差别仅在 `data:` URI 声明可以
//!   在嗅探失败时补位（Go 只覆盖响应头，不认 `data:` 声明）。
//! - **GIF 判定**：Go 只查 3 字节 `GIF` 前缀；本 crate 收紧到 `GIF87a` / `GIF89a`。
//! - **状态码**：与 Go 一致，只接受 200（不把 206 / 3xx 当成功）。
//! - **大小上限**：`max_bytes = 0` 表示不限制，与 Go 只在 `> 0` 时限流一致。
//! - **base64 实现**：用 workspace 的 `base64` crate（`GeneralPurpose` 引擎），配置为
//!   标准字母表 + 填充模式 `Indifferent` + 允许尾随位，解码前剥离 ASCII 空白。与 Go
//!   `base64.StdEncoding` 的差异只有这些放宽项（Go 要求严填充、不忽略空白）：OneBot
//!   客户端常省略填充或把长负载换行，严格模式会误判为非法。
//! - **`file://` 安全化**：百分号解码后拒绝含 `..` 的路径并转绝对路径，对齐 Go
//!   `handlers/message_parser.go` 的 `safeLocalPath`（裸路径不做绝对化，保持调用方原样）。
//! - **裸 base64 判定**：只有解码后能嗅出媒体头，或长度 ≥32 且含 `=` / `+` 时才当内联
//!   base64（避免把 `20240101` 这类文件名当数据）。
//! - **错误**：统一 [`Error`]，不再有 Go 那样散落的 `fmt.Errorf` 字符串。
//!
//! # 与 Go 的差异（M2b）
//! 图床侧的差异（重试的错误集合、URL 校验、状态码收紧、文件部件 MIME、文件名清洗）集中在
//! [`hosting`] 的模块文档里；尺寸探测的差异（JPEG 严格标记、Go 宽高写反、WebP/BMP 补齐）在
//! [`image`] 里。
//!
//! # 与 Go 的差异（M2c）
//! 细节在 [`compress`] 的模块文档里，要点：
//! - **尺寸缩放是新增能力**：Go `images/Compress.go` / `easycompress.go` 只按质量阶梯重编码、
//!   从不缩放；本 crate 先等比缩到 `max_side` 再进阶梯（`max_side = 0` 回到 Go 行为）。
//! - **PNG 默认保留透明**：Go `compressPNG` 一律转白底 JPEG；本 crate 只在显式要求 JPEG 时才压白底。
//! - **没有「阈值 0 原样返回」短路**：Go `image_sizelimit == 0` 直接返回入参；本 crate 由调用方
//!   决定（`target_bytes = None` = 不设体积目标，仍会缩放 + 按首档编码）。
//! - **GIF 不在范围**：Go 会逐帧重编码 GIF，M2c 只解码 JPEG / PNG / WebP（GIF / BMP 报
//!   [`Error::UnsupportedImageFormat`]）。
//! - **编码产物不逐字节对齐**：Go 用标准库默认参数，这里用 `image` crate 默认参数，像素与体积量级
//!   接近但不保证相等。
//!
//! # 与 Go 的差异（M2d）
//! 细节在 [`silk`] 的模块文档里，要点：
//! - **不嵌入二进制**：Go `go:embed exec/*` 带 8 个平台的 `silk_codec-*`；本 crate 只调用外部
//!   `ffmpeg` / `pts`（体积与平台矩阵留 M4），部署环境必须自带这两个工具。
//! - **没有内建快速路径**：Go 的 WAV 直提、MP3 解码、线性重采样、立体声下混全部交给 ffmpeg
//!   （`-ar` / `-ac 1`）。
//! - **错误不再静默**：Go 的工具缺失 / 非零退出只打日志并返回 `nil`；这里返回
//!   [`Error::SilkUnavailable`] / [`Error::SilkCodec`]。
//! - **产物位置**：Go 写到临时目录再读回内存；这里 `pts` 直接写调用方给的输出路径。
//!
//! # 与 Go 的差异（P56-T2 QR）
//! - **容错级命名陷阱**：Go `qrcode.High` 常量是 ISO **Q** 级（25%），go-qrcode 的
//!   `Highest` 才是 ISO H 级（version.go:19-31）；本 crate 用 `EcLevel::Q` 对齐 Go 实际行为。
//! - **矩阵级 parity**：模块矩阵与 go-qrcode（主仓 go.mod 同版本）金样 8/9 逐模块一致
//!   （`tests/qr_golden.json`，9 URL × 5 尺寸），余 1 条为版本分歧（见下条）；PNG 字节不
//!   跨库逐字节对齐——Go 是 2 色调色板 + BestCompression + 逐像素 floor 映射，这里是
//!   Luma + image 默认压缩 + Nearest 重采样（lead 裁决接受，与 M2c「编码产物不逐字节对齐」同口径）。
//! - **长混合内容版本分歧**：长混合字母数字内容两库分段优化器不同，可能选不同 QR 版本
//!   （模块数不同，如 529 字符混合样 Go v22 / Rust v24）；**解码载荷相同**，由 rqrr
//!   解码测试验证（lead 裁决 (a)）；同 URL / 同容错级 / 同尺寸语义的矩阵级 parity 口径不变。
//! - **空 URL 不是错误**：两库都接受（version 1）；超出 version 40 Q 级容量才报
//!   [`Error::QrEncode`]。
//!
//! # 依赖边界
//! 只用 workspace 已锁定并已在本 crate 依赖图里的 crate（`reqwest` / `tokio` / `base64` /
//! `thiserror` / `rustls` / `tracing`）。M2c 起新增 `image`（workspace 早已声明，此前未入图）——
//! 这是 M2c 唯一的新依赖边。M2d 不新增依赖：silk 走 `std::process` + `std::env`。P7 的图床
//! 签名同样不新增依赖：COS 的 HMAC-SHA1 与 QQ频道的 MD5 在 [`hosting`] 内按 RFC 实现。
//! P56-T2 新增 `qrcode`（workspace 已声明；crate 级补隐式 `image` feature——workspace 行的
//! `pic` 是空 feature，不出 image 渲染）：其对 image 的依赖是 default-features=false 的 0.25，
//! 与 workspace 统一为单版本 0.25.10（cargo tree 核实）。`rqrr` 仅 dev-dep（解码等价验证），
//! image 依赖同样统一到 0.25.10。
//!
//! # 未做项（留给 M2e+）
//! - silk 编解码器的**内嵌二进制**（`silk/exec/` 的 8 个平台产物）：M2d 只调用外部 `ffmpeg` /
//!   `pts`，体积与平台矩阵留到 M4。
//! - GIF 逐帧压缩（Go `compressGIF`），以及 BMP 解码。
//! - Bilibili 的分片 / 大文件上传（Go 也只有单请求 `imagehosting/bilibili.go`）。
//! - 视频处理（抽帧 / 转码 / 体积压缩）。
//! - 上传到 QQ（`/v2/.../files` 的 `file_data` / 分片上传）——由 `gensokyo-qq` + actions 负责。

pub mod compress;
pub mod data_uri;
pub mod error;
pub mod fetch;
pub mod hosting;
pub mod image;
pub mod oss;
pub mod qr;
pub mod redirect;
pub mod silk;
pub mod sniff;
pub mod source;

pub use compress::{
    CompressOptions, DEFAULT_MAX_SIDE, GO_QUALITY_LADDER, ImageFormat, compress_image,
};
pub use data_uri::{DataUri, cq_base64, decode_base64, encode_base64, to_data_uri};
pub use error::{Error, Result};
pub use fetch::{
    BoxFuture, DEFAULT_MAX_BYTES, DEFAULT_TIMEOUT, FetchRequest, Fetched, Fetcher, HttpTransport,
    MediaTransport, TransportResponse,
};
pub use hosting::{
    BilibiliBackend, BilibiliConfig, BilibiliPlan, ChatGlmBackend, ChatGlmPlan, CosBackend,
    CosConfig, CosPlan, HostingBackend, HostingConfig, HostingKind, HostingTransport, MultipartBody,
    NatureBackend, NaturePlan, QqChannelBackend, QqChannelConfig, QqChannelPlan, RetryPolicy,
    SignedPlan, UkakaBackend, UploadBody, UploadMethod, UploadRequest, UploadResponse, XingyeBackend,
    is_retryable, upload, upload_with_retry,
};
pub use image::image_size;
pub use oss::{
    AliyunOssConfig, BaiduBosConfig, OssConfig, OssKind, TencentCosConfig, upload as oss_upload,
};
pub use qr::qr_png_base64;
pub use redirect::{
    LazyHttpProbe, ProbeResponse, RedirectProbe, is_short_lived_image_url, resolve_redirect,
};
pub use silk::{CommandRunner, DEFAULT_SAMPLE_RATE, ProcessOutput, ProcessRunner, SilkCodec};
pub use sniff::{ImageKind, sniff_content_type, sniff_image};
pub use source::MediaSource;
