//! 启动装配：`config.yml` → `ConfigManager` → `AppState` → 正向 WS / HTTP 运行时。
//!
//! 顺序对齐 Go `main.go`：
//! 1. `ensure_config_file`（首启按模板生成 `config.yml`）；
//! 2. `ConfigManager::new` 加载快照；
//! 3. 装配 [`AppState`]（见 [`build_state`]）：有 `app_id` + `client_secret` 时装 QQ 客户端与
//!    action 全集（47 个注册名 = 内置 3 + `gensokyo-actions` 41 + 别名的 3），否则降级为
//!    内置 3 个；
//! 4. `Runtime::start` 按配置启动正向 WS 与正向 HTTP API；
//! 5. 打印 action 数量与**实际**监听地址（`127.0.0.1:0` 这类覆盖后的真实端口），Ctrl-C 后按
//!    [`DEFAULT_SHUTDOWN_TIMEOUT`] 优雅关闭。
//!
//! M1j 只到「能发请求的客户端 + 注册表」：启动期不联网（access_token 在首次调用上游时才惰性
//! 换取），不启动反向 WS / webhook，不装配 idmap / echo —— 无 idmap 时 [`ActionContext`]
//! 只接受 32 位 OpenID，虚拟号互转留给 M1h-b。

use std::path::Path;
use std::sync::Arc;

use gensokyo_actions::{ActionContext, QqApi};
use gensokyo_config::{ConfigManager, LoadError, Settings, TemplateError, ensure_config_file};
use gensokyo_core::Error as CoreError;
use gensokyo_qq::{HttpTransport, OpenApiClient, ReqwestTransport, TokenConfig, TokenManager};
use gensokyo_runtime::{
    AppState, DEFAULT_SHUTDOWN_TIMEOUT, Runtime, RuntimeHandle, RuntimeOptions,
};
use thiserror::Error;

use crate::logging;

/// 默认配置文件名（与 Go 的工作目录约定一致）。
pub const CONFIG_FILE: &str = "config.yml";

