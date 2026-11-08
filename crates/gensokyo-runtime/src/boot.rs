//! 任务树与启动器：把正向 WS / HTTP API / webhook / 反向 WS 装配成可关闭的 [`RuntimeHandle`]。
//!
//! 绑定地址来源（对齐 Go `main.go`）：
//! - 正向 WS：`0.0.0.0:{port}`。Go 把 WS 挂在主服务器（`serverPort`）上；Rust 侧仍是独立
//!   listener（WebUI 路由已并进来，Go 主服务器的其余路由暂未合并）。
//! - HTTP API：`http_address` 原样使用（空则不启动）。
//! - webhook（`webhook_path`）：Go 与正向 WS 共用主服务器端口，这里 merge 进上面两个
//!   listener 的路由表（见 [`crate::webhook`] 模块文档）。
//! - WebUI（`!disable_webui`，可用 [`AppState::with_webui`] 覆盖）：静态页与 `/api` 登录
//!   路由挂到两个 listener。HTTP API 端口上 `POST`（`/api` 以外）仍归 action，优先级见
//!   [`crate::http_api`]；WS 端口只借出 WebUI 的 GET 面（Go 同样把 WebUI 挂主服务器端口，
//!   action 留在 `http_address` 的 `NoRoute`）。两个 listener 都保留 `GET /health` 探活文本。
//!   会话清理是个 10 分钟定时任务，随 [`RuntimeHandle`] 一起停。
//! - 反向 WS（`ws_address[]`）：纯出站连接，不需要监听端口；地址全为空则不启动。
//!
//! 启动的第一步是 `Runtime::attach_echo`：状态里没有 echo 时按配置建一份（`ramdom_seq`
//! 跟随 [`gensokyo_config::Settings::get_ramdom_seq`]）并启动清理线程，供入站喂懒 message_id
//! 池、出站回写 msg_id 映射（Go 侧 echo 同样是启动期全局单例）。
//!
//! [`RuntimeOptions`] 可覆盖两个绑定地址：测试用 `127.0.0.1:0` 取随机端口，避免端口冲突。
//!
//! [`RuntimeOptions::config_watch`]（默认开）在启动时拉起 [`gensokyo_config::ConfigWatcher`]：
//! 配置变动 → 快照热更新（热字段逐项 `info`）/ 重启项逐项 `warn` 提示需重启；监听启动失败只
//! `warn`、不阻断启动，监听线程随 [`RuntimeHandle`] 关闭一起停止。
//!
//! listener 只在启动时创建：`http_address` 不是配置层的重启项，热改该字段会更新快照但
//! 不会迁移监听端口（与 Go 只在启动时 `ListenAndServe` 一致），需要新端口请重启进程。
//! token 与其余热字段在每次请求 / 每次握手时现读快照。
//!
//! 关闭语义见 [`RuntimeHandle::shutdown`]：先停反向 WS 客户端（否则它们会在 listener
//! 收摊期间继续重连），再给全部 listener 发信号并逐个 await，整体受
//! [`DEFAULT_SHUTDOWN_TIMEOUT`] 约束；句柄未 `shutdown` 就被 drop 时由 `Drop` 兜底
//! 中止任务，避免端口继续监听。

use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use gensokyo_config::{ConfigWatcher, ReloadOutcome, WatcherHandle};
use gensokyo_core::{Error, Result};
use gensokyo_storage::EchoStore;
use gensokyo_webui::{WebUi, WebUiState};
use tokio::net::TcpListener;
use tokio::sync::mpsc::Receiver;
use tokio::sync::oneshot;
use tokio::task::{AbortHandle, JoinHandle};

use crate::event_hub::EventHub;
use crate::state::AppState;
use crate::ws_client::{WsClient, WsClientHandle};

/// 优雅关闭的默认时限：超过它则强制中止残留任务（WS 长连接不会无限拖住退出）。
pub const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// WebUI 会话清理间隔：会话 TTL 是 24h，10 分钟扫一次足够，且每次只短暂持写锁。
const WEBUI_CLEANUP_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// 配置重载结果的通道容量：单次事件只产出一条结果，8 足够吸收连续保存。
const CONFIG_RELOAD_CHANNEL: usize = 8;

