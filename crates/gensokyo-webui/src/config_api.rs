//! `GET /api/config`：登录后返回**脱敏后**的关键字段。
//!
//! 白名单式序列化：只列管理台需要展示的字段，`token` / `client_secret` /
//! `server_user_password` 等敏感项永远不进响应体。新增字段必须显式加到 [`PublicConfig`]，
//! 不做 `Settings` 整体序列化。
//!
//! 账号维度的三条 `/{uin}/config`（Go `webui/api.go:259-308`，前端 `api.ts:834/868/905`）：
//!
//! - `GET` → `{content}`，正文是 `config.yml` **原文**（前端 `AccountConfigEditorView.vue`
//!   把它放进 YAML 编辑器再原样 PATCH 回来，因此这里不能换成脱敏投影：编辑器保存时会把
//!   脱敏占位写回文件，`token` / `server_user_password` 会被抹掉）；
//! - `PATCH` → 校验 YAML 后写盘并 `reload()`，重启项只落盘不生效（Go 是写完重启进程）；
//! - `DELETE` → 删文件并按首启模板重建（含内网 IP 替换），再 `reload()`。
//!
//! 三条都要求登录，且 `{uin}` 必须是配置里的 `app_id`（Go 用 `appIDStr` 拼路径，其它值
//! 落不到任何分支 → 404）。

use std::sync::Arc;

use axum::Json;
use axum::extract::Path;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use gensokyo_config::Settings;
use serde::Serialize;
use serde_json::json;

use crate::auth::{guard_uin, is_authenticated, unauthorized};
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

/// `GET`/`PATCH`/`DELETE /api/{uin}/config` 的请求/响应体（前端 `AccountConfigFile`）。
#[derive(Debug, Serialize, serde::Deserialize)]
pub struct AccountConfigFile {
    /// `config.yml` 原文。
    pub content: String,
}

/// `GET /api/{uin}/config`：回 `config.yml` 原文（Go `AccountConfigReadHandler`）。
pub(crate) async fn account_config_read(
    State(state): State<Arc<WebUiState>>,
    Path(uin): Path<u64>,
    headers: HeaderMap,
) -> Response {
    if let Some(reject) = guard_uin(&state, &headers, uin) {
        return reject;
    }
    match std::fs::read_to_string(state.config().path()) {
        Ok(content) => Json(AccountConfigFile { content }).into_response(),
        Err(err) => {
            tracing::error!(error = %err, "WebUI 读取配置失败");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Unable to read config file" })),
            )
                .into_response()
        }
    }
}

/// `PATCH /api/{uin}/config`：写回 `config.yml` 并触发热重载（Go `handlePatchConfig`）。
pub(crate) async fn account_config_write(
    State(state): State<Arc<WebUiState>>,
    Path(uin): Path<u64>,
    headers: HeaderMap,
    body: Result<Json<AccountConfigFile>, JsonRejection>,
) -> Response {
    if let Some(reject) = guard_uin(&state, &headers, uin) {
        return reject;
    }
    let Ok(Json(update)) = body else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid request body" })),
        )
            .into_response();
    };
    // 先解析再落盘：Go 是"写完直接重启"，坏 YAML 会让重启后的进程起不来；这里把非法
    // 内容挡在写盘之前，旧文件与旧快照都不受影响。
    if let Err(err) = gensokyo_config::parse_str(&update.content) {
        tracing::warn!(error = %err, "WebUI 写配置被拒绝：内容不是合法 YAML");
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid config content" })),
        )
            .into_response();
    }
    if let Err(err) = std::fs::write(state.config().path(), update.content.as_bytes()) {
        tracing::error!(error = %err, "WebUI 写配置失败");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "Failed to write to config file" })),
        )
            .into_response();
    }
    // 重启项（Go `restartRequiredFields`）只落盘不生效：`reload()` 回 `RestartRequired`
    // 且不交换快照，与 Go"要重启才生效"的口径一致；本进程不自杀。
    match state.config().reload() {
        Ok(outcome) => tracing::info!(?outcome, "WebUI 写配置后热重载"),
        Err(err) => {
            tracing::error!(error = %err, "WebUI 写配置后热重载失败");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Failed to reload config file" })),
            )
                .into_response();
        }
    }
    // Go 只回 `message`；这里多带写入后的原文——前端保存后把它写回编辑器
    // （`AccountConfigEditorView.vue:54` 读 `data.content`），缺了编辑区会变空白。
    Json(json!({ "message": "Config updated successfully", "content": update.content }))
        .into_response()
}

/// `DELETE /api/{uin}/config`：删除并重建首启模板（Go `handleDeleteConfig` → `DeleteConfig`）。
pub(crate) async fn account_config_delete(
    State(state): State<Arc<WebUiState>>,
    Path(uin): Path<u64>,
    headers: HeaderMap,
) -> Response {
    if let Some(reject) = guard_uin(&state, &headers, uin) {
        return reject;
    }
    let path = state.config().path().to_path_buf();
    if let Err(err) = std::fs::remove_file(&path) {
        tracing::error!(error = %err, "WebUI 删除配置失败");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": err.to_string() })),
        )
            .into_response();
    }
    // 与 Go 一致：重建带内网 IP 的模板（`template/config.yml`，占位符替换见 template 模块）。
    if let Err(err) = gensokyo_config::ensure_config_file(&path) {
        tracing::error!(error = %err, "WebUI 重建配置模板失败");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": err.to_string() })),
        )
            .into_response();
    }
    // Go 在这里重启进程；Rust 不自杀，改为立刻重载——模板与旧配置的差异多为重启项，
    // 通常停在 RestartRequired，旧快照继续生效（与"改完要重启"的语义一致）。
    match state.config().reload() {
        Ok(outcome) => tracing::info!(?outcome, "WebUI 重置配置后热重载"),
        Err(err) => tracing::warn!(error = %err, "WebUI 重置配置后热重载失败"),
    }
    StatusCode::NO_CONTENT.into_response()
}
