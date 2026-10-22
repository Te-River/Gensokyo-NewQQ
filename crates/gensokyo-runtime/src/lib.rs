//! gensokyo-runtime：入站转换、四种连接、dispatch、队列与限速、优雅关闭。
//!
//! M0 只建立 crate 边界；任务树与四连接（反向 WS / 正向 WS / HTTP API / webhook）在 M1 落地。

/// 装配与任务树（`main.go` 的 `runtime` 对应物）。
pub mod boot {}
/// QQ 事件 → OneBot 事件（`Processor/`）。
pub mod inbound {}
/// 有界队列 + 令牌桶（`messagequeue/`）。
pub mod queue {}
/// 反向 WS 客户端与重连（`wsclient/`）。
pub mod ws_client {}