/// 运行时覆盖项：绑定地址（`None` 表示按配置，见模块文档）与热重载开关。
#[derive(Debug, Clone)]
pub struct RuntimeOptions {
    /// 正向 WS 绑定地址覆盖。
    pub ws_bind: Option<String>,
    /// HTTP API 绑定地址覆盖。
    pub http_bind: Option<String>,
    /// 是否启动 `config.yml` 热重载监听（默认 `true`；测试与嵌入场景可关）。
    pub config_watch: bool,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            ws_bind: None,
            http_bind: None,
            // 手写 Default：派生会给 `false`，而生产默认要监听配置文件
            config_watch: true,
        }
    }
}

/// 启动器（无状态，方法即入口）。
#[derive(Debug)]
pub struct Runtime;

impl Runtime {
    /// 生产装配：未注入 echo 时按配置创建一个挂进状态（Go 侧 echo 是启动期全局单例）。
    ///
    /// 存储只占内存（懒消息池 / seq / SSM 队列 / msg_id 映射），不落盘；已注入时原样返回，
    /// 测试与嵌入场景可复用同一份存储。清理线程启动失败只 warn：过期映射不清理只影响内存。
    fn attach_echo(state: AppState) -> AppState {
        if state.echo().is_some() {
            return state;
        }
        let echo = EchoStore::new().with_ramdom_seq(state.config().snapshot().get_ramdom_seq());
        if let Err(err) = echo.start_cleanup_routine() {
            tracing::warn!(error = %err, "echo 清理线程启动失败，映射不会定期清理");
        }
        state.with_echo(Arc::new(echo))
    }

    /// 按当前配置快照启动全部启用的 listener。
    ///
    /// # Errors
    /// `ws_server_path` 非法或地址为空时返回 [`Error::Config`]；绑定失败时返回
    /// [`Error::Network`]。此时已绑定的 listener 随函数返回一起释放，不会留下半启动的服务。
    pub async fn start(state: AppState) -> Result<RuntimeHandle> {
        Self::start_with(state, RuntimeOptions::default()).await
    }

