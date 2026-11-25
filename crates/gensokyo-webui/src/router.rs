//! `WebUi::router`：管理 API + 静态资源回退。
//!
//! 返回的 `Router` 尚未绑定端口——挂到哪个 listener 由 runtime 决定。
//!
//! 两条前缀指向同一份内容：根路径（M3 起的既有地址）与 Go 的 `/webui`（`main.go:539-545`
//! 把整个 WebUI 挂在该前缀下，老前端相对路径、用户书签、nginx 反代都用它）。

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
    ///
    /// 同一份 API 路由表挂两次：根路径（M3 起的既有地址）与 Go 的 `/webui` 前缀
    /// （`main.go:539-545` 的 `webuiGroup := r.Group("/webui")`，五个方法各一条
    /// `/*filepath`；老前端生成器给的 `basePath = "."` 落在 `/webui/` 页面上就变成
    /// `/webui/api/…`）。静态资源不必单独挂前缀：`fallback` 的 [`assets::serve`] 自己剥前缀。
    pub fn router(state: Arc<WebUiState>) -> Router {
        Router::new()
            .merge(api_routes(""))
            .merge(api_routes("/webui"))
            .fallback(assets::serve)
            .with_state(state)
    }
}

/// 管理 API 路由表；`prefix` 为空表示根路径，`/webui` 表示 Go 的既有前缀。
fn api_routes(prefix: &str) -> Router<Arc<WebUiState>> {
    Router::new()
        .route(&format!("{prefix}/api/login"), post(auth::login))
        .route(&format!("{prefix}/api/logout"), post(auth::logout))
        .route(
            &format!("{prefix}/api/check-login-status"),
            get(auth::check_login_status),
        )
        .route(
            &format!("{prefix}/api/checkLoginStatus"),
            get(auth::check_login_status),
        )
        .route(&format!("{prefix}/api/status"), get(status::system_status))
        .route(&format!("{prefix}/api/accounts"), get(accounts::list))
        .route(
            &format!("{prefix}/api/config"),
            get(config_api::public_config),
        )
        .route(
            &format!("{prefix}/api/{{uin}}/config"),
            get(config_api::account_config_read)
                .patch(config_api::account_config_write)
                .delete(config_api::account_config_delete),
        )
        .route(
            &format!("{prefix}/api/{{uin}}/process/status"),
            get(process::process_status),
        )
        .route(
            &format!("{prefix}/api/{{uin}}/process"),
            delete(process::process_stop),
        )
        .route(
            &format!("{prefix}/api/{{uin}}/process/logs"),
            get(process::logs),
        )
        .route(
            &format!("{prefix}/api/{{uin}}/api"),
            post(process::api_forward),
        )
        .route(&format!("{prefix}/api/logs"), get(process::logs))
}
