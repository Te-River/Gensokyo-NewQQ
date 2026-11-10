//! 联合缓存（Go `unioncache/unioncache.go`）。
//!
//! `id` 与 `union_openid` 之间的一对一双向映射。`union_id` 模式下列给客户端的 ID 是 union，
//! 而 QQ 的 markdown @ 只认原始 OpenID，所以入站要存、出站 markdown 要反查：
//!
//! | 方向 | Go 调用点 | 本仓接线点 |
//! |------|-----------|-----------|
//! | 写入 `id → union` | `ProcessGroupMessage.go:55`、`ProcessGroupNormalMessage.go:46`（群消息作者） | `gensokyo-runtime/src/inbound.rs` 的 `Inbound::group_message`（两种群消息形态共用，P38） |
//! | 反查 `union → id`（出站） | `mdutil/markdown_at_replace.go:66`（`send_group_msg.go:1531` 发送前） | `gensokyo-runtime/src/deferred.rs` 的 `compose_markdown`（P38） |
//! | 反查 `id → union`（入站） | `ProcessInlineSearch.go:113`（`union_id` 打开时把 `group_member_openid` 换成 union） | `gensokyo-runtime/src/inbound.rs` 的 `Inbound::interaction`（P42） |
//!
//! 三个读写点均已接（P42 补齐最后两项）。inline search 的替换范围与 Go 一致但比字面窄：
//! `fromuid` 在 `ProcessInlineSearch.go:39` 就取走了 `GroupMemberOpenID`，`:112-116` 的替换
//! 只改回显给客户端的 `data.group_member_openid`，`user_id` / `real_user_id` 仍是原始 OpenID。
//! 群消息侧的 `union_id` 切换（上报与 idmap 改用 union）在 `inbound.rs` 的 `group_message`。
//!
//! Go 用两个 `sync.Map` 存两张表，写路径由一把 `Mutex` 串行化，保证两张表原子一致
//! （`unioncache.go:15-41`）；本实现用 `Mutex<Inner>` 同时保护两张 `HashMap`，语义相同。
//! 读也过锁：这张表只存"近期活跃的群成员"，不是热路径，Go 的 `sync.Map` 读优化在这里
//! 不值得多一份复杂度。
//!
//! # 与 Go 的差异（明确列出）
//!
//! - **生产用进程级共享实例**：[`UnionCache::shared`] 对位 Go 的包级变量——写入方（入站）
//!   与读取方（出站 markdown）不共享任何别的句柄，只能靠同一张表见面；[`UnionCache::new`]
//!   留给单测造独立实例（Go 没有这条路径）。
//! - **没有过期/容量上限**：Go 也没有 TTL 或清理路径（`Clear` 注释写的是"仅用于测试或
//!   受控重置"），条目只在 `store` 重绑与 `delete_*` 时消失；这里照抄，不引入 TTL。
//! - **空键**：与 Go 一致，`store` 遇到空 `id` 或空 `union_openid` 直接忽略。
//! - **`union_id` 打开但 `union_openid` 为空**：Go 群消息会无条件把 `author.id` 置空
//!   （`ProcessGroupMessage.go:60`），`inbound.rs` 保留原 id（空 ID 事件对客户端只是噪声）。
//! - **锁中毒**：`Mutex` 中毒后用 `PoisonError::into_inner` 恢复，全模块没有 `unwrap`。

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

/// `id ↔ union_openid` 双向映射（Go `unioncache` 包级两张 `sync.Map`）。
#[derive(Debug, Default)]
pub struct UnionCache {
    inner: Mutex<Inner>,
}

/// 两张表放在同一把锁里：任何写入都同时维护正反两向，读到的组合永远一致。
#[derive(Debug, Default)]
struct Inner {
    id_to_union: HashMap<String, String>,
    union_to_id: HashMap<String, String>,
}

