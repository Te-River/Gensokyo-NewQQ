//! `ArcSwap` 配置快照与热重载。
//!
//! 快照只在"变更不含重启项"时原子交换，读路径（[`ConfigManager::snapshot`]）永远无锁；
//! 含重启项的变更原样上报 [`ReloadOutcome::RestartRequired`]，**不自行重启进程**——是否
//! 重启由 runtime 决定。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::diff::{diff_tags, is_restart_required};
use crate::loader::{LoadError, load};
use crate::settings::Settings;

/// 一次 [`ConfigManager::reload`] 的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadOutcome {
    /// 文件内容与上次生效的配置一致。
    Unchanged,
    /// 已交换快照；`changed` 为发生变化的 YAML tag。
    Applied {
        /// 发生变化的 YAML tag（升序）。
        changed: Vec<String>,
    },
    /// 变更含重启项；**快照未交换**，旧配置继续生效。
    RestartRequired {
        /// 发生变化的 YAML tag（升序，含热字段）。
        changed: Vec<String>,
    },
}

/// 热重载错误。
#[derive(Debug, Error)]
pub enum ReloadError {
    /// 重新读取或解析配置文件失败；此时旧快照保持不变。
    #[error(transparent)]
    Load(#[from] LoadError),
    /// 字段级 diff 失败（`Settings` 的 serde 实现被改坏时才会发生）。
    #[error("比较配置字段失败: {0}")]
    Diff(#[from] serde_norway::Error),
}

/// 配置快照管理器：文件路径 + 无锁快照 + 上次**已生效**的配置。
#[derive(Debug)]
pub struct ConfigManager {
    path: PathBuf,
    snapshot: ArcSwap<Settings>,
    /// 最近一次真正生效的配置（重启项待生效时保持旧值），作为下次 diff 的基准。
    /// 用它而不是 `snapshot` 做基准，是为了让"重启项未生效"的变更在每次 reload 时
    /// 仍被报告出来，而不是被一次热字段变更顺带静默应用。
    last_applied: Mutex<Arc<Settings>>,
    /// 串行化 `reload()`：快照交换与 `last_applied` 更新必须成对发生，
    /// 否则并发 reload 会让两者分叉（快照已换、基准仍旧）。
    reload_lock: Mutex<()>,
}

impl ConfigManager {
    /// 加载一次配置并初始化快照。
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, LoadError> {
        let path = path.into();
        let settings = Arc::new(load(&path)?);
        Ok(Self {
            path,
            snapshot: ArcSwap::new(Arc::clone(&settings)),
            last_applied: Mutex::new(settings),
            reload_lock: Mutex::new(()),
        })
    }

    /// 配置文件路径。
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 当前生效的配置快照（无锁读）。
    #[must_use]
    pub fn snapshot(&self) -> Arc<Settings> {
        self.snapshot.load_full()
    }

    /// 重新读取配置文件并与上次生效的配置做字段级 diff。
    ///
    /// - 无变化 → [`ReloadOutcome::Unchanged`]；
    /// - 有变化且不含重启项 → 交换快照，[`ReloadOutcome::Applied`]；
    /// - 含重启项 → **不交换快照**，[`ReloadOutcome::RestartRequired`]。
    pub fn reload(&self) -> Result<ReloadOutcome, ReloadError> {
        // 整个 reload（读盘 → diff → 交换 → 更新基准）串行化：并发调用时后到者
        // 看到的是前者已生效的结果，不会出现快照与 last_applied 分叉。
        let _guard = self
            .reload_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let next = Arc::new(load(&self.path)?);

        let changed = {
            let last = self.last_applied.lock().unwrap_or_else(|e| e.into_inner());
            diff_tags(&last, &next)?
        };

        if changed.is_empty() {
            debug!(path = %self.path.display(), "配置文件未变化");
            return Ok(ReloadOutcome::Unchanged);
        }

        let restart: Vec<&String> = changed
            .iter()
            .filter(|tag| is_restart_required(tag))
            .collect();
        if !restart.is_empty() {
            warn!(
                path = %self.path.display(),
                changed = ?changed,
                restart_required = ?restart,
                "配置变更包含重启项，快照保持不变"
            );
            return Ok(ReloadOutcome::RestartRequired { changed });
        }

        self.snapshot.store(Arc::clone(&next));
        *self.last_applied.lock().unwrap_or_else(|e| e.into_inner()) = next;
        info!(
            path = %self.path.display(),
            changed = ?changed,
            "配置已热重载"
        );
        Ok(ReloadOutcome::Applied { changed })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, port: &str, bind_prefix: &str) {
        let body = format!(
            "version: 1\nsettings:\n  port: \"{port}\"\n  bind_prefix: \"{bind_prefix}\"\n"
        );
        std::fs::write(path, body).expect("写测试配置失败");
    }

    #[test]
    fn applied_swaps_snapshot() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        write(&path, "5700", "/bind");

        let manager = ConfigManager::new(&path).expect("加载失败");
        assert_eq!(manager.snapshot().port, "5700");

        write(&path, "5700", "/bind2");
        assert_eq!(
            manager.reload().expect("reload 失败"),
            ReloadOutcome::Applied {
                changed: vec!["bind_prefix".to_owned()]
            }
        );
        assert_eq!(manager.snapshot().bind_prefix, "/bind2");
        assert_eq!(
            manager.reload().expect("reload 失败"),
            ReloadOutcome::Unchanged
        );
    }

    #[test]
    fn restart_required_keeps_snapshot() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        write(&path, "5700", "/bind");

        let manager = ConfigManager::new(&path).expect("加载失败");
        write(&path, "5701", "/bind");
        assert_eq!(
            manager.reload().expect("reload 失败"),
            ReloadOutcome::RestartRequired {
                changed: vec!["port".to_owned()]
            }
        );
        assert_eq!(manager.snapshot().port, "5700", "重启项不进入快照");
        // 未处理的待重启变更在下次 reload 时仍要报出来
        assert!(matches!(
            manager.reload().expect("reload 失败"),
            ReloadOutcome::RestartRequired { .. }
        ));
    }

