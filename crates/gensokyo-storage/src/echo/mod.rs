//! echo 内存映射层（Go `echo/echo.go` + `echo/messageidmap.go`）。
//!
//! 覆盖四类进程内状态：消息 ID 懒加载、msg_seq、SSM 补发队列，以及 echo 键值映射
//! （消息类型 / message_id / event_id / 递归计数 / REFIDX / 行号缓存）。全部是可丢的
//! 内存数据：进程重启即清空；[`EchoStore::cleanup`] 清空除懒消息表之外的映射（与 Go
//! `cleanupGlobalMaps` 一致）。
//!
//! # key 格式（与 Go 一致）
//!
//! | 用途 | key | Go 来源 |
//! |------|-----|---------|
//! | 消息类型 / msg_id（群、C2C 等 int64 场景） | `<app_id>_<s>` | `GenerateKey` |
//! | msg_id v2（群 × 用户） | `<app_id>_<group_id>_<user_id>` | `GenerateKeyv2` |
//! | event_id（int64 群号） | `<app_id>_<group_id>` | `GenerateKeyEventID` |
//! | event_id（字符串群号） | `<app_id>_<group_id>` | `GenerateKeyEventIDV2` |
//! | msg_id v3（字符串场景） | `<app_id>_<s>` | `GenerateKeyv3` |
//! | 懒消息（群维度） | `<group_id>` | `AddLazyMessageId` |
//! | 懒消息（群 × 用户） | `<group_id>.<user_id>` | `AddLazyMessageIdv2` |
//! | seq | 调用方给的 key（实际为 message_id） | `AddMappingSeq` 等 |
//! | SSM 关联标识 | `ssm-<group_id 前 8 字节>-<递增计数>` | `NextSSMCorrelationID` |
//! | REFIDX | `<message_id>` | `StoreRefIdx` |
//!
//! # 时间窗口
//!
//! - 懒消息窗口默认 4 分钟（[`DEFAULT_LAZY_WINDOW`]），由 [`EchoStore::with_window`] 覆盖；
//!   只保留 `timestamp` 严格晚于 `now - window` 的记录，读到有效记录时顺带裁剪过期项。
//! - 清理周期默认 30 分钟（[`DEFAULT_CLEANUP_PERIOD`]），由
//!   [`EchoStore::start_cleanup_routine_with_period`] 覆盖；测试可直接调
//!   [`EchoStore::cleanup`]，不必等周期。
//! - 时钟可注入（[`EchoStore::with_clock`]），冻结后即可验证窗口，无需 `sleep`。
//!
//! # 行号派生
//!
//! [`EchoStore::store_cache_in_memory`] 与 Go `idmap.GenerateRowID` 逐位一致：MD5(UTF-8) →
//! 十六进制串 → 只保留 ASCII 数字 → 取前 `length` 位（不足右侧补 `0`）→ 十进制解析；从 9 位
//! 起冲突则加长到 18 位，反表 `行号 → ID` 同步登记。已知向量见单测
//! `row_id_matches_go_generate_row_id`。
//!
//! # 与 Go 的差异（明确列出）
//!
//! - **实例化**：Go 是包级 `sync.Map` 单例；本 crate 用显式的 [`EchoStore`]，可多实例并存
//!   （生产每进程一份即可），也便于单测互不干扰。
//! - **`ramdom_seq` 来源**：Go 读 `config.GetRamDomSeq()`；本 crate 由
//!   [`EchoStore::with_ramdom_seq`] 注入（默认关闭，与模板 `ramdom_seq: false` 一致）。
//!   随机源是 splitmix64，而非 Go 的 `math/rand`（不引额外依赖）。命中路径与 Go 相同：
//!   只有 key 缺失且开关打开时才取 1..=10000。
//! - **[`EchoStore::pop_global_stack_multi`]** 无元素时返回空 `Vec`（Go 返回 `nil`），且与 Go 一样
//!   **不删除**元素；[`EchoStore::remove_from_global_stack`]**对越界下标静默忽略**（Go 的
//!   `index < 0` 分支在 `usize` 下不可达）。
//! - **[`MessageGroupPair::payload`]**：Go 存 `*dto.MessageToCreate`；本层只存不解释的字符串，
//!   消息体类型留给 actions/runtime 层（M1e 不引协议类型）。
//! - **整数宽度**：Go `int` 是 64 位；`int_mapping` 与 seq 的值用 `i32` 收窄（对齐 Go 侧实际
//!   取值范围），上层不应写入超过 `i32::MAX` 的计数。
//! - **seq 溢出**：`saturating_add`，永不 panic（Go 回绕）。
//! - **群 ID 登记**：Go `generateDefaultMessageID` 先 `idmap.StoreIDv2(groupID)` 登记群虚拟 ID
//!   （随后查消息类型但丢弃结果）再返回 `"2000"`；本层只返回 `"2000"`，群 ID 登记由上层负责。
//! - **清理**：Go `cleanupGlobalMaps`（30 分钟 tick）清空除懒消息表**之外**的全部映射——
//!   `messageStore.records`（`echo/messageidmap.go`）没有任何清理路径，窗口内的被动回复 msg_id
//!   必须活到 4 分钟结束。本实现同样不清懒消息表，SSM 计数器也不随清理重置。
//!
//! # 并发
//!
//! seq 的读-改-写由一把 `Mutex<()>` 串行化（Go 的 `globalStringToIntMappingSeq.mu`）；其余 map
//! 用 `RwLock<HashMap>`，锁中毒用 `PoisonError::into_inner` 恢复，因此全模块没有 `unwrap`/`panic`。
//! 锁用 `std`（与 `idmap` 模块一致），不额外引依赖。

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

