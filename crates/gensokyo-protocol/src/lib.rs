//! gensokyo-protocol：OneBot V11 模型、CQ 码、OutboundPlan、handler 注册表。
//!
//! 该 crate 永不依赖 `qq` / `storage` / `media`，保证协议层可纯 JSON 单测。

pub mod action;
pub mod cqcode;
pub mod event;
pub mod outbound;
pub mod registry;
