//! 任务树与启动器：把正向 WS / HTTP API / webhook / 反向 WS / QQ 上游网关装配成可关闭的
//! [`RuntimeHandle`]。
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
//! - QQ 上游 WS 网关（[`crate::gateway`]）：凭据齐备时订阅 dispatch 事件并发布到事件总线；
//!   [`RuntimeOptions::gateway`] 默认开启，缺凭据或传输构建失败只 `warn` 跳过。
//!
//! 启动的第一步是 `Runtime::attach_echo`：状态里没有 echo 时按配置建一份（`ramdom_seq`
//! 跟随 [`gensokyo_config::Settings::get_ramdom_seq`]）并启动清理线程，供入站喂懒 message_id
//! 池、出站回写 msg_id 映射（Go 侧 echo 同样是启动期全局单例）。
//!
//! 紧接着是 `Runtime::attach_idmap`：状态里没有 idmap 时按配置打开身份库 / 消息库，挂进状态与
//! [`gensokyo_actions::ActionContext`]，入站虚拟 ID 与 `send_*` 的虚拟号反查才可用（Go 侧
//! `idmap.InitializeDB` 同样是启动期全局单例）。打开失败只 `warn`：Go 在同样情形 `Fatalf`
//! 退出，这里选择降级——入站回退原始 OpenID、虚拟号反查报 `1500`，比进程起不来好排查。
//!
//! 再往后是 `Runtime::attach_botstats`：状态里没有统计库时打开 `bot-stats.redb`
//! （[`DEFAULT_BOTSTATS_PATH`]；Go 侧 `botstats.InitializeDB`，`main.go:272`）。打开失败只
//! `warn`——当日收发计数不落盘、`get_status` 统计字段报 0，`/metrics` 的内存计数照常。
//!
//! 最后是 `Runtime::attach_bot_id`：装配了 QQ 客户端时向 `GET /users/@me` 取机器人真实 ID
//! （Go `main.go:254` 的 `api.Me` → `main.go:304` 的 `handlers.BotID = me.ID`），注入状态与
//! [`gensokyo_actions::ActionContext`]，入站 BotID→AppID、出站 AppID→BotID 的正文替换才生效。
//! 这是启动期唯一一次主动联网（含换 access_token）；失败只 `warn`，BotID 缺失时两侧都不替换
//! （Go 取不到 `Me` 时进 `nologin` 分支，整段初始化都跳过）。
//!
//! `lotus && lotus_grpc` 时还会起 idmap gRPC 服务端（tonic；Go `main_grpc.go`）：监听
//! `0.0.0.0:{lotus_grpc_port}`，注册与 Go 共用 `proto/idmap.proto` 的 17 个 RPC，句柄与两个
//! listener 共用同一套关闭语义。起来之后本地 `/getid` 按 Go `main.go:507` 隐藏；起不来
//! （idmap 未装配 / 端口被占）只 `warn` 并继续挂载本地路由，不让功能静默消失。
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
//! 关闭语义见 [`RuntimeHandle::shutdown`]：先停反向 WS 客户端与 QQ 上游网关（否则它们会在
//! listener 收摊期间继续重连），再给全部 listener 发信号并逐个 await，整体受
//! [`DEFAULT_SHUTDOWN_TIMEOUT`] 约束；句柄未 `shutdown` 就被 drop 时由 `Drop` 兜底
//! 中止任务，避免端口继续监听。

use std::fmt;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use gensokyo_config::{ConfigWatcher, ReloadOutcome, Settings, WatcherHandle};
use gensokyo_core::{Error, Result};
use gensokyo_qq::{ReqwestTransport, TokenConfig, TokenManager};
use gensokyo_storage::idmap::grpc::{IdMapService, pb};
use gensokyo_storage::idmap::{DEFAULT_IDENTITY_PATH, DEFAULT_MSG_PATH, IdMapOptions, IdMapStore};
use gensokyo_storage::short_url::{DEFAULT_SHORT_URL_PATH, ShortUrlStore};
use gensokyo_storage::{BotStatsStore, DEFAULT_BOTSTATS_PATH, EchoStore};
use gensokyo_webui::{WebUi, WebUiState};
use tokio::net::TcpListener;
use tokio::sync::mpsc::Receiver;
use tokio::sync::oneshot;
use tokio::task::{AbortHandle, JoinHandle};

