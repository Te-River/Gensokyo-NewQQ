//! gensokyo-qq：QQ 官方 API 适配层（token 刷新、OpenAPI v2、群管理、DTO）。
//!
//! 只做协议与传输，不做 WebSocket、runtime、具体 action 业务（那些在 `gensokyo-runtime` /
//! `gensokyo-actions`）。
//!
//! # 覆盖范围（M1g）
//! - [`token::TokenManager`]：`app_id` / `client_secret` / sandbox 开关；异步互斥缓存；
//!   距过期 60 秒内自动刷新；[`token::Clock`] 可注入；[`token::TokenConfig::with_token_url`]
//!   可覆盖 token 端点（等价 Go `develop_access_token_dir`）。
//! - [`http::HttpTransport`]：所有请求的唯一出口；生产实现
//!   [`http::ReqwestTransport`]（reqwest + rustls），测试实现只需一个记录请求的 fake。
//! - [`openapi::OpenApiClient`]：群 / C2C 发消息、群 / C2C 上传文件，以及
//!   [`group`] 中的群管理子集。
//! - [`dto`]：`MessageToCreate`、`MessageResponse`、`MediaResponse` 与群管理结构。
//!
//! # 与 botgo 的差异
//! - **token 刷新**：botgo 启动后台 goroutine 定时刷新；本 crate 不创建 runtime，改为
//!   `token()` 惰性检查 + 异步互斥串行化。刷新窗口内拉取失败时，旧 token 若尚未真正过期
//!   就继续使用（`tracing::warn`），已过期才返回错误；旧缓存始终保留。
//! - **token 端点**：默认官方 `getAppAccessToken`；`TokenConfig::with_token_url` 可改为自定义
//!   地址（`GET`、无请求体，对齐 Go `develop_access_token_dir`），响应解析规则不变。
//! - **重试**：botgo 用 resty `SetRetryCount(3)` 对所有请求重试；本 crate 不在传输层重试。
//!   消息发送非幂等，重试属于 runtime 层的 `RetryPolicy`（见 `reports/refactor/09`）。
//! - **超时**：连接超时 10s（与 botgo TLS 握手超时一致）；请求总超时 10s（botgo 默认 3s，
//!   这里放宽以容纳媒体上传），可用 [`http::ReqwestTransport::from_client`] 覆盖。
//! - **TLS provider**：本 crate 只启用 `rustls-no-provider`，provider（ring / aws-lc-rs）由
//!   二进制注入（[`http::ReqwestTransport::with_provider`]），workspace 不强制 C 工具链。
//! - **消息体校验**：`msg_id` / `event_id` / `is_wakeup` 互斥在发送前直接拒绝；botgo 交给
//!   服务端报错。
//! - **富媒体上传**：`/v2/.../files` 使用 JSON（`file_type` + `url` / `file_data`），与
//!   botgo `RichMediaMessage` 一致；botgo 的 multipart 仅用于频道消息，v2 群 / C2C 没有
//!   multipart 上传路径。
//! - **错误映射**：`ApiError { http_status, code, message, trace_id }`；`trace_id` 取自
//!   `X-Tps-trace-ID`，响应体无业务码时 `code` 退回 HTTP 状态码。
//!
//! # 安全
//! - 鉴权头固定 `Authorization: QQBot <token>`，另带 `X-Union-Appid`。
//! - token 与 `client_secret` 不出现在 `Debug` 输出或错误信息里。
//! - openid 拼进 URL 前按 RFC 3986 path segment 编码。
//!
//! # 未实现端点
//! - WebSocket / gateway、`/gateway/bot`、`/users/@me`。
//! - 频道（guild / channel）与私信频道（DM）全套接口。
//! - 消息撤回、流式消息、`input_notify` / `card` 的强类型模型（可用
//!   [`dto::MessageToCreate::with_extra`] 透传）。
//! - 入群申请审批、自动审批策略、成员黑名单、批量移出、自定义菜单、指令面板。
//! - 分片上传（`upload_prepare` / `upload_part_finish`）与富媒体合并；当前只做
//!   `file_data` / `url` 直传（botgo `handlers` 侧对 >5MB 文件另走分片）。

pub mod dto;
pub mod error;
pub mod group;
pub mod http;
pub mod openapi;
pub mod token;

pub use dto::{
    BotInGroupState, FileUpload, GroupInfo, GroupMember, GroupMemberList, JoinRequest,
    JoinRequestList, Markdown, MediaResponse, MessageResponse, MessageToCreate,
    RestrictChatSetting, file_type, msg_type,
};
pub use error::{ApiError, Error, Result};
pub use http::{
    DEFAULT_CONNECT_TIMEOUT, DEFAULT_REQUEST_TIMEOUT, HttpMethod, HttpRequest, HttpResponse,
    HttpTransport, ReqwestTransport,
};
pub use openapi::{DEFAULT_BASE_URL, OpenApiClient, SANDBOX_BASE_URL};
pub use token::{Clock, REFRESH_AHEAD_SECS, SystemClock, TokenConfig, TokenManager};