    #[test]
    fn concurrent_reload_keeps_snapshot_and_baseline_in_sync() {
        use std::sync::{Arc, Barrier};

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        write(&path, "5700", "/bind");

        let manager = Arc::new(ConfigManager::new(&path).expect("加载失败"));
        write(&path, "5700", "/bind2");

        // 8 个线程同时 reload：reload_lock 串行化"读盘 → diff → 交换 → 更新基准"，
        // 精确结果是 1 个 Applied + 7 个 Unchanged；无锁时会有 ≥2 个线程
        // 拿旧基准各 diff 一次，都报 Applied（快照与基准分叉）
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let manager = Arc::clone(&manager);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    manager.reload().expect("并发 reload 失败")
                })
            })
            .collect();

        let mut applied = 0;
        let mut unchanged = 0;
        for handle in handles {
            match handle.join().expect("线程 panic") {
                ReloadOutcome::Applied { changed } => {
                    assert_eq!(changed, vec!["bind_prefix".to_owned()], "变更项");
                    applied += 1;
                }
                ReloadOutcome::Unchanged => unchanged += 1,
                other => panic!("并发 reload 只应产生 Applied/Unchanged，实际 {other:?}"),
            }
        }
        assert_eq!(applied, 1, "恰好一个线程完成快照交换");
        assert_eq!(unchanged, 7, "其余线程必须读到已推进的基准");

        let snapshot = manager.snapshot();
        assert_eq!(snapshot.bind_prefix, "/bind2", "快照必须是最终内容");
        let baseline = manager
            .last_applied
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        assert_eq!(
            diff_tags(&baseline, &snapshot).expect("diff 失败"),
            Vec::<String>::new(),
            "并发 reload 后 snapshot 与 last_applied 必须一致"
        );
        // 基准与快照一致：再 reload 一次必然 Unchanged
        assert_eq!(
            manager.reload().expect("reload 失败"),
            ReloadOutcome::Unchanged
        );
    }
}
