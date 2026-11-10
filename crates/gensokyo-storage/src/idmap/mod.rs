//! idmap 本地存储（redb）：身份 ↔ vUIN、消息 ID ↔ 虚拟 ID。
//!
//! 表的定义、key 构造与 value 编码逐条对齐 Go `idmap/map_service.go`，差异见 crate 文档。

pub(crate) mod encoding;
pub mod grpc;

mod batch;
mod lookup_cache;

pub use batch::IdMapWriteBatch;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use redb::{
    Database, ReadTransaction, ReadableDatabase, ReadableTable, TableDefinition, WriteTransaction,
};
use tracing::{debug, warn};

use crate::StorageError;
use crate::echo::generate_row_id;
use encoding::{decode_i64, encode_i64, normalize_identity, vuin_identity_key, vuin_prefix};
use lookup_cache::{LookupCache, LookupKey, LookupValue};

/// 身份库默认文件名（Go `openid-map.db` 的 redb 等价物）。
pub const DEFAULT_IDENTITY_PATH: &str = "openid-map.redb";
/// 消息库默认文件名（Go `msgid-map.db` 的 redb 等价物）。
pub const DEFAULT_MSG_PATH: &str = "msgid-map.redb";
/// 消息 ID 的默认 TTL（Go `GetMsgIDTTLSeconds` 的缺省值）。
pub const DEFAULT_MSG_TTL: Duration = Duration::from_secs(3600);
/// 名称缓存默认 TTL（Go `StoreUserName`/`GetUserName` 的 10 分钟）。
pub const DEFAULT_USER_NAME_TTL: Duration = Duration::from_secs(600);
/// 名称缓存条数达到该值时，写入操作顺带清理过期项（Go 由后台协程每 10 分钟清理）。
pub const USER_NAME_SWEEP_THRESHOLD: usize = 1024;
/// 读路径热查询缓存的默认容量（条）；`0` 关闭缓存。
pub const DEFAULT_LOOKUP_CACHE_CAPACITY: usize = 4096;
/// 读路径热查询缓存的默认单条 TTL；`Duration::ZERO` 关闭缓存。
pub const DEFAULT_LOOKUP_CACHE_TTL: Duration = Duration::from_secs(60);

/// 身份库：正表与反表。
///
/// `pub(crate)`：JSONL 导入器（[`crate::importer`]）在同一 crate 内按表名逐字节搬运，
/// 与 Go 一致的表名/key/value 编码是导入器的唯一契约，重建一份就会漂移。
pub(crate) const IDENTITY_TO_VUIN: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("identity_to_vuin");
pub(crate) const VUIN_TO_IDENTITY: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("vuin_to_identity");
pub(crate) const LAST_SEEN: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("last_seen");

/// 身份库：配置 KV（Go `ConfigBucket` = `"config"`），key 为 `"<section>:<subtype>"`，
/// value 是原始字符串字节（Go `WriteConfig` 直接 `b.Put(key, []byte(value))`）。
pub(crate) const CONFIG: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("config");

/// 身份库：用户信息 KV（Go `UserInfoBucket` = `"UserInfo"`）。
///
/// key 为 `"<rawID>:<UserID>"`、value 为 `structs.FriendData` 的 JSON 字节
/// （Go `StoreUserInfo` / `ListAllUsers`）。表名沿用本 crate 其余表的 snake_case 约定
/// （Go 的 bucket 名是 `UserInfo`，bbolt 按名字精确匹配，这里只保留同一张表的语义）。
pub(crate) const USER_INFO: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("user_info");

/// 两个库各自的 `meta` 表（Go 同名 bucket）。
pub(crate) const META: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("meta");

/// 消息库：正表、反表、过期时间与"群 × 用户 最新消息"。
pub(crate) const MSG_TO_VIRTUAL: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("msg_to_virtual");
pub(crate) const VIRTUAL_TO_MSG: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("virtual_to_msg");
pub(crate) const EXPIRES_AT: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("expires_at");
pub(crate) const LATEST_BY_GROUP_USER: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("latest_by_group_user");
pub(crate) const LATEST_EXPIRES_AT: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("latest_expires_at");

/// Go `identityCounterKey` / `msgCounterKey` / `schemaVersionKey`。
pub(crate) const CURRENT_VUIN_KEY: &str = "current_vuin";
pub(crate) const CURRENT_MSG_KEY: &str = "current_msg";
const SCHEMA_VERSION_KEY: &str = "schema_version";
const SCHEMA_VERSION: &[u8] = b"2";
/// Go `defaultLastSeenScope`。
const LAST_SEEN_SCOPE: &str = "default";

/// `hash_id` 模式下 vUIN 的最短/最长位数（Go `allocateVuinTx` 的 `9..=18`）。
pub(crate) const HASH_VUIN_MIN_DIGITS: usize = 9;
pub(crate) const HASH_VUIN_MAX_DIGITS: usize = 18;

/// [`IdMapOptions`] 的 AppID 占位值：空串。
///
/// [`IdMapStore::open`] 拒绝该值：空 AppID 会让 OpenID 的 key 变成
/// `openid:QQ::<raw>`，与 Go 的 `openid:QQ:<AppID>:<raw>` 永不相等，
/// 且这种 key 一旦写进库就无法再与 Go 对齐。
pub const UNSET_APP_ID: &str = "";

/// [`IdMapStore`] 的打开参数。
#[derive(Debug, Clone)]
pub struct IdMapOptions {
    /// 身份库路径，默认 [`DEFAULT_IDENTITY_PATH`]。
    pub identity_path: PathBuf,
    /// 消息库路径，默认 [`DEFAULT_MSG_PATH`]。
    pub msg_path: PathBuf,
    /// QQ AppID，参与 OpenID 的 key（Go `config.GetAppIDStr()`）。
    pub app_id: String,
    /// Go `idmap_isolation` 开启时的 `uin`；`None` 等价关闭隔离。
    pub uin: Option<String>,
    /// Go `hash_id`：vUIN 用 MD5 派生（`GenerateRowID`）；`false` 顺序递增。
    pub hash_id: bool,
    /// 消息 ID 的 TTL，默认 [`DEFAULT_MSG_TTL`]。
    pub msg_ttl: Duration,
    /// 名称缓存 TTL，默认 [`DEFAULT_USER_NAME_TTL`]。
    pub user_name_ttl: Duration,
    /// 读路径热查询缓存容量（条），默认 [`DEFAULT_LOOKUP_CACHE_CAPACITY`]；`0` 关闭缓存。
    ///
    /// 只影响读速度：写入提交后相关 key 立即失效，命中与否对外不可观测。
    pub lookup_cache_capacity: usize,
    /// 读路径热查询缓存单条 TTL，默认 [`DEFAULT_LOOKUP_CACHE_TTL`]；`Duration::ZERO` 关闭缓存。
    pub lookup_cache_ttl: Duration,
}

impl Default for IdMapOptions {
    /// 除 AppID 外全部就绪：AppID 是 [`UNSET_APP_ID`] 占位，
    /// [`IdMapStore::open`] 会拒绝，调用方必须用 [`IdMapOptions::with_app_id`] 显式设置。
    fn default() -> Self {
        Self {
            identity_path: PathBuf::from(DEFAULT_IDENTITY_PATH),
            msg_path: PathBuf::from(DEFAULT_MSG_PATH),
            app_id: UNSET_APP_ID.to_owned(),
            uin: None,
            hash_id: false,
            msg_ttl: DEFAULT_MSG_TTL,
            user_name_ttl: DEFAULT_USER_NAME_TTL,
            lookup_cache_capacity: DEFAULT_LOOKUP_CACHE_CAPACITY,
            lookup_cache_ttl: DEFAULT_LOOKUP_CACHE_TTL,
        }
    }
}

impl IdMapOptions {
    /// 指定两个库的路径，其余取默认值。
    #[must_use]
    pub fn new(identity_path: impl Into<PathBuf>, msg_path: impl Into<PathBuf>) -> Self {
        Self {
            identity_path: identity_path.into(),
            msg_path: msg_path.into(),
            ..Self::default()
        }
    }

    /// 设置 AppID。
    #[must_use]
    pub fn with_app_id(mut self, app_id: impl Into<String>) -> Self {
        self.app_id = app_id.into();
        self
    }