/// 启动失败的原因。
#[derive(Debug, Error)]
pub enum StartupError {
    /// 首启生成配置模板失败。
    #[error("生成配置模板失败: {0}")]
    Template(#[from] TemplateError),
    /// 配置解析/读取失败。
    #[error("加载配置失败: {0}")]
    Load(#[from] LoadError),
    /// 绑定端口或校验路由路径失败。
    #[error("启动运行时失败: {0}")]
    Runtime(#[from] CoreError),
    /// 注册 Ctrl-C 处理器失败。
    #[error("监听 Ctrl-C 失败: {0}")]
    Signal(#[source] std::io::Error),
}

/// 加载后的配置。
#[derive(Debug)]
pub struct LoadedConfig {
    /// 配置快照管理器。
    pub config: Arc<ConfigManager>,
    /// 本次是否新建了 `config.yml`（首启）。
    pub created: bool,
}

/// 已启动的运行时。
#[derive(Debug)]
pub struct Started {
    /// 运行时句柄（关闭入口 + 实际监听地址）。
    pub handle: RuntimeHandle,
    /// 正向 WS 的路由路径（未启用时为空串）。
    pub ws_path: String,
    /// 注册表里的 action 名数量（含别名）：无 QQ 凭据时是 3，装配齐全时是 47。
    pub actions: usize,
    /// 是否装配了 QQ 客户端（即 `AppState::with_actions` 生效）。
    pub qq: bool,
}

/// 读取（必要时按模板生成）`config.yml`。
///
/// # Errors
/// 模板生成或配置解析失败时返回 [`StartupError`]。
pub fn load_config(path: impl AsRef<Path>) -> Result<LoadedConfig, StartupError> {
    let path = path.as_ref();
    let created = ensure_config_file(path)?;
    let config = Arc::new(ConfigManager::new(path)?);
    Ok(LoadedConfig { config, created })
}

/// 安装进程级 rustls provider（ring）。
///
/// `gensokyo-qq` / `gensokyo-media` 只启用 `rustls-no-provider`，provider 由二进制注入。
/// 返回**本次调用**是否新装了 provider：已装好时直接返回 `false`（`install_default` 在已有
/// provider 时会失败，先查再装即可让重复调用幂等）。安装失败只 warn、不 panic——后续
/// [`ReqwestTransport::new`] 会自行报错并触发无 QQ 降级。
#[must_use]
pub fn install_crypto_provider() -> bool {
    if rustls::crypto::CryptoProvider::get_default().is_some() {
        return false;
    }
    match rustls::crypto::ring::default_provider().install_default() {
        Ok(()) => {
            tracing::debug!("已安装 rustls provider: ring");
            true
        }
        Err(_) => {
            tracing::warn!("rustls provider 未能安装（已被其他组件占用），沿用现有 provider");
            false
        }
    }
}

/// 从配置快照提取 QQ 凭据；`app_id == 0` 或 `client_secret` 全空白视为未配置。
///
/// `token`（机器人令牌）不参与 OpenAPI v2 鉴权（上游用 `app_id` + `client_secret` 换
/// access_token），所以不作为装配条件；`develop_access_token_dir` 暂无访问器，token 端点
/// 固定官方地址。这里只做**非空**校验，不验证凭据真伪——占位符同样算已配置，无效凭据要到
/// 首次调用上游时才以 action 错误（`1500`）暴露。
fn token_config(snapshot: &Settings) -> Option<TokenConfig> {
    let app_id = snapshot.get_app_id();
    let client_secret = snapshot.get_client_secret().trim();
    if app_id == 0 || client_secret.is_empty() {
        return None;
    }
    Some(
        TokenConfig::new(app_id.to_string(), client_secret)
            .with_sandbox(snapshot.get_sandbox_mode()),
    )
}

/// 用给定传输层装配 QQ 客户端与 action 上下文。
///
/// 泛型参数只为测试注入假传输层（不联网）；生产路径见 [`build_state`]。
fn qq_client<T: HttpTransport + 'static>(
    config: Arc<ConfigManager>,
    transport: Arc<T>,
    tokens: TokenConfig,
) -> (Arc<OpenApiClient<T>>, Arc<ActionContext>) {
    let sandbox = tokens.sandbox();
    let tokens = Arc::new(TokenManager::new(Arc::clone(&transport), tokens));
    let api = Arc::new(OpenApiClient::new(transport, tokens).with_sandbox(sandbox));
    let context = Arc::new(ActionContext::new(
        Arc::clone(&api) as Arc<dyn QqApi>,
        config,
    ));
    (api, context)
}

/// 按配置装配 [`AppState`]：有 QQ 凭据就装 action 全集，否则降级为内置 3 个。
///
/// 降级只 warn、不返回错误：WS / HTTP 照常启动，`send_*` 因未注册直接回 `1404`，
/// `get_login_info` 只回显配置里的 `user_id` / `nickname`（不代表 QQ 登录态）。
#[must_use]
pub fn build_state(config: Arc<ConfigManager>) -> AppState {
    // provider 只需装一次；已装过时返回 false 属正常，这里不关心返回值。
    let _ = install_crypto_provider();
    let Some(tokens) = token_config(&config.snapshot()) else {
        tracing::warn!(
            "未配置 QQ 凭据（app_id=0 或 client_secret 为空），仅启动 WS/HTTP；QQ action 不注册"
        );
        return AppState::with_builtin_handlers(config);
    };
    let transport = match ReqwestTransport::new() {
        Ok(transport) => Arc::new(transport),
        Err(err) => {
            tracing::warn!(
                error = %err,
                "构建 QQ HTTP 传输失败，降级为无 QQ 模式（仅内置 action）"
            );
            return AppState::with_builtin_handlers(config);
        }
    };
    let (api, context) = qq_client(Arc::clone(&config), transport, tokens);
    AppState::with_actions(config, context).with_api(api)
}

/// 用已加载的配置启动运行时；`options` 可覆盖绑定地址（测试用 `127.0.0.1:0`）。
///
/// # Errors
/// `ws_server_path` 非法或绑定失败时返回 [`StartupError::Runtime`]。
pub async fn start_with_config(
    config: Arc<ConfigManager>,
    options: RuntimeOptions,
) -> Result<Started, StartupError> {
    let ws_path = config.snapshot().get_ws_server_path().trim().to_owned();
    let state = build_state(config);
    let actions = state.registry().len();
    let qq = state.actions().is_some();
    let handle = Runtime::start_with(state, options).await?;
    Ok(Started {
        handle,
        ws_path,
        actions,
        qq,
    })
}

/// 实际监听地址的展示文本（一行一个 listener）。
#[must_use]
pub fn address_lines(started: &Started) -> Vec<String> {
    let mut lines = Vec::with_capacity(2);
    match started.handle.ws_addr() {
        Some(addr) => lines.push(format!(
            "正向 WS 监听: {addr}/{}",
            started.ws_path.trim_start_matches('/')
        )),
        None => lines.push("正向 WS 未启动（enable_ws_server=false）".to_owned()),
    }
    match started.handle.http_addr() {
        Some(addr) => lines.push(format!("HTTP API 监听: {addr}")),
        None => lines.push("HTTP API 未启动（http_address 为空）".to_owned()),
    }
    lines
}

/// `gensokyo` / `gensokyo run` 主链：装配 → 打印监听地址 → 等 Ctrl-C → 带超时关闭。
///
/// # Errors
/// 配置、绑定或信号注册失败时返回 [`StartupError`]；此时不会留下监听端口。
pub async fn serve(faststart: bool) -> Result<(), StartupError> {
    let loaded = load_config(CONFIG_FILE)?;
    // 日志级别来自 config.yml，所以要在配置加载之后初始化；`RUST_LOG` 优先。
    logging::init(loaded.config.snapshot().get_log_level());
    if loaded.created {
        tracing::info!(path = CONFIG_FILE, "config.yml 不存在，已按模板生成");
    }
    if faststart {
        tracing::info!("faststart：没有可跳过的额外初始化步骤，行为与 run 相同");
    }

    let started = start_with_config(loaded.config, RuntimeOptions::default()).await?;
    println!(
        "已注册 action: {} 个（{}）",
        started.actions,
        if started.qq {
            "QQ 客户端已装配"
        } else {
            "未装配 QQ 客户端，仅内置 action"
        }
    );
    for line in address_lines(&started) {
        println!("{line}");
    }
    if started.handle.ws_addr().is_none() && started.handle.http_addr().is_none() {
        tracing::warn!(
            "没有任何 listener 启动：检查 config.yml 的 enable_ws_server 与 http_address"
        );
    }
    tracing::info!("按 Ctrl-C 退出");

    tokio::signal::ctrl_c()
        .await
        .map_err(StartupError::Signal)?;
    tracing::info!("收到 Ctrl-C，开始优雅关闭（超时 {DEFAULT_SHUTDOWN_TIMEOUT:?}）");
    started
        .handle
        .shutdown_with_timeout(DEFAULT_SHUTDOWN_TIMEOUT)
        .await;
    tracing::info!("运行时已关闭");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::time::Duration;

    use gensokyo_config::ConfigManager;
    use gensokyo_qq::{
        Error as QqError, HttpRequest, HttpResponse, HttpTransport, Result as QqResult,
    };
    use gensokyo_runtime::{AppState, RuntimeOptions};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::time::timeout;

    use super::{
        address_lines, build_state, install_crypto_provider, load_config, qq_client,
        start_with_config, token_config,
    };

    /// 两个 listener 都启用；绑定地址由 `RuntimeOptions` 覆盖成随机端口。
    const CONFIG: &str = r#"version: 1
settings:
  port: "5700"
  enable_ws_server: true
  ws_server_path: ws
  ws_server_token: ""
  http_address: "127.0.0.1:1"
  http_access_token: ""
"#;

    fn loopback_options() -> RuntimeOptions {
        RuntimeOptions {
            ws_bind: Some("127.0.0.1:0".to_owned()),
            http_bind: Some("127.0.0.1:0".to_owned()),
        }
    }

    /// 加载（必要时生成 `config.yml`）+ 启动；端口压到 `127.0.0.1:0`。
    async fn start_at(path: &std::path::Path) -> super::Started {
        let loaded = load_config(path).expect("加载配置失败");
        start_with_config(loaded.config, loopback_options())
            .await
            .expect("启动运行时失败")
    }

    async fn http_get(addr: SocketAddr, path: &str) -> String {
        let mut stream = TcpStream::connect(addr).await.expect("连接 HTTP API 失败");
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .expect("写请求失败");
        let mut response = String::new();
        // `Connection: close`：服务端回完即关，读到 EOF 为止。
        timeout(
            Duration::from_secs(10),
            stream.read_to_string(&mut response),
        )
        .await
        .expect("读取响应超时")
        .expect("读响应失败");
        response
    }

    #[tokio::test]
    async fn runtime_starts_and_answers_health_check() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, CONFIG).expect("写测试配置失败");

        let started = start_at(&path).await;
        assert_eq!(started.ws_path, "ws", "应读取 ws_server_path");
        assert!(!started.qq, "测试配置没有 QQ 凭据，应走降级路径");
        assert_eq!(started.actions, 3, "无凭据时只注册内置 3 个 action");
        let http = started.handle.http_addr().expect("HTTP API 应已监听");
        let ws = started.handle.ws_addr().expect("正向 WS 应已监听");
        assert_ne!(http.port(), ws.port(), "两个 listener 不应共用端口");
        assert!(
            address_lines(&started)
                .iter()
                .any(|line| line.contains(&http.to_string())),
            "地址展示应含实际监听端口"
        );

        // 探活固定走独立路径：WebUI 挂载后 `/` 是前端首页。
        let response = http_get(http, "/health").await;
        assert!(response.contains("200 OK"), "响应: {response}");
        assert!(
            response.contains("gensokyo-runtime: ok"),
            "健康检查正文: {response}"
        );
        assert!(
            response
                .to_ascii_lowercase()
                .contains("content-type: text/plain"),
            "健康检查应回 text/plain: {response}"
        );

        started
            .handle
            .shutdown_with_timeout(Duration::from_secs(5))
            .await;
        match timeout(Duration::from_secs(5), TcpStream::connect(http)).await {
            Ok(Err(_)) => {} // 连接被拒：端口已释放
            Ok(Ok(_)) => panic!("关闭后端口仍在监听: {http}"),
            Err(_) => panic!("关闭后连接一直挂起: {http}"),
        }
    }

    #[tokio::test]
    async fn missing_config_is_generated_then_started() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");

        let started = start_at(&path).await;
        assert!(path.exists(), "首启应写出 config.yml");
        let written = std::fs::read_to_string(&path).expect("读回 config.yml 失败");
        // 模板里的 <YOUR_SERVER_DIR> 会被替换成本机 IP，故只做结构性断言。
        assert!(written.contains("settings:"), "生成内容应含 settings 段");
        assert!(
            written.len() > gensokyo_config::CONFIG_TEMPLATE.len() / 2,
            "生成内容不应是半截文件"
        );
        assert!(
            started.handle.ws_addr().is_some(),
            "模板默认 enable_ws_server=true，WS 应已监听"
        );
        assert!(
            started.handle.http_addr().is_none(),
            "模板默认 http_address 为空，不应启动 HTTP API"
        );

        // 模板 disable_webui=false 且 ws_server_path=ws：WebUI 占着 WS 端口的 `/`，
        // 探活同样只能走 `/health`。
        let ws = started.handle.ws_addr().expect("WS 应已监听");
        let response = http_get(ws, "/health").await;
        assert!(response.contains("200 OK"), "响应: {response}");
        assert!(
            response.contains("gensokyo-runtime: ok"),
            "健康检查正文: {response}"
        );

        started
            .handle
            .shutdown_with_timeout(Duration::from_secs(5))
            .await;
    }

