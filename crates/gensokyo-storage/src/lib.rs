//! gensokyo-storage：idmap、echo、cookie、JSONL 导入、IDMapService gRPC。
//!
//! M1d 落地 [`idmap`] 的**本地**存储（redb），M1e 落地 [`echo`] 的进程内映射与
//! [`importer`] 的 JSONL 导入，M1f 落地 [`idmap::grpc`] 的 `IDMapService`（tonic，17 个 RPC
//! 中 15 个已接线，2 个明确 `unimplemented`，见该模块文档）。HTTP 远程回退留给后续里程碑。
//!
//! # 库文件
//!
//! 两个 redb 文件，等价于 Go 的两个 bbolt 库：
//!
//! | 用途 | Go 文件名 | 本 crate 默认文件名 |
//! |------|-----------|---------------------|
//! | 身份 ↔ vUIN | `openid-map.db` | `openid-map.redb` |
//! | 消息 ID ↔ 虚拟 ID | `msgid-map.db` | `msgid-map.redb` |
//! | 当日收发消息统计 | `botstats.db` | `bot-stats.redb` |
//! | 短链 | `gensokyo.db`（bucket `shortURLs`） | `short-urls.redb` |
//!
//! 路径由 [`IdMapOptions`] 指定；文件格式本身与 bbolt 不同（redb 有自己的页格式），
//! 但**表名、key 与 value 的字节编码与 Go 保持一致**，[`importer`] 因此可以逐行搬运。
//!
//! # 表结构（镜像 Go bbolt bucket）
//!
//! 身份库：
//!
//! | 表 | key | value |
//! |----|-----|-------|
//! | `identity_to_vuin` | `normalizeIdentity().Key`（`openid:QQ:<AppID>:<raw>` / `ruin:<平台>:<序号>:<id>` / `raw:<raw>`） | 8 字节大端 `uint64` vUIN |
//! | `vuin_to_identity` | `"<十进制 vUIN>:<Key>"`（Go `vuinIdentityKey`） | 原始身份串（Go `identity.Raw`） |
//! | `last_seen` | `"default:<十进制 vUIN>"`（Go `lastSeenKey`） | 原始身份串 |
//! | `config` | `"<section>:<subtype>"`（Go `joinSectionAndKey`） | 原始字符串字节（Go `WriteConfig`） |
//! | `meta` | `current_vuin` / `schema_version` | 8 字节大端 `uint64` / `"2"` |
//!
//! 消息库：
//!
//! | 表 | key | value |
//! |----|-----|-------|
//! | `msg_to_virtual` | 真实消息 ID | 8 字节大端 `uint64` 虚拟 ID |
//! | `virtual_to_msg` | `"<十进制虚拟 ID>"` | 真实消息 ID |
//! | `expires_at` | `"<十进制虚拟 ID>"` | 8 字节大端 `uint64`（Unix 秒） |
//! | `latest_by_group_user` | `"<群>:<用户>"` | 真实消息 ID |
//! | `latest_expires_at` | `"<群>:<用户>"` | 8 字节大端 `uint64`（Unix 秒） |
//! | `meta` | `current_msg` | 8 字节大端 `uint64` |
//!
//! 所有 value 都以 `&[u8]` 存取，整数字段与 Go `putInt64`/`readInt64` 完全一致
//! （8 字节大端）；字符串字段就是 UTF-8 原始字节。key 用 `&str`，redb 按字节序
//! 排序，与 bbolt 的字节序一致，因此 `vuin_to_identity` 的前缀扫描语义相同。
//!
//! # 与 Go 的差异（明确列出）
//!
//! - **过期语义**：Go `RetrieveMsgID` 把 TTL 过期并入 `ErrKeyNotFound`；本 crate 返回
//!   [`StorageError::Expired`]，用 [`StorageError::is_not_found`] 得到 Go 的单值语义。
//!   另外 Go 用 `exp < now`（严格小于），本 crate 用 `expires_at <= now`。
//!   `store_cache`/`store_cache_with_ttl` 与 Go `StoreMsgID` 一致，TTL ≤ 0（含截断成 0
//!   的亚秒 TTL）时回退 3600 秒；`store_latest_msg_id` 与 Go `StoreLatestMsgID` 一致
//!   **没有**回退，TTL ≤ 0 时过期时间就是 `now`，读时立即过期。
//! - **损坏数据**：Go 遇到长度不是 8 字节的 vUIN 会当作"没有"并重新分配；本 crate
//!   返回 [`StorageError::Corrupt`]，避免静默覆盖反查表。
//! - **空消息 ID**：Go `StoreMsgID("")` 会把空串写进正表；本 crate 拒绝并返回
//!   [`StorageError::Corrupt`]（`idmap/mod.rs` 的 `store_cache_with_ttl`）。
//! - **ASCII-only 数字**：Go `isDigits` 用 `unicode.IsDigit`（接受阿拉伯数字等 Unicode
//!   数字）；本 crate 只认 ASCII `0-9`（`idmap/encoding.rs` 的 `is_ascii_digits`），
//!   `rUIN` 的 base32 解码结果同理。
//! - **哈希 vUIN**：Go `HashID=true` 时用 `GenerateRowID` 派生 vUIN；本 crate 由
//!   [`IdMapOptions::hash_id`] 选择（`false` 走 Go 的顺序递增默认路径）。
//!   注意 Go 的配置默认是 `hash_id: true`，接入配置层时要显式打开。
//! - **`uin_prefix` 隔离**：Go 从配置读 `idmap_isolation`/`uin`；本 crate 由
//!   [`IdMapOptions::uin`] 显式传入（`None` 等价 Go 关闭隔离）。
//! - **名称缓存**：Go 用 `sync.Map` + 后台每 10 分钟清理；本 crate 用 `Mutex<HashMap>`
//!   + 读时惰性过期，写满 [`USER_NAME_SWEEP_THRESHOLD`] 条时顺带清理过期项。
//! - **`UserInfo` 桶**：Go `initNewDBs` 在身份库额外建 `config` / `UserInfo` 两个 bucket。
//!   `config` 由 M1f 补上（[`IdMapStore::write_config`] / [`IdMapStore::read_config`] /
//!   [`IdMapStore::delete_config`]，供 gRPC `WriteConfigV2` 等使用）；`UserInfo`（表名
//!   `user_info`）由 M1e 的导入器补上；读侧 `ListAllUsers` 已暴露为
//!   [`IdMapStore::list_all_users`]（P64a，`get_friend_list` 是首个调用方），写侧
//!   `StoreUserInfo` 仍未暴露（P64b，全新部署该表只有导入器数据）。
//! - **批量写**：Go 每个 `StoreIDv2` / `StoreMsgID` / `WriteConfigv2` 各自一次写事务（一次
//!   fsync）；[`IdMapStore::with_write_batch`] 把一次事件内的多次写入收进同一个事务。成功
//!   路径逐键结果与 Go 一致，只有提交次数变少；失败路径是整批回滚（Go 保留失败点之前的写入）。
//! - **读路径热查询缓存**：`lookup_virtual_value` / `retrieve_row_by_id` 带进程内有界 LRU
//!   （`DEFAULT_LOOKUP_CACHE_CAPACITY` 条、TTL `DEFAULT_LOOKUP_CACHE_TTL`，可配 0 关闭）。
//!   写入提交后精确失效，关闭时行为与不缓存完全相同——Go 没有这层缓存，但它不影响任何可观测
//!   结果（详见 `idmap/lookup_cache.rs` 的模块文档）。
//!
//! # 并发
//!
//! [`IdMapStore`] 可在多线程间共享（`Send + Sync`）。redb 同一库内只允许一个写事务，
//! 计数器在写事务内自增，因此并发分配不会重号；实测 8 线程 × 100 次分配得到连续的
//! `1..=800`。