use md5::{Digest, Md5};
use tracing::{debug, warn};

use crate::StorageError;

/// 懒消息的默认时间窗口（Go `GetLazyMessagesId` 的 4 分钟）。
pub const DEFAULT_LAZY_WINDOW: Duration = Duration::from_secs(4 * 60);
/// 后台清理的默认周期（Go `startCleanupRoutine` 的 30 分钟）。
pub const DEFAULT_CLEANUP_PERIOD: Duration = Duration::from_secs(30 * 60);
/// 懒消息无可用记录时返回的 message_id（Go `generateDefaultMessageID`）。
///
/// `"2000"` 在 Go 侧是"主动推送"的特殊值，见 `AGENTS.md` 的 LazyMessageId 说明。
pub const DEFAULT_MESSAGE_ID: &str = "2000";
/// `ramdom_seq` 开启时随机 seq 的上界（含），等价 Go `rng.Intn(10000)+1`。
pub const RANDOM_SEQ_MAX: i32 = 10_000;
/// SSM 关联标识里 groupID 前缀的字节数（Go `groupID[:8]`）。
pub const SSM_GROUP_PREFIX_LEN: usize = 8;

/// 随机 seq 的取值上界（`u64` 形态，避免取模时转换）。
const RANDOM_SEQ_SPAN: u64 = 10_000;
/// 行号尝试的起始/终止十进制位数（Go `digits := 9; digits <= 18`）。
const ROW_ID_MIN_DIGITS: usize = 9;
const ROW_ID_MAX_DIGITS: usize = 18;
/// `message_scene.ext[]` 里引用消息索引的前缀（Go `msg_idx=`）。
const EXT_REF_IDX_PREFIX: &str = "msg_idx=";

/// 可注入时钟：返回"现在"。
///
/// 默认 [`SystemTime::now`]。测试可冻结时间：
/// `Arc::new(move || *frozen.lock()...)`，从而在毫秒内验证 4 分钟窗口。
pub type Clock = Arc<dyn Fn() -> SystemTime + Send + Sync>;

/// 入站消息的 `message_scene`（Go `dto.MessageScene` 的最小投影）。
///
/// M1e 只关心 `ext[]` 里的 `msg_idx=REFIDX_*`，其余字段由协议层自己持有。
#[derive(Debug, Clone, Default)]
pub struct MessageScene {
    /// `ext` 元素形如 `msg_idx=REFIDX_xxx` / `ref_msg_idx=TMP_xxx` / `auth_token=xxx`。
    pub ext: Vec<String>,
}

impl MessageScene {
    /// 用 `ext` 列表构造（便于调用方从反序列化结果直接转过来）。
    #[must_use]
    pub fn from_ext<I, S>(ext: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            ext: ext.into_iter().map(Into::into).collect(),
        }
    }

    /// 取第一个 `msg_idx=` 的值（Go `StoreRefIdxFromScene` 命中第一个前缀即返回）。
    ///
    /// 值为空时返回 `None`，且不再看后续元素（与 Go 的 `return` 语义一致）。
    #[must_use]
    pub fn ref_idx(&self) -> Option<&str> {
        let raw = self
            .ext
            .iter()
            .find_map(|item| item.strip_prefix(EXT_REF_IDX_PREFIX))?;
        (!raw.is_empty()).then_some(raw)
    }
}

/// SSM 补发队列的元素（Go `MessageGroupPair`）。
#[derive(Debug, Clone)]
pub struct MessageGroupPair {
    /// 群/会话标识，参与关联标识的 groupID 前缀。
    pub group: String,
    /// 入队时间；[`EchoStore::push_global_stack`] 覆盖写入（Go 同）。
    pub enqueue_time: Option<SystemTime>,
    /// 关联标识；空串时由 push 自动生成 `ssm-<group 前 8 字节>-<计数>`。
    pub correlation_id: String,
    /// 待补发的消息体。Go 存 `*dto.MessageToCreate`；本层只保存不解释，内容由上层决定。
    pub payload: Option<String>,
}

impl MessageGroupPair {
    /// 只填 `group`，其余字段留给 [`EchoStore::push_global_stack`] 补齐。
    #[must_use]
    pub fn new(group: impl Into<String>) -> Self {
        Self {
            group: group.into(),
            enqueue_time: None,
            correlation_id: String::new(),
            payload: None,
        }
    }
}

