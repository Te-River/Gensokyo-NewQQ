//! idmap 读路径的有界热查询缓存（P35）。
//!
//! 只缓存两条纯读路径的结果——`identity → vUIN`（[`super::IdMapStore::lookup_virtual_value`]）
//! 与 `vUIN → 原始身份串`（[`super::IdMapStore::retrieve_row_by_id`]）——写入路径在**提交成功**
//! 之后立刻失效受影响的 key，因此「写后立即读」与不缓存时逐值相同；负结果（`NotFound`）也缓存，
//! 分配/解绑同样会失效它。
//!
//! # 一致性
//!
//! 失效与读回填的竞争有两种失败形态：
//!
//! 1. **回填窗口**：读事务在提交**之前**取到旧值、提交 / 失效**之后**才回填。写入方在
//!    `commit` 之后**先** [`LookupCache::bump_epoch`] 再失效；读者回填前比对代际号，
//!    读窗口内有提交就放弃这次结果（[`LookupCache::epoch`]）。
//! 2. **命中窗口**：提交成功到失效执行之间，`get` 仍可能命中提交前的条目。失效紧随
//!    `bump_epoch` 之后，窗口是两条指令级的微操作；缓存只影响性能，下一轮写入或 TTL 会纠正。
//!
//! 于是缓存里不会长期留下「提交前读到的值」，最坏只是少缓存一次。
//!
//! # 多实例
//!
//! redb 对库文件加排他锁，同一份文件只能有一个写进程，因此同机多实例要么各自一份库（互不
//! 可见，缓存谈不上脏），要么走 gRPC 由宿主进程独占（[`super::grpc`]）。真正的跨进程共享
//! 形态（Go 用 bbolt + gRPC 服务端）在这里仍只有宿主进程持有库，缓存不会越过进程边界。
//! TTL 是给「绕过 [`super::IdMapStore`] 直接写库」的路径（导入器已显式失效）留的兜底。

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// 缓存键：两类纯读各一种。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum LookupKey {
    /// 归一化身份 key（`openid:QQ:<AppID>:<raw>` / `ruin:…` / `raw:…`）。
    Identity(String),
    /// 十进制 vUIN。
    Vuin(i64),
}

/// 缓存值。`Missing` 是负缓存，等价于 `StorageError::NotFound`。
#[derive(Debug, Clone)]
pub(super) enum LookupValue {
    /// `identity → vUIN` 的命中结果。
    Vuin(i64),
    /// `vUIN → 原始身份串` 的命中结果。
    Raw(String),
    /// 查不到（负缓存）。
    Missing,
}

/// 一条缓存项。
#[derive(Debug)]
struct Entry {
    value: LookupValue,
    stored_at: Instant,
    /// 在 [`Inner::order`] 里的序号（最近使用越大）。
    seq: u64,
}

/// 缓存本体：`Mutex` 保护的 LRU + 无锁代际号。
#[derive(Debug)]
pub(super) struct LookupCache {
    /// 容量上限，0 表示关闭（读写都直接打库）。
    capacity: usize,
    /// 单条最长存活时间，0 表示立即过期（等于关闭）。
    ttl: Duration,
    inner: Mutex<Inner>,
    /// 写入代际号：每次提交后前进，读者据此判断读窗口是否被写覆盖。
    epoch: AtomicU64,
}

/// LRU 索引：`entries` 存值，`order` 按最近使用时间排序（`pop_first` 即淘汰对象）。
#[derive(Debug)]
struct Inner {
    entries: HashMap<LookupKey, Entry>,
    order: BTreeMap<u64, LookupKey>,
    next_seq: u64,
}

impl LookupCache {
    /// 按配置建一个缓存（容量 0 = 关闭）。
    pub(super) fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            capacity,
            ttl,
            inner: Mutex::new(Inner {
                entries: HashMap::new(),
                order: BTreeMap::new(),
                next_seq: 0,
            }),
            epoch: AtomicU64::new(0),
        }
    }

    /// 关闭状态（读写都不碰缓存）。
    #[must_use]
    pub(super) fn is_disabled(&self) -> bool {
        self.capacity == 0 || self.ttl.is_zero()
    }

    /// 查缓存；未命中 / 已过期返回 `None`。
    #[must_use]
    pub(super) fn get(&self, key: &LookupKey) -> Option<LookupValue> {
        if self.is_disabled() {
            return None;
        }
        let mut inner = self.lock();
        if inner
            .entries
            .get(key)
            .is_some_and(|entry| entry.stored_at.elapsed() > self.ttl)
        {
            inner.remove(key);
            return None;
        }
        let value = inner.entries.get(key)?.value.clone();
        inner.touch(key);
        Some(value)
    }

    /// 回填一条结果（超容量时淘汰最久未使用的一条）。
    pub(super) fn insert(&self, key: LookupKey, value: LookupValue) {
        if self.is_disabled() {
            return;
        }
        let mut inner = self.lock();
        inner.insert(key, value, self.capacity);
    }

    /// 失效若干个 key（写入提交之后调用）。
    pub(super) fn invalidate(&self, keys: &[LookupKey]) {
        if self.is_disabled() || keys.is_empty() {
            return;
        }
        let mut inner = self.lock();
        for key in keys {
            inner.remove(key);
        }
    }

    /// 清空并前进代际号：给「绕过 [`super::IdMapStore`] 直接写库」的路径（JSONL 导入）用。
    pub(super) fn clear(&self) {
        self.bump_epoch();
        if self.is_disabled() {
            return;
        }
        let mut inner = self.lock();
        inner.entries.clear();
        inner.order.clear();
    }

    /// 前进代际号（写入提交之后、失效之前调用；先行关闭回填窗口）。
    pub(super) fn bump_epoch(&self) {
        if !self.is_disabled() {
            self.epoch.fetch_add(1, Ordering::Release);
        }
    }

    /// 当前代际号。
    #[must_use]
    pub(super) fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // 缓存不是一致性来源：互斥锁中毒只说明别的线程 panic 过，继续复用旧数据也不会更糟
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Inner {
    fn remove(&mut self, key: &LookupKey) {
        if let Some(entry) = self.entries.remove(key) {
            self.order.remove(&entry.seq);
        }
    }

    /// 命中后重排：只有确实存在的项才进 `order`，否则会留下淘汰不掉的幽灵序号。
    fn touch(&mut self, key: &LookupKey) {
        let Some(previous) = self.entries.get(key).map(|entry| entry.seq) else {
            return;
        };
        self.order.remove(&previous);
        let next = self.next_seq.wrapping_add(1);
        self.next_seq = next;
        if let Some(entry) = self.entries.get_mut(key) {
            entry.seq = next;
        }
        self.order.insert(next, key.clone());
    }

    fn insert(&mut self, key: LookupKey, value: LookupValue, capacity: usize) {
        let seq = self.next_seq.wrapping_add(1);
        self.next_seq = seq;
        if let Some(previous) = self.entries.insert(
            key.clone(),
            Entry {
                value,
                stored_at: Instant::now(),
                seq,
            },
        ) {
            self.order.remove(&previous.seq);
        }
        self.order.insert(seq, key);
        while self.entries.len() > capacity {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }
}