pub mod botstats;
mod error;
pub mod idmap;
pub mod importer;
/// 短链库与纯函数（Go `url/shorturl.go`）。
pub mod short_url;
/// `id ↔ union_openid` 双向映射（Go `unioncache/`）。
pub mod union_cache;

pub use botstats::{BotStats, BotStatsStore, DEFAULT_BOTSTATS_PATH};
pub use echo::{Clock, EchoStore, MessageGroupPair, MessageScene};
pub use error::StorageError;
pub use idmap::grpc::{IdMapClient, IdMapService};
pub use idmap::{IdMapOptions, IdMapStore, USER_NAME_SWEEP_THRESHOLD, UserInfoRow};
pub use importer::{ImportError, ImportOptions, ImportReport, OnVuinConflict};
pub use short_url::{
    DEFAULT_SHORT_URL_PATH, ShortUrlStore, UrlRejection, is_malicious, redirect_target,
    short_url_link, validate_url,
};
pub use union_cache::UnionCache;

/// 小写十六进制编码（`short_url` 的短码与 `runtime` 的短链 token 校验共用）。
pub fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // `fmt::Write for String` 不会失败，忽略返回值即可
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// 消息 ID 懒加载 / msg_seq / SSM 补发队列 / REFIDX 等进程内映射（Go `echo/`）。
pub mod echo;
