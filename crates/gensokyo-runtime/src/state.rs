//! 运行时共享状态。
//!
//! [`AppState`] 只强依赖「配置快照 + handler 注册表」；action 依赖容器
//! （[`gensokyo_actions::ActionContext`]）、idmap / echo / OpenAPI 是 M1h-b（入站事件）与
//! M1i（`send_*`）才需要的句柄，用 builder 选择性注入，缺省 `None`。
//! 这样 M1h-a 的传输层可以单独测试，不必先把存储与上游客户端装配起来。

use std::sync::Arc;

use gensokyo_actions::ActionContext;
use gensokyo_config::ConfigManager;
use gensokyo_protocol::Registry;
use gensokyo_qq::{OpenApiClient, ReqwestTransport};
use gensokyo_storage::{EchoStore, IdMapStore};

use crate::builtin;
use crate::dispatcher::Dispatcher;
use crate::event_hub::EventHub;
use crate::media::MediaRuntime;

/// 传输层与 handler 共享的状态。
#[derive(Debug, Clone)]
pub struct AppState {
    config: Arc<ConfigManager>,
    registry: Arc<Registry>,
    actions: Option<Arc<ActionContext>>,
    idmap: Option<Arc<IdMapStore>>,
    echo: Option<Arc<EchoStore>>,
    api: Option<Arc<OpenApiClient<ReqwestTransport>>>,
    media: Arc<MediaRuntime>,
    hub: EventHub,
    webui: Option<bool>,
}

impl AppState {
    /// 用配置与注册表构造；可选句柄全部为 `None`，事件总线用默认容量。
    ///
    /// 媒体下载器总是存在（[`MediaRuntime::default`] 延迟建 HTTP 客户端，构造不会失败）。
    #[must_use]
    pub fn new(config: Arc<ConfigManager>, registry: Arc<Registry>) -> Self {
        Self {
            config,
            registry,
            actions: None,
            idmap: None,
            echo: None,
            api: None,
            media: Arc::new(MediaRuntime::default()),
            hub: EventHub::default(),
            webui: None,
        }
    }

    /// 构造只注册 M1h-a 内置 action 的状态（[`builtin::register_all`]）。
    #[must_use]
    pub fn with_builtin_handlers(config: Arc<ConfigManager>) -> Self {
        let mut registry = Registry::new();
        builtin::register_all(&mut registry, Arc::clone(&config));
        Self::new(config, Arc::new(registry))
    }

    /// 构造「内置 action + `gensokyo_actions` 全集」的状态。
    ///
    /// 注册表 = 3 个内置 action（`get_status` / `get_version_info` / `get_login_info`）+
    /// [`gensokyo_actions::register_all`]（41 个注册名 + 3 个 Go 兼容别名）；deferred 执行链
    /// 需要 [`ActionContext`] 里的 [`gensokyo_actions::QqApi`] 才能把 `send_*` 真正发到 QQ。
    #[must_use]
    pub fn with_actions(config: Arc<ConfigManager>, actions: Arc<ActionContext>) -> Self {
        Self::with_builtin_handlers(config).with_action_context(actions)
    }

    /// 挂上 action 依赖并重建注册表（内置 3 个 + action 全集）。
    ///
    /// 注意：注册表被整体替换，此前经 [`AppState::new`] 注入的自定义 handler 不会保留。
    #[must_use]
    pub fn with_action_context(mut self, actions: Arc<ActionContext>) -> Self {
        let mut registry = Registry::new();
        builtin::register_all(&mut registry, Arc::clone(&self.config));
        gensokyo_actions::register_all(&mut registry, Arc::clone(&actions));
        self.registry = Arc::new(registry);
        self.actions = Some(actions);
        self
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

    /// 覆盖 deferred 的媒体下载器（单测注入假传输；生产用默认 HTTP 实现）。
    #[must_use]
    pub fn with_media(mut self, media: Arc<MediaRuntime>) -> Self {
        self.media = media;
        self
    }

    /// 换成外部构造的事件总线（多个 `AppState` 想共享一条总线时使用）。
    #[must_use]
    pub fn with_hub(mut self, hub: EventHub) -> Self {
        self.hub = hub;
        self
    }

    /// 覆盖 WebUI 挂载开关（缺省跟随配置 `disable_webui`，测试与嵌入场景用）。
    #[must_use]
    pub fn with_webui(mut self, enabled: bool) -> Self {
        self.webui = Some(enabled);
        self
    }

    /// WebUI 是否挂载：显式覆盖优先，否则取 `!disable_webui`。
    ///
    /// 这里放在 `AppState` 而不是 `RuntimeOptions`，是为了不打断
    /// `RuntimeOptions { ws_bind, http_bind }` 这种外部结构体字面量（`gensokyo-bin`）。
    #[must_use]
    pub fn webui_enabled(&self) -> bool {
        self.webui
            .unwrap_or_else(|| !self.config.snapshot().get_disable_webui())
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

    /// action 依赖容器（未装配时为 `None`，`send_*` 的 deferred 计划此时只能回 1500）。
    #[must_use]
    pub fn actions(&self) -> Option<&ActionContext> {
        self.actions.as_deref()
    }

    /// action 依赖容器的 `Arc`（[`crate::dispatcher::Dispatcher`] 要持有）。
    #[must_use]
    pub fn actions_arc(&self) -> Option<Arc<ActionContext>> {
        self.actions.clone()
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

    /// deferred 的媒体下载器。
    #[must_use]
    pub fn media(&self) -> &MediaRuntime {
        self.media.as_ref()
    }

    /// OpenAPI 客户端（未注入时为 `None`）。
    #[must_use]
    pub fn api(&self) -> Option<&OpenApiClient<ReqwestTransport>> {
        self.api.as_deref()
    }

    /// 基于当前注册表的分发器（克隆一个 `Arc`，开销可忽略）。
    ///
    /// 装配了 [`ActionContext`] 时一并挂上，deferred 计划才会被真正执行；echo 一并传递，
    /// 发送成功的 `message_id` 才会回写到同一个存储。
    #[must_use]
    pub fn dispatcher(&self) -> Dispatcher {
        Dispatcher::new(Arc::clone(&self.registry))
            .with_actions(self.actions_arc())
            .with_media(Arc::clone(&self.media))
            .with_echo(self.echo.clone())
    }
}