use crate::event_hub::EventHub;
use crate::gateway::{GatewayClient, GatewayHandle, GatewayOptions};
use crate::reporter::{PostUrlReporter, PostUrlReporterHandle};
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
    /// 是否启动 QQ 上游 WS 网关订阅（默认 `true`；缺凭据时自动跳过，测试可关）。
    pub gateway: bool,
    /// 是否启动 `post_url` 事件上报（默认 `true`；配置为空时空转）。
    pub post_url: bool,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            ws_bind: None,
            http_bind: None,
            // 手写 Default：派生会给 `false`，而生产默认要监听配置文件
            config_watch: true,
            gateway: true,
            post_url: true,
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

    /// 生产装配：未注入 idmap 时按配置打开两个库，并挂进状态与 [`gensokyo_actions::ActionContext`]。
    ///
    /// Go 侧 idmap 是启动期全局单例（`idmap.InitializeDB`，打开失败直接 `Fatalf`）；这里失败
    /// 只 warn、不阻断启动，降级面见 [`Runtime::attach_idmap_with`]。已注入时原样返回，测试与
    /// 嵌入场景可复用同一份存储。
    fn attach_idmap(state: AppState) -> AppState {
        let options = idmap_options(&state.config().snapshot());
        Self::attach_idmap_with(state, options)
    }

    /// [`Runtime::attach_idmap`] 的实现主体（测试用它注入临时路径）。
    fn attach_idmap_with(state: AppState, options: IdMapOptions) -> AppState {
        if state.idmap().is_some() {
            return state;
        }
        let store = match IdMapStore::open(options) {
            Ok(store) => Arc::new(store),
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "idmap 打开失败，虚拟号互转不可用（入站回退原始 OpenID）"
                );
                return state;
            }
        };
        // handler 捕获的是注册时的 `Arc<ActionContext>`，换 idmap 只能整份重建再重注册；
        // 生产路径的注册集（内置 3 + `gensokyo_actions::register_all`）因此保持不变。
        let state = state.with_idmap(Arc::clone(&store));
        match state.actions_arc() {
            Some(actions) => {
                let actions = Arc::new(actions.as_ref().clone().with_idmap(store));
                state.with_action_context(actions)
            }
            None => state,
        }
    }

    /// 生产装配：未注入 botstats 时打开默认统计库并挂进状态（Go `botstats.InitializeDB`，
    /// `main.go:272`；文件名见 [`DEFAULT_BOTSTATS_PATH`]）。
    ///
    /// 打开失败只 warn、不阻断启动：当日收发计数不落盘、`get_status` 统计字段报 0，
    /// `/metrics` 的内存计数照常。已注入时原样返回，测试与嵌入场景可复用同一份存储。
    fn attach_botstats(state: AppState) -> AppState {
        Self::attach_botstats_with(state, DEFAULT_BOTSTATS_PATH)
    }

    /// [`Runtime::attach_botstats`] 的实现主体（测试用它注入临时路径）。
    fn attach_botstats_with(state: AppState, path: impl AsRef<Path>) -> AppState {
        if state.botstats().is_some() {
            return state;
        }
        match BotStatsStore::open(path) {
            Ok(store) => state.with_botstats(Arc::new(store)),
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "botstats 打开失败，收发统计不落盘（get_status 统计报 0，/metrics 照常）"
                );
                state
            }
        }
    }

    /// 启动装配：打开短链库（`short-urls.redb`），给 `/url` 的两条路由用。
    ///
    /// Go 在 `url.init()` 里打开 `gensokyo.db`，失败直接 `panic`（`shorturl.go:48-76`）；
    /// 这里只 `warn`：`/url` 回 500，其余功能照常启动。状态里已有库时原样返回（测试/嵌入
    /// 方先用 [`AppState::with_short_url`] 注入临时库）。
    ///
    /// `/url` 与 Go 一样**不鉴权**（`main.go:566-567` 直接挂 handler，只靠 `lotus_password`
    /// 的 token 规则），启动时提示一次，避免误把端口直接暴露到公网。
    fn attach_short_url(state: AppState) -> AppState {
        tracing::warn!("/url 无鉴权，与 Go 一致；请勿把该端口直接暴露到公网");
        if state.short_url().is_some() {
            return state;
        }
        match ShortUrlStore::open(DEFAULT_SHORT_URL_PATH) {
            Ok(store) => state.with_short_url(Arc::new(store)),
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "短链库打开失败，/url 不可用（发短链的调用会拿到空链接）"
                );
                state
            }
        }
    }

    /// 启动装配：向 `GET /users/@me` 取机器人真实 ID 并注入状态与
    /// [`gensokyo_actions::ActionContext`]（Go `main.go:254` 的 `api.Me` → `main.go:304`）。
    ///
    /// 未装配 QQ 客户端（无凭据）时不发请求；取失败只 `warn`：BotID 缺失时入站 / 出站都
    /// 不做 AppID⇄BotID 替换（Go 取不到 `Me` 时进 `nologin` 分支，整段初始化都跳过）。
    /// 已注入时原样返回。
    async fn attach_bot_id(state: AppState) -> AppState {
        if state.bot_id().is_some() {
            return state;
        }
        let Some(api) = state.api() else {
            return state;
        };
        match api.me().await {
            Ok(me) => {
                tracing::info!(bot_id = %me.id, "已获取机器人 ID");
                Self::inject_bot_id(state, &me.id)
            }
            Err(err) => {
                tracing::warn!(error = %err, "获取机器人 ID 失败，文本不做 AppID⇄BotID 替换");
                state
            }
        }
    }

    /// 把 BotID 写进状态并重建 [`gensokyo_actions::ActionContext`]：handler 捕获的是注册时的
    /// `Arc`，与 [`Runtime::attach_idmap_with`] 同一处理方式。
    fn inject_bot_id(state: AppState, bot_id: &str) -> AppState {
        let state = state.with_bot_id(bot_id);
        match state.actions_arc() {
            Some(actions) => {
                let actions = Arc::new(actions.as_ref().clone().with_bot_id(bot_id));
                state.with_action_context(actions)
            }
            None => state,
        }
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
        // 锚定 uptime 起点：Go 在包初始化时记 `mylog.StartTime`，这里贴近进程启动（幂等）。
        gensokyo_core::telemetry::metrics::mark_start();
        let state = Self::attach_echo(state);
        let state = Self::attach_idmap(state);
        let state = Self::attach_botstats(state);
        let state = Self::attach_short_url(state);
        let state = Self::attach_bot_id(state).await;
        let settings = state.config().snapshot();

        // Go `main.go:507`：`lotus_grpc` 打开时 idmap 由 gRPC 接管，本地 `/getid` 不再注册。
        // 是否接管看服务端**真的起来**没有——起不来就退回本地路由（Go 那里 `Fatalf`）。
        let idmap_grpc = match idmap_grpc_bind(&settings) {
            Ok(Some(bind)) => start_idmap_grpc(&state, &bind).await,
            Ok(None) => None,
            Err(reason) => {
                tracing::warn!(reason, "gRPC idmap 服务端不启动，/getid 保持挂载");
                None
            }
        };

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

        // `/uploadpic*` 与 `/getid`：Go 挂在主服务器 `r`，这里挂主 listener；HTTP 端口只在
        // 主 listener 不存在时兜底（见下方 `http` 分支）。
        let upload = crate::upload_http::router(state.clone(), idmap_grpc.is_none());

        // 短链库（Go `url.init()` 打开 `gensokyo.db`，失败 `panic`）：打不开只 warn，
        // `/url` 的 handler 回 500，其余功能照常——与各 `attach_*` 的口径一致。
        let short_url = crate::http_api::short_url_router(state.clone());

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

        // `/uploadpic*` 与 `/getid` 归主 listener（Go 的 `r`）；主 listener 不存在时才落 HTTP API
        // 端口。短链 `/url` 同理（Go `main.go:566-567` 也挂在 `r` 上）。
        let main_routes_on_http =
            upload_falls_back_to_http(ws_target.is_some(), http_listener.is_some());

        let ws = if let (Some((_, path)), Some((listener, addr))) = (ws_target, ws_listener) {
            let app = merge_webui(
                merge_webhook(crate::ws_server::router(state.clone(), &path), &webhook),
                &webui,
            );
            // Go `main.go:508/523-524` 的 /getid、/uploadpic* 挂在主服务器 `r` 上
            let app = merge_upload(app, &upload).merge(short_url.clone());
            Some(serve("正向 WS", listener, addr, app))
        } else {
            None
        };
        let http = http_listener.map(|(listener, addr)| {
            let app = http_app(&state, &webui, &webhook);
            // 主 listener（WS 端口，对应 Go 始终存在的 `r`）不存在时，上传端点与短链落到
            // HTTP API 端口，否则这些路由就没有监听者
            let app = if main_routes_on_http {
                merge_upload(app, &upload).merge(short_url.clone())
            } else {
                app
            };
            serve("HTTP API", listener, addr, app)
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

        // QQ 上游 WS 网关是入站链的源头：凭据齐备时订阅 dispatch 事件，缺凭据只 warn。
        let gateway = if options.gateway {
            start_gateway(&state)
        } else {
            tracing::info!("gateway=false，QQ 上游 WS 网关未启动");
            None
        };

        // post_url 事件上报：订阅总线后逐事件 POST（Go `BroadcastMessageToAll` 尾部的
        // `go PostMessageToUrls(message)`）。配置为空时任务空转，热重载后自动生效。
        let post_url = if options.post_url {
            Some(PostUrlReporter::start(state.clone()))
        } else {
            tracing::info!("post_url=false，事件上报未启动");
            None
        };

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
            idmap_grpc,
            ws_client,
            gateway,
            post_url,
            hub: state.hub().clone(),
            webui_cleanup,
            config_watch,
        })
    }
}

