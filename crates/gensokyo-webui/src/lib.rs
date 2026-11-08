//! gensokyo-webui：内嵌前端资源（`webui/dist`）与管理 API。
//!
//! 交付范围：
//!
//! - **静态资源**：`rust-embed` 在编译期把仓库根 `webui/dist` 打进二进制；`/` 与非 API 的
//!   SPA 路径回退 `index.html`，形如文件的未命中路径 404（见 [`assets`]）。
//! - **登录**：`POST /api/login` 比对 `server_user_name` / `server_user_password`，成功下发
//!   24h 会话 cookie（`login_cookie`，HttpOnly + SameSite=Lax + Path=/），返回
//!   `{isLoggedIn:true,cookie}`，失败 401 `{isLoggedIn:false}`（见 [`auth`]）。
//! - **会话**：默认落 `config.yml` 同目录的 `cookie.redb`（redb），重启后登录态恢复；
//!   打不开时降级纯内存并 warn（见 [`auth`] 与 [`session_db`]）。
//! - **查询**：`GET /api/check-login-status`（Go 路径，始终 200 + `isLoggedIn`）、
//!   `GET /api/checkLoginStatus`（旧名，同一处理函数）、`POST /api/logout`（Rust 扩展）、
//!   `GET /api/accounts`、`GET /api/status`、`GET /api/config`（脱敏白名单字段）。
//! - **账号配置**：`GET/PATCH/DELETE /api/{uin}/config`（原文读 / 校验后写盘并热重载 /
//!   删档重建模板，见 [`config_api`]）。
//! - **进程面与日志**：`GET /api/{uin}/process/status`、`DELETE /api/{uin}/process`（如实回
//!   501：无进程守护）、`POST /api/{uin}/api`（Go 只有 `Invalid API name` 分支）、
//!   `GET /api/{uin}/process/logs` 与 `GET /api/logs`（Go 回空数组，见 [`process`]）。
//!
//! **尚未交付**：`PUT /api/{uin}`（Go `HandleAppIDRequest`，前端不调用）、日志的 WebSocket
//! 推送（需要先给 `gensokyo-bin` 的 tracing subscriber 挂采集层，超出本 crate 边界）。
//!
//! 依赖方向：webui → config。路由状态 [`WebUiState`] 只持有 [`gensokyo_config::ConfigManager`]，
//! **不依赖** runtime 的 `AppState`——否则 runtime 挂载 WebUI 路由时会形成环形依赖。

pub mod accounts;
pub mod assets;
pub mod auth;
pub mod config_api;
pub mod process;
pub mod router;
pub mod session_db;
pub mod state;
pub mod status;

pub use auth::{SESSION_COOKIE, SESSION_TTL, SessionStore};
pub use config_api::PublicConfig;
pub use process::ProcessInfo;
pub use session_db::SESSION_DB_FILE;
pub use state::WebUiState;

/// WebUI 路由入口（`WebUi::router`，见 [`router`]）。
#[derive(Debug, Clone, Copy)]
pub struct WebUi;
