//! gensokyo-runtime：入站转换、四种连接、dispatch、队列与限速、优雅关闭。
//!
//! # M1h-a 覆盖范围（正向连接 + dispatch）
//! - [`AppState`]：配置快照 + handler 注册表；idmap / echo / OpenAPI 句柄以 builder
//!   注入，缺省 `None`（不强制 M1h-a 的调用方构造）。
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
//! # 未做项（后续里程碑）
//! - 反向 WS 客户端与重连（[`ws_client`]，M1h-b）、webhook 上报。
//! - QQ 事件 → OneBot 事件转换（[`inbound`]，M1h-b）。因此正向 WS 连接建立时**不发送**
//!   Go 的 `meta_event/lifecycle/connect` 元事件。
//! - 具体 `send_*` / 群管理 action（M1i）、有界队列与令牌桶（[`queue`]）。
//! - WebUI 与 Go 主服务器路由：M1h-a 的 WS 独立监听 `0.0.0.0:{port}`（Go 是挂在主服务器
//!   同一端口上），后续里程碑合并。

pub mod auth;
pub mod boot;
pub mod builtin;
pub mod dispatcher;
pub mod http_api;
/// QQ 事件 → OneBot 事件（`Processor/`，M1h-b）。
pub mod inbound {}
/// 有界队列 + 令牌桶（`messagequeue/`，M1i）。
pub mod queue {}
pub mod state;
/// 反向 WS 客户端与重连（`wsclient/`，M1h-b）。
pub mod ws_client {}
pub mod ws_server;

pub use boot::{DEFAULT_SHUTDOWN_TIMEOUT, Runtime, RuntimeHandle, RuntimeOptions};
pub use dispatcher::{Dispatcher, RETCODE_BAD_REQUEST, RETCODE_INTERNAL, RETCODE_UNKNOWN_ACTION};
pub use state::AppState;
