//! `GET /api/config`：登录后返回**脱敏后**的关键字段。
//!
//! 白名单式序列化：只列管理台需要展示的字段，`token` / `client_secret` /
//! `server_user_password` 等敏感项永远不进响应体。新增字段必须显式加到 [`PublicConfig`]，
//! 不做 `Settings` 整体序列化。写配置（Go 的 `PATCH /api/config`）不在 M3 范围。

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use gensokyo_config::Settings;
use serde::Serialize;

use crate::auth::{is_authenticated, unauthorized};
use crate::state::WebUiState;

/// 管理台可见的配置字段（脱敏白名单）。
#[derive(Debug, Serialize)]
pub struct PublicConfig {
    /// QQ 开放平台 AppID。
    pub app_id: u64,
    /// 主服务端口（字符串，与 Go `port` 同型）。
    pub port: String,
    /// 是否作为正向 WS 服务端。
    pub enable_ws_server: bool,
    /// 正向 WS 路径。
    pub ws_server_path: String,
    /// 分层架构模式（空/非法值已由访问器回退 `new`）。
    pub arch_mode: String,
    /// 是否禁用 WebUI。
    pub disable_webui: bool,
    /// 图床后端枚举（0 表示未启用）。
    pub oss_type: i64,
}

impl PublicConfig {
    /// 从配置快照取白名单字段。
    #[must_use]
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            app_id: settings.get_app_id(),
            port: settings.get_port_value().to_owned(),
            enable_ws_server: settings.get_enable_ws_server(),
            ws_server_path: settings.get_ws_server_path().to_owned(),
            arch_mode: settings.get_arch_mode().to_owned(),
            disable_webui: settings.get_disable_webui(),
            oss_type: settings.get_oss_type(),
        }
    }
}

/// `GET /api/config`：未登录 401，登录后回 [`PublicConfig`]。
pub(crate) async fn public_config(
    State(state): State<Arc<WebUiState>>,
    headers: HeaderMap,
) -> Response {
    if !is_authenticated(&state, &headers) {
        return unauthorized();
    }
    axum::Json(PublicConfig::from_settings(&state.config().snapshot())).into_response()
}