/// 进程级共享 HTTP 传输：首次成功构建后缓存，之后每次取用只 `Arc::clone`。四个调用点
/// （QQ 上游网关、action 客户端、post_url 上报、union_webhook 扇出）由此共用同一个
/// `reqwest::Client`，连接池与 TLS 会话缓存随之共享。
///
/// 失败**不写缓存**：`OnceLock` 只在成功路径被初始化，构建失败原样返回 `Err`，下次调用
/// 重新尝试。这样「这一次没装上 rustls provider」不会被固化成全进程状态，各调用点的降级
/// 语义（`warn` / `None` / 无 QQ 模式）保持原样。
///
/// # Errors
/// 与 [`ReqwestTransport::new`] 相同：未安装进程级 rustls provider，或 reqwest 客户端
/// 构建失败。
pub fn shared_transport() -> Result<Arc<ReqwestTransport>, gensokyo_qq::Error> {
    if let Some(cached) = SHARED_TRANSPORT.get() {
        return Ok(Arc::clone(cached));
    }
    #[cfg(test)]
    BUILD_ATTEMPTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let fresh = ReqwestTransport::new()?;
    // 并发首建时多个调用都能走到这里：`get_or_init` 只保留先到的那个，后到的丢掉自己这份，
    // 保证全进程只有一份底层客户端。
    Ok(Arc::clone(SHARED_TRANSPORT.get_or_init(|| Arc::new(fresh))))
}

/// 共享传输缓存：只存**成功**构建的实例（失败不落缓存，见 [`shared_transport`]）。
static SHARED_TRANSPORT: OnceLock<Arc<ReqwestTransport>> = OnceLock::new();