    /// 设置隔离用的 `uin`（等价 Go `idmap_isolation` + `uin`）。
    #[must_use]
    pub fn with_uin(mut self, uin: impl Into<String>) -> Self {
        self.uin = Some(uin.into());
        self
    }

    /// 设置 `hash_id`：`true` 时 vUIN 与 Go 一致按 MD5 派生（等价 Go `hash_id: true`）。
    #[must_use]
    pub fn with_hash_id(mut self, hash_id: bool) -> Self {
        self.hash_id = hash_id;
        self
    }

    /// 设置消息 ID 的 TTL。
    #[must_use]
    pub fn with_msg_ttl(mut self, ttl: Duration) -> Self {
        self.msg_ttl = ttl;
        self
    }

    /// 设置名称缓存 TTL。
    #[must_use]
    pub fn with_user_name_ttl(mut self, ttl: Duration) -> Self {
        self.user_name_ttl = ttl;
        self
    }

    /// 设置读路径热查询缓存容量（条）；`0` 关闭缓存，行为与不缓存时逐值相同。
    #[must_use]
    pub fn with_lookup_cache_capacity(mut self, capacity: usize) -> Self {
        self.lookup_cache_capacity = capacity;
        self
    }

    /// 设置读路径热查询缓存单条 TTL；`Duration::ZERO` 关闭缓存。
    #[must_use]
    pub fn with_lookup_cache_ttl(mut self, ttl: Duration) -> Self {
        self.lookup_cache_ttl = ttl;
        self
    }
}

/// 名称缓存项。
#[derive(Debug, Clone)]
struct UserNameEntry {
    name: String,
    stored_at: Instant,
}

/// 一条身份映射（Go `IdentityMapping`）：正表的一行加该 vUIN 的名称缓存。
///
/// 字段即 `/getid` type 19 的响应字段（`real_id` / `virtual_id` / `username`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityMapping {
    /// 原始身份串（反表值，已剥离 `uin:` 前缀）。
    pub real_id: String,
    /// 正表里该身份对应的 vUIN。
    pub virtual_id: i64,
    /// 名称缓存，未缓存时为空串（Go `GetUserName` 未命中返回 `""`）。
    pub username: String,
}

/// [`IdMapStore::resolve_rebind_identity`] 的结果：正表 key + 反表要写的原始身份串。
struct NormalizedRebind {
    key: String,
    raw: String,
}

/// idmap 本地存储。
///
/// `Send + Sync`：内部两个 redb [`Database`]（各自串行写事务）、一个名称缓存互斥锁，以及读
/// 路径的有界热查询缓存（[`lookup_cache`]，写入提交后精确失效）。
#[derive(Debug)]
pub struct IdMapStore {
    options: IdMapOptions,
    identity_db: Database,
    msg_db: Database,
    user_names: Mutex<HashMap<i64, UserNameEntry>>,
    lookup_cache: LookupCache,
}

impl IdMapStore {
    /// 打开（必要时创建）两个库并建表。
    ///
    /// # Errors
    ///
    /// AppID 为空、路径不可写、两个库指向同一文件、或 redb 打开/建表失败时返回
    /// [`StorageError`]。
    pub fn open(options: IdMapOptions) -> Result<Self, StorageError> {
        if options.app_id.is_empty() {
            return Err(StorageError::Corrupt(
                "IdMapOptions.app_id 不能为空：OpenID key 需要 app_id 参与（Go config.GetAppIDStr()），\
                 空 AppID 会写出 openid:QQ::<raw> 这种与 Go 永不相等且不可迁移的 key"
                    .to_owned(),
            ));
        }
        if options.identity_path == options.msg_path {
            return Err(StorageError::Corrupt(
                "身份库与消息库不能指向同一文件".to_owned(),
            ));
        }
        let identity_db = Database::create(&options.identity_path).map_err(rb)?;
        let msg_db = Database::create(&options.msg_path).map_err(rb)?;
        let lookup_cache = LookupCache::new(options.lookup_cache_capacity, options.lookup_cache_ttl);
        let store = Self {
            options,
            identity_db,
            msg_db,
            user_names: Mutex::new(HashMap::new()),
            lookup_cache,
        };
        store.init_schema()?;
        debug!(
            identity_path = %store.options.identity_path.display(),
            msg_path = %store.options.msg_path.display(),
            "idmap 存储已打开"
        );
        Ok(store)
    }

    /// 打开参数。
    #[must_use]
    pub fn options(&self) -> &IdMapOptions {
        &self.options
    }

