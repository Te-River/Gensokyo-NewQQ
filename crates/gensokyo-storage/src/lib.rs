//! gensokyo-storage：idmap、echo、cookie、bbolt 导入、IDMapService gRPC。
//!
//! M0 只建立 crate 边界；redb 表结构与 bbolt 导入在 M1 落地。

/// 虚拟 ID ↔ OpenID 双向映射（`idmap/`）。
pub mod idmap {}
/// 消息 ID / Seq / SSM 映射（`echo/`）。
pub mod echo {}
/// 联合缓存（`unioncache/`）。
pub mod union_cache {}