/// echo 内存存储。
///
/// Go 的包级 `sync.Map` 单例在这里变成显式实例；[`Clone`] 只复制共享句柄（Arc），
/// 所有实例看到同一份映射，因此可以放心交给后台清理线程。
#[derive(Clone)]
pub struct EchoStore {
    lazy_window: Duration,
    ramdom_seq: bool,
    clock: Clock,
    inner: Arc<EchoInner>,
}

impl fmt::Debug for EchoStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EchoStore")
            .field("lazy_window", &self.lazy_window)
            .field("ramdom_seq", &self.ramdom_seq)
            .finish_non_exhaustive()
    }
}

impl Default for EchoStore {
    fn default() -> Self {
        Self::new()
    }
}

impl EchoStore {
    /// 默认配置：4 分钟懒消息窗口、`ramdom_seq` 关闭、系统时钟、无后台清理线程。
    #[must_use]
    pub fn new() -> Self {
        Self {
            lazy_window: DEFAULT_LAZY_WINDOW,
            ramdom_seq: false,
            clock: Arc::new(SystemTime::now),
            inner: Arc::new(EchoInner::new()),
        }
    }

    /// 覆盖懒消息窗口（测试常用）。窗口为 0 时任何记录都会被视为过期。
    #[must_use]
    pub fn with_window(mut self, window: Duration) -> Self {
        self.lazy_window = window;
        self
    }

    /// 注入时钟，用于冻结/推进"现在"。
    #[must_use]
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// 等价 Go `config.GetRamDomSeq()`：key 缺失时是否取 1..=10000 的随机 seq。
    #[must_use]
    pub fn with_ramdom_seq(mut self, enabled: bool) -> Self {
        self.ramdom_seq = enabled;
        self
    }

    /// 当前懒消息窗口。
    #[must_use]
    pub fn lazy_window(&self) -> Duration {
        self.lazy_window
    }

    /// 清空映射（Go `cleanupGlobalMaps`）。
    ///
    /// 懒消息表**不清**：Go 的 `messageStore.records` 没有任何清理路径，清掉会让窗口内写入的
    /// 被动回复 msg_id 提前失效、回落到 [`DEFAULT_MESSAGE_ID`]（主动推送，可能触发 22009 +
    /// SSM 补发）。SSM 计数器不重置、后台清理线程不停（Go 同）。
    pub fn cleanup(&self) {
        Self::cleanup_inner(&self.inner);
    }

    /// 清理的实际实现：后台线程持 `Weak` 句柄时也走这里。
    fn cleanup_inner(inner: &EchoInner) {
        write_map(&inner.msg_types).clear();
        write_map(&inner.msg_ids).clear();
        write_map(&inner.event_ids).clear();
        write_map(&inner.int_mapping).clear();
        write_map(&inner.ref_idx).clear();
        write_map(&inner.cache_rows).clear();
        write_map(&inner.cache_ids).clear();
        inner.seq.clear();
        inner
            .ssm
            .stack
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        debug!(target: "gensokyo_storage::echo", "echo 内存映射已清空（懒消息表除外）");
    }

    /// 启动后台清理线程，周期 [`DEFAULT_CLEANUP_PERIOD`]。
    ///
    /// 已启动时为空操作（等价 Go 的 `sync.Once`）。
    ///
    /// # Errors
    ///
    /// 线程创建失败时返回 [`io::Error`]；不 panic。
    pub fn start_cleanup_routine(&self) -> io::Result<()> {
        self.start_cleanup_routine_with_period(DEFAULT_CLEANUP_PERIOD)
    }

    /// 启动后台清理线程，周期可配。
    ///
    /// `period` 为 0 会抬到 1ms，避免忙等。线程只持 `Weak` 句柄：所有 `EchoStore`
    /// 释放后，下一次 tick（或 channel 断开）即退出，无需显式 [`EchoStore::stop_cleanup_routine`]。
    ///
    /// # Errors
    ///
    /// 线程创建失败时返回 [`io::Error`]；不 panic。
    pub fn start_cleanup_routine_with_period(&self, period: Duration) -> io::Result<()> {
        let mut slot = self
            .inner
            .cleanup
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if slot.is_some() {
            return Ok(());
        }

        let period = if period.is_zero() {
            Duration::from_millis(1)
        } else {
            period
        };
        let (stop, ticks) = mpsc::channel::<()>();
        // 只持弱引用：`Sender` 在 `EchoInner` 内，强引用会形成环导致不显式 stop 时永不释放。
        // 所有 `EchoStore` 释放后 upgrade 失败（或 channel 断开），线程自行退出。
        let weak = Arc::downgrade(&self.inner);
        let join = thread::Builder::new()
            .name("gensokyo-echo-cleanup".to_owned())
            .spawn(move || {
                loop {
                    match ticks.recv_timeout(period) {
                        Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => {
                            let Some(inner) = weak.upgrade() else {
                                break;
                            };
                            EchoStore::cleanup_inner(&inner);
                        }
                    }
                }
            })?;
        *slot = Some(CleanupHandle { stop, join });
        debug!(target: "gensokyo_storage::echo", period_ms = period.as_millis(), "echo 清理线程已启动");
        Ok(())
    }

