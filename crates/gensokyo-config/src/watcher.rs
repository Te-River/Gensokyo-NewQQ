//! `notify` 文件监听：配置文件变动 → 去抖 → [`ConfigManager::reload`] → channel 上报。
//!
//! 监听的是**配置文件所在目录**而不是文件本身：多数编辑器保存时走"写临时文件 + rename"，
//! 直接监听文件会在 rename 后丢掉句柄，漏掉后续事件。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use notify::{Event, RecursiveMode, Watcher};
use thiserror::Error;
use tokio::sync::mpsc::Sender;
use tokio::sync::mpsc::error::TrySendError;
use tracing::{debug, info, warn};

use crate::manager::{ConfigManager, ReloadOutcome};

/// 去抖窗口：编辑器一次保存往往连发多个事件（写临时文件 + rename）。
const DEBOUNCE: Duration = Duration::from_millis(300);
/// 空闲轮询周期。`notify` 的事件通道是 std mpsc，无法与停止信号 select，
/// 因此用短轮询让 [`WatcherHandle::stop`] 在有限时间内结束线程。
const IDLE_POLL: Duration = Duration::from_millis(100);
/// `stop()`/`Drop` 等待线程退出的上限：正常不超过一个去抖窗口 + 一次 reload。
/// 超时后放弃 join，保证关闭路径不会挂住调用方。
const STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// 启动文件监听失败。
#[derive(Debug, Error)]
pub enum WatcherError {
    /// `notify` 初始化或注册监听失败。
    #[error("启动文件监听失败: {0}")]
    Notify(#[from] notify::Error),
    /// 监听线程创建失败。
    #[error("创建监听线程失败: {0}")]
    Thread(#[from] std::io::Error),
}

/// 配置文件监听器（无状态入口）。
#[derive(Debug)]
pub struct ConfigWatcher;

impl ConfigWatcher {
    /// 启动监听：配置文件变动后调用 `manager.reload()`，结果经 `tx` 上报。
    ///
    /// 只有目标文件的事件会触发重载；一个去抖窗口内的多个事件合并为一次 reload。
    pub fn start(
        manager: Arc<ConfigManager>,
        tx: Sender<ReloadOutcome>,
    ) -> Result<WatcherHandle, WatcherError> {
        let target = manager.path().to_path_buf();
        // 裸相对路径（`config.yml`）的 parent 是空串，按当前目录监听
        let dir = match target.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        };

        let (event_tx, event_rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            // 事件线程已退出时 send 会失败：监听器即将随线程一起销毁，无需额外处理
            let _ = event_tx.send(res);
        })?;
        watcher.watch(&dir, RecursiveMode::NonRecursive)?;

        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let watch_target = target.clone();
        let join = thread::Builder::new()
            .name("gensokyo-config-watch".to_owned())
            .spawn(move || {
                // watcher 移入线程：线程结束即注销监听
                let _watcher = watcher;
                run(&manager, &tx, &event_rx, &stop_flag, &watch_target);
            })?;

        info!(
            dir = %dir.display(),
            file = %target.display(),
            "配置文件监听已启动"
        );
        Ok(WatcherHandle {
            stop,
            join: Some(join),
        })
    }
}