impl UnionCache {
    /// 新建空缓存。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 进程级共享实例（Go 的包级两张 `sync.Map`）。
    ///
    /// 入站群消息写入、出站 markdown 反查都取这一份：两边没有别的共同句柄，共享实例是它们
    /// 唯一的会合点。单测要互不干扰时用 [`UnionCache::new`]。
    pub fn shared() -> &'static Self {
        static SHARED: OnceLock<UnionCache> = OnceLock::new();
        SHARED.get_or_init(Self::new)
    }

    /// 建立 1:1 映射（Go `Store`）。
    ///
    /// 与 Go 相同：`id` 或 `union_openid` 之前绑定的另一侧会被清掉，保证一对一。
    /// 任一参数为空时忽略（Go `unioncache.go:16-19`）。
    pub fn store(&self, id: &str, union_openid: &str) {
        if id.is_empty() || union_openid.is_empty() {
            return;
        }
        let mut inner = self.lock();

        // 先取旧值再改表：否则不可变借用与可变借用同时存在。
        let old_union = inner.id_to_union.get(id).cloned();
        if let Some(old) = old_union
            && old != union_openid
        {
            inner.union_to_id.remove(&old);
        }
        let old_id = inner.union_to_id.get(union_openid).cloned();
        if let Some(old) = old_id
            && old != id
        {
            inner.id_to_union.remove(&old);
        }

        inner
            .id_to_union
            .insert(id.to_owned(), union_openid.to_owned());
        inner
            .union_to_id
            .insert(union_openid.to_owned(), id.to_owned());
    }

    /// 按 `id` 取 `union_openid`（Go `Union`）。查不到返回 `None`。
    #[must_use]
    pub fn union(&self, id: &str) -> Option<String> {
        self.lock().id_to_union.get(id).cloned()
    }

    /// 按 `union_openid` 取 `id`（Go `ID`）。查不到返回 `None`。
    #[must_use]
    pub fn id(&self, union_openid: &str) -> Option<String> {
        self.lock().union_to_id.get(union_openid).cloned()
    }

    /// 按 `id` 删除，并删掉反向映射（Go `DeleteID`）。
    pub fn delete_id(&self, id: &str) {
        if id.is_empty() {
            return;
        }
        let mut inner = self.lock();
        if let Some(union) = inner.id_to_union.remove(id) {
            inner.union_to_id.remove(&union);
        }
    }

    /// 按 `union_openid` 删除，并删掉反向映射（Go `DeleteUnion`）。
    pub fn delete_union(&self, union_openid: &str) {
        if union_openid.is_empty() {
            return;
        }
        let mut inner = self.lock();
        if let Some(id) = inner.union_to_id.remove(union_openid) {
            inner.id_to_union.remove(&id);
        }
    }

    /// 清空两张表（Go `Clear`）。
    pub fn clear(&self) {
        let mut inner = self.lock();
        inner.id_to_union.clear();
        inner.union_to_id.clear();
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use super::*;

    #[test]
    fn maps_both_directions() {
        let cache = UnionCache::new();
        cache.store("user-1", "union-1");
        assert_eq!(cache.union("user-1").as_deref(), Some("union-1"));
        assert_eq!(cache.id("union-1").as_deref(), Some("user-1"));
        assert_eq!(cache.union("user-2"), None);
        assert_eq!(cache.id("union-2"), None);
    }

    #[test]
    fn rebinding_one_side_drops_the_stale_pair() {
        let cache = UnionCache::new();
        cache.store("user-1", "union-1");

        // 同一个 id 换 union：旧 union 的反向映射必须消失
        cache.store("user-1", "union-2");
        assert_eq!(cache.union("user-1").as_deref(), Some("union-2"));
        assert_eq!(cache.id("union-2").as_deref(), Some("user-1"));
        assert_eq!(cache.id("union-1"), None, "旧 union 不应再指向任何 id");

        // 同一个 union 换 id：旧 id 的正向映射必须消失
        cache.store("user-9", "union-2");
        assert_eq!(cache.union("user-9").as_deref(), Some("union-2"));
        assert_eq!(cache.id("union-2").as_deref(), Some("user-9"));
        assert_eq!(cache.union("user-1"), None, "旧 id 不应再指向任何 union");
    }

    #[test]
    fn empty_keys_are_ignored() {
        let cache = UnionCache::new();
        cache.store("", "union-1");
        cache.store("user-1", "");
        assert_eq!(cache.union(""), None);
        assert_eq!(cache.id("union-1"), None);
        // 空键不会污染后续正常写入
        cache.store("user-1", "union-1");
        assert_eq!(cache.union("user-1").as_deref(), Some("union-1"));
    }

    #[test]
    fn deletes_remove_both_directions_and_clear_empties_everything() {
        let cache = UnionCache::new();
        cache.store("user-1", "union-1");
        cache.store("user-2", "union-2");

        cache.delete_id("user-1");
        assert_eq!(cache.union("user-1"), None);
        assert_eq!(cache.id("union-1"), None);
        assert_eq!(
            cache.id("union-2").as_deref(),
            Some("user-2"),
            "不相关条目保留"
        );

        cache.delete_union("union-2");
        assert_eq!(cache.union("user-2"), None);
        assert_eq!(cache.id("union-2"), None);

        cache.store("user-3", "union-3");
        cache.clear();
        assert_eq!(cache.union("user-3"), None);
        assert_eq!(cache.id("union-3"), None);
    }

    #[test]
    fn concurrent_writes_keep_the_two_tables_atomic() {
        // 8 个线程反复抢同一个 id：无论谁最后写入，正反两向必须指向同一对，
        // 且落败的 union 不能残留反向映射（Go 的 Mutex 语义）。
        let cache = Arc::new(UnionCache::new());
        let mut handles = Vec::new();
        for worker in 0..8 {
            let cache = Arc::clone(&cache);
            handles.push(thread::spawn(move || {
                let union = format!("union-{worker}");
                for _ in 0..100 {
                    cache.store("shared-id", &union);
                }
            }));
        }
        for handle in handles {
            handle.join().expect("工作线程不应 panic");
        }

        let winner = cache.union("shared-id").expect("共享 id 必有绑定");
        assert_eq!(cache.id(&winner).as_deref(), Some("shared-id"));
        for worker in 0..8 {
            let union = format!("union-{worker}");
            if union != winner {
                assert_eq!(cache.id(&union), None, "落败的 union 不应残留反向映射");
            }
        }
    }

    #[test]
    fn disjoint_concurrent_writes_are_all_visible() {
        let cache = Arc::new(UnionCache::new());
        let mut handles = Vec::new();
        for worker in 0..8u32 {
            let cache = Arc::clone(&cache);
            handles.push(thread::spawn(move || {
                for index in 0..50u32 {
                    let id = format!("user-{worker}-{index}");
                    let union = format!("union-{worker}-{index}");
                    cache.store(&id, &union);
                }
            }));
        }
        for handle in handles {
            handle.join().expect("工作线程不应 panic");
        }

        for worker in 0..8u32 {
            for index in 0..50u32 {
                let id = format!("user-{worker}-{index}");
                let union = format!("union-{worker}-{index}");
                assert_eq!(cache.union(&id).as_deref(), Some(union.as_str()));
                assert_eq!(cache.id(&union).as_deref(), Some(id.as_str()));
            }
        }
    }
}
