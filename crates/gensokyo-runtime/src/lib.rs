//! gensokyo-runtime：入站转换、四种连接、dispatch、队列与限速、优雅关闭。
//!
//! # M1h-a 覆盖范围（正向连接 + dispatch）
//! - [`AppState`]：配置快照 + handler 注册表；idmap / echo / OpenAPI 句柄以 builder
//!   注入，缺省 `None`（不强制调用方构造）。
//! - [`Dispatcher`]：`ActionRequest<Value>` → [`gensokyo_protocol::Registry`] →
//!   `ActionResponse<Value>`。未知 action `retcode=1404`、参数/JSON 非法 `1400`、
//!   handler panic 隔离为 `1500`；`echo` 始终原样回传。handler 是异步的，直接 await，
//!   不再经 `spawn_blocking` 调度线程池。
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
//!   覆盖 botgo 的 12 类自定义事件（群消息/群开关/关系链/群成员/入群申请）加
//!   `GROUP_AT_MESSAGE_CREATE` / `C2C_MESSAGE_CREATE` 与 `lifecycle/connect`、`heartbeat`
//!   元事件；未知事件返回空列表，不 panic。
//! - [`intent::IntentPlan`]：`text_intent` 的 17 个 handler 名 → botgo intent 位
//!   （对齐 Go `main.go:316-353`，含 `discover_unknown_events` 与摘位策略），启动时写日志。
//! - [`ws_client::WsClient`]：读 `ws_address[]` 逐地址连接（`tokio-tungstenite`），
//!   `reconnect_times` 指数退避、`heart_beat_interval` 心跳、`ws_token[]` +
//!   URL `access_token` 鉴权，可优雅停止。
//! - [`webhook`]：`webhook_path` 上的 `POST` 回调 → `Inbound` → `EventHub`，回 `{"op":12}`。
//! - [`boot::Runtime`]：以上四块与 M1h-a 的 listener 一起装配，全部由
//!   [`RuntimeHandle::shutdown`] 收摊。
//!
//! # M3 WebUI 挂载
//! - `disable_webui=false`（默认）时 [`boot::Runtime`] 建一份 [`gensokyo_webui::WebUiState`]
//!   （配置走 `ConfigManager` 快照，热重载即时生效），把 `WebUi::router` 挂到 HTTP API 与
//!   正向 WS 两个 listener；[`AppState::with_webui`] 可覆盖开关。
//! - 优先级（挂载后）：`POST` 且路径**不在** `/api`、`/api/**` 下 → action；其余 → WebUI
//!   （`GET /` 静态页、`GET /api/*` 查询、`POST /api/login|logout`）。WebUI 的静态回退只认
//!   `GET`/`HEAD`，未注册的 `/api` 回 404。细节见 [`http_api`] 模块文档。
//! - `disable_webui=true` 时路由表回到 M1h-a 原样：`GET /` 是健康检查文本，`POST` 任意路径
//!   都是 action（含 `/api/**`）。
//! - 会话清理：每 10 分钟一次 `SessionStore::cleanup`，定时任务随 [`RuntimeHandle`] 停止。
//!
//! # M1i deferred 发送执行
//! - [`AppState::with_actions`]：注册表 = 内置 3 个 action + `gensokyo_actions::register_all`
//!   （41 个注册名 + 3 个 Go 兼容别名），并挂上 [`AppState::actions`]（`ActionContext`）。
//! - [`dispatcher::Dispatcher`]：handler 回 `data.deferred == true` 时执行
//!   [`deferred::execute`]——反序列化 `SendRequest`、收件人反查 OpenID、`MessageOut` 合成
//!   `MessageToCreate`（文本 + `[CQ:at]` → `<@openid>` + 单个 media + reply `msg_id`），按
//!   target 调 `QqApi::send_group_message` / `send_c2c_message`，最终 `data = {"message_id": …}`。
//! - [`media::MediaRuntime`]：图片 / 语音 / 视频 / 文件的 `url` / 本地 / `base64` 来源先经
//!   `gensokyo_media::Fetcher` 取字节，再调 `QqApi::upload_group_file` / `upload_c2c_file`
//!   换 `file_info`（单条上限 5MB，细节见 [`media`]）。
//! - 发送成功的 `message_id` 回写 `EchoStore`（行号双向映射，Go `StoreCacheInMemory`）；
//!   `[CQ:reply]` 的虚拟号经 idmap / echo 还原成真实消息 ID。
//! - 失败分档：请求侧问题（媒体来源 / 大小 / 类型、reply 无法还原、不支持的段）`1400`，
//!   其余（未装配 `ActionContext`、下载 / 上传 / 上游失败）`1500`。
//!
//! # 未做项（后续里程碑）
//! - 消息段：图片 / 语音 / 视频 / ark 卡片 / markdown / card 一律停留在文本形态，
//!   `array_value` 不生效（细节见 [`inbound`] 模块文档）。
//! - `INTERACTION_CREATE`、`MESSAGE_AUDIT_*`、`FORUM_*`、`AUDIO_*`、频道 `GUILD_*` 未转换。
//! - webhook 的 ed25519 签名校验与 `op=13` 回调校验（workspace 无 ed25519 依赖），
//!   `union_webhook` 扇出亦未做。
//! - 反向 WS 上收到的 OneBot action 不 dispatch；`sendFailures` 补发队列未实现。
//! - 具体 `send_*` / 群管理 action（M1i）、有界队列与令牌桶（[`queue`]）。
//!   群管理其余端点、有界队列与令牌桶仍属后续里程碑。
//! - deferred 的媒体片段级能力：markdown / keyboard / card / music / stream / input_notify
//!   段仍返回 `1400`；超 5MB 媒体缺分片上传；`active` / `wakeup` 标记被忽略。
//! - 正向 WS 连接建立时仍**不发送** `meta_event/lifecycle/connect`（只有反向 WS 发），
//!   与 Go 的行为差异保留到正向 WS 的 M1h-b 后续对齐。
//! - WebUI 与 Go 主服务器路由：M1h-a 的 WS 独立监听 `0.0.0.0:{port}`（Go 是挂在主服务器
//!   同一端口上），后续里程碑合并。

