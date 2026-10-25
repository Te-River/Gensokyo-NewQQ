//! WebUI 路由状态（`Arc<WebUiState>` 作为 axum state）。
//!
//! 只持有 [`ConfigManager`] 与会话存储，**刻意不依赖** runtime 的 `AppState`：runtime 要把
//! WebUI 路由挂进主服务器，webui 若反向依赖 runtime 就形成环。配置读走
//! [`ConfigManager::snapshot`]，因此热重载对 `/api/config` 立即生效。

use std::sync::Arc;
use std::time::Duration;

use gensokyo_config::ConfigManager;

use crate::auth::{SESSION_TTL, SessionStore};

/// WebUI 路由状态。
#[derive(Debug)]
pub struct WebUiState {
    config: Arc<ConfigManager>,
    sessions: SessionStore,
}

impl WebUiState {
    /// 生产装配：会话 24h TTL。
    #[must_use]
    pub fn new(config: Arc<ConfigManager>) -> Self {
        Self::with_ttl(config, SESSION_TTL)
    }

    /// 指定会话 TTL（测试过期路径用；生产请用 [`WebUiState::new`]）。
    #[must_use]
    pub fn with_ttl(config: Arc<ConfigManager>, ttl: Duration) -> Self {
        Self {
            config,
            sessions: SessionStore::new(ttl),
        }
    }

    /// 配置快照管理器。
    #[must_use]
    pub fn config(&self) -> &ConfigManager {
        &self.config
    }

    /// 会话存储（登出、清理与测试直接访问）。
    #[must_use]
    pub fn sessions(&self) -> &SessionStore {
        &self.sessions
    }
}
