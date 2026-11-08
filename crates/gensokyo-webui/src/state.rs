//! WebUI 路由状态（`Arc<WebUiState>` 作为 axum state）。
//!
//! 只持有 [`ConfigManager`] 与会话存储，**刻意不依赖** runtime 的 `AppState`：runtime 要把
//! WebUI 路由挂进主服务器，webui 若反向依赖 runtime 就形成环。配置读走
//! [`ConfigManager::snapshot`]，因此热重载对 `/api/config`、`/api/accounts` 立即生效。

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
    /// 生产装配：会话 24h TTL，落 `config.yml` 同目录的 `cookie.redb`。
    #[must_use]
    pub fn new(config: Arc<ConfigManager>) -> Self {
        let sessions = SessionStore::open_default(SESSION_TTL, config.path());
        Self { config, sessions }
    }

    /// 纯内存会话 + 指定 TTL（测试过期路径用；生产请用 [`WebUiState::new`]）。
    #[must_use]
    pub fn with_ttl(config: Arc<ConfigManager>, ttl: Duration) -> Self {
        Self::with_store(config, SessionStore::new(ttl))
    }

    /// 指定会话存储（持久化测试用自定义 `cookie.redb` 路径）。
    #[must_use]
    pub fn with_store(config: Arc<ConfigManager>, sessions: SessionStore) -> Self {
        Self { config, sessions }
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
