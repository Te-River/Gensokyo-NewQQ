//! `WebUi::router`：管理 API + 静态资源回退。
//!
//! 返回的 `Router` 尚未绑定端口——挂到哪个 listener 由 runtime 决定。

use std::sync::Arc;

use axum::Router;
use axum::routing::{delete, get, post};

use crate::WebUi;
use crate::accounts;
use crate::assets;
use crate::auth;
use crate::config_api;
use crate::process;
use crate::state::WebUiState;
use crate::status;

impl WebUi {
    /// 构建 WebUI 路由（axum state 为 [`WebUiState`]）。
    ///
    /// `/api/check-login-status` 是 Go 的路径，`/api/checkLoginStatus` 是 M3 起的旧名，
    /// 两者指向同一处理函数，保持既有调用方不 404。
    ///
    /// `{uin}` 段与 Go 一样必须是配置里的 `app_id`，否则 404（见 `auth::guard_uin`）。
    pub fn router(state: Arc<WebUiState>) -> Router {
        Router::new()
            .route("/api/login", post(auth::login))
            .route("/api/logout", post(auth::logout))
            .route("/api/check-login-status", get(auth::check_login_status))
            .route("/api/checkLoginStatus", get(auth::check_login_status))
            .route("/api/status", get(status::system_status))
            .route("/api/accounts", get(accounts::list))
            .route("/api/config", get(config_api::public_config))
            .route(
                "/api/{uin}/config",
                get(config_api::account_config_read)
                    .patch(config_api::account_config_write)
                    .delete(config_api::account_config_delete),
            )
            .route("/api/{uin}/process/status", get(process::process_status))
            .route("/api/{uin}/process", delete(process::process_stop))
            .route("/api/{uin}/process/logs", get(process::logs))
            .route("/api/{uin}/api", post(process::api_forward))
            .route("/api/logs", get(process::logs))
            .fallback(assets::serve)
            .with_state(state)
    }
}
