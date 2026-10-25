//! `WebUi::router`：四个 API 路由 + 静态资源回退。
//!
//! 返回的 `Router` 尚未绑定端口——挂到哪个 listener 由 runtime 决定（M3 只交付路由）。

use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};

use crate::WebUi;
use crate::assets;
use crate::auth;
use crate::config_api;
use crate::state::WebUiState;

impl WebUi {
    /// 构建 WebUI 路由（axum state 为 [`WebUiState`]）。
    pub fn router(state: Arc<WebUiState>) -> Router {
        Router::new()
            .route("/api/login", post(auth::login))
            .route("/api/logout", post(auth::logout))
            .route("/api/checkLoginStatus", get(auth::check_login_status))
            .route("/api/config", get(config_api::public_config))
            .fallback(assets::serve)
            .with_state(state)
    }
}
