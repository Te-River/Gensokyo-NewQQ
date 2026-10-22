//! gensokyo-storage：idmap、echo、cookie、bbolt 导入、IDMapService gRPC。
//!
//! M1d 落地 [`idmap`] 的**本地**存储（redb），对齐 Go `idmap/map_service.go` +
//! `idmap/service.go` 的本地路径；gRPC/HTTP 远程回退与 bbolt 导入留给 M1e，
//! echo 层（消息 ID 懒加载 / SSM / Seq）留给后续里程碑。
//!
//! # 库文件
//!
//! 两个 redb 文件，等价于 Go 的两个 bbolt 库：
//!
//! | 用途 | Go 文件名 | 本 crate 默认文件名 |
//! |------|-----------|---------------------|
//! | 身份 ↔ vUIN | `openid-map.db` | `openid-map.redb` |
//! | 消息 ID ↔ 虚拟 ID | `msgid-map.db` | `msgid-map.redb` |
//!
//! 路径由 [`IdMapOptions`] 指定；文件格式本身与 bbolt 不同（redb 有自己的页格式），
//! 但**表名、key 与 value 的字节编码与 Go 保持一致**，M1e 的导入器可以逐行搬运。
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
//! - **哈希 vUIN**：Go `HashID=true` 时用 `GenerateRowID` 派生 vUIN；M1d 只实现单调
//!   递增（Go 的默认路径），哈希模式随配置层接入后再补。
//! - **`uin_prefix` 隔离**：Go 从配置读 `idmap_isolation`/`uin`；本 crate 由
//!   [`IdMapOptions::uin`] 显式传入（`None` 等价 Go 关闭隔离）。
//! - **名称缓存**：Go 用 `sync.Map` + 后台每 10 分钟清理；本 crate 用 `Mutex<HashMap>`
//!   + 读时惰性过期，写满 [`USER_NAME_SWEEP_THRESHOLD`] 条时顺带清理过期项。
//! - **身份库缺 `config` / `UserInfo` 桶**：Go `initNewDBs` 在身份库额外建 `config` /
//!   `UserInfo` 两个 bucket（配置与用户信息复用）；本 crate M1d 未建，**M1e 的 bbolt
//!   导入器落地前必须补上**，否则导入器在身份库里无表可写。
//!
//! # 并发
//!
//! [`IdMapStore`] 可在多线程间共享（`Send + Sync`）。redb 同一库内只允许一个写事务，
//! 计数器在写事务内自增，因此并发分配不会重号；实测 8 线程 × 100 次分配得到连续的
//! `1..=800`。

mod error;
pub mod idmap;

pub use error::StorageError;
pub use idmap::{IdMapOptions, IdMapStore, USER_NAME_SWEEP_THRESHOLD};

/// 消息 ID / Seq / SSM 映射（`echo/`），后续里程碑落地。
pub mod echo {}

/// 联合缓存（`unioncache/`），后续里程碑落地。
pub mod union_cache {}