    /// 同 [`Runtime::start`]，允许覆盖绑定地址（测试与嵌入场景）。
    ///
    /// # Errors
    /// 同 [`Runtime::start`]。
    pub async fn start_with(state: AppState, options: RuntimeOptions) -> Result<RuntimeHandle> {
        let state = Self::attach_echo(state);
        let settings = state.config().snapshot();

        // 注册表在 `AppState` 装配时创建（内置 3 个 + `gensokyo_actions::register_all`）；
        // 这里只记录规模，`send_*` 的 deferred 计划由 dispatcher 在执行链上完成。
        tracing::info!(actions = state.registry().len(), "action 注册表已就绪");

        // Go 在启动时按 text_intent 算 intent 位并打印（main.go:316-353）；Rust 侧不维持
        // botgo WS 会话，这里只把解析结果落进日志（未知名字在 [`IntentPlan`] 内 warn）。
        let intent_plan = crate::intent::IntentPlan::from_settings(&settings);
        tracing::info!(
            bits = intent_plan.bits(),
            handlers = ?intent_plan.enabled_handlers(),
            "注册 intents"
        );

        // 先把「绑定地址 + 路由路径」全部算出来并校验，再统一绑定：非法 ws_server_path
        // 会让 `Router::route` panic，在这里提前变成可诊断的启动错误。
        let ws_target = if settings.get_enable_ws_server() {
            let path = crate::ws_server::route_path(settings.get_ws_server_path())?;
            let bind = options
                .ws_bind
                .unwrap_or_else(|| format!("0.0.0.0:{}", settings.get_port().trim()));
            Some((bind, path))
        } else {
            tracing::info!("enable_ws_server=false，正向 WS 未启动");
            None
        };

        let http_bind = if settings.get_http_address().trim().is_empty() {
            tracing::info!("http_address 为空，HTTP API 未启动");
            None
        } else {
            Some(
                options
                    .http_bind
                    .unwrap_or_else(|| settings.get_http_address().trim().to_owned()),
            )
        };

        // webhook 路径同样在绑定前校验：非法值会让 `Router::route` panic。
        let webhook = if settings.get_webhook_path().trim().is_empty() {
            tracing::info!("webhook_path 为空，webhook 未启用");
            None
        } else {
            let path =
                crate::ws_server::route_path_named("webhook_path", settings.get_webhook_path())?;
            Some(crate::webhook::router(state.clone(), &path))
        };

        // WebUI 在绑定前建好：路由表要它，会话清理任务也要它（两处共享同一个 `Arc`）。
        let webui = if state.webui_enabled() {
            Some(Arc::new(WebUiState::new(state.config_arc())))
        } else {
            tracing::info!("disable_webui=true，WebUI 未挂载");
            None
        };

        // 先把两个 listener 都绑好再 spawn：任一绑定失败时，已绑定的 socket 随局部变量
        // 释放关闭，不会出现「WS 已在跑、HTTP 启动失败」的半启动状态。
        let ws_listener = match &ws_target {
            Some((bind, _)) => Some(bind_listener("正向 WS", bind).await?),
            None => None,
        };
        let http_listener = match &http_bind {
            Some(bind) => Some(bind_listener("HTTP API", bind).await?),
            None => None,
        };

        let ws = if let (Some((_, path)), Some((listener, addr))) = (ws_target, ws_listener) {
            let app = merge_webui(
                merge_webhook(crate::ws_server::router(state.clone(), &path), &webhook),
                &webui,
            );
            Some(serve("正向 WS", listener, addr, app))
        } else {
            None
        };
        let http = http_listener.map(|(listener, addr)| {
            serve(
                "HTTP API",
                listener,
                addr,
                http_app(&state, &webui, &webhook),
            )
        });

        if webhook.is_some() && ws.is_none() && http.is_none() {
            tracing::warn!("webhook_path 已配置，但正向 WS 与 HTTP API 都没启动，回调没有监听端口");
        }

        // 反向 WS 是出站连接：与 listener 成败无关，地址非空就启动。
        let has_ws_address = settings
            .get_ws_address()
            .iter()
            .any(|address| !address.trim().is_empty());
        let ws_client = has_ws_address.then(|| WsClient::start(state.clone()));

        // 会话清理任务只在 WebUI 挂载时存在；句柄未 `shutdown` 就 drop 时由 `Drop` 中止。
        let webui_cleanup = webui.as_ref().map(spawn_session_cleanup);

        // 配置热重载：监听失败只 warn（运行期不该因配置文件监听挂掉整个进程），
        // 成功的句柄随 `RuntimeHandle` 一起关闭。
        let config_watch = if options.config_watch {
            spawn_config_watch(&state)
        } else {
            tracing::debug!("config_watch=false，配置热重载未启动");
            None
        };

        Ok(RuntimeHandle {
            ws,
            http,
            ws_client,
            hub: state.hub().clone(),
            webui_cleanup,
            config_watch,
        })
    }
}

/// 把 webhook 路由并进 listener 的路由表；没配 `webhook_path` 时原样返回。
///
/// webhook 只注册 `POST`，与 WS 的 `GET` / 健康检查的 `GET` 不冲突，可以安全 merge。
fn merge_webhook(app: Router, webhook: &Option<Router>) -> Router {
    match webhook {
        Some(webhook) => app.merge(webhook.clone()),
        None => app,
    }
}

/// 把 WebUI 路由并进 WS listener；未挂载时原样返回。
///
/// 这里不加 action 门：WS 端口没有 action 路由，未匹配的非 GET 请求由 WebUI 的静态回退
/// 回 405，保持 M1h-a 的「只有 HTTP API 端口收 action」。
///
/// `/health` 一并挂上：WebUI 的静态回退把 `/health` 当成 SPA 路由回首页，探活就再也拿不到
/// 文本。模板默认 `disable_webui=false` 且 `http_address` 为空，主 listener 正是这个 WS
/// 端口，所以这条健康路由不能只在 HTTP API 端口存在。
fn merge_webui(app: Router, webui: &Option<Arc<WebUiState>>) -> Router {
    match webui {
        Some(webui) => app
            .merge(WebUi::router(Arc::clone(webui)))
            .merge(crate::http_api::health_router()),
        None => app,
    }
}

