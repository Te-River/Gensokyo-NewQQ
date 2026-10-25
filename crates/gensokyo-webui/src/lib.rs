//! gensokyo-webui：内嵌前端资源（`webui/dist`）与管理 API。
//!
//! M3 交付范围：
//!
//! - **静态资源**：`rust-embed` 在编译期把仓库根 `webui/dist` 打进二进制；`/` 与非 API 的
//!   SPA 路径回退 `index.html`，形如文件的未命中路径 404（见 [`assets`]）。
//! - **登录**：`POST /api/login` 比对 `server_user_name` / `server_user_password`，成功下发
//!   24h 会话 cookie（`login_cookie`，HttpOnly + SameSite=Lax + Path=/），返回 `{code:0}`，
//!   失败 401 `{code:1}`（见 [`auth`]）。
//! - **会话**：进程内 `RwLock<HashMap<token, 过期时刻>>`；重启后登录态丢失。
//! - `GET /api/checkLoginStatus`、`POST /api/logout`、`GET /api/config`（脱敏白名单字段）。
//!
//! **M3 明确不做（M3b 起）**：会话持久化到 `cookie.db`（Go 用 bbolt 落盘）、SSE 日志推送、
//! 监控图表、`PATCH /api/config` 等写配置接口。详见 crate `README.md`。
//!
//! 依赖方向：webui → config。路由状态 [`WebUiState`] 只持有 [`gensokyo_config::ConfigManager`]，
//! **不依赖** runtime 的 `AppState`——否则 runtime 挂载 WebUI 路由时会形成环形依赖。

pub mod assets;
pub mod auth;
pub mod config_api;
pub mod router;
pub mod state;

pub use auth::{SESSION_COOKIE, SESSION_TTL, SessionStore};
pub use config_api::PublicConfig;
pub use state::WebUiState;

/// WebUI 路由入口（`WebUi::router`，见 [`router`]）。
#[derive(Debug, Clone, Copy)]
pub struct WebUi;