    /// 停止后台清理线程（未启动时为空操作）。
    pub fn stop_cleanup_routine(&self) {
        let handle = self
            .inner
            .cleanup
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(CleanupHandle { stop, join }) = handle {
            let _ = stop.send(());
            if join.join().is_err() {
                warn!(target: "gensokyo_storage::echo", "echo 清理线程异常退出");
            }
        }
    }

    // ---- 懒消息（Go messageidmap.go） ----

    /// 记录群维度的一条 message_id（Go `AddLazyMessageId`）。不做窗口裁剪，读时才裁。
    pub fn add_lazy_message_id(&self, group_id: &str, message_id: &str, timestamp: SystemTime) {
        self.push_lazy(group_id.to_owned(), message_id, timestamp);
    }

    /// 记录群 × 用户维度的一条 message_id（Go `AddLazyMessageIdv2`）。
    pub fn add_lazy_message_id_v2(
        &self,
        group_id: &str,
        user_id: &str,
        message_id: &str,
        timestamp: SystemTime,
    ) {
        self.push_lazy(lazy_key_v2(group_id, user_id), message_id, timestamp);
    }

    /// 取群维度窗口内可复用的 message_id（Go `GetLazyMessagesId`）。
    ///
    /// 按记录写入顺序遍历窗口内的有效记录：优先 `usage_count == 0` 且时间更新的记录；若一条
    /// `usage_count == 0` 的记录都没有，取**第一个**有效记录（Go 的 `else if selectedRecord ==
    /// nil` 兜底）。无记录/全部过期时返回 [`DEFAULT_MESSAGE_ID`]。读路径顺带把裁剪后的记录写回，
    /// **不递增 `usage_count`**：同一回复链的多段回复必须复用同一 msg_id，让 seq 在同一 msg_id
    /// 上连续递增（Issue #19）。
    #[must_use]
    pub fn get_lazy_messages_id(&self, group_id: &str) -> String {
        self.select_lazy(group_id)
    }

    /// 取群 × 用户维度窗口内可复用的 message_id（Go `GetLazyMessagesIdv2`）。
    #[must_use]
    pub fn get_lazy_messages_id_v2(&self, group_id: &str, user_id: &str) -> String {
        self.select_lazy(&lazy_key_v2(group_id, user_id))
    }

    fn push_lazy(&self, key: String, message_id: &str, timestamp: SystemTime) {
        let mut lazy = write_map(&self.inner.lazy);
        lazy.entry(key).or_default().push(LazyRecord {
            message_id: message_id.to_owned(),
            timestamp,
            usage_count: 0,
        });
    }

    /// 裁剪 + 选择 + 写回（Go 读路径的三件事，一次写锁内完成）。
    fn select_lazy(&self, key: &str) -> String {
        let now = (self.clock)();
        let window = self.lazy_window;
        let mut lazy = write_map(&self.inner.lazy);
        let Some(records) = lazy.get_mut(key) else {
            return DEFAULT_MESSAGE_ID.to_owned();
        };

        // 逐条对齐 Go `messageidmap.go:107-140`：`usage_count == 0` 的记录只有在时间更新时
        // 才替换已选记录；一旦选中过记录，后续 `usage_count > 0` 的记录不再替换。
        let mut valid: Vec<LazyRecord> = Vec::with_capacity(records.len());
        let mut selected: Option<usize> = None;
        for record in records.iter() {
            if !within_window(record.timestamp, now, window) {
                continue;
            }
            let index = valid.len();
            valid.push(record.clone());
            if record.usage_count == 0 {
                if selected.is_none_or(|current| record.timestamp > valid[current].timestamp) {
                    selected = Some(index);
                }
            } else if selected.is_none() {
                selected = Some(index);
            }
        }

        // Go 把裁剪后的切片写回（空切片也保留 key）；`usage_count` 只写不改。
        *records = valid;

        selected.map_or_else(
            || DEFAULT_MESSAGE_ID.to_owned(),
            |index| records[index].message_id.clone(),
        )
    }

    // ---- echo 键值映射（Go echo.go:163-233） ----

    /// 记录 `<app_id>_<s>` 的消息类型（Go `AddMsgType`）。
    pub fn add_msg_type(&self, app_id: &str, s: i64, msg_type: &str) {
        write_map(&self.inner.msg_types).insert(echo_key(app_id, s), msg_type.to_owned());
    }

    /// 按 key 取消息类型，缺失返回空串（Go `GetMsgTypeByKey`）。
    #[must_use]
    pub fn get_msg_type_by_key(&self, key: &str) -> String {
        read_map(&self.inner.msg_types)
            .get(key)
            .cloned()
            .unwrap_or_default()
    }

    /// 记录 `<app_id>_<s>` 的 message_id（Go `AddMsgIDv3`）。
    pub fn add_msg_id_v3(&self, app_id: &str, s: &str, msg_id: &str) {
        write_map(&self.inner.msg_ids).insert(echo_key_v3(app_id, s), msg_id.to_owned());
    }