/// HTTP API 端口的路由表：挂 WebUI 时 action 退居「`POST` 且非 `/api`」的门后。
///
/// 两个分支互斥的关键在 axum：`Router::merge` 不允许两边都带自定义回退，而 action 本身就是
/// 回退，所以挂了 WebUI 就不能再 merge [`crate::http_api::router`]，改由
/// [`crate::http_api::with_webui_router`] 组装（优先级说明见该函数）。
fn http_app(state: &AppState, webui: &Option<Arc<WebUiState>>, webhook: &Option<Router>) -> Router {
    let app = match webui {
        Some(webui) => {
            crate::http_api::with_webui_router(WebUi::router(Arc::clone(webui)), state.clone())
        }
        None => crate::http_api::router(state.clone()),
    };
    merge_webhook(app, webhook)
}

/// 后台清理过期 WebUI 会话；任务句柄存进 [`RuntimeHandle`]，随句柄停止。
fn spawn_session_cleanup(webui: &Arc<WebUiState>) -> JoinHandle<()> {
    let webui = Arc::clone(webui);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(WEBUI_CLEANUP_INTERVAL);
        // `interval` 的第一次 tick 立即返回：先消费掉，避免启动瞬间做无意义的清扫。
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let removed = webui.sessions().cleanup();
            if removed > 0 {
                tracing::debug!(removed, "WebUI 会话已过期清理");
            }
        }
    })
}

/// 启动配置热重载：文件监听线程 + 结果日志任务。
///
/// 监听启动失败只 `warn` 并返回 `None`：配置热重载是运行期增强，不该成为启动失败点；
/// 后续配置变更需要重启进程才能生效。
fn spawn_config_watch(state: &AppState) -> Option<ConfigWatch> {
    let (tx, rx) = tokio::sync::mpsc::channel(CONFIG_RELOAD_CHANNEL);
    let watcher = match ConfigWatcher::start(state.config_arc(), tx) {
        Ok(watcher) => watcher,
        Err(err) => {
            tracing::warn!(
                error = %err,
                "配置热重载监听启动失败，配置变更需重启进程生效"
            );
            return None;
        }
    };
    Some(ConfigWatch {
        watcher,
        task: tokio::spawn(log_reload_outcomes(rx)),
    })
}

/// 消费重载结果并落日志：热字段逐项 `info`，重启项逐项 `warn`。
async fn log_reload_outcomes(mut rx: Receiver<ReloadOutcome>) {
    while let Some(outcome) = rx.recv().await {
        if matches!(outcome, ReloadOutcome::Unchanged) {
            tracing::debug!("配置文件未变化");
        }
        for (level, field) in reload_log_entries(&outcome) {
            match level {
                ReloadLevel::Applied => tracing::info!(field, "配置已热重载，无需重启"),
                ReloadLevel::RestartRequired => {
                    tracing::warn!(field, "配置变更需重启生效，快照保持不变");
                }
            }
        }
    }
}

/// 重载结果的字段级日志分类（纯函数）：热字段 `Applied`、重启项 `RestartRequired`。
///
/// `Unchanged` 不产生条目；字段序沿用 [`ReloadOutcome`] 的升序。
fn reload_log_entries(outcome: &ReloadOutcome) -> Vec<(ReloadLevel, &str)> {
    match outcome {
        ReloadOutcome::Unchanged => Vec::new(),
        ReloadOutcome::Applied { changed } => changed
            .iter()
            .map(|tag| (ReloadLevel::Applied, tag.as_str()))
            .collect(),
        ReloadOutcome::RestartRequired { changed } => changed
            .iter()
            .map(|tag| (ReloadLevel::RestartRequired, tag.as_str()))
            .collect(),
    }
}

/// 日志级别分类：热字段走 `info`，重启项走 `warn`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReloadLevel {
    /// 已交换快照，即时生效。
    Applied,
    /// 含重启项，快照未交换。
    RestartRequired,
}

/// 配置热重载的两半：文件监听线程（阻塞句柄）+ 结果日志任务（tokio）。
#[derive(Debug)]
struct ConfigWatch {
    watcher: WatcherHandle,
    task: JoinHandle<()>,
}

/// 停止监听后等日志任务收尾的上限：监听线程正常退出后发送端随即 drop，
/// 任务在下一轮 `recv` 立刻返回；超过该时限说明监听线程未按期退出，直接中止。
const CONFIG_TASK_STOP_GRACE: Duration = Duration::from_secs(1);

