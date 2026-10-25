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
//!   [`hosting::HostingKind::from_oss_type`]（0 = 未启用，1~3 = 内置 OSS，均返回 `None`）。
//! - [`hosting::HostingBackend`]：`async fn upload(&self, data, filename) -> Result<String>`；
//!   已落地[`hosting::ChatGlmBackend`]（oss_type 7）与 [`hosting::UkakaBackend`]（oss_type 8），
//!   URL / 表单字段 / 成功判定都照 Go `imagehosting/`。
//! - [`hosting::upload_plan`]：纯函数，把「后端 + 文件名 + 字节」变成可断言的
//!   [`hosting::UploadPlan`]；[`hosting::upload`] 按种类分派到对应后端。
//! - [`hosting::HostingTransport`]：上传递出口（POST / PUT / GET），生产实现复用
//!   [`fetch::HttpTransport`] 的 `reqwest::Client`，测试用 Fake 断言 multipart。
//!
//! # 与 Go 的差异（M2a）
//! - **SSRF / 重定向 / 私网拦截**：Go `internal/application/media/fetcher.go` 有
//!   `checkSSRF`（DNS 解析 + 私网/回环拒绝）与 `MaxRedirects`。本 crate **不做**这些，
//!   `reqwest` 默认跟随最多 10 次重定向；SSRF 属于接入层的策略，M2a 不落地（见未做项）。
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
//! 图床侧的差异（不重试、URL 校验、文件部件 MIME、文件名清洗）集中在 [`hosting`] 的模块
//! 文档里；尺寸探测的差异（JPEG 严格标记、Go 宽高写反、WebP/BMP 补齐）在 [`image`] 里。
//!
//! # 依赖边界
//! 只用 workspace 已锁定并已在本 crate 依赖图里的 crate（`reqwest` / `tokio` / `base64` /
//! `thiserror` / `rustls` / `tracing`）。M2b **不新增任何依赖边**：图床响应用
//! [`hosting`] 内部的极简 JSON 读取器，multipart body 手写，避免 `Cargo.lock` 变动。
//!
//! # 未做项（留给 M2c+）
//! - 图片压缩（`images/Compress.go` 的 JPEG 质量阶梯、GIF 处理）——需要 `image` crate。
//! - COS 自签上传（`imagehosting/cos.go` 的 q-sign）与 Nature 的内置 COS 密钥直传。
//! - Bilibili 登录态上传与分片（`imagehosting/bilibili.go`）。
//! - 星野的 PUT 直传（`imagehosting/signed.go` 的 xingye 分支）与 QQ频道（oss_type 6）上传。
//! - silk 音频编解码（`silk/` + `go-silk/` 的 C ABI 封装）。
//! - 上传到 QQ（`/v2/.../files` 的 `file_data` / 分片上传）——由 `gensokyo-qq` + actions 负责。
//! - QR 码生成。
//! - SSRF 防护、重定向次数限制、DNS 私网拦截；图床上传的库内重试与退避。

pub mod data_uri;
pub mod error;
pub mod fetch;
pub mod hosting;
pub mod image;
pub mod sniff;
pub mod source;

/// silk 音频编解码（`silk/` + `go-silk/` 的 C ABI 封装）。
pub mod silk {}

pub use data_uri::{DataUri, cq_base64, decode_base64, encode_base64, to_data_uri};
pub use error::{Error, Result};
pub use fetch::{
    BoxFuture, DEFAULT_MAX_BYTES, DEFAULT_TIMEOUT, FetchRequest, Fetched, Fetcher, HttpTransport,
    MediaTransport, TransportResponse,
};
pub use hosting::{
    ChatGlmBackend, DirectMultipartPlan, HostingBackend, HostingKind, HostingTransport,
    MultipartBody, SignedPlan, UkakaBackend, UploadBody, UploadMethod, UploadPlan, UploadRequest,
    UploadResponse, upload, upload_plan,
};
pub use image::image_size;
pub use sniff::{ImageKind, sniff_content_type, sniff_image};
pub use source::MediaSource;