/// @全员：CQ / 字面标签归一 + 每群每自然日 1 次的计数限流。
pub mod at_everyone;
pub mod auth;
pub mod boot;
pub mod builtin;
/// `send_*` 的 deferred 计划执行（计划 → `MessageToCreate` → QQ 上游）。
pub mod deferred;
pub mod dispatcher;
pub mod event_hub;
pub mod http_api;
/// QQ 事件 → OneBot 事件（`Processor/`，M1h-b）。
pub mod inbound;
/// `text_intent` → botgo intent 位（`main.go`，M1h-c）。
pub mod intent;
/// deferred 媒体链路：`MediaRef` → 字节 → QQ `files` 端点 → `file_info`。
pub mod media;
/// 有界队列 + 令牌桶（`messagequeue/`，M1i）。
pub mod queue {}
pub mod state;
/// 反向 HTTP 回调（`server/webhook.go`，M1h-b）。
pub mod webhook;
/// 反向 WS 客户端与重连（`wsclient/`，M1h-b）。
pub mod ws_client;
pub mod ws_server;

pub use boot::{DEFAULT_SHUTDOWN_TIMEOUT, Runtime, RuntimeHandle, RuntimeOptions};
pub use deferred::DEFERRED_FIELD;
pub use dispatcher::{Dispatcher, RETCODE_BAD_REQUEST, RETCODE_INTERNAL, RETCODE_UNKNOWN_ACTION};
pub use event_hub::EventHub;
pub use inbound::Inbound;
pub use intent::IntentPlan;
pub use media::{FetchedMedia, MAX_MEDIA_BYTES, MediaRuntime};
pub use state::AppState;
pub use webhook::{ACK_BODY as WEBHOOK_ACK_BODY, MAX_BODY_BYTES as WEBHOOK_MAX_BODY_BYTES};
pub use ws_client::{WsClient, WsClientHandle, WsClientOptions};