    /// 取 `<app_id>_<s>` 的 message_id，缺失返回空串（Go `GetMsgIDv3`）。
    #[must_use]
    pub fn get_msg_id_v3(&self, app_id: &str, s: &str) -> String {
        self.get_msg_id_by_key(&echo_key_v3(app_id, s))
    }

    /// 记录群 × 用户的 message_id（Go `AddMsgIDv2`）。
    pub fn add_msg_id_v2(&self, app_id: &str, group_id: i64, user_id: i64, msg_id: &str) {
        write_map(&self.inner.msg_ids)
            .insert(echo_key_v2(app_id, group_id, user_id), msg_id.to_owned());
    }

    /// 记录 `<app_id>_<s>` 的 message_id（Go `AddMsgID`）。
    pub fn add_msg_id(&self, app_id: &str, s: i64, msg_id: &str) {
        write_map(&self.inner.msg_ids).insert(echo_key(app_id, s), msg_id.to_owned());
    }

    /// 按 key 取 message_id，缺失返回空串（Go `GetMsgIDByKey`）。
    #[must_use]
    pub fn get_msg_id_by_key(&self, key: &str) -> String {
        read_map(&self.inner.msg_ids)
            .get(key)
            .cloned()
            .unwrap_or_default()
    }

    /// 记录 int64 群号的 event_id（Go `AddEvnetID`，函数名拼写已修正）。
    pub fn add_event_id(&self, app_id: &str, group_id: i64, event_id: &str) {
        write_map(&self.inner.event_ids)
            .insert(echo_key_event_id(app_id, group_id), event_id.to_owned());
    }

    /// 记录字符串群号的 event_id（Go `AddEvnetIDv2`）。
    pub fn add_event_id_v2(&self, app_id: &str, group_id: &str, event_id: &str) {
        write_map(&self.inner.event_ids)
            .insert(echo_key_event_id_v2(app_id, group_id), event_id.to_owned());
    }

    /// 按 key 取 event_id，缺失返回空串（Go `GetEventIDByKey`）。
    #[must_use]
    pub fn get_event_id_by_key(&self, key: &str) -> String {
        read_map(&self.inner.event_ids)
            .get(key)
            .cloned()
            .unwrap_or_default()
    }

    /// 按 `<app_id>_<group_id>` 取群消息类型（Go `GetMessageTypeByGroupidv2`）。
    ///
    /// Go 的 `GroupID interface{}` 接受 int/int64/string；这里统一要求字符串。
    #[must_use]
    pub fn get_message_type_by_group_id_v2(&self, app_id: &str, group_id: &str) -> String {
        self.get_msg_type_by_key(&echo_key_v3(app_id, group_id))
    }

    // ---- int64 → i32 递归计数（Go echo.go:236-247；Go `int` 为 64 位，这里收窄） ----

    /// 记录递归调用计数（Go `AddMapping`）。
    ///
    /// Go 的值为 `int`（64 位）；本层收窄为 `i32`，调用方不应传入超出范围的计数。
    pub fn add_mapping(&self, key: i64, value: i32) {
        write_map(&self.inner.int_mapping).insert(key, value);
    }

    /// 取递归调用计数，缺失返回 0（Go `GetMapping`）。
    #[must_use]
    pub fn get_mapping(&self, key: i64) -> i32 {
        read_map(&self.inner.int_mapping)
            .get(&key)
            .copied()
            .unwrap_or_default()
    }

    // ---- msg_seq（Go echo.go:249-306） ----

    /// 直接写入 seq。
    ///
    /// Go 已把它标为 Deprecated：`get_mapping_seq` + `add_mapping_seq` 组合有读-改-写竞态，
    /// 新代码用 [`EchoStore::next_mapping_seq`] / [`EchoStore::current_and_increment_mapping_seq`]。
    ///
    /// 值收窄为 `i32`（Go `int` 是 64 位）；递增在 [`EchoStore::next_mapping_seq`] 处饱和。
    pub fn add_mapping_seq(&self, key: &str, value: i32) {
        self.inner.seq.set(key, value);
    }

    /// 取 seq（Go `GetMappingSeq`）。
    ///
    /// key 缺失且 `ramdom_seq` 打开时返回 1..=10000 的随机值（不写回），否则返回 0。
    #[must_use]
    pub fn get_mapping_seq(&self, key: &str) -> i32 {
        read_map(&self.inner.seq.values)
            .get(key)
            .copied()
            .unwrap_or_else(|| {
                if self.ramdom_seq {
                    random_in_seq_range()
                } else {
                    0
                }
            })
    }

    /// 原子递增 seq 并返回**递增后**的值（Go `NextMappingSeq`）。
    ///
    /// 值饱和于 `i32::MAX`，不回绕、不 panic（Go 会回绕）。
    pub fn next_mapping_seq(&self, key: &str) -> i32 {
        self.inner.seq.current_and_advance(key, self.ramdom_seq).1
    }

    /// 原子取当前 seq 再推进内部值，返回**递增前**的值（Go `CurrentAndIncrementMappingSeq`）。
    pub fn current_and_increment_mapping_seq(&self, key: &str) -> i32 {
        self.inner.seq.current_and_advance(key, self.ramdom_seq).0
    }

