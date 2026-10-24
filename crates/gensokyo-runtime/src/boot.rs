//! 任务树与启动器：把正向 WS / HTTP API 装配成可关闭的 [`RuntimeHandle`]。
//!
//! 绑定地址来源（对齐 Go `main.go`）：
//! - 正向 WS：`0.0.0.0:{port}`。Go 把 WS 挂在主服务器（`serverPort`）上；Rust 侧 M1h-a
//!   还没有主服务器与 WebUI 路由，先独立监听，后续里程碑合并。
//! - HTTP API：`http_address` 原样使用（空则不启动）。
//!
//! [`RuntimeOptions`] 可覆盖两者：测试用 `127.0.0.1:0` 取随机端口，避免端口冲突。
//!
//! listener 只在启动时创建：`http_address` 不是配置层的重启项，热改该字段会更新快照但
//! 不会迁移监听端口（与 Go 只在启动时 `ListenAndServe` 一致），需要新端口请重启进程。
//! token 与其余热字段在每次请求 / 每次握手时现读快照。
//!
//! 关闭语义见 [`RuntimeHandle::shutdown`]：先给全部 listener 发信号再逐个 await，整体
//! 受 [`DEFAULT_SHUTDOWN_TIMEOUT`] 约束；句柄未 `shutdown` 就被 drop 时由 `Drop` 兜底
//! 中止任务，避免端口继续监听。

use std::fmt;
use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use gensokyo_core::{Error, Result};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::{AbortHandle, JoinHandle};

use crate::state::AppState;

/// 优雅关闭的默认时限：超过它则强制中止残留任务（WS 长连接不会无限拖住退出）。
pub const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// 绑定地址覆盖；`None` 表示按配置（见模块文档）。
#[derive(Debug, Clone, Default)]
pub struct RuntimeOptions {
    /// 正向 WS 绑定地址覆盖。
    pub ws_bind: Option<String>,
    /// HTTP API 绑定地址覆盖。
    pub http_bind: Option<String>,
}

/// 启动器（无状态，方法即入口）。
#[derive(Debug)]
pub struct Runtime;

impl Runtime {
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
        let settings = state.config().snapshot();

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
            let app = crate::ws_server::router(state.clone(), &path);
            Some(serve("正向 WS", listener, addr, app))
        } else {
            None
        };
        let http = http_listener.map(|(listener, addr)| {
            serve(
                "HTTP API",
                listener,
                addr,
                crate::http_api::router(state.clone()),
            )
        });

        Ok(RuntimeHandle { ws, http })
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

/// 运行时句柄：实际监听地址 + 关闭入口。
#[derive(Debug)]
pub struct RuntimeHandle {
    ws: Option<ServerHandle>,
    http: Option<ServerHandle>,
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

    /// 优雅关闭：停止接受新连接，等待存量请求处理完，最多等 [`DEFAULT_SHUTDOWN_TIMEOUT`]。
    ///
    /// 升级后的 WS 长连接是否计入 graceful shutdown 取决于 hyper 版本（实测当前版本不阻塞、
    /// 立即返回），超时 + 强制中止仍然保留：任何版本上都不会让调用方无限挂住。需要立刻收摊
    /// 时用 [`RuntimeHandle::abort`]。
    pub async fn shutdown(self) {
        self.shutdown_with_timeout(DEFAULT_SHUTDOWN_TIMEOUT).await;
    }

    /// 同 [`RuntimeHandle::shutdown`]，自定义超时（测试用短超时避免拖慢套件）。
    pub async fn shutdown_with_timeout(mut self, limit: Duration) {
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

    /// 立即中止两个 listener 的任务（不等存量连接）。
    pub fn abort(&self) {
        for server in [self.ws.as_ref(), self.http.as_ref()].into_iter().flatten() {
            server.task.abort();
        }
    }
}

impl Drop for RuntimeHandle {
    /// 句柄没走 `shutdown` 就被丢弃时兜底：不 abort 的话 listener 任务会一直活着，
    /// 端口继续监听而调用方无从关闭。
    fn drop(&mut self) {
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
    use super::normalize_bind;

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
}