    /// 带完整凭据的配置（不联网：传输层在下面换成假实现）。
    const CREDENTIALED: &str = r#"version: 1
settings:
  app_id: 102000001
  client_secret: s3cret
  sandbox_mode: false
  enable_ws_server: false
  http_address: ""
"#;

    /// 所有请求都直接失败的传输层：证明装配路径不触网。
    #[derive(Debug)]
    struct OfflineTransport;

    impl HttpTransport for OfflineTransport {
        fn request(
            &self,
            _req: HttpRequest,
        ) -> impl Future<Output = QqResult<HttpResponse>> + Send {
            std::future::ready(Err(QqError::Transport("测试环境不联网".to_owned())))
        }
    }

    /// 写临时 `config.yml` 并加载快照（快照已读入内存，临时目录可随函数返回删除）。
    fn config_manager(yaml: &str) -> Arc<ConfigManager> {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, yaml).expect("写测试配置失败");
        Arc::new(ConfigManager::new(&path).expect("加载配置失败"))
    }

    #[test]
    fn provider_install_is_idempotent() {
        // 同进程其他测试可能先装过：只断言重复调用安全、且最终能拿到 provider。
        let _ = install_crypto_provider();
        assert!(
            !install_crypto_provider(),
            "重复安装应复用已有 provider 并返回 false"
        );
        assert!(
            rustls::crypto::CryptoProvider::get_default().is_some(),
            "安装后应能取到 provider"
        );
    }

    #[test]
    fn credentials_need_non_zero_app_id_and_secret() {
        let no_app_id =
            gensokyo_config::parse_str("app_id: 0\nclient_secret: s3cret\n").expect("解析配置失败");
        assert!(token_config(&no_app_id).is_none(), "app_id=0 应视为未配置");

        let blank_secret =
            gensokyo_config::parse_str("app_id: 102000001\nclient_secret: \"   \"\n")
                .expect("解析配置失败");
        assert!(
            token_config(&blank_secret).is_none(),
            "空白 client_secret 应视为未配置"
        );

        let configured = gensokyo_config::parse_str("app_id: 102000001\nclient_secret: s3cret\n")
            .expect("解析配置失败");
        assert!(
            token_config(&configured).is_some(),
            "app_id + client_secret 齐全应可装配"
        );
    }

    #[test]
    fn missing_credentials_keep_only_builtin_actions() {
        let state = build_state(config_manager(
            "version: 1\nsettings:\n  enable_ws_server: false\n  http_address: \"\"\n",
        ));
        assert!(state.actions().is_none(), "无凭据不应装配 ActionContext");
        assert_eq!(state.registry().len(), 3, "降级后只剩内置 3 个 action");
    }

    #[test]
    fn credentials_wire_action_context_without_idmap() {
        let manager = config_manager(CREDENTIALED);
        let tokens = token_config(&manager.snapshot()).expect("凭据齐全应可装配");
        assert_eq!(tokens.app_id(), "102000001");

        let (api, context) = qq_client(Arc::clone(&manager), Arc::new(OfflineTransport), tokens);
        assert_eq!(
            api.base_url(),
            gensokyo_qq::DEFAULT_BASE_URL,
            "sandbox_mode=false 应走正式基址"
        );
        assert!(
            context.idmap().is_none(),
            "M1j 不装配 idmap：只接受 32 位 OpenID"
        );

        let state = AppState::with_actions(manager, context);
        assert_eq!(
            state.registry().len(),
            gensokyo_actions::IMPLEMENTED_ACTIONS.len() + 3 + 3,
            "内置 3 + IMPLEMENTED_ACTIONS + 别名 3"
        );
        assert!(state.actions().is_some(), "ActionContext 应随状态一起挂上");
    }
}