/// 测试专用的构建尝试计数：用来断言「失败的构建不缓存」。
#[cfg(test)]
static BUILD_ATTEMPTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 启动 QQ 上游 WS 网关（任务 #70）。
///
/// 凭据齐备时用 `ReqwestTransport` + [`TokenManager`] 装配；缺 `app_id` / `client_secret`
/// 或 TLS 传输建不起来时只 `warn` 跳过——入站链退回 webhook / 测试注入，不影响 WS/HTTP 启动。
fn start_gateway(state: &AppState) -> Option<GatewayHandle> {
    let settings = state.config().snapshot();
    let app_id = settings.get_app_id();
    let client_secret = settings.get_client_secret().trim();
    if app_id == 0 || client_secret.is_empty() {
        tracing::warn!(
            "未配置 app_id/client_secret，QQ 上游 WS 网关不订阅事件（入站事件不会到达）"
        );
        return None;
    }

    let transport = match shared_transport() {
        Ok(transport) => transport,
        Err(err) => {
            tracing::warn!(error = %err, "构建 QQ HTTP 传输失败，QQ 上游 WS 网关不启动");
            return None;
        }
    };
    let token_config = TokenConfig::new(app_id.to_string(), client_secret)
        .with_sandbox(settings.get_sandbox_mode());
    let tokens = Arc::new(TokenManager::new(Arc::clone(&transport), token_config));
    Some(
        GatewayClient::new(transport, tokens, state, GatewayOptions::default()).start(),
    )
}