    /// Go `IncrementMappingSeq`：行为等同 [`EchoStore::next_mapping_seq`]。
    pub fn increment_mapping_seq(&self, key: &str) -> i32 {
        self.next_mapping_seq(key)
    }

    // ---- SSM 补发队列（Go echo.go:308-365） ----

    /// 生成 SSM 关联标识：`ssm-<group_id 前 8 字节>-<递增计数>`（Go `NextSSMCorrelationID`）。
    ///
    /// 不足 8 字节时取全部；非 ASCII 且第 8 字节落在字符中间时退化为整个 group_id
    /// （Go 按字节切，可能切出非法 UTF-8）。
    #[must_use]
    pub fn next_ssm_correlation_id(&self, group_id: &str) -> String {
        self.inner.ssm.next_correlation_id(group_id)
    }

    /// 入队，自动填 `enqueue_time` 与空的 `correlation_id`（Go `PushGlobalStack`）。
    pub fn push_global_stack(&self, mut pair: MessageGroupPair) {
        pair.enqueue_time = Some((self.clock)());
        if pair.correlation_id.is_empty() {
            pair.correlation_id = self.next_ssm_correlation_id(&pair.group);
        }
        // 日志字段在 move 之前取出：pair 要整条入栈。
        let correlation_id = pair.correlation_id.clone();
        let group = pair.group.clone();
        let mut stack = self
            .inner
            .ssm
            .stack
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        stack.push(pair);
        let len = stack.len();
        drop(stack);
        debug!(
            target: "gensokyo_storage::echo",
            correlation_id = %correlation_id,
            group = %group,
            stack_len = len,
            "SSM 已入队"
        );
    }

    /// 取出前 `count` 个元素但**不删除**（Go `PopGlobalStackMulti`）。
    ///
    /// `count == 0` 或队列为空时返回空 `Vec`；`count` 超长时截到队列长度。
    #[must_use]
    pub fn pop_global_stack_multi(&self, count: usize) -> Vec<MessageGroupPair> {
        let stack = self
            .inner
            .ssm
            .stack
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        stack.iter().take(count).cloned().collect()
    }

    /// 删除指定下标的元素；越界下标静默忽略（Go `RemoveFromGlobalStack`）。
    pub fn remove_from_global_stack(&self, index: usize) {
        let mut stack = self
            .inner
            .ssm
            .stack
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if index < stack.len() {
            stack.remove(index);
        }
    }

