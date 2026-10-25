//! 运行时共享状态。
//!
//! [`AppState`] 只强依赖「配置快照 + handler 注册表」；idmap / echo / OpenAPI 是 M1h-b
//! （入站事件）与 M1i（`send_*`）才需要的句柄，用 builder 选择性注入，缺省 `None`。
//! 这样 M1h-a 的传输层可以单独测试，不必先把存储与上游客户端装配起来。

use std::sync::Arc;

use gensokyo_config::ConfigManager;
use gensokyo_protocol::Registry;
use gensokyo_qq::{OpenApiClient, ReqwestTransport};
use gensokyo_storage::{EchoStore, IdMapStore};

use crate::builtin;
use crate::dispatcher::Dispatcher;
use crate::event_hub::EventHub;

/// 传输层与 handler 共享的状态。
#[derive(Debug, Clone)]
pub struct AppState {
    config: Arc<ConfigManager>,
    registry: Arc<Registry>,
    idmap: Option<Arc<IdMapStore>>,
    echo: Option<Arc<EchoStore>>,
    api: Option<Arc<OpenApiClient<ReqwestTransport>>>,
    hub: EventHub,
}

impl AppState {
    /// 用配置与注册表构造；可选句柄全部为 `None`，事件总线用默认容量。
    #[must_use]
    pub fn new(config: Arc<ConfigManager>, registry: Arc<Registry>) -> Self {
        Self {
            config,
            registry,
            idmap: None,
            echo: None,
            api: None,
            hub: EventHub::default(),
        }
    }

    /// 构造只注册 M1h-a 内置 action 的状态（[`builtin::register_all`]）。
    #[must_use]
    pub fn with_builtin_handlers(config: Arc<ConfigManager>) -> Self {
        let mut registry = Registry::new();
        builtin::register_all(&mut registry, Arc::clone(&config));
        Self::new(config, Arc::new(registry))
    }

    /// 注入 idmap（M1h-b 的入站转换需要虚拟 ID ↔ OpenID 映射）。
    #[must_use]
    pub fn with_idmap(mut self, idmap: Arc<IdMapStore>) -> Self {
        self.idmap = Some(idmap);
        self
    }

    /// 注入 echo 存储。
    #[must_use]
    pub fn with_echo(mut self, echo: Arc<EchoStore>) -> Self {
        self.echo = Some(echo);
        self
    }

    /// 注入 QQ OpenAPI 客户端（M1i 的 `send_*` action 需要）。
    #[must_use]
    pub fn with_api(mut self, api: Arc<OpenApiClient<ReqwestTransport>>) -> Self {
        self.api = Some(api);
        self
    }

    /// 换成外部构造的事件总线（多个 `AppState` 想共享一条总线时使用）。
    #[must_use]
    pub fn with_hub(mut self, hub: EventHub) -> Self {
        self.hub = hub;
        self
    }

    /// 配置快照管理器（每次请求现读快照，热重载即时生效）。
    #[must_use]
    pub fn config(&self) -> &ConfigManager {
        self.config.as_ref()
    }

    /// 配置快照管理器的 `Arc`（[`crate::inbound::Inbound`] 要持有）。
    #[must_use]
    pub fn config_arc(&self) -> Arc<ConfigManager> {
        Arc::clone(&self.config)
    }

    /// 事件总线：入站发布、反向 WS 订阅。
    #[must_use]
    pub fn hub(&self) -> &EventHub {
        &self.hub
    }

    /// handler 注册表。
    #[must_use]
    pub fn registry(&self) -> &Registry {
        self.registry.as_ref()
    }

    /// idmap 句柄（未注入时为 `None`）。
    #[must_use]
    pub fn idmap(&self) -> Option<&IdMapStore> {
        self.idmap.as_deref()
    }

    /// idmap 句柄的 `Arc`（[`crate::inbound::Inbound`] 要持有）。
    #[must_use]
    pub fn idmap_arc(&self) -> Option<Arc<IdMapStore>> {
        self.idmap.clone()
    }

    /// echo 句柄（未注入时为 `None`）。
    #[must_use]
    pub fn echo(&self) -> Option<&EchoStore> {
        self.echo.as_deref()
    }

    /// OpenAPI 客户端（未注入时为 `None`）。
    #[must_use]
    pub fn api(&self) -> Option<&OpenApiClient<ReqwestTransport>> {
        self.api.as_deref()
    }

    /// 基于当前注册表的分发器（克隆一个 `Arc`，开销可忽略）。
    #[must_use]
    pub fn dispatcher(&self) -> Dispatcher {
        Dispatcher::new(Arc::clone(&self.registry))
    }
}