/// idmap 的打开参数：AppID 参与 OpenID 的 key、`hash_id` 决定 vUIN 的派生方式、
/// `idmap_isolation` 开启时带 `uin` 前缀（对齐 Go `uinPrefix`）。
///
/// 路径没有配置项，用 `gensokyo_storage::idmap::DEFAULT_*`——与 Go 一样落在工作目录，
/// 只是扩展名 / 文件格式不同（redb vs bbolt），见该 crate 的差异清单。
fn idmap_options(settings: &Settings) -> IdMapOptions {
    let mut options = IdMapOptions::new(DEFAULT_IDENTITY_PATH, DEFAULT_MSG_PATH)
        .with_app_id(settings.get_app_id().to_string())
        .with_hash_id(settings.get_hash_id());
    if settings.get_idmap_isolation() {
        options = options.with_uin(settings.get_uin().to_string());
    }
    options
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

/// 把 `/uploadpic*` + `/getid` 并进 listener 的路由表。
///
/// 这些路由自带鉴权（`uploadAuth` / `IDMapAuthMiddleware`），必须放在 action 门之外合并：
/// 合并进已经 `layer(action_gate)` 的 HTTP 路由表，`POST /uploadpic` 才不会被 action 吞掉。
fn merge_upload(app: Router, upload: &Router) -> Router {
    app.merge(upload.clone())
}

/// 上传端点是否要落到 HTTP API 端口：只有主 listener（正向 WS 端口）缺席时才兜底。
///
/// Go 的主服务器 `r` 永远监听 `port`，`/uploadpic*` 与 `/getid` 都挂在它上面；Rust 的主
/// listener 由 `enable_ws_server` 决定，缺席时选 HTTP API 端口作为最近的等价物。
fn upload_falls_back_to_http(ws_listener: bool, http_listener: bool) -> bool {
    !ws_listener && http_listener
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
    /// idmap gRPC 服务端（仅 `lotus && lotus_grpc` 且绑定成功时存在）。
    idmap_grpc: Option<ServerHandle>,
    ws_client: Option<WsClientHandle>,
    /// QQ 上游 WS 网关（`gateway=false`、缺凭据或传输构建失败时为 `None`）。
    gateway: Option<GatewayHandle>,
    /// `post_url` 事件上报（`post_url=false` 或 HTTP 传输构建失败时为 `None`）。
    post_url: Option<PostUrlReporterHandle>,
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

    /// idmap gRPC 服务端的实际监听地址（未启用或绑定失败为 `None`）。
    #[must_use]
    pub fn idmap_grpc_addr(&self) -> Option<SocketAddr> {
        self.idmap_grpc.as_ref().map(|server| server.addr)
    }

    /// 本次启动的反向 WS 地址数量（0 表示没有 `ws_address`）。
    #[must_use]
    pub fn ws_client_addresses(&self) -> usize {
        self.ws_client
            .as_ref()
            .map_or(0, WsClientHandle::address_count)
    }

    /// QQ 上游 WS 网关是否已启动（`gateway=false`、缺凭据或传输构建失败时为 `false`）。
    #[must_use]
    pub fn gateway_started(&self) -> bool {
        self.gateway.is_some()
    }

    /// `post_url` 事件上报是否已启动（`post_url=false` 或 HTTP 传输构建失败时为 `false`）。
    ///
    /// 配置为空时任务仍在跑（空转），热重载新增 URL 后自动开始上报。
    #[must_use]
    pub fn post_url_started(&self) -> bool {
        self.post_url
            .as_ref()
            .is_some_and(PostUrlReporterHandle::is_running)
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

        // QQ 网关也是出站连接：先发关闭信号再收 listener。
        if let Some(gateway) = self.gateway.take() {
            gateway.shutdown(limit).await;
            tracing::debug!("QQ 上游 WS 网关已关闭");
        }

        // post_url 上报也是出站任务：先停它，避免 listener 收摊期间还在 POST。
        if let Some(reporter) = self.post_url.take() {
            reporter.shutdown(limit).await;
            tracing::debug!("post_url 事件上报已关闭");
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

        let mut servers: Vec<ServerHandle> =
            [self.ws.take(), self.http.take(), self.idmap_grpc.take()]
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
        if let Some(gateway) = &self.gateway {
            gateway.abort();
        }
        if let Some(reporter) = &self.post_url {
            reporter.abort();
        }
        if let Some(task) = &self.webui_cleanup {
            task.abort();
        }
        // 监听线程由 `WatcherHandle` 的 `Drop` 停止；这里只先停日志任务。
        if let Some(watch) = &self.config_watch {
            watch.task.abort();
        }
        for server in [
            self.ws.as_ref(),
            self.http.as_ref(),
            self.idmap_grpc.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
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
        if let Some(gateway) = self.gateway.take() {
            tracing::warn!("RuntimeHandle 未 shutdown 即被 drop，已中止 QQ 上游 WS 网关");
            drop(gateway);
        }
        if let Some(reporter) = self.post_url.take() {
            tracing::warn!("RuntimeHandle 未 shutdown 即被 drop，已中止 post_url 事件上报");
            drop(reporter);
        }
        for server in [self.ws.take(), self.http.take(), self.idmap_grpc.take()]
            .into_iter()
            .flatten()
        {
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

/// idmap gRPC 的监听地址（Go `main_grpc.go` 的 `net.Listen("tcp", ":"+lotus_grpc_port)`）。
///
/// 只在 `lotus && lotus_grpc` 时接管：Go 的同一开关会跳过本地 `/getid` 注册
/// （`main.go:507`）。端口非法时返回原因字符串——调用方 `warn` 后继续挂载 `/getid`，
/// 不让可选服务把启动路径拖住。
///
/// # 与 Go 的差异
///
/// Go `main_grpc.go:26-35` 在 `lotus=true` 时走的是**客户端**（连 `server_dir:port` 的父
/// 进程），只有 `lotus=false` 才 `grpcServer.Serve`（且阻塞主线程）。本仓库还没有 idmap
/// gRPC 客户端，因此这里改为起服务端：`/getid` 仍按 Go 隐藏，idmap 至少保留一条远程入口，
/// 不会随开关一起静默消失。
fn idmap_grpc_bind(settings: &Settings) -> Result<Option<String>, String> {
    if !(settings.get_lotus() && settings.get_lotus_grpc()) {
        return Ok(None);
    }
    let port = settings.get_lotus_grpc_port();
    let port = u16::try_from(port)
        .map_err(|_| format!("lotus_grpc_port={port} 不是合法端口（0~65535）"))?;
    // Go 的 `":port"` 监听全接口；这里显式写 `0.0.0.0`，与正向 WS 的绑定口径一致。
    Ok(Some(format!("0.0.0.0:{port}")))
}

/// 启动 idmap gRPC 服务端（Go `main_grpc.go:37-46`：`grpc.NewServer` +
/// `proto.RegisterIDMapServiceServer(grpcServer, &idmap.Server{})`）。
///
/// 返回 `None` 表示没起来（idmap 未装配 / 监听失败）：调用方据此继续挂载本地 `/getid`，
/// 否则被隐藏的这条路由会连同 gRPC 一起消失。Go 该分支监听失败直接 `Fatalf`；这里与其它
/// attach 口径一致，可选能力失败只 `warn`。
async fn start_idmap_grpc(state: &AppState, bind: &str) -> Option<ServerHandle> {
    let Some(store) = state.idmap_arc() else {
        tracing::warn!("idmap 未装配，gRPC idmap 服务端不启动（/getid 保持挂载）");
        return None;
    };
    let listener = match TcpListener::bind(bind).await {
        Ok(listener) => listener,
        Err(err) => {
            tracing::warn!(bind, error = %err, "gRPC idmap 服务端监听失败（/getid 保持挂载）");
            return None;
        }
    };
    let addr = match listener.local_addr() {
        Ok(addr) => addr,
        Err(err) => {
            tracing::warn!(bind, error = %err, "gRPC idmap 服务端读取监听地址失败（/getid 保持挂载）");
            return None;
        }
    };

    let (shutdown, shutdown_rx) = oneshot::channel::<()>();
    let service = pb::id_map_service_server::IdMapServiceServer::new(IdMapService::new(store));
    let task = tokio::spawn(async move {
        // 复用已绑定的 listener：端口冲突在启动期暴露，不必等 `serve` 内部报错再回退。
        // tonic 对 `Some(Err)` 只记日志并继续 accept，与 Go `grpcServer.Serve` 的重试一致。
        let incoming = futures::stream::unfold(listener, |listener| async move {
            match listener.accept().await {
                Ok((stream, _)) => Some((Ok::<_, std::io::Error>(stream), listener)),
                Err(err) => Some((Err(err), listener)),
            }
        });
        if let Err(err) = tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming_shutdown(incoming, async move {
                let _ = shutdown_rx.await;
            })
            .await
        {
            tracing::error!(error = %err, "gRPC idmap 服务端异常退出");
        }
    });
    tracing::info!(%addr, "idmap gRPC 服务端已启动");
    Some(ServerHandle {
        kind: "idmap gRPC",
        addr,
        shutdown: Some(shutdown),
        task,
    })
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
        // ConnectInfo 是 `/getid`「无密码只允许本地访问」与 per-IP 频控的唯一可信来源
        let server = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
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
    use std::sync::Arc;
    use std::time::Duration;

    use gensokyo_config::{ConfigManager, ReloadOutcome};
    use gensokyo_storage::idmap::grpc::IdMapClient;
    use gensokyo_storage::idmap::{IdMapOptions, IdMapStore};
    use tempfile::TempDir;
    use tonic::transport::Endpoint;

    use super::{
        AppState, ReloadLevel, idmap_grpc_bind, normalize_bind, reload_log_entries,
        start_idmap_grpc, upload_falls_back_to_http,
    };

    fn settings(directory: &TempDir, yaml: &str) -> Arc<gensokyo_config::Settings> {
        let path = directory.path().join("config.yml");
        std::fs::write(&path, format!("version: 1\nsettings:\n{yaml}")).expect("写配置");
        ConfigManager::new(&path).expect("读配置").snapshot()
    }

    /// 共享传输：构建失败**不落缓存**，下一次调用会重新尝试。
    ///
    /// 前提：本 crate 的测试进程不安装 rustls provider（只有 `gensokyo-bin` 装），所以两次
    /// 取用都必然失败——正好用来验证「一次失败不会被固化成全进程状态」。
    #[test]
    fn shared_transport_failure_is_not_cached() {
        let before = super::BUILD_ATTEMPTS.load(std::sync::atomic::Ordering::Relaxed);
        assert!(super::shared_transport().is_err(), "未装 provider 时应报错");
        assert!(
            super::shared_transport().is_err(),
            "失败不应被缓存，第二次仍走构建并报错"
        );
        let attempts =
            super::BUILD_ATTEMPTS.load(std::sync::atomic::Ordering::Relaxed) - before;
        assert!(attempts >= 2, "两次失败应各尝试构建一次，实际 {attempts} 次");
    }

    /// 配置开关：只有 `lotus && lotus_grpc` 才起 gRPC idmap 服务端。
    #[test]
    fn idmap_grpc_bind_follows_lotus_grpc_switch() {
        let directory = TempDir::new().expect("临时目录");
        assert_eq!(
            idmap_grpc_bind(&settings(&directory, "  server_dir: x\n")),
            Ok(None),
            "默认不起 gRPC"
        );
        assert_eq!(
            idmap_grpc_bind(&settings(&directory, "  lotus_grpc: true\n")),
            Ok(None),
            "lotus=false 不起 gRPC（Go 该分支会阻塞在 Serve）"
        );
        assert_eq!(
            idmap_grpc_bind(&settings(
                &directory,
                "  lotus: true\n  lotus_grpc: true\n  lotus_grpc_port: 9800\n"
            )),
            Ok(Some("0.0.0.0:9800".to_owned()))
        );
        assert!(
            idmap_grpc_bind(&settings(
                &directory,
                "  lotus: true\n  lotus_grpc: true\n  lotus_grpc_port: -1\n"
            ))
            .is_err(),
            "非法端口应返回原因而不是 panic"
        );
    }

    /// 起服务 + 一个 RPC 往返：`StoreIDV2` 分配、`RetrieveRowByIDV2` 反查，且与本地 store 同库。
    #[tokio::test(flavor = "multi_thread")]
    async fn idmap_grpc_serves_rpc_round_trip() {
        let directory = TempDir::new().expect("临时目录");
        let path = directory.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings:\n  app_id: 10000001\n").expect("写配置");
        let manager = Arc::new(ConfigManager::new(&path).expect("读配置"));
        let store = Arc::new(
            IdMapStore::open(
                IdMapOptions::new(
                    directory.path().join("identity.redb"),
                    directory.path().join("messages.redb"),
                )
                .with_app_id("10000001"),
            )
            .expect("打开 idmap"),
        );
        let state = AppState::with_builtin_handlers(manager).with_idmap(Arc::clone(&store));

        let handle = start_idmap_grpc(&state, "127.0.0.1:0")
            .await
            .expect("gRPC idmap 服务端应启动");
        let endpoint = Endpoint::from_shared(format!("http://{}", handle.addr)).expect("endpoint");
        let mut client = IdMapClient::connect(endpoint).await.expect("连接 gRPC");

        let row = client
            .store_idv2("openid-p28")
            .await
            .expect("StoreIDV2")
            .row;
        assert!(row > 0);
        let id = client
            .retrieve_row_by_idv2(row.to_string())
            .await
            .expect("RetrieveRowByIDV2")
            .id;
        assert_eq!(id, "openid-p28");
        // 服务端注入的就是本地那份 store，不是另开一个库
        assert_eq!(
            store.retrieve_row_by_id(row).expect("本地反查"),
            "openid-p28"
        );

        let notify = handle.shutdown.expect("关闭信号");
        let _ = notify.send(());
        tokio::time::timeout(Duration::from_secs(5), handle.task)
            .await
            .expect("服务端应在超时内退出")
            .expect("任务不应 panic");
    }

    #[test]
    fn bare_port_becomes_wildcard_bind() {
        assert_eq!(normalize_bind("5700"), "0.0.0.0:5700");
        assert_eq!(normalize_bind(" 0 "), "0.0.0.0:0");
        assert_eq!(normalize_bind(":5700"), "0.0.0.0:5700");
        assert_eq!(normalize_bind(" :8080 "), "0.0.0.0:8080");
    }

    #[test]
    fn upload_routes_prefer_the_main_listener() {
        // 主 listener 在场：只挂主 listener
        assert!(!upload_falls_back_to_http(true, true));
        assert!(!upload_falls_back_to_http(true, false));
        // 主 listener 缺席：HTTP API 端口兜底；两个都没有时无处可挂
        assert!(upload_falls_back_to_http(false, true));
        assert!(!upload_falls_back_to_http(false, false));
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

    /// 生产启动路径（`Runtime::start_with` 的第二步）按配置装配 idmap，且不覆盖已注入的实例。
    #[test]
    fn attach_idmap_injects_store_once() {
        use std::sync::Arc;

        use gensokyo_config::ConfigManager;
        use gensokyo_storage::idmap::IdMapOptions;

        use super::Runtime;
        use crate::state::AppState;

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings: {}\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        let options = IdMapOptions::new(dir.path().join("id.redb"), dir.path().join("msg.redb"))
            .with_app_id("102000001");

        let state = Runtime::attach_idmap_with(
            AppState::with_builtin_handlers(Arc::clone(&manager)),
            options.clone(),
        );
        let injected = state.idmap_arc().expect("启动路径应自动装配 idmap");

        let state = Runtime::attach_idmap_with(state, options);
        let kept = state.idmap_arc().expect("idmap 仍在");
        assert!(Arc::ptr_eq(&injected, &kept), "已注入时不应被替换");
    }

    /// 生产启动路径的统计库装配（`Runtime::start_with` 的第三步）：打开统计库，且不覆盖已注入的实例。
    #[test]
    fn attach_botstats_injects_store_once() {
        use std::sync::Arc;

        use gensokyo_config::ConfigManager;

        use super::Runtime;
        use crate::state::AppState;

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings: {}\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        let stats_path = dir.path().join("bot-stats.redb");

        let state = Runtime::attach_botstats_with(
            AppState::with_builtin_handlers(Arc::clone(&manager)),
            &stats_path,
        );
        let injected = state.botstats_arc().expect("启动路径应自动装配 botstats");

        let state = Runtime::attach_botstats_with(state, &stats_path);
        let kept = state.botstats_arc().expect("botstats 仍在");
        assert!(Arc::ptr_eq(&injected, &kept), "已注入时不应被替换");
    }

    /// 统计库打开失败只降级为「无 botstats」：不 panic、不阻断启动。
    #[test]
    fn attach_botstats_failure_keeps_state_without_store() {
        use std::sync::Arc;

        use gensokyo_config::ConfigManager;

        use super::Runtime;
        use crate::state::AppState;

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings: {}\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        // 父目录不存在：redb 建库失败，走 warn 降级分支。
        let missing = dir.path().join("missing").join("bot-stats.redb");

        let state =
            Runtime::attach_botstats_with(AppState::with_builtin_handlers(manager), missing);
        assert!(
            state.botstats().is_none(),
            "打开失败不应 panic，应降级为无 botstats"
        );
    }

    /// 注册期单元为空、启动期才填：已注册的 `get_status` 必须能读到装配后的统计
    /// （Go `botstats.InitializeDB` 晚于 handler 注册、`GetStats` 在请求时读全局 `db`）。
    #[tokio::test]
    async fn attach_botstats_reaches_registered_get_status() {
        use std::sync::Arc;

        use gensokyo_config::ConfigManager;
        use gensokyo_protocol::action::ActionRequest;

        use super::Runtime;
        use crate::state::AppState;

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings: {}\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));

        let state = Runtime::attach_botstats_with(
            AppState::with_builtin_handlers(manager),
            dir.path().join("bot-stats.redb"),
        );
        state
            .botstats()
            .expect("应已装配 botstats")
            .record_received()
            .expect("记收");

        let response = state
            .dispatcher()
            .dispatch(&ActionRequest {
                action: "get_status".to_owned(),
                params: None,
                echo: None,
            })
            .await;
        assert_eq!(response.retcode, 0);
        let data = response.data.expect("data");
        assert_eq!(data["stat"]["message_received"], 1);
    }

    /// 打开失败只降级为「无 idmap」：不 panic、不阻断启动（Go 在同样情形 `Fatalf`）。
    #[test]
    fn attach_idmap_failure_keeps_state_without_idmap() {
        use std::sync::Arc;

        use gensokyo_config::ConfigManager;
        use gensokyo_storage::idmap::IdMapOptions;

        use super::Runtime;
        use crate::state::AppState;

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings: {}\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        // 父目录不存在：redb 建库失败，走 warn 降级分支。
        let missing = dir.path().join("missing");
        let options = IdMapOptions::new(missing.join("id.redb"), missing.join("msg.redb"))
            .with_app_id("102000001");

        let state = Runtime::attach_idmap_with(AppState::with_builtin_handlers(manager), options);
        assert!(
            state.idmap().is_none(),
            "打开失败不应 panic，应降级为无 idmap"
        );
    }

    /// `hash_id` 随配置打开：vUIN 走 MD5 派生（9~18 位）且同 OpenID 稳定。
    #[test]
    fn attach_idmap_hash_id_follows_settings() {
        use std::sync::Arc;

        use gensokyo_config::ConfigManager;

        use super::Runtime;
        use crate::state::AppState;

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(
            &path,
            "version: 1\nsettings:\n  app_id: 102000001\n  hash_id: true\n",
        )
        .expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));

        let mut options = super::idmap_options(&manager.snapshot());
        assert_eq!(options.app_id, "102000001");
        assert!(options.hash_id, "hash_id 应随配置打开");
        options.identity_path = dir.path().join("id.redb");
        options.msg_path = dir.path().join("msg.redb");

        let state = Runtime::attach_idmap_with(AppState::with_builtin_handlers(manager), options);
        let store = state.idmap_arc().expect("idmap 应已装配");
        let openid = "0123456789abcdef0123456789abcdef";
        let (_, first) = store.retrieve_virtual_value(openid).expect("分配 vUIN");
        let (_, again) = store.retrieve_virtual_value(openid).expect("命中 vUIN");
        assert_eq!(first, again, "同一 OpenID 的 vUIN 必须稳定");
        assert!(
            first.to_string().len() >= 9,
            "hash_id 模式的 vUIN 是 9~18 位派生值，不应是顺序递增的 1: {first}"
        );
    }

    /// 已装配的 `ActionContext` 一并拿到 idmap，虚拟号反查可用。
    #[test]
    fn attach_idmap_reaches_action_context() {
        use std::sync::Arc;

        use gensokyo_actions::{ActionContext, QqApi};
        use gensokyo_config::ConfigManager;
        use gensokyo_qq::{
            Error as QqError, HttpRequest, HttpResponse, HttpTransport, OpenApiClient,
            Result as QqResult, TokenConfig, TokenManager,
        };
        use gensokyo_storage::idmap::IdMapOptions;

        use super::Runtime;
        use crate::state::AppState;

        /// 所有请求都直接失败的传输层：证明装配路径不触网。
        #[derive(Debug)]
        struct OfflineTransport;

        impl HttpTransport for OfflineTransport {
            fn request(
                &self,
                _req: HttpRequest,
            ) -> impl std::future::Future<Output = QqResult<HttpResponse>> + Send {
                std::future::ready(Err(QqError::Transport("测试环境不联网".to_owned())))
            }
        }

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings:\n  app_id: 102000001\n")
            .expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));

        let transport = Arc::new(OfflineTransport);
        let tokens = Arc::new(TokenManager::new(
            Arc::clone(&transport),
            TokenConfig::new("102000001", "s3cret"),
        ));
        let api = Arc::new(OpenApiClient::new(transport, tokens));
        let context = Arc::new(ActionContext::new(
            api as Arc<dyn QqApi>,
            Arc::clone(&manager),
        ));
        let state = AppState::with_actions(manager, context);
        assert!(
            state
                .actions()
                .expect("应有 ActionContext")
                .idmap()
                .is_none(),
            "装配前 ActionContext 不应有 idmap"
        );

        let options = IdMapOptions::new(dir.path().join("id.redb"), dir.path().join("msg.redb"))
            .with_app_id("102000001");
        let state = Runtime::attach_idmap_with(state, options);
        let store = state.idmap_arc().expect("idmap 应已装配");
        let openid = "0123456789abcdef0123456789abcdef";
        let (_, vuin) = store.retrieve_virtual_value(openid).expect("分配 vUIN");

        let context = state.actions().expect("ActionContext 应随状态保留");
        assert!(
            context.idmap().is_some(),
            "ActionContext 应拿到同一份 idmap"
        );
        assert_eq!(
            context
                .resolve_openid("user_id", &vuin.to_string(), "user_openid")
                .expect("虚拟号应能反查"),
            openid
        );
    }

    /// BotID 注入要同时落进状态与 `ActionContext`：出站正文替换读的是后者。
    #[test]
    fn inject_bot_id_reaches_state_and_action_context() {
        use std::sync::Arc;

        use gensokyo_actions::{ActionContext, QqApi};
        use gensokyo_config::ConfigManager;
        use gensokyo_qq::{
            Error as QqError, HttpRequest, HttpResponse, HttpTransport, OpenApiClient,
            Result as QqResult, TokenConfig, TokenManager,
        };

        use super::Runtime;
        use crate::state::AppState;

        /// 所有请求都直接失败的传输层：装配路径不触网。
        #[derive(Debug)]
        struct OfflineTransport;

        impl HttpTransport for OfflineTransport {
            fn request(
                &self,
                _req: HttpRequest,
            ) -> impl std::future::Future<Output = QqResult<HttpResponse>> + Send {
                std::future::ready(Err(QqError::Transport("测试环境不联网".to_owned())))
            }
        }

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings:\n  app_id: 102000001\n")
            .expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        let transport = Arc::new(OfflineTransport);
        let tokens = Arc::new(TokenManager::new(
            Arc::clone(&transport),
            TokenConfig::new("102000001", "s3cret"),
        ));
        let api = Arc::new(OpenApiClient::new(transport, tokens));
        let context = Arc::new(ActionContext::new(
            api as Arc<dyn QqApi>,
            Arc::clone(&manager),
        ));
        let state = AppState::with_actions(manager, context);
        assert!(state.bot_id().is_none(), "装配前状态里不应有 BotID");
        assert!(
            state
                .actions()
                .expect("应有 ActionContext")
                .bot_id()
                .is_none(),
            "装配前 ActionContext 不应有 BotID"
        );

        let state = Runtime::inject_bot_id(state, "2633000001");
        assert_eq!(state.bot_id(), Some("2633000001"));
        assert_eq!(
            state
                .actions()
                .expect("ActionContext 应随状态保留")
                .bot_id(),
            Some("2633000001"),
            "出站替换读的是 ActionContext 里的 BotID"
        );
    }
}