/// 监听线程主循环。
fn run(
    manager: &ConfigManager,
    tx: &Sender<ReloadOutcome>,
    rx: &Receiver<notify::Result<Event>>,
    stop: &AtomicBool,
    target: &Path,
) {
    // 因 channel 满而尚未送达的重载结果。reload 已经交换了快照并推进 last_applied，
    // 丢掉它会让下一次事件 diff 得到 Unchanged，changed 标签永久丢失——必须补发。
    let mut pending: Option<ReloadOutcome> = None;

    while !stop.load(Ordering::SeqCst) {
        // 每个空闲轮询周期（IDLE_POLL）重试一次 try_send：消费端恢复读取后能补收历史
        // 变更，持续不读时也只是 100ms 尝试一次，不会 busy-loop。
        if let Some(outcome) = pending.take() {
            match tx.try_send(outcome) {
                Ok(()) => debug!("补发此前未送达的重载结果"),
                Err(TrySendError::Full(outcome)) => pending = Some(outcome),
                Err(TrySendError::Closed(_)) => {
                    info!("reload 接收端已关闭，监听线程退出");
                    break;
                }
            }
        }

        match rx.recv_timeout(IDLE_POLL) {
            Ok(Ok(event)) => {
                if !touch_target(&event, target) {
                    debug!(kind = ?event.kind, paths = ?event.paths, "忽略非目标文件事件");
                    continue;
                }
                // 去抖：等写盘落定，并吞掉同一批事件的其余部分
                thread::sleep(DEBOUNCE);
                while rx.try_recv().is_ok() {}
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                match manager.reload() {
                    Ok(outcome) => {
                        info!(?outcome, "配置文件变动，重载完成");
                        // 非阻塞发送：消费端停读 + channel 满时不能把监听线程挂死；
                        // 发不出去的结果留在 pending，由循环顶部按 IDLE_POLL 节奏重试
                        match tx.try_send(outcome) {
                            Ok(()) => {}
                            Err(TrySendError::Full(outcome)) => {
                                warn!("reload 接收端未消费、channel 已满，重载结果等待补发");
                                // 已有待补发的旧结果时，Unchanged 不携带新信息，不覆盖它
                                if pending.is_none()
                                    || !matches!(&outcome, ReloadOutcome::Unchanged)
                                {
                                    pending = Some(outcome);
                                }
                            }
                            Err(TrySendError::Closed(_)) => {
                                info!("reload 接收端已关闭，监听线程退出");
                                break;
                            }
                        }
                    }
                    // 解析失败时保留旧快照，等用户改好再触发下一次事件
                    Err(err) => warn!(error = %err, "配置文件变动但重载失败，保留旧快照"),
                }
            }
            Ok(Err(err)) => warn!(error = %err, "文件监听事件错误"),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    info!("配置文件监听已停止");
}

/// 事件是否命中目标配置文件。
fn touch_target(event: &Event, target: &Path) -> bool {
    event.paths.iter().any(|path| same_path(path, target))
}

/// 路径比较：父目录规范化后比较文件名。
///
/// 父目录先 `canonicalize` 解析符号链接：macOS 上 tempdir 给出 `/var/folders/...`，
/// 而 FSEvents 上报 `/private/var/folders/...`（`/var` 是符号链接），不解析就永远匹配不上。
/// 目标文件在 rename 期间可能暂不存在，因此**只**规范化父目录；两边路径仍可能相对/绝对混用。
/// `canonicalize` 失败时回退 `absolute`（不解析符号链接但不失败）。
///
/// Windows 文件名大小写不敏感（编辑器可能把 `Config.yml` 的 rename 事件报成 `config.yml`），
/// 因此只对文件名做 ASCII 大小写不敏感比较，目录部分保持精确。
fn same_path(a: &Path, b: &Path) -> bool {
    let (a_parent, b_parent) = match (resolve_parent(a), resolve_parent(b)) {
        (Some(a_parent), Some(b_parent)) => (a_parent, b_parent),
        _ => return absolute_eq(a, b),
    };
    if a_parent != b_parent {
        return false;
    }
    match (a.file_name(), b.file_name()) {
        (Some(a_name), Some(b_name)) if cfg!(windows) => a_name.eq_ignore_ascii_case(b_name),
        (Some(a_name), Some(b_name)) => a_name == b_name,
        // 路径没有文件名（根目录、`..` 等）：退回整路径比较
        _ => absolute_eq(a, b),
    }
}

/// 规范化路径的父目录：优先 `canonicalize`（解析符号链接），失败回退 `absolute`。
fn resolve_parent(path: &Path) -> Option<PathBuf> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    std::fs::canonicalize(parent)
        .or_else(|_| std::path::absolute(parent))
        .ok()
}

/// 整路径比较（父目录或文件名缺失时的退路）：转绝对路径后精确比较。
fn absolute_eq(a: &Path, b: &Path) -> bool {
    match (std::path::absolute(a), std::path::absolute(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// 监听句柄：`stop()` 或 `Drop` 都会停止线程并释放监听器。
#[derive(Debug)]
pub struct WatcherHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl WatcherHandle {
    /// 停止监听并等待线程退出。
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let Some(join) = self.join.take() else {
            return;
        };
        let deadline = Instant::now() + STOP_TIMEOUT;
        while !join.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        if join.is_finished() {
            if join.join().is_err() {
                warn!("配置文件监听线程异常退出");
            }
        } else {
            // 线程会在下一个检查点自行退出；这里不再阻塞调用方
            warn!("配置文件监听线程未在超时内退出，放弃等待");
        }
    }
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}