    /// 两个库是否都还没有任何记录（`meta` 内的 `schema_version` 不算数据）。
    ///
    /// 供导入器判断「默认要求目标库为空」（`--merge` 的反面）与启动期自动导入的空库检测。
    ///
    /// # Errors
    ///
    /// redb 事务打开失败时返回 [`StorageError`]。
    pub fn is_empty(&self) -> Result<bool, StorageError> {
        let txn = self.identity_db.begin_read().map_err(rb)?;
        for table in [
            IDENTITY_TO_VUIN,
            VUIN_TO_IDENTITY,
            LAST_SEEN,
            CONFIG,
            USER_INFO,
        ] {
            if !table_is_empty(&txn, table)? {
                return Ok(false);
            }
        }
        let txn = self.msg_db.begin_read().map_err(rb)?;
        for table in [
            MSG_TO_VIRTUAL,
            VIRTUAL_TO_MSG,
            EXPIRES_AT,
            LATEST_BY_GROUP_USER,
            LATEST_EXPIRES_AT,
        ] {
            if !table_is_empty(&txn, table)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// 身份库句柄（导入器在单写事务里跨表搬运时用）。
    pub(crate) fn identity_db(&self) -> &Database {
        &self.identity_db
    }

    /// 消息库句柄（同上）。
    pub(crate) fn msg_db(&self) -> &Database {
        &self.msg_db
    }

    /// 清空读路径缓存并前进代际号：给绕过本类型直接写库的路径（JSONL 导入）用。
    pub(crate) fn invalidate_lookup_cache(&self) {
        self.lookup_cache.clear();
    }

    /// 建表并写入 schema 版本（Go `initNewDBs` 的建 bucket + `schema_version`）。
    ///
    /// 与 Go 一致，每次打开都**无条件**覆盖 `schema_version`；读到未知版本时按 Go
    /// 语义覆盖，但先记 warn（Go 是静默覆盖）。
    fn init_schema(&self) -> Result<(), StorageError> {
        let txn = self.identity_db.begin_write().map_err(rb)?;
        {
            let mut meta = txn.open_table(META).map_err(rb)?;
            let _ = txn.open_table(IDENTITY_TO_VUIN).map_err(rb)?;
            let _ = txn.open_table(VUIN_TO_IDENTITY).map_err(rb)?;
            let _ = txn.open_table(LAST_SEEN).map_err(rb)?;
            let _ = txn.open_table(CONFIG).map_err(rb)?;
            let _ = txn.open_table(USER_INFO).map_err(rb)?;
            let existing = meta
                .get(SCHEMA_VERSION_KEY)
                .map_err(rb)?
                .map(|guard| guard.value().to_vec());
            if let Some(bytes) = existing
                && bytes != SCHEMA_VERSION
            {
                warn!(
                    found = %String::from_utf8_lossy(&bytes),
                    expected = %String::from_utf8_lossy(SCHEMA_VERSION),
                    "idmap schema_version 未知，按 Go 语义覆盖"
                );
            }
            meta.insert(SCHEMA_VERSION_KEY, SCHEMA_VERSION)
                .map_err(rb)?;
        }
        txn.commit().map_err(rb)?;

        let txn = self.msg_db.begin_write().map_err(rb)?;
        {
            let _ = txn.open_table(META).map_err(rb)?;
            let _ = txn.open_table(MSG_TO_VIRTUAL).map_err(rb)?;
            let _ = txn.open_table(VIRTUAL_TO_MSG).map_err(rb)?;
            let _ = txn.open_table(EXPIRES_AT).map_err(rb)?;
            let _ = txn.open_table(LATEST_BY_GROUP_USER).map_err(rb)?;
            let _ = txn.open_table(LATEST_EXPIRES_AT).map_err(rb)?;
        }
        txn.commit().map_err(rb)?;
        Ok(())
    }

    /// 查身份对应的 vUIN，查不到则分配新 vUIN（Go `storeIdentity`）。
    ///
    /// 返回值的第一项是归一化后的原始身份串（Go `identity.Raw`），等于
    /// [`IdMapStore::retrieve_row_by_id`] 反查到的值。
    ///
    /// # Errors
    ///
    /// redb 事务失败或计数器/表数据损坏时返回 [`StorageError`]。
    pub fn retrieve_virtual_value(&self, identity: &str) -> Result<(String, i64), StorageError> {
        self.with_write_batch(|batch| batch.retrieve_virtual_value(identity))
    }

    /// 在已打开的写事务里查/分配身份（[`IdMapWriteBatch`] 与
    /// [`IdMapStore::retrieve_virtual_value`] 的唯一实现）。
    ///
    /// 批内重复解析同一身份时，写事务里的正表已含本事务未提交的行，因此不会重复分配。
    fn resolve_identity_tx(
        &self,
        txn: &WriteTransaction,
        normalized: &encoding::NormalizedIdentity,
    ) -> Result<(i64, bool), StorageError> {
        let existing = {
            let forward = txn.open_table(IDENTITY_TO_VUIN).map_err(rb)?;
            let value = forward.get(normalized.key.as_str()).map_err(rb)?;
            match value {
                Some(guard) => Some(decode_vuin(guard.value())?),
                None => None,
            }
        };
        match existing {
            Some(vuin) => {
                touch_last_seen_tx(txn, vuin, &normalized.raw)?;
                Ok((vuin, false))
            }
            None => {
                // 写事务内读 counter 与已占用的 vUIN：`next_free_vuin`/`hash_vuin` 读的是已提交
                // 状态，但 counter 取本事务视角（含本批此前的分配），因此批内不会重号。
                let current = read_counter_tx(txn, CURRENT_VUIN_KEY)?;
                let planned = if self.options.hash_id {
                    self.hash_vuin(&normalized.raw)?
                } else {
                    self.next_free_vuin(current)?
                };
                let vuin = allocate_vuin_tx(txn, planned, normalized, self.options.hash_id)?;
                Ok((vuin, true))
            }
        }
    }

    /// 只查身份对应的 vUIN，**不分配**（Go `lookupVirtualIdentity` / `RetrieveVirtualValue`
    /// 的新库路径：查不到就是查不到）。
    ///
    /// # Errors
    ///
    /// 身份未登记返回 [`StorageError::NotFound`]；redb 事务失败或正表数据损坏返回对应变体。
    pub fn lookup_virtual_value(&self, identity: &str) -> Result<i64, StorageError> {
        let normalized =
            normalize_identity(identity, &self.options.app_id, self.options.uin.as_deref());
        let key = LookupKey::Identity(normalized.key.clone());
        if let Some(value) = self.lookup_cache.get(&key) {
            return match value {
                LookupValue::Vuin(vuin) => Ok(vuin),
                _ => Err(StorageError::NotFound(format!("身份 {}", normalized.raw))),
            };
        }
        // 只有读窗口内没有提交发生（代际号未变）才回填：否则可能把提交前的旧值写成缓存
        let epoch = self.lookup_cache.epoch();
        let txn = self.identity_db.begin_read().map_err(rb)?;
        let forward = txn.open_table(IDENTITY_TO_VUIN).map_err(rb)?;
        let result = match forward.get(normalized.key.as_str()).map_err(rb)? {
            Some(guard) => decode_vuin(guard.value()),
            None => Err(StorageError::NotFound(format!("身份 {}", normalized.raw))),
        };
        drop(forward);
        drop(txn);
        if self.lookup_cache.epoch() == epoch {
            match &result {
                Ok(vuin) => self.lookup_cache.insert(key, LookupValue::Vuin(*vuin)),
                // 负结果同样缓存：@ 未登记用户是高频路径；分配身份时会精确失效这条
                Err(err) if err.is_not_found() => {
                    self.lookup_cache.insert(key, LookupValue::Missing);
                }
                Err(_) => {}
            }
        }
        result
    }

    /// 从 vUIN 反查身份（Go `retrieveIdentity`）。
    ///
    /// 与 Go 一致：先读 `last_seen`，normalize 后校验正表仍指向该 vUIN，命中即返回；
    /// 否则扫描反表（同一 vUIN 下可能有多个身份 key，Go 迁移遗留），优先返回
    /// `openid:` 前缀或 OpenID 形态的原始串，再回退第一个。
    ///
    /// 结果带读路径缓存（[`IdMapOptions::lookup_cache_capacity`]）：写入提交后精确失效，
    /// 因此与不缓存时逐值相同，只是重复反查不必再开读事务。
    ///
    /// # Errors
    ///
    /// 查不到时返回 [`StorageError::NotFound`]。
    pub fn retrieve_row_by_id(&self, vuin: i64) -> Result<String, StorageError> {
        if vuin <= 0 {
            return Err(StorageError::NotFound(format!("vUIN={vuin}")));
        }
        let key = LookupKey::Vuin(vuin);
        if let Some(value) = self.lookup_cache.get(&key) {
            return match value {
                LookupValue::Raw(raw) => Ok(raw),
                _ => Err(StorageError::NotFound(format!("vUIN={vuin}"))),
            };
        }
        // 只有读窗口内没有提交发生（代际号未变）才回填：否则可能把提交前的旧值写成缓存
        let epoch = self.lookup_cache.epoch();
        let result = self.read_row_by_id(vuin);
        if self.lookup_cache.epoch() == epoch {
            match &result {
                Ok(raw) => self.lookup_cache.insert(key, LookupValue::Raw(raw.clone())),
                // 负结果同样缓存：`force_unbind` 会精确失效对应 vUIN
                Err(err) if err.is_not_found() => {
                    self.lookup_cache.insert(key, LookupValue::Missing);
                }
                Err(_) => {}
            }
        }
        result
    }

    /// [`IdMapStore::retrieve_row_by_id`] 的不带缓存实现。
    fn read_row_by_id(&self, vuin: i64) -> Result<String, StorageError> {
        let txn = self.identity_db.begin_read().map_err(rb)?;
        {
            let last_seen = txn.open_table(LAST_SEEN).map_err(rb)?;
            let seen = last_seen
                .get(last_seen_key(vuin).as_str())
                .map_err(rb)?
                .map(|guard| guard.value().to_vec());
            if let Some(bytes) = seen {
                let raw = decode_text(&bytes)?;
                if !raw.is_empty() {
                    let normalized =
                        normalize_identity(&raw, &self.options.app_id, self.options.uin.as_deref());
                    let forward = txn.open_table(IDENTITY_TO_VUIN).map_err(rb)?;
                    let mapped = forward
                        .get(normalized.key.as_str())
                        .map_err(rb)?
                        .map(|guard| decode_vuin(guard.value()))
                        .transpose()?;
                    if mapped == Some(vuin) {
                        return Ok(raw);
                    }
                }
            }
        }
        let prefix = vuin_prefix(vuin);
        let table = txn.open_table(VUIN_TO_IDENTITY).map_err(rb)?;

        let mut fallback: Option<String> = None;
        for entry in table.range(prefix.as_str()..).map_err(rb)? {
            let (key, value) = entry.map_err(rb)?;
            let key = key.value();
            if !key.starts_with(prefix.as_str()) {
                break;
            }
            let raw = decode_text(value.value())?;
            let identity_key = key.strip_prefix(prefix.as_str()).unwrap_or_default();
            if identity_key.starts_with("openid:") || (is_openid(&raw) && !raw.starts_with("rUIN-"))
            {
                return Ok(raw);
            }
            if fallback.is_none() {
                fallback = Some(raw);
            }
        }
        fallback.ok_or_else(|| StorageError::NotFound(format!("vUIN={vuin}")))
    }

    /// 枚举全部身份映射（Go `ListAllIdentities`）：只读正表，按 key 升序。
    ///
    /// `real_id` 取反表里保存的原始身份串。Go 用的是 `stripUinPrefix(正向 key)`，也就是
    /// `openid:QQ:<AppID>:<raw>` 这种归一化 key，与其字段注释“OpenID（去除 uin: 前缀）”不符；
    /// 这里返回调用方真正想要的原始串（反表缺行时回退 key）。顺序与 Go 的 bbolt `ForEach`
    /// 一致，都是 key 字节序。隔离重绑定写的 alias（`raw:<十进制 vUIN>`）不算身份，
    /// 按 Go 跳过复合键的口径过滤。
    ///
    /// # Errors
    ///
    /// redb 事务失败或表数据损坏时返回 [`StorageError`]。
    pub fn list_identities(&self) -> Result<Vec<IdentityMapping>, StorageError> {
        let txn = self.identity_db.begin_read().map_err(rb)?;
        let forward = txn.open_table(IDENTITY_TO_VUIN).map_err(rb)?;
        let reverse = txn.open_table(VUIN_TO_IDENTITY).map_err(rb)?;
        let mut mappings = Vec::new();
        for entry in forward.iter().map_err(rb)? {
            let (key, value) = entry.map_err(rb)?;
            let identity_key = key.value();
            // Go `ListAllIdentities` 跳过含 `:` 的复合键（隔离 alias 等）；redb 的真实身份
            // key 一律带 scheme 前缀（`openid:` / `ruin:` / `raw:`），照抄会一条不剩，故只
            // 跳过隔离模式下 `write_rebind_alias` 写的 `raw:<十进制 vUIN>`。
            if self.options.uin.is_some() && is_isolation_alias(identity_key) {
                continue;
            }
            let virtual_id = decode_vuin(value.value())?;
            let reverse_key = vuin_identity_key(virtual_id, identity_key);
            let real_id = match reverse.get(reverse_key.as_str()).map_err(rb)? {
                Some(guard) => decode_text(guard.value())?,
                None => identity_key.to_owned(),
            };
            let username = self.get_user_name(virtual_id).unwrap_or_default();
            mappings.push(IdentityMapping {
                real_id,
                virtual_id,
                username,
            });
        }
        Ok(mappings)
    }

    /// 强制解绑（Go `ForceUnbindID`）：删除正表条目与该身份的全部反对表条目。
    ///
    /// `id` 支持 Go 的两种入参：原始身份串，或十进制 vUIN（后者经
    /// [`IdMapStore::retrieve_row_by_id`] 反查原始身份）。返回删除的**反表**条目数
    /// （Go 口径：正表不计数）。
    ///
    /// 与 Go 的差异：Go 删正表时用 `uinKey(raw)`（没配 `uin` 时就是 `raw`），永远命中不了
    /// `openid:QQ:<AppID>:<raw>` 这类真实 key，解绑后正表残留；这里按 `normalize_identity`
    /// 算出真实 key 再删，正反查同时失效。
    ///
    /// # Errors
    ///
    /// `id` 为空、vUIN 反查不到、或 redb 事务/表数据失败时返回 [`StorageError`]。
    pub fn force_unbind(&self, id: &str) -> Result<usize, StorageError> {
        if id.is_empty() {
            return Err(StorageError::Corrupt("id 不能为空".to_owned()));
        }
        let raw = match id.parse::<i64>() {
            Ok(vuin) => self.retrieve_row_by_id(vuin).map_err(|err| {
                if err.is_not_found() {
                    StorageError::NotFound(format!("虚拟 ID {id} 未找到对应 OpenID"))
                } else {
                    err
                }
            })?,
            Err(_) => id.to_owned(),
        };
        let normalized =
            normalize_identity(&raw, &self.options.app_id, self.options.uin.as_deref());
        let txn = self.identity_db.begin_write().map_err(rb)?;
        let (removed, unbound_vuins) = {
            let mut forward = txn.open_table(IDENTITY_TO_VUIN).map_err(rb)?;
            // 删之前取出被解绑的 vUIN：反查缓存按 vUIN 存，解绑后必须一并失效
            let mut vuins: Vec<i64> = forward
                .remove(normalized.key.as_str())
                .map_err(rb)?
                .as_ref()
                .and_then(|guard| decode_i64(guard.value()))
                .into_iter()
                .collect();
            let mut reverse = txn.open_table(VUIN_TO_IDENTITY).map_err(rb)?;
            // 先收集 key 再删：redb 的迭代器持有表的不可变借用。
            let mut stale = Vec::new();
            for entry in reverse.iter().map_err(rb)? {
                let (key, value) = entry.map_err(rb)?;
                if decode_text(value.value())? == normalized.raw {
                    stale.push(key.value().to_owned());
                }
            }
            for key in &stale {
                // key 形态 `<vUIN>:<identity key>`：前缀就是反查缓存要失效的 vUIN
                if let Some(vuin) = key
                    .split_once(':')
                    .and_then(|(head, _)| head.parse::<i64>().ok())
                {
                    vuins.push(vuin);
                }
                reverse.remove(key.as_str()).map_err(rb)?;
            }
            (stale.len(), vuins)
        };
        txn.commit().map_err(rb)?;
        // 先 bump 关闭回填窗口，再失效清理命中窗口
        self.lookup_cache.bump_epoch();
        let mut dirty = vec![LookupKey::Identity(normalized.key)];
        dirty.extend(unbound_vuins.into_iter().map(LookupKey::Vuin));
        self.lookup_cache.invalidate(&dirty);
        Ok(removed)
    }

    /// 把身份重绑定到另一个 vUIN（Go `UpdateVirtualValue`，`idmap/service.go:1405`）。
    ///
    /// # Go 的桶 → 这里的表
    ///
    /// | Go（同一张 `IdentityBucketName`） | 这里（正表 + 反表） |
    /// |----------------------------------|---------------------|
    /// | 旧库反表 `uinRowKey(old)` / `row-<old>` | 无旧库：Rust 只有一份身份库，这一步不存在 |
    /// | 新库反表 `uinRowKey(old)` → `stripUinPrefix` | 反表前缀 `"<old>:"` 扫描，取回原始身份串 |
    /// | 新库正向扫描：值（8 字节）等于 old 的 key | 正表扫描 8 字节 BE 值等于 old 的 key |
    /// | `uin:<id>` = BE(new) + `uin:row-<new>` = `uin:<id>` | 正表 `identity_key` = BE(new) + 反表 `"<new>:<identity_key>"` = 原始身份串 |
    /// | `new == 0`：删 `uin:<id>`，再扫删所有指向同一身份的反查条目 | 同（反表比对 key 后缀与 value） |
    ///
    /// **隔离模式**（Go `GetIdmapIsolation`，这里体现为 [`IdMapOptions::uin`] 已配置）：
    /// Go 在正向桶里写一条 alias `uinKey(new) → BE(old)` 后**直接返回**——不删旧行、不做
    /// exists 检查、不写反查。这里同样只写正表 `raw:<new>`（`normalize_identity("<new>")`
    /// 的结果）→ BE(old)，于是 `lookup_virtual_value("<uin>:<new>")` 命中旧 vUIN
    /// （前缀会被 `normalize_identity` 剥掉），其余映射原样保留。
    ///
    /// 与 Go 的差异：Go 只删 `uinRowKey(old)` 这一个反查 key；redb 反表是
    /// `"<vUIN>:<identity_key>"`，同一 vUIN 下可能挂着多条迁移遗留 key（1:N，见
    /// [`IdMapStore::force_unbind`]）。这里把整个 `"<old>:"` 家族都当作该 vUIN 的身份：
    /// `new != 0` 时整族迁到 `"<new>:"`，`new == 0` 时整族删除，否则会留下"正表仍指 old、
    /// 反表已没有该行"的悬挂映射。
    ///
    /// # Errors
    ///
    /// 旧 vUIN 正反查都落空 → [`StorageError::NotFound`]（Go 的文案是 `不存在:%v`）；
    /// `new` 已被占用 → [`StorageError::AlreadyExists`]（Go 是 `%v :已存在`）；
    /// redb 事务失败 → [`StorageError::Redb`]。
    pub fn update_virtual_value(&self, old_vuin: i64, new_vuin: i64) -> Result<(), StorageError> {
        let identity = self.resolve_rebind_identity(old_vuin)?;
        if self.options.uin.is_some() {
            return self.write_rebind_alias(old_vuin, new_vuin);
        }
        let txn = self.identity_db.begin_write().map_err(rb)?;
        // 要迁/要删的身份：反表 key 后缀 + 该行保存的原始身份串（正表要跟着一起改）
        let mut migrated: Vec<(String, String)> = Vec::new();
        // 被删反表行的 head（`"<vUIN>:"` 前缀）：反查缓存按 vUIN 存，必须一并失效
        let mut unbound_vuins = vec![old_vuin];
        {
            let mut forward = txn.open_table(IDENTITY_TO_VUIN).map_err(rb)?;
            let mut reverse = txn.open_table(VUIN_TO_IDENTITY).map_err(rb)?;
            // Go 的 exists 检查：命中就直接返回，两表都不动。放进写事务里做——redb 同一
            // 时刻只有一个写者，否则两个并发 type=5 会都通过读检查，把两个身份写进同一 new。
            if new_vuin != 0 && prefix_exists(&reverse, new_vuin)? {
                return Err(StorageError::AlreadyExists(new_vuin));
            }
            let prefix = vuin_prefix(old_vuin);
            let mut stale = Vec::new();
            for entry in reverse.range(prefix.as_str()..).map_err(rb)? {
                let (key, value) = entry.map_err(rb)?;
                if !key.value().starts_with(prefix.as_str()) {
                    break;
                }
                let identity_key = key
                    .value()
                    .strip_prefix(prefix.as_str())
                    .unwrap_or_default()
                    .to_owned();
                migrated.push((identity_key, decode_text(value.value())?));
                stale.push(key.value().to_owned());
            }
            for key in &stale {
                reverse.remove(key.as_str()).map_err(rb)?;
            }
            // 反表缺行、只能靠正表兜底时，被选中的身份也要一起迁/删
            if !migrated.iter().any(|(key, _)| key == &identity.key) {
                migrated.push((identity.key.clone(), identity.raw.clone()));
            }
            if new_vuin != 0 {
                let mapped = encode_i64(new_vuin);
                for (key, raw) in &migrated {
                    forward
                        .insert(key.as_str(), mapped.as_slice())
                        .map_err(rb)?;
                    reverse
                        .insert(vuin_identity_key(new_vuin, key).as_str(), raw.as_bytes())
                        .map_err(rb)?;
                }
            } else {
                // new == 0：彻底解绑——删正向映射，否则下次反查仍能拿到 OpenID，等于没解绑。
                for (key, _) in &migrated {
                    forward.remove(key.as_str()).map_err(rb)?;
                }
                // 再扫删所有指向同一身份的重复反表行（迁移期间可能 2 row ↔ 1 身份）
                let mut duplicates = Vec::new();
                for entry in reverse.iter().map_err(rb)? {
                    let (key, value) = entry.map_err(rb)?;
                    let key = key.value();
                    let identity_of_key = key.split_once(':').map_or("", |(_, rest)| rest);
                    // 字节比较：与 Go 的 `string(v) == string(key)` 同义，非 UTF-8 行不会
                    // 让整次解绑失败
                    if identity_of_key == identity.key || value.value() == identity.raw.as_bytes() {
                        duplicates.push(key.to_owned());
                    }
                }
                for key in &duplicates {
                    if let Some(head) = key
                        .split_once(':')
                        .and_then(|(head, _)| head.parse::<i64>().ok())
                    {
                        unbound_vuins.push(head);
                    }
                    reverse.remove(key.as_str()).map_err(rb)?;
                }
            }
        }
        txn.commit().map_err(rb)?;
        // 先 bump 关闭回填窗口，再精确失效：被迁/被删身份的 key 与全部受影响的 vUIN
        self.lookup_cache.bump_epoch();
        let mut dirty: Vec<LookupKey> = migrated
            .into_iter()
            .map(|(key, _)| LookupKey::Identity(key))
            .collect();
        dirty.push(LookupKey::Vuin(new_vuin));
        dirty.extend(unbound_vuins.into_iter().map(LookupKey::Vuin));
        self.lookup_cache.invalidate(&dirty);
        Ok(())
    }

    /// 两次 [`IdMapStore::update_virtual_value`]（Go `UpdateVirtualValuev2Pro`，
    /// `idmap/service.go:2036`）。
    ///
    /// **非原子**：第一次成功、第二次失败会留下半完成状态——Go 如此，这里照抄，不额外回滚。
    ///
    /// # Errors
    ///
    /// 任一次重绑定失败时返回该次的错误；第二次不会被尝试。
    pub fn update_virtual_value_pro(
        &self,
        old_virtual_value_1: i64,
        new_virtual_value_1: i64,
        old_virtual_value_2: i64,
        new_virtual_value_2: i64,
    ) -> Result<(), StorageError> {
        self.update_virtual_value(old_virtual_value_1, new_virtual_value_1)?;
        self.update_virtual_value(old_virtual_value_2, new_virtual_value_2)
    }

    /// [`IdMapStore::update_virtual_value`] 的第一段：`old vUIN → (正向 key, 原始身份串)`。
    ///
    /// 与 Go 一样两段兜底：反表前缀扫描（Go 的 `uinRowKey(old)`），落空后扫正表里值等于
    /// `old` 的 key。反表命中时 identity key 直接取 key 的后缀（比 `normalize_identity`
    /// 再算一遍更准：迁移遗留 key 也能原样命中）。
    fn resolve_rebind_identity(&self, vuin: i64) -> Result<NormalizedRebind, StorageError> {
        let txn = self.identity_db.begin_read().map_err(rb)?;
        let reverse = txn.open_table(VUIN_TO_IDENTITY).map_err(rb)?;
        let prefix = vuin_prefix(vuin);
        let mut fallback: Option<NormalizedRebind> = None;
        for entry in reverse.range(prefix.as_str()..).map_err(rb)? {
            let (key, value) = entry.map_err(rb)?;
            let key = key.value();
            if !key.starts_with(prefix.as_str()) {
                break;
            }
            let identity_key = key.strip_prefix(prefix.as_str()).unwrap_or_default();
            let raw = decode_text(value.value())?;
            // 与 `read_row_by_id` 同口径：openid 形态优先，其余留作兜底
            if identity_key.starts_with("openid:") || (is_openid(&raw) && !raw.starts_with("rUIN-"))
            {
                return Ok(NormalizedRebind {
                    key: identity_key.to_owned(),
                    raw,
                });
            }
            if fallback.is_none() {
                fallback = Some(NormalizedRebind {
                    key: identity_key.to_owned(),
                    raw,
                });
            }
        }
        if let Some(found) = fallback {
            return Ok(found);
        }
        let forward = txn.open_table(IDENTITY_TO_VUIN).map_err(rb)?;
        for entry in forward.iter().map_err(rb)? {
            let (key, value) = entry.map_err(rb)?;
            if decode_i64(value.value()) != Some(vuin) {
                continue;
            }
            let identity_key = key.value().to_owned();
            // Go 的反查值就是正向 key；redb 反查值才是原始身份串。反表缺行时先用
            // `last_seen`（分配时写入的原始串）兜底，仍对不上才退回 key。
            let raw = match reverse
                .get(vuin_identity_key(vuin, &identity_key).as_str())
                .map_err(rb)?
            {
                Some(guard) => decode_text(guard.value())?,
                None => {
                    let last_seen = txn.open_table(LAST_SEEN).map_err(rb)?;
                    let seen = last_seen
                        .get(last_seen_key(vuin).as_str())
                        .map_err(rb)?
                        .map(|guard| decode_text(guard.value()))
                        .transpose()?;
                    match seen {
                        Some(raw)
                            if normalize_identity(
                                &raw,
                                &self.options.app_id,
                                self.options.uin.as_deref(),
                            )
                            .key == identity_key =>
                        {
                            raw
                        }
                        _ => identity_key.clone(),
                    }
                }
            };
            return Ok(NormalizedRebind {
                key: identity_key,
                raw,
            });
        }
        Err(StorageError::NotFound(format!("vUIN={vuin}")))
    }

    /// 隔离模式的重绑定：只加一条 alias（Go `UpdateVirtualValue` 的 `GetIdmapIsolation` 分支）。
    fn write_rebind_alias(&self, old_vuin: i64, new_vuin: i64) -> Result<(), StorageError> {
        let alias = normalize_identity(
            &new_vuin.to_string(),
            &self.options.app_id,
            self.options.uin.as_deref(),
        );
        let txn = self.identity_db.begin_write().map_err(rb)?;
        {
            let mut forward = txn.open_table(IDENTITY_TO_VUIN).map_err(rb)?;
            let mapped = encode_i64(old_vuin);
            forward
                .insert(alias.key.as_str(), mapped.as_slice())
                .map_err(rb)?;
        }
        txn.commit().map_err(rb)?;
        self.lookup_cache.bump_epoch();
        self.lookup_cache
            .invalidate(&[LookupKey::Identity(alias.key)]);
        Ok(())
    }

    /// vUIN 对应的 `last_seen` 原始身份串（Go `readLastSeenTx`）。
    ///
    /// 分配与命中身份时都会写该表（Go `touchLastSeenTx`）。
    ///
    /// # Errors
    ///
    /// redb 事务失败或数据损坏时返回 [`StorageError`]。
    pub fn last_seen(&self, vuin: i64) -> Result<Option<String>, StorageError> {
        if vuin <= 0 {
            return Ok(None);
        }
        let key = last_seen_key(vuin);
        let txn = self.identity_db.begin_read().map_err(rb)?;
        let table = txn.open_table(LAST_SEEN).map_err(rb)?;
        match table.get(key.as_str()).map_err(rb)? {
            Some(guard) => Ok(Some(decode_text(guard.value())?)),
            None => Ok(None),
        }
    }

    /// 分配或复用真实消息 ID 对应的虚拟 ID（Go `StoreMsgID`），TTL 取
    /// [`IdMapOptions::msg_ttl`]。
    ///
    /// 复用命中时刷新过期时间（与 Go 一致）。
    ///
    /// # Errors
    ///
    /// 消息 ID 为空、redb 事务失败或数据损坏时返回 [`StorageError`]。
    pub fn store_cache(&self, msg_id: &str) -> Result<i64, StorageError> {
        self.store_cache_with_ttl(msg_id, self.options.msg_ttl)
    }

    /// 同 [`IdMapStore::store_cache`]，但显式指定 TTL（测试与按请求覆盖用）。
    ///
    /// TTL 按秒截断；截断后 ≤ 0 时按 Go `StoreMsgID` 回退 [`DEFAULT_MSG_TTL`]（1 小时），
    /// 因此 `Duration::ZERO` 与亚秒 TTL 都拿到 1 小时有效期。
    ///
    /// # Errors
    ///
    /// 消息 ID 为空、redb 事务失败或数据损坏时返回 [`StorageError`]。
    pub fn store_cache_with_ttl(&self, msg_id: &str, ttl: Duration) -> Result<i64, StorageError> {
        self.with_write_batch(|batch| batch.store_cache_with_ttl(msg_id, ttl))
    }

    /// 在已打开的写事务里查/分配虚拟消息 ID（[`IdMapWriteBatch`] 的唯一实现）。
    fn store_cache_tx(
        &self,
        txn: &WriteTransaction,
        msg_id: &str,
        ttl: Duration,
    ) -> Result<(i64, bool), StorageError> {
        let expires = expires_after(unix_now(), ttl);
        let existing = {
            let forward = txn.open_table(MSG_TO_VIRTUAL).map_err(rb)?;
            let value = forward.get(msg_id).map_err(rb)?;
            match value {
                Some(guard) => Some(decode_vuin(guard.value())?),
                None => None,
            }
        };
        match existing {
            Some(virtual_id) => {
                let key = virtual_id.to_string();
                let expires_bytes = encode_i64(expires);
                let mut expires_table = txn.open_table(EXPIRES_AT).map_err(rb)?;
                expires_table
                    .insert(key.as_str(), expires_bytes.as_slice())
                    .map_err(rb)?;
                let mut reverse = txn.open_table(VIRTUAL_TO_MSG).map_err(rb)?;
                reverse
                    .insert(key.as_str(), msg_id.as_bytes())
                    .map_err(rb)?;
                Ok((virtual_id, false))
            }
            None => {
                let current = read_counter_tx(txn, CURRENT_MSG_KEY)?;
                let planned = self.next_free_msg_id(current)?;
                let virtual_id = allocate_msg_tx(txn, planned, msg_id, expires)?;
                Ok((virtual_id, true))
            }
        }
    }

    /// 从虚拟消息 ID 反查真实消息 ID（Go `RetrieveMsgID`）。
    ///
    /// # Errors
    ///
    /// 不存在返回 [`StorageError::NotFound`]；TTL 过期返回 [`StorageError::Expired`]
    /// （Go 把两者都映射为 `ErrKeyNotFound`，用 [`StorageError::is_not_found`] 可得到
    /// 同样的单值语义）。
    pub fn retrieve_row_by_cache(&self, vuin: i64) -> Result<String, StorageError> {
        if vuin <= 0 {
            return Err(StorageError::NotFound(format!("虚拟消息 ID={vuin}")));
        }
        let key = vuin.to_string();
        let txn = self.msg_db.begin_read().map_err(rb)?;
        {
            let expires_table = txn.open_table(EXPIRES_AT).map_err(rb)?;
            if let Some(guard) = expires_table.get(key.as_str()).map_err(rb)? {
                let expires = decode_vuin(guard.value())?;
                // Go：exp > 0 && exp < now 才算过期；这里 exp <= now 即过期（零 TTL 立即失效）
                if expires > 0 && expires <= unix_now() {
                    return Err(StorageError::Expired(format!("虚拟消息 ID={vuin}")));
                }
            }
        }
        let reverse = txn.open_table(VIRTUAL_TO_MSG).map_err(rb)?;
        match reverse.get(key.as_str()).map_err(rb)? {
            Some(guard) => Ok(decode_text(guard.value())?),
            None => Err(StorageError::NotFound(format!("虚拟消息 ID={vuin}"))),
        }
    }

    /// 记录"群 × 用户"的最新消息 ID（Go `StoreLatestMsgID`）。
    ///
    /// Go 对空参数静默返回，这里保持一致。TTL 取 [`IdMapOptions::msg_ttl`]；
    /// 与 Go 相同**没有** 1 小时回退——TTL ≤ 0 时过期时间就是 `now`，
    /// 随后按 `expires <= now` 立即过期。
    ///
    /// # Errors
    ///
    /// redb 事务失败时返回 [`StorageError`]。
    pub fn store_latest_msg_id(
        &self,
        group: &str,
        user: &str,
        msg_id: &str,
    ) -> Result<(), StorageError> {
        self.with_write_batch(|batch| batch.store_latest_msg_id(group, user, msg_id))
    }

    /// 取"群 × 用户"的最新未过期消息 ID（Go `GetLatestMsgID`）。
    ///
    /// # Errors
    ///
    /// 从未记录（没有过期时间）返回 [`StorageError::NotFound`]；记录存在但已过期返回
    /// [`StorageError::Expired`]（Go 把两者都映射为 `ErrKeyNotFound`，用
    /// [`StorageError::is_not_found`] 可得到同样的单值语义）。
    pub fn get_latest_msg_id(&self, group: &str, user: &str) -> Result<String, StorageError> {
        let key = latest_key(group, user);
        let txn = self.msg_db.begin_read().map_err(rb)?;
        {
            let expires_table = txn.open_table(LATEST_EXPIRES_AT).map_err(rb)?;
            let expires = match expires_table.get(key.as_str()).map_err(rb)? {
                Some(guard) => decode_vuin(guard.value())?,
                None => 0,
            };
            // Go：exp == 0 视为"没有记录"；exp < now 才算过期
            if expires <= 0 {
                return Err(StorageError::NotFound(format!("最新消息 key={key}")));
            }
            if expires <= unix_now() {
                return Err(StorageError::Expired(format!("最新消息 key={key}")));
            }
        }
        let table = txn.open_table(LATEST_BY_GROUP_USER).map_err(rb)?;
        match table.get(key.as_str()).map_err(rb)? {
            Some(guard) => Ok(decode_text(guard.value())?),
            None => Err(StorageError::NotFound(format!("最新消息 key={key}"))),
        }
    }

    /// 写 vUIN 对应的用户名到内存缓存（Go `StoreUserName`）。
    pub fn store_user_name(&self, vuin: i64, name: &str) {
        let mut cache = self
            .user_names
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if cache.len() >= USER_NAME_SWEEP_THRESHOLD {
            let ttl = self.options.user_name_ttl;
            let now = Instant::now();
            cache.retain(|_, entry| now.duration_since(entry.stored_at) <= ttl);
        }
        cache.insert(
            vuin,
            UserNameEntry {
                name: name.to_owned(),
                stored_at: Instant::now(),
            },
        );
    }

    /// 取 vUIN 对应的用户名，过期或不存在返回 `None`（Go `GetUserName`）。
    #[must_use]
    pub fn get_user_name(&self, vuin: i64) -> Option<String> {
        let mut cache = self
            .user_names
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let ttl = self.options.user_name_ttl;
        if cache
            .get(&vuin)
            .is_none_or(|entry| entry.stored_at.elapsed() > ttl)
        {
            cache.remove(&vuin);
            return None;
        }
        cache.get(&vuin).map(|entry| entry.name.clone())
    }

    /// 写配置（Go `WriteConfig`）：身份库 `config` 表，key `"<section>:<subtype>"`，
    /// value 是原始字符串字节（与 bbolt 的 `b.Put(key, []byte(value))` 逐字节一致）。
    ///
    /// # Errors
    ///
    /// redb 事务失败时返回 [`StorageError`]。
    pub fn write_config(
        &self,
        section: &str,
        subtype: &str,
        value: &str,
    ) -> Result<(), StorageError> {
        self.with_write_batch(|batch| batch.write_config(section, subtype, value))
    }

    /// 读配置（Go `ReadConfig`）：键不存在返回 [`StorageError::NotFound`]。
    ///
    /// # Errors
    ///
    /// 键不存在、value 不是合法 UTF-8、redb 事务失败时返回 [`StorageError`]。
    pub fn read_config(&self, section: &str, subtype: &str) -> Result<String, StorageError> {
        let key = config_key(section, subtype);
        let txn = self.identity_db.begin_read().map_err(rb)?;
        let table = txn.open_table(CONFIG).map_err(rb)?;
        match table.get(key.as_str()).map_err(rb)? {
            Some(guard) => decode_text(guard.value()),
            None => Err(StorageError::NotFound(format!("config {key}"))),
        }
    }

    /// 删配置（Go `DeleteConfig`）：返回删除前该键是否存在。
    ///
    /// Go 对不存在的键也算成功（bbolt `Delete` 幂等），gRPC 侧因此恒回 `success`。
    ///
    /// # Errors
    ///
    /// redb 事务失败时返回 [`StorageError`]。
    pub fn delete_config(&self, section: &str, subtype: &str) -> Result<bool, StorageError> {
        let key = config_key(section, subtype);
        let txn = self.identity_db.begin_write().map_err(rb)?;
        let removed = {
            let mut table = txn.open_table(CONFIG).map_err(rb)?;
            table.remove(key.as_str()).map_err(rb)?.is_some()
        };
        txn.commit().map_err(rb)?;
        Ok(removed)
    }

    /// 枚举 `"<section>:<subtype>"` 键且值等于 `value` 的全部 section（Go
    /// `FindKeysBySubAndType`，`get_group_list` 靠它枚举已注册群）。
    ///
    /// 只读事务全扫 CONFIG 表；命中的键剥掉 `":<subtype>"` **后缀**得到 section
    /// （Go 是 `strings.Split(key, ":")[0]` 取第一个冒号前段，OpenID 形态 section
    /// 不含冒号，两种切法结果一致，剥后缀语义更稳），返回排序去重结果。
    ///
    /// # Errors
    ///
    /// redb 事务失败或 value 读取失败时返回 [`StorageError`]。
    pub fn find_config_sections(
        &self,
        subtype: &str,
        value: &str,
    ) -> Result<Vec<String>, StorageError> {
        // 纯防御（现无空参调用方）：空 subtype 会让「以 :subtype 结尾」退化成全表命中，
        // 空 value 会命中所有空值行
        debug_assert!(
            !subtype.is_empty() && !value.is_empty(),
            "find_config_sections 不接受空 subtype / value"
        );
        let suffix = format!(":{subtype}");
        let txn = self.identity_db.begin_read().map_err(rb)?;
        let table = txn.open_table(CONFIG).map_err(rb)?;
        let mut sections = Vec::new();
        for entry in table.iter().map_err(rb)? {
            let (key, stored) = entry.map_err(rb)?;
            // edition 2024 let 链：后缀命中与值命中一次判完
            if let Some(section) = key.value().strip_suffix(suffix.as_str())
                && stored.value() == value.as_bytes()
            {
                sections.push(section.to_owned());
            }
        }
        sections.sort();
        sections.dedup();
        Ok(sections)
    }

    /// 从已提交状态里找下一个未被占用的 vUIN（Go `allocateVuinTx` 的前缀扫描）。
    fn next_free_vuin(&self, current: i64) -> Result<i64, StorageError> {
        let mut candidate = if current <= 0 {
            1
        } else {
            current.saturating_add(1)
        };
        let txn = self.identity_db.begin_read().map_err(rb)?;
        let table = txn.open_table(VUIN_TO_IDENTITY).map_err(rb)?;
        while prefix_exists(&table, candidate)? {
            candidate = candidate
                .checked_add(1)
                .ok_or_else(|| StorageError::Corrupt("vUIN 空间耗尽".to_owned()))?;
        }
        Ok(candidate)
    }

    /// Go `allocateVuinTx` 的 `hash_id` 分支：MD5 派生 9 位 vUIN，撞号就加长到 18 位。
    ///
    /// 与 [`IdMapStore::next_free_vuin`] 一致，读的是已提交状态（写事务内看不到本事务未提交
    /// 的写入），因此并发写入方仍靠 `prefix_exists` 的已提交视图排重。
    fn hash_vuin(&self, raw: &str) -> Result<i64, StorageError> {
        let txn = self.identity_db.begin_read().map_err(rb)?;
        let table = txn.open_table(VUIN_TO_IDENTITY).map_err(rb)?;
        for digits in HASH_VUIN_MIN_DIGITS..=HASH_VUIN_MAX_DIGITS {
            let candidate = generate_row_id(raw, digits)?;
            if !prefix_exists(&table, candidate)? {
                return Ok(candidate);
            }
        }
        Err(StorageError::Corrupt(format!(
            "身份 {raw} 在 {HASH_VUIN_MIN_DIGITS}~{HASH_VUIN_MAX_DIGITS} 位内无法派生唯一 vUIN"
        )))
    }

    /// 从已提交状态里找下一个未被占用的虚拟消息 ID（Go `allocateMsgIDTx`）。
    fn next_free_msg_id(&self, current: i64) -> Result<i64, StorageError> {
        let mut candidate = if current <= 0 {
            1
        } else {
            current.saturating_add(1)
        };
        let txn = self.msg_db.begin_read().map_err(rb)?;
        let table = txn.open_table(VIRTUAL_TO_MSG).map_err(rb)?;
        while prefix_exists(&table, candidate)? {
            candidate = candidate
                .checked_add(1)
                .ok_or_else(|| StorageError::Corrupt("虚拟消息 ID 空间耗尽".to_owned()))?;
        }
        Ok(candidate)
    }
}

/// 在写事务里分配 vUIN：推进计数器并写正反两表 + `last_seen`。
fn allocate_vuin_tx(
    txn: &WriteTransaction,
    planned: i64,
    identity: &encoding::NormalizedIdentity,
    hash_derived: bool,
) -> Result<i64, StorageError> {
    let mut meta = txn.open_table(META).map_err(rb)?;
    let mut forward = txn.open_table(IDENTITY_TO_VUIN).map_err(rb)?;
    let mut reverse = txn.open_table(VUIN_TO_IDENTITY).map_err(rb)?;
    let mut last_seen = txn.open_table(LAST_SEEN).map_err(rb)?;

    let current = match meta.get(CURRENT_VUIN_KEY).map_err(rb)? {
        Some(guard) => decode_vuin(guard.value())?,
        None => 0,
    };
    // 顺序模式：planned 已跳过反表中占用中的 vUIN；并发写入方可能已推进 counter，
    // 取两者较大者，保证同一事务内分配到的 vUIN 不重号。
    // 哈希模式（Go `allocateVuinTx`）：vUIN 由 MD5 决定，不随 counter 改写。
    let vuin = if !hash_derived && current >= planned {
        current
            .checked_add(1)
            .ok_or_else(|| StorageError::Corrupt("vUIN 空间耗尽".to_owned()))?
    } else {
        planned
    };

    // Go 只在 vUIN 更大时推进计数器（哈希派生值通常远高于顺序计数）。
    let counter = encode_i64(vuin.max(current));
    meta.insert(CURRENT_VUIN_KEY, counter.as_slice())
        .map_err(rb)?;
    // 身份映射的值永远是本次分配的 vUIN，不能跟着计数器走。
    let mapped = encode_i64(vuin);
    forward
        .insert(identity.key.as_str(), mapped.as_slice())
        .map_err(rb)?;
    let reverse_key = vuin_identity_key(vuin, &identity.key);
    reverse
        .insert(reverse_key.as_str(), identity.raw.as_bytes())
        .map_err(rb)?;
    let seen_key = last_seen_key(vuin);
    last_seen
        .insert(seen_key.as_str(), identity.raw.as_bytes())
        .map_err(rb)?;
    Ok(vuin)
}

/// 在写事务里分配虚拟消息 ID：推进计数器并写正反两表 + 过期时间。
fn allocate_msg_tx(
    txn: &WriteTransaction,
    planned: i64,
    msg_id: &str,
    expires: i64,
) -> Result<i64, StorageError> {
    let mut meta = txn.open_table(META).map_err(rb)?;
    let mut forward = txn.open_table(MSG_TO_VIRTUAL).map_err(rb)?;
    let mut reverse = txn.open_table(VIRTUAL_TO_MSG).map_err(rb)?;
    let mut expires_table = txn.open_table(EXPIRES_AT).map_err(rb)?;

    let current = match meta.get(CURRENT_MSG_KEY).map_err(rb)? {
        Some(guard) => decode_vuin(guard.value())?,
        None => 0,
    };
    let virtual_id = if current >= planned {
        current.checked_add(1)
    } else {
        Some(planned)
    }
    .ok_or_else(|| StorageError::Corrupt("虚拟消息 ID 空间耗尽".to_owned()))?;

    let counter = encode_i64(virtual_id);
    meta.insert(CURRENT_MSG_KEY, counter.as_slice())
        .map_err(rb)?;
    forward.insert(msg_id, counter.as_slice()).map_err(rb)?;
    let key = counter_text(virtual_id);
    reverse
        .insert(key.as_str(), msg_id.as_bytes())
        .map_err(rb)?;
    expires_table
        .insert(key.as_str(), encode_i64(expires).as_slice())
        .map_err(rb)?;
    Ok(virtual_id)
}

/// 写 `last_seen`（Go `touchLastSeenTx`）。
fn touch_last_seen_tx(txn: &WriteTransaction, vuin: i64, raw: &str) -> Result<(), StorageError> {
    let mut table = txn.open_table(LAST_SEEN).map_err(rb)?;
    table
        .insert(last_seen_key(vuin).as_str(), raw.as_bytes())
        .map_err(rb)?;
    Ok(())
}

/// 在写事务里读 8 字节大端计数器（Go `readInt64(meta.Get(key))`）。
fn read_counter_tx(txn: &WriteTransaction, key: &str) -> Result<i64, StorageError> {
    let table = txn.open_table(META).map_err(rb)?;
    match table.get(key).map_err(rb)? {
        Some(guard) => decode_vuin(guard.value()),
        None => Ok(0),
    }
}

/// 在写事务里记录"群 × 用户"最新消息 ID（Go `StoreLatestMsgID` 的落盘部分）。
fn store_latest_tx(
    txn: &WriteTransaction,
    group: &str,
    user: &str,
    msg_id: &str,
    ttl: Duration,
) -> Result<(), StorageError> {
    let key = latest_key(group, user);
    let expires = unix_now().saturating_add(ttl_seconds(ttl));
    let mut table = txn.open_table(LATEST_BY_GROUP_USER).map_err(rb)?;
    table.insert(key.as_str(), msg_id.as_bytes()).map_err(rb)?;
    let expires_bytes = encode_i64(expires);
    let mut expires_table = txn.open_table(LATEST_EXPIRES_AT).map_err(rb)?;
    expires_table
        .insert(key.as_str(), expires_bytes.as_slice())
        .map_err(rb)?;
    Ok(())
}

/// 反表里是否存在 `"<vUIN>:"` 前缀的 key（Go `vuinExistsTx`）。
///
/// 泛型落在表类型上：读事务给 `ReadOnlyTable`，写事务（重绑定的 exists 检查）给 `Table`。
pub(crate) fn prefix_exists<T>(table: &T, vuin: i64) -> Result<bool, StorageError>
where
    T: ReadableTable<&'static str, &'static [u8]>,
{
    let prefix = vuin_prefix(vuin);
    let mut range = table.range(prefix.as_str()..).map_err(rb)?;
    match range.next() {
        Some(entry) => {
            let (key, _) = entry.map_err(rb)?;
            Ok(key.value().starts_with(prefix.as_str()))
        }
        None => Ok(false),
    }
}

/// 表里是否一条记录都没有（[`IdMapStore::is_empty`] 用）。
fn table_is_empty(
    txn: &ReadTransaction,
    definition: TableDefinition<'static, &'static str, &'static [u8]>,
) -> Result<bool, StorageError> {
    let table = txn.open_table(definition).map_err(rb)?;
    let mut range = table.iter().map_err(rb)?;
    Ok(range.next().is_none())
}

fn is_openid(value: &str) -> bool {
    value.len() == encoding::OPENID_LEN
}

/// 隔离重绑定写的 alias key（`write_rebind_alias` 的 `raw:<十进制 vUIN>`）。
fn is_isolation_alias(identity_key: &str) -> bool {
    identity_key
        .strip_prefix("raw:")
        .is_some_and(|row| row.parse::<i64>().is_ok())
}

/// Go `lastSeenKey`：`"default:<十进制 vUIN>"`。
pub(crate) fn last_seen_key(vuin: i64) -> String {
    format!("{LAST_SEEN_SCOPE}:{vuin}")
}

/// Go `joinSectionAndKey`：`"<section>:<subtype>"`。
pub(crate) fn config_key(section: &str, subtype: &str) -> String {
    format!("{section}:{subtype}")
}

/// Go `StoreLatestMsgID` 的 key 构造：`"<群>:<用户>"`。
pub(crate) fn latest_key(group: &str, user: &str) -> String {
    format!("{group}:{user}")
}

pub(crate) fn counter_text(value: i64) -> String {
    value.to_string()
}

pub(crate) fn decode_vuin(bytes: &[u8]) -> Result<i64, StorageError> {
    decode_i64(bytes).ok_or_else(|| StorageError::Corrupt("期望 8 字节大端整数".to_owned()))
}

fn decode_text(bytes: &[u8]) -> Result<String, StorageError> {
    String::from_utf8(bytes.to_vec())
        .map_err(|_| StorageError::Corrupt("身份串/消息 ID 不是合法 UTF-8".to_owned()))
}

/// redb 的具体错误统一归入 [`StorageError::Redb`]。
pub(crate) fn rb<E: Into<redb::Error>>(err: E) -> StorageError {
    StorageError::Redb(err.into())
}

/// 当前 Unix 秒（`botstats` 的当日判据也用它）。
pub(crate) fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

fn ttl_seconds(ttl: Duration) -> i64 {
    i64::try_from(ttl.as_secs()).unwrap_or(i64::MAX)
}

/// Go `StoreMsgID` 的过期时间：`now + ttl`，不晚于 `now` 时回退 [`DEFAULT_MSG_TTL`]。
///
/// 覆盖 TTL ≤ 0 与被截断成 0 的亚秒 TTL；`store_latest_msg_id` 不走这里（Go 无回退）。
fn expires_after(now: i64, ttl: Duration) -> i64 {
    let expires = now.saturating_add(ttl_seconds(ttl));
    if expires > now {
        expires
    } else {
        now.saturating_add(ttl_seconds(DEFAULT_MSG_TTL))
    }
}

#[cfg(test)]
mod tests;
