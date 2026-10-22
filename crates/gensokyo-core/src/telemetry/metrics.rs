//! 轻量指标注册表（原子计数）。
//!
//! Prometheus 导出与 `/metrics` 路由在 M1 接到 `gensokyo-webui` 的监控 API。

use std::sync::atomic::{AtomicU64, Ordering};

/// 单调递增计数器。
#[derive(Debug, Default)]
pub struct Counter(AtomicU64);

impl Counter {
    /// `const` 构造，便于在 `static` 中声明。
    #[must_use]
    pub const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    /// 计数 +1。
    pub fn inc(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    /// 计数 +n。
    pub fn add(&self, n: u64) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }

    /// 读取当前值。
    #[must_use]
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}
