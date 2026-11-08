//! `GET /api/accounts`：管理台的机器人列表（Go `webui/api.go:HandleAccountsRequest`）。
//!
//! Go 直接回一行 `{"uin": GetAppID(), "predefined": false, "process_created": true}`：
//! Go 版一个进程只跑一个 AppID，列表天然只有一项。Rust 侧同样只认配置里的 `app_id`
//! （没有独立的账号库），因此最多一项；`app_id` 未配置（0）时回空列表——Go 会回
//! `uin: 0`，这里如实表示"没有账号"，避免前端把一个不存在的 0 号账号画进选择器。

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::auth::{is_authenticated, unauthorized};
use crate::state::WebUiState;

/// 账号列表项（Go `HandleAccountsRequest` 的三个字段）。
#[derive(Debug, Serialize)]
pub struct AccountListItem {
    /// 机器人 AppID。
    pub uin: u64,
    /// 是否为预置账号；Go 固定 `false`。
    pub predefined: bool,
    /// 进程是否已创建；Go 固定 `true`。
    pub process_created: bool,
}

/// `GET /api/accounts`：未登录 401，登录后回账号列表。
pub(crate) async fn list(State(state): State<Arc<WebUiState>>, headers: HeaderMap) -> Response {
    if !is_authenticated(&state, &headers) {
        return unauthorized();
    }
    let app_id = state.config().snapshot().get_app_id();
    let accounts = if app_id == 0 {
        Vec::new()
    } else {
        vec![AccountListItem {
            uin: app_id,
            predefined: false,
            process_created: true,
        }]
    };
    Json(accounts).into_response()
}