    /// 当前队列长度（Go 入队日志里的 `len(stack)`）。
    #[must_use]
    pub fn global_stack_len(&self) -> usize {
        self.inner
            .ssm
            .stack
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    // ---- REFIDX（Go echo.go:412-455） ----

    /// 记录 message_id → REFIDX；任一参数为空则忽略（Go `StoreRefIdx`）。
    pub fn store_ref_idx(&self, msg_id: &str, ref_idx: &str) {
        if msg_id.is_empty() || ref_idx.is_empty() {
            return;
        }
        write_map(&self.inner.ref_idx).insert(msg_id.to_owned(), ref_idx.to_owned());
    }

    /// 反查 REFIDX，缺失返回空串（Go `GetRefIdx`）。
    #[must_use]
    pub fn get_ref_idx(&self, msg_id: &str) -> String {
        if msg_id.is_empty() {
            return String::new();
        }
        read_map(&self.inner.ref_idx)
            .get(msg_id)
            .cloned()
            .unwrap_or_default()
    }

    /// 删除 REFIDX 映射（Go `DeleteRefIdx`）。
    pub fn delete_ref_idx(&self, msg_id: &str) {
        if msg_id.is_empty() {
            return;
        }
        write_map(&self.inner.ref_idx).remove(msg_id);
    }

    /// 从 `message_scene.ext[]` 提取 `msg_idx=` 并存储（Go `StoreRefIdxFromScene`）。
    pub fn store_ref_idx_from_scene(&self, msg_id: &str, scene: &MessageScene) {
        if msg_id.is_empty() {
            return;
        }
        if let Some(ref_idx) = scene.ref_idx() {
            self.store_ref_idx(msg_id, ref_idx);
        }
    }

    // ---- 行号内存缓存（Go echo.go:368-410，M1e 不接 redb） ----

    /// 分配（或复用）ID 的行号并缓存双向映射（Go `StoreCacheInMemory`）。
    ///
    /// 记录 `<id, 行号>` 并在反表登记 `<行号, id>`（Go `StoreCacheInMemory`）。
    ///
    /// 与 Go 一样从 9 位十进制开始派生化行号、冲突则加长到 18 位；行号由 ID 按
    /// `idmap.GenerateRowID` 稳定派生（MD5，见 `generate_row_id`），同一 ID 永远得到同一行号。
    /// 写锁覆盖"查 + 占位"，同 ID 并发不会触发重试，9→18 位循环仅作防御。
    ///
    /// # Errors
    ///
    /// 18 位内仍无法登记（防御路径；正常路径不可达）时返回
    /// [`StorageError::Corrupt`]。
    pub fn store_cache_in_memory(&self, id: &str) -> Result<i64, StorageError> {
        // 持写锁完成"查 + 占位"，等价 Go 的 `Load` + `LoadOrStore`：同一 ID 的并发调用
        // 不会把彼此逼到更长的位数，因此下面的 `Entry::Vacant` 必成立。
        let mut rows = write_map(&self.inner.cache_rows);
        if let Some(&row) = rows.get(id) {
            return Ok(row);
        }

        let mut row = 0;
        for digits in ROW_ID_MIN_DIGITS..=ROW_ID_MAX_DIGITS {
            row = generate_row_id(id, digits)?;
            // 写锁保证同 ID 并发不会走到这里重试；长度循环只作防御（对齐 Go 的冲突分支）。
            if let Entry::Vacant(entry) = rows.entry(id.to_owned()) {
                entry.insert(row);
                break;
            }
            if digits == ROW_ID_MAX_DIGITS {
                return Err(StorageError::Corrupt(format!(
                    "无法在 {ROW_ID_MAX_DIGITS} 位内为消息 ID 生成唯一行号"
                )));
            }
        }
        drop(rows);

        // Go 无条件写反表：行号撞车时后写覆盖，与 Go 的 `globalReverseMapMsgid.Store` 一致。
        write_map(&self.inner.cache_ids).insert(row, id.to_owned());
        Ok(row)
    }

    /// 按行号反查 ID（Go `GetCacheIDFromMemoryByRowID`）。
    ///
    /// 非十进制整数（含溢出）按 `0` 处理，与 Go 忽略 `strconv.ParseInt` 错误的行为一致。
    #[must_use]
    pub fn get_cache_id_from_memory_by_row_id(&self, row_id: &str) -> Option<String> {
        let row = row_id.parse::<i64>().unwrap_or(0);
        read_map(&self.inner.cache_ids).get(&row).cloned()
    }
}

// ---- echo key 构造（Go `EchoMapping.GenerateKey*`） ----

/// `<app_id>_<s>`：消息类型与 int64 场景 message_id 共用（Go `GenerateKey`）。
#[must_use]
pub fn echo_key(app_id: &str, s: i64) -> String {
    format!("{app_id}_{s}")
}

/// `<app_id>_<group_id>_<user_id>`（Go `GenerateKeyv2`）。
#[must_use]
pub fn echo_key_v2(app_id: &str, group_id: i64, user_id: i64) -> String {
    format!("{app_id}_{group_id}_{user_id}")
}

/// `<app_id>_<group_id>`：int64 群号的 event_id（Go `GenerateKeyEventID`）。
#[must_use]
pub fn echo_key_event_id(app_id: &str, group_id: i64) -> String {
    format!("{app_id}_{group_id}")
}

/// `<app_id>_<group_id>`：字符串群号的 event_id（Go `GenerateKeyEventIDV2`）。
#[must_use]
pub fn echo_key_event_id_v2(app_id: &str, group_id: &str) -> String {
    format!("{app_id}_{group_id}")
}

/// `<app_id>_<s>`：字符串场景的 message_id（Go `GenerateKeyv3`）。
#[must_use]
pub fn echo_key_v3(app_id: &str, s: &str) -> String {
    format!("{app_id}_{s}")
}

// ---- 内部状态 ----

/// 一条懒消息记录（Go `messageRecord`）。
#[derive(Debug, Clone)]
struct LazyRecord {
    message_id: String,
    timestamp: SystemTime,
    /// 与 Go 字段同名；本层只写不改，选择时"0 优先"。
    usage_count: u32,
}

/// 共享状态：所有 [`EchoStore`] 句柄看到同一份映射。
struct EchoInner {
    msg_types: RwLock<HashMap<String, String>>,
    msg_ids: RwLock<HashMap<String, String>>,
    event_ids: RwLock<HashMap<String, String>>,
    int_mapping: RwLock<HashMap<i64, i32>>,
    seq: SeqTable,
    ssm: SsmQueue,
    ref_idx: RwLock<HashMap<String, String>>,
    lazy: RwLock<HashMap<String, Vec<LazyRecord>>>,
    /// ID → 行号（Go `globalSyncMapMsgid`）。
    cache_rows: RwLock<HashMap<String, i64>>,
    /// 行号 → ID（Go `globalReverseMapMsgid`）。
    cache_ids: RwLock<HashMap<i64, String>>,
    cleanup: Mutex<Option<CleanupHandle>>,
}

impl EchoInner {
    fn new() -> Self {
        Self {
            msg_types: RwLock::new(HashMap::new()),
            msg_ids: RwLock::new(HashMap::new()),
            event_ids: RwLock::new(HashMap::new()),
            int_mapping: RwLock::new(HashMap::new()),
            seq: SeqTable::new(),
            ssm: SsmQueue::new(),
            ref_idx: RwLock::new(HashMap::new()),
            lazy: RwLock::new(HashMap::new()),
            cache_rows: RwLock::new(HashMap::new()),
            cache_ids: RwLock::new(HashMap::new()),
            cleanup: Mutex::new(None),
        }
    }
}

/// seq 表：读-改-写必须整段持锁，否则多段回复会拿到相同 seq 触发 QQ 40054005。
struct SeqTable {
    gate: Mutex<()>,
    values: RwLock<HashMap<String, i32>>,
}

impl SeqTable {
    fn new() -> Self {
        Self {
            gate: Mutex::new(()),
            values: RwLock::new(HashMap::new()),
        }
    }