impl ConfigWatch {
    /// 停止监听并等日志任务收尾。
    ///
    /// [`WatcherHandle::stop`] 在监听线程上短轮询（最长 2s），放进阻塞池执行，
    /// 避免占住异步 worker；监听线程退出后发送端 drop，日志任务读到 `None` 自然结束。
    async fn stop(self) {
        let Self { watcher, task } = self;
        if tokio::task::spawn_blocking(move || watcher.stop())
            .await
            .is_err()
        {
            tracing::warn!("配置监听线程停止任务异常退出");
        }
        // 监听线程未在 `stop` 的超时内退出时发送端仍存活，不能无限等它：
        // 有界等待后直接中止，保证关闭路径不会挂住。
        let abort = task.abort_handle();
        if tokio::time::timeout(CONFIG_TASK_STOP_GRACE, task)
            .await
            .is_err()
        {
            abort.abort();
        }
    }
}

/// 一个已启动的 listener。
struct ServerHandle {
    kind: &'static str,
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl fmt::Debug for ServerHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerHandle")
            .field("kind", &self.kind)
            .field("addr", &self.addr)
            .field("shutdown_armed", &self.shutdown.is_some())
            .finish_non_exhaustive()
    }
}

/// 运行时句柄：实际监听地址 + 反向 WS 客户端 + 事件总线 + 关闭入口。
#[derive(Debug)]
pub struct RuntimeHandle {
    ws: Option<ServerHandle>,
    http: Option<ServerHandle>,
    ws_client: Option<WsClientHandle>,
    hub: EventHub,
    /// WebUI 会话清理任务（未挂 WebUI 时为 `None`）。
    webui_cleanup: Option<JoinHandle<()>>,
    /// 配置热重载监听（`config_watch=false` 或监听启动失败时为 `None`）。
    config_watch: Option<ConfigWatch>,
}

impl RuntimeHandle {
    /// 正向 WS 的实际监听地址（未启动为 `None`）。
    #[must_use]
    pub fn ws_addr(&self) -> Option<SocketAddr> {
        self.ws.as_ref().map(|server| server.addr)
    }

    /// HTTP API 的实际监听地址（未启动为 `None`）。
    #[must_use]
    pub fn http_addr(&self) -> Option<SocketAddr> {
        self.http.as_ref().map(|server| server.addr)
    }

    /// 本次启动的反向 WS 地址数量（0 表示没有 `ws_address`）。
    #[must_use]
    pub fn ws_client_addresses(&self) -> usize {
        self.ws_client
            .as_ref()
            .map_or(0, WsClientHandle::address_count)
    }

    /// 事件总线：入站发布、反向 WS 订阅；外部也可订阅做旁路观测。
    #[must_use]
    pub fn hub(&self) -> &EventHub {
        &self.hub
    }

    /// 配置热重载是否已启动（`config_watch=false` 或监听启动失败时为 `false`）。
    #[must_use]
    pub fn config_watch_enabled(&self) -> bool {
        self.config_watch.is_some()
    }

    /// 优雅关闭：先停反向 WS，再停止接受新连接、等待存量请求处理完，最多等
    /// [`DEFAULT_SHUTDOWN_TIMEOUT`]。
    ///
    /// 升级后的 WS 长连接是否计入 graceful shutdown 取决于 hyper 版本（实测当前版本不阻塞、
    /// 立即返回），超时 + 强制中止仍然保留：任何版本上都不会让调用方无限挂住。需要立刻收摊
    /// 时用 [`RuntimeHandle::abort`]。
    pub async fn shutdown(self) {
        self.shutdown_with_timeout(DEFAULT_SHUTDOWN_TIMEOUT).await;
    }

