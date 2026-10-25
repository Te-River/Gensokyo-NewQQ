//! gensokyo-runtime：入站转换、四种连接、dispatch、队列与限速、优雅关闭。
//!
//! # M1h-a 覆盖范围（正向连接 + dispatch）
//! - [`AppState`]：配置快照 + handler 注册表；idmap / echo / OpenAPI 句柄以 builder
//!   注入，缺省 `None`（不强制调用方构造）。
//! - [`Dispatcher`]：`ActionRequest<Value>` → [`gensokyo_protocol::Registry`] →
//!   `ActionResponse<Value>`。未知 action `retcode=1404`、参数/JSON 非法 `1400`、
//!   handler panic 隔离为 `1500`；`echo` 始终原样回传。
//! - [`builtin`]：`get_status` / `get_version_info` / `get_login_info`（字段对齐 Go
//!   `handlers/`）。
//! - [`ws_server`]：正向 WS 服务端（`enable_ws_server` / `ws_server_path` /
//!   `ws_server_token`），多客户端并发，断开即清理。
//! - [`http_api`]：正向 HTTP API（`http_address` / `http_access_token`），`POST` 任意路径，
//!   `GET /` 健康检查。
//! - [`boot::Runtime`]：装配两个 listener，返回实际监听地址与优雅关闭句柄。
//!
//! # M1h-b 覆盖范围（反向 WS + webhook + 入站事件转换）
//! - [`event_hub::EventHub`]：`tokio::sync::broadcast` 事件总线；webhook 发布、反向 WS
//!   订阅，一条事件可同时送多个 OneBot 客户端。
//! - [`inbound::Inbound`]：QQ 事件 `t` + `d` → OneBot [`gensokyo_protocol::Event`]。
//!   已覆盖 8 个 `t`：`GROUP_AT_MESSAGE_CREATE` / `GROUP_MESSAGE_CREATE` /
//!   `C2C_MESSAGE_CREATE` / `GROUP_ADD_ROBOT` / `GROUP_DEL_ROBOT` / `GROUP_MSG_REJECT` /
//!   `GROUP_MSG_RECEIVE` / `FRIEND_ADD`，外加 `lifecycle/connect` 与 `heartbeat` 元事件；
//!   未知事件返回空列表，不 panic。
//! - [`ws_client::WsClient`]：读 `ws_address[]` 逐地址连接（`tokio-tungstenite`），
//!   `reconnect_times` 指数退避、`heart_beat_interval` 心跳、`ws_token[]` +
//!   URL `access_token` 鉴权，可优雅停止。
//! - [`webhook`]：`webhook_path` 上的 `POST` 回调 → `Inbound` → `EventHub`，回 `{"op":12}`。
//! - [`boot::Runtime`]：以上四块与 M1h-a 的 listener 一起装配，全部由
//!   [`RuntimeHandle::shutdown`] 收摊。
//!
//! # 未做项（后续里程碑）
//! - 消息段：图片 / 语音 / 视频 / ark 卡片 / markdown / card 一律停留在文本形态，
//!   `array_value` 不生效（细节见 [`inbound`] 模块文档）。
//! - 群管理事件全量：`GROUP_MEMBER_ADD/REMOVE`、`GROUP_JOIN_REQUEST`、
//!   `INTERACTION_CREATE`、频道 `GUILD_*` 未转换。
//! - webhook 的 ed25519 签名校验与 `op=13` 回调校验（workspace 无 ed25519 依赖），
//!   `union_webhook` 扇出亦未做。
//! - 反向 WS 上收到的 OneBot action 不 dispatch；`sendFailures` 补发队列未实现。
//! - 具体 `send_*` / 群管理 action（M1i）、有界队列与令牌桶（[`queue`]）。
//! - 正向 WS 连接建立时仍**不发送** `meta_event/lifecycle/connect`（只有反向 WS 发），
//!   与 Go 的行为差异保留到正向 WS 的 M1h-b 后续对齐。
//! - WebUI 与 Go 主服务器路由：M1h-a 的 WS 独立监听 `0.0.0.0:{port}`（Go 是挂在主服务器
//!   同一端口上），后续里程碑合并。

pub mod auth;
pub mod boot;
pub mod builtin;
pub mod dispatcher;
pub mod event_hub;
pub mod http_api;
/// QQ 事件 → OneBot 事件（`Processor/`，M1h-b）。
pub mod inbound;
/// 有界队列 + 令牌桶（`messagequeue/`，M1i）。
pub mod queue {}
pub mod state;
/// 反向 HTTP 回调（`server/webhook.go`，M1h-b）。
pub mod webhook;
/// 反向 WS 客户端与重连（`wsclient/`，M1h-b）。
pub mod ws_client;
pub mod ws_server;

pub use boot::{DEFAULT_SHUTDOWN_TIMEOUT, Runtime, RuntimeHandle, RuntimeOptions};
pub use dispatcher::{Dispatcher, RETCODE_BAD_REQUEST, RETCODE_INTERNAL, RETCODE_UNKNOWN_ACTION};
pub use event_hub::EventHub;
pub use inbound::Inbound;
pub use state::AppState;
pub use webhook::{ACK_BODY as WEBHOOK_ACK_BODY, MAX_BODY_BYTES as WEBHOOK_MAX_BODY_BYTES};
pub use ws_client::{WsClient, WsClientHandle, WsClientOptions};
