//! 启动装配：`config.yml` → `ConfigManager` → `AppState` → 正向 WS / HTTP 运行时。
//!
//! 顺序对齐 Go `main.go`：
//! 1. `ensure_config_file`（首启按模板生成 `config.yml`）；
//! 2. `ConfigManager::new` 加载快照；
//! 3. `AppState::with_builtin_handlers`（内部即 `builtin::register_all`：`get_status` /
//!    `get_version_info` / `get_login_info`）；
//! 4. `Runtime::start` 按配置启动正向 WS 与正向 HTTP API；
//! 5. 打印**实际**监听地址（`127.0.0.1:0` 这类覆盖后的真实端口），Ctrl-C 后按
//!    [`DEFAULT_SHUTDOWN_TIMEOUT`] 优雅关闭。
//!
//! M1j 不连接 QQ、不启动反向 WS：`app_id` / `token` 等字段只进配置快照。

use std::path::Path;
use std::sync::Arc;

use gensokyo_config::{ConfigManager, LoadError, TemplateError, ensure_config_file};
use gensokyo_core::Error as CoreError;
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

/// 用已加载的配置启动运行时；`options` 可覆盖绑定地址（测试用 `127.0.0.1:0`）。
///
/// # Errors
/// `ws_server_path` 非法或绑定失败时返回 [`StartupError::Runtime`]。
pub async fn start_with_config(
    config: Arc<ConfigManager>,
    options: RuntimeOptions,
) -> Result<Started, StartupError> {
    let ws_path = config.snapshot().get_ws_server_path().trim().to_owned();
    let state = AppState::with_builtin_handlers(config);
    let handle = Runtime::start_with(state, options).await?;
    Ok(Started { handle, ws_path })
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
        tracing::info!("faststart：M1j 没有额外的初始化步骤，行为与 run 相同");
    }

    let started = start_with_config(loaded.config, RuntimeOptions::default()).await?;
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
    use std::net::SocketAddr;
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::time::timeout;

    use super::{address_lines, load_config, start_with_config};
    use gensokyo_runtime::RuntimeOptions;

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
        let http = started.handle.http_addr().expect("HTTP API 应已监听");
        let ws = started.handle.ws_addr().expect("正向 WS 应已监听");
        assert_ne!(http.port(), ws.port(), "两个 listener 不应共用端口");
        assert!(
            address_lines(&started)
                .iter()
                .any(|line| line.contains(&http.to_string())),
            "地址展示应含实际监听端口"
        );

        let response = http_get(http, "/").await;
        assert!(response.contains("200 OK"), "响应: {response}");
        assert!(
            response.contains("gensokyo-runtime: ok"),
            "健康检查正文: {response}"
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

        started
            .handle
            .shutdown_with_timeout(Duration::from_secs(5))
            .await;
    }
}