    /// 同 [`RuntimeHandle::shutdown`]，自定义超时（测试用短超时避免拖慢套件）。
    pub async fn shutdown_with_timeout(mut self, limit: Duration) {
        // 先停出站客户端：否则 listener 收摊期间它们还在按退避重连。
        if let Some(client) = self.ws_client.take() {
            let addresses = client.address_count();
            client.shutdown(limit).await;
            tracing::debug!(addresses, "反向 WS 客户端已关闭");
        }

        // 会话清理是纯定时器：没有需要落盘的收尾，直接中止。
        if let Some(task) = self.webui_cleanup.take() {
            task.abort();
        }

        // 配置监听要显式 stop：监听线程等不到接收端关闭就会一直在 100ms 轮询里空转，
        // 仅 abort 日志任务不足以让它退出。`stop` 在阻塞池里等，最多 2s。
        if let Some(watch) = self.config_watch.take() {
            watch.stop().await;
            tracing::debug!("配置热重载监听已停止");
        }

        let mut servers: Vec<ServerHandle> = [self.ws.take(), self.http.take()]
            .into_iter()
            .flatten()
            .collect();
        let total = servers.len();

        // 先对全部 listener 发关停信号，再逐个 await：串行 await 会让后一个在前一个的
        // 长连接上白等，两个端口也就不是一个时间点开始收摊。
        for server in &mut servers {
            if let Some(notify) = server.shutdown.take() {
                let _ = notify.send(());
            }
        }

        // 超时后 `wait_all` 整体被丢弃，里面的 JoinHandle 等于 detach（任务继续跑、端口
        // 继续监听），所以必须另存 AbortHandle 兜底。
        let aborts: Vec<AbortHandle> = servers
            .iter()
            .map(|server| server.task.abort_handle())
            .collect();
        let wait_all = async move {
            for server in servers {
                let ServerHandle {
                    kind, addr, task, ..
                } = server;
                match task.await {
                    Ok(()) => tracing::debug!(%addr, kind, "服务已退出"),
                    Err(err) if err.is_cancelled() => {
                        tracing::debug!(%addr, kind, "服务任务已取消");
                    }
                    Err(err) => tracing::warn!(%addr, kind, error = %err, "服务任务异常退出"),
                }
            }
        };

        match tokio::time::timeout(limit, wait_all).await {
            Ok(()) => tracing::debug!(total, "全部服务已优雅退出"),
            Err(_) => {
                for abort in &aborts {
                    abort.abort();
                }
                tracing::warn!(total, ?limit, "优雅关闭超时，已强制中止残留服务任务");
            }
        }
    }

    /// 立即中止全部任务（不等存量连接）。
    pub fn abort(&self) {
        if let Some(client) = &self.ws_client {
            client.abort();
        }
        if let Some(task) = &self.webui_cleanup {
            task.abort();
        }
        // 监听线程由 `WatcherHandle` 的 `Drop` 停止；这里只先停日志任务。
        if let Some(watch) = &self.config_watch {
            watch.task.abort();
        }
        for server in [self.ws.as_ref(), self.http.as_ref()].into_iter().flatten() {
            server.task.abort();
        }
    }
}

impl Drop for RuntimeHandle {
    /// 句柄没走 `shutdown` 就被丢弃时兜底：不 abort 的话 listener 任务会一直活着，
    /// 端口继续监听而调用方无从关闭；反向 WS 任务也会继续重连。
    fn drop(&mut self) {
        if let Some(task) = self.webui_cleanup.take() {
            task.abort();
        }
        if let Some(watch) = self.config_watch.take() {
            tracing::warn!("RuntimeHandle 未 shutdown 即被 drop，已停止配置热重载监听");
            watch.task.abort();
            // `WatcherHandle::drop` 会停止监听线程（最多等 2s），避免线程泄漏。
            drop(watch.watcher);
        }
        if let Some(client) = self.ws_client.take() {
            tracing::warn!("RuntimeHandle 未 shutdown 即被 drop，已中止反向 WS 客户端");
            drop(client);
        }
        for server in [self.ws.take(), self.http.take()].into_iter().flatten() {
            if server.task.is_finished() {
                tracing::debug!(addr = %server.addr, kind = server.kind, "服务任务已结束，无需中止");
                continue;
            }
            server.task.abort();
            tracing::warn!(
                addr = %server.addr,
                kind = server.kind,
                "RuntimeHandle 未 shutdown 即被 drop，已中止监听任务"
            );
        }
    }
}