    fn set(&self, key: &str, value: i32) {
        let _gate = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        write_map(&self.values).insert(key.to_owned(), value);
    }

    /// 返回 `(递增前, 递增后)`；key 缺失时按 `ramdom_seq` 决定起点。
    fn current_and_advance(&self, key: &str, ramdom_seq: bool) -> (i32, i32) {
        let _gate = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        let current = read_map(&self.values)
            .get(key)
            .copied()
            .unwrap_or_else(|| if ramdom_seq { random_in_seq_range() } else { 0 });
        // Go 会回绕；这里饱和，永不 panic。
        let next = current.saturating_add(1);
        write_map(&self.values).insert(key.to_owned(), next);
        (current, next)
    }

    fn clear(&self) {
        let _gate = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        write_map(&self.values).clear();
    }
}

/// SSM 队列：计数与栈分开（计数跨清理存活，Go 同）。
struct SsmQueue {
    counter: AtomicU64,
    stack: Mutex<Vec<MessageGroupPair>>,
}

impl SsmQueue {
    fn new() -> Self {
        Self {
            counter: AtomicU64::new(0),
            stack: Mutex::new(Vec::new()),
        }
    }

    fn next_correlation_id(&self, group_id: &str) -> String {
        let counter = self.counter.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        // Go 按字节切前 8 个；这里用 get 避免切在 UTF-8 字符中间导致 panic。
        let prefix = group_id.get(..SSM_GROUP_PREFIX_LEN).unwrap_or(group_id);
        format!("ssm-{prefix}-{counter}")
    }
}

/// 后台清理线程的句柄。
struct CleanupHandle {
    stop: mpsc::Sender<()>,
    join: JoinHandle<()>,
}

// ---- 小工具 ----

/// 读锁；中毒不 panic，直接取回内层数据（与 `idmap` 模块一致）。
fn read_map<K, V>(map: &RwLock<HashMap<K, V>>) -> RwLockReadGuard<'_, HashMap<K, V>> {
    map.read().unwrap_or_else(PoisonError::into_inner)
}

/// 写锁；中毒不 panic，直接取回内层数据（与 `idmap` 模块一致）。
fn write_map<K, V>(map: &RwLock<HashMap<K, V>>) -> RwLockWriteGuard<'_, HashMap<K, V>> {
    map.write().unwrap_or_else(PoisonError::into_inner)
}

/// Go `groupID + "." + userID`。
fn lazy_key_v2(group_id: &str, user_id: &str) -> String {
    format!("{group_id}.{user_id}")
}

/// Go `record.timestamp.After(now - window)`：严格小于窗口才算有效。
///
/// 时间戳在未来（时钟回拨）时视为有效，与 Go 的比较结果一致。
fn within_window(timestamp: SystemTime, now: SystemTime, window: Duration) -> bool {
    match now.duration_since(timestamp) {
        Ok(age) => age < window,
        Err(_) => true,
    }
}

/// splitmix64 的全局状态：等价 Go 的全局 `math/rand`，无需额外依赖。
static RANDOM_STATE: AtomicU64 = AtomicU64::new(0x9E37_79B9_7F4A_7C15);

fn next_random() -> u64 {
    let mut z = RANDOM_STATE
        .fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// 1..=10000 的随机 seq（Go `rng.Intn(10000) + 1`）。
fn random_in_seq_range() -> i32 {
    i32::try_from(next_random() % RANDOM_SEQ_SPAN).map_or(1, |value| value + 1)
}

/// Go `idmap.GenerateRowID`：MD5 → 十六进制 → 只保留 ASCII 数字 → 取前 `length` 位
/// （不足右侧补 `0`）→ 十进制解析。
fn generate_row_id(id: &str, length: usize) -> Result<i64, StorageError> {
    let digest = Md5::digest(id.as_bytes());
    let mut digits = String::with_capacity(digest.len() * 2);
    for &byte in digest.iter() {
        // 等价 Go 逐字符过滤 `hex.EncodeToString` 的结果：每个字节取高/低半字节。
        for nibble in [byte >> 4, byte & 0x0f] {
            if nibble < 10 {
                digits.push(char::from(b'0' + nibble));
            }
        }
    }
    row_id_from_digits(&digits, length)
}

/// 按 Go 的规则把数字串裁到 `length` 位（不足右侧补 `0`）并解析为 `i64`。
fn row_id_from_digits(digits: &str, length: usize) -> Result<i64, StorageError> {
    let mut text = String::with_capacity(length);
    text.extend(digits.chars().take(length));
    while text.len() < length {
        text.push('0');
    }
    text.parse::<i64>()
        .map_err(|_| StorageError::Corrupt(format!("行号派生失败：无法解析 {text}")))
}

#[cfg(test)]
mod tests;