async fn bind_listener(kind: &'static str, raw: &str) -> Result<(TcpListener, SocketAddr)> {
    let bind = normalize_bind(raw);
    if bind.is_empty() {
        return Err(Error::Config(format!("{kind} 绑定地址为空")));
    }
    let listener = TcpListener::bind(&bind)
        .await
        .map_err(|err| Error::Network(format!("{kind} 监听 {bind} 失败: {err}")))?;
    let addr = listener
        .local_addr()
        .map_err(|err| Error::Network(format!("{kind} 读取监听地址失败: {err}")))?;
    Ok((listener, addr))
}

fn serve(kind: &'static str, listener: TcpListener, addr: SocketAddr, app: Router) -> ServerHandle {
    tracing::info!(%addr, kind, "服务已启动");
    let (shutdown, shutdown_rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let server = axum::serve(listener, app).with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        });
        if let Err(err) = server.await {
            tracing::error!(error = %err, kind, "服务异常退出");
        }
    });
    ServerHandle {
        kind,
        addr,
        shutdown: Some(shutdown),
        task,
    }
}

/// 仅端口号的配置（如 `port: "5700"`、`:5700`）按 `0.0.0.0:<port>` 绑定，与 Go
/// `"0.0.0.0:"+serverPort` 一致；其余地址原样交给 `TcpListener::bind`。
fn normalize_bind(raw: &str) -> String {
    let raw = raw.trim();
    if !raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return format!("0.0.0.0:{raw}");
    }
    // `:5700` 只给了端口，显式补 `0.0.0.0`：与 Go 的 `"0.0.0.0:"+port` 同义。
    if let Some(port) = raw
        .strip_prefix(':')
        .filter(|port| !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return format!("0.0.0.0:{port}");
    }
    raw.to_owned()
}

#[cfg(test)]
mod tests {
    use gensokyo_config::ReloadOutcome;

    use super::{ReloadLevel, normalize_bind, reload_log_entries};

    #[test]
    fn bare_port_becomes_wildcard_bind() {
        assert_eq!(normalize_bind("5700"), "0.0.0.0:5700");
        assert_eq!(normalize_bind(" 0 "), "0.0.0.0:0");
        assert_eq!(normalize_bind(":5700"), "0.0.0.0:5700");
        assert_eq!(normalize_bind(" :8080 "), "0.0.0.0:8080");
    }

    #[test]
    fn explicit_address_is_kept() {
        assert_eq!(normalize_bind("127.0.0.1:0"), "127.0.0.1:0");
        assert_eq!(normalize_bind("[::1]:8080"), "[::1]:8080");
        assert_eq!(normalize_bind(""), "");
        assert_eq!(normalize_bind(":"), ":");
    }

    #[test]
    fn reload_outcome_splits_hot_and_restart_fields() {
        let applied = ReloadOutcome::Applied {
            changed: vec!["bind_prefix".to_owned(), "nickname".to_owned()],
        };
        assert_eq!(
            reload_log_entries(&applied),
            vec![
                (ReloadLevel::Applied, "bind_prefix"),
                (ReloadLevel::Applied, "nickname"),
            ],
            "热字段全部走 info"
        );

        let restart = ReloadOutcome::RestartRequired {
            changed: vec!["log_level".to_owned(), "port".to_owned()],
        };
        assert_eq!(
            reload_log_entries(&restart),
            vec![
                (ReloadLevel::RestartRequired, "log_level"),
                (ReloadLevel::RestartRequired, "port"),
            ],
            "重启项全部走 warn"
        );

        assert!(
            reload_log_entries(&ReloadOutcome::Unchanged).is_empty(),
            "无变化不产生日志条目"
        );
    }

    /// 生产启动路径（`Runtime::start_with` 的第一步）按配置装配 echo，且不覆盖已注入的实例。
    #[test]
    fn attach_echo_injects_store_once() {
        use std::sync::Arc;

        use gensokyo_config::ConfigManager;

        use super::Runtime;
        use crate::state::AppState;

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings: {}\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));

        let state = Runtime::attach_echo(AppState::with_builtin_handlers(Arc::clone(&manager)));
        let injected = state.echo_arc().expect("启动路径应自动装配 echo");

        let state = Runtime::attach_echo(state);
        let kept = state.echo_arc().expect("echo 仍在");
        assert!(Arc::ptr_eq(&injected, &kept), "已注入时不应被替换");
    }
}
