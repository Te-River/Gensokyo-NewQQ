//! Action 响应信封：Go 侧按 action 分套形状，这里给出「哪条 action 用哪套」的映射表。
//!
//! - **A 发送类**（`send_group_msg` 等）与 **C 查询类**（`get_group_info` 等）用
//!   [`EnvelopeKind::Full`]：`data` 恒在，`message` / `echo` 恒在。
//! - **B 管理类**（23 条）用 [`EnvelopeKind::Compact`]：Go 的 `sendActionResult` /
//!   `sendActionResultWithData` 没有 `group_id` / `user_id`，`echo` 带 `omitempty`，失败
//!   一律 `retcode 100`。
//!
//! 表逐条来自 Go 源码，**不要按 action 名字猜**：无 `data` 的在
//! `handlers/set_group_add_request.go:54-82`，`data` 恒 `null` 的在
//! `handlers/join_approval_strategy.go:80,105`，`data` 恒 `{}` 的在
//! `handlers/delete_panel.go:29` 与 `handlers/set_panel_target.go:56`，其余成功路径回上游
//! 响应体（`sendActionResultWithData(client, message, resp)`）。

/// 响应信封模式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EnvelopeKind {
    /// A 发送类 / C 查询类：`data` 恒序列化（`None` → `null`），`message` / `echo` 恒在，
    /// `group_id` / `user_id` 按需。
    #[default]
    Full,
    /// B 管理类：Go `sendActionResult` 系列形状，`data` 形态由 [`CompactData`] 决定。
    Compact(CompactData),
}

/// Compact 信封里 `data` 的形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactData {
    /// 没有 `data` 键（Go `sendActionResult`；所有失败路径也都是这个形状）。
    Omit,
    /// `data` 恒在，取 handler 的返回值。
    AsIs,
    /// `data` 恒为 `{}`（Go `sendActionResultWithData(..., map[string]interface{}{})`）。
    EmptyObject,
    /// `data` 恒为 `null`（Go `sendActionResultWithData(..., nil)`）。
    Null,
}

/// B 管理类的 23 条 action 及其 `data` 形态。
const COMPACT_ACTIONS: &[(&str, CompactData)] = &[
    // 无 data：Go 全部走 `sendActionResult`。
    ("set_group_add_request", CompactData::Omit),
    ("set_group_ban", CompactData::Omit),
    ("set_group_whole_ban", CompactData::Omit),
    ("set_group_card", CompactData::Omit),
    // `sendActionResultWithData(client, message, nil)`。
    ("join_approval_strategy_execute", CompactData::Null),
    ("join_approval_strategy_delete", CompactData::Null),
    // `sendActionResultWithData(client, message, map[string]interface{}{})`。
    ("delete_panel", CompactData::EmptyObject),
    ("set_panel_target", CompactData::EmptyObject),
    // 成功路径回上游响应体。
    ("set_group_kick", CompactData::AsIs),
    ("set_group_member_blacklist", CompactData::AsIs),
    ("create_panel", CompactData::AsIs),
    ("set_panel", CompactData::AsIs),
    ("get_panel", CompactData::AsIs),
    ("get_panel_list", CompactData::AsIs),
    ("set_custom_menu", CompactData::AsIs),
    ("get_custom_menu", CompactData::AsIs),
    ("get_group_bot_state", CompactData::AsIs),
    ("get_group_join_request_list", CompactData::AsIs),
    ("get_group_member_blacklist", CompactData::AsIs),
    ("join_approval_strategy_create", CompactData::AsIs),
    ("join_approval_strategy_list", CompactData::AsIs),
    ("join_approval_strategy_update", CompactData::AsIs),
    ("join_approval_strategy_whitelist", CompactData::AsIs),
];

/// 该 action 的响应信封模式；不在 B 类表里的（A 发送 / C 查询 / Rust 扩展）一律 [`EnvelopeKind::Full`]。
#[must_use]
pub fn envelope_for(action: &str) -> EnvelopeKind {
    match COMPACT_ACTIONS.iter().find(|(name, _)| *name == action) {
        Some((_, data)) => EnvelopeKind::Compact(*data),
        None => EnvelopeKind::Full,
    }
}

#[cfg(test)]
mod tests {
    use super::{COMPACT_ACTIONS, CompactData, EnvelopeKind, envelope_for};

    /// 23 条 + 名字唯一：重复的条目会让后面的形态被前面的遮住。
    #[test]
    fn compact_table_is_the_go_management_set() {
        assert_eq!(COMPACT_ACTIONS.len(), 23);
        let mut names: Vec<&str> = COMPACT_ACTIONS.iter().map(|(name, _)| *name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 23, "B 类表里有重复 action");
    }

    /// 四种 `data` 形态各归其位（Go 源码锚点见模块文档）。
    #[test]
    fn data_shapes_follow_the_go_constructors() {
        for action in [
            "set_group_add_request",
            "set_group_ban",
            "set_group_whole_ban",
            "set_group_card",
        ] {
            assert_eq!(
                envelope_for(action),
                EnvelopeKind::Compact(CompactData::Omit),
                "{action} 在 Go 里没有 data"
            );
        }
        for action in [
            "join_approval_strategy_execute",
            "join_approval_strategy_delete",
        ] {
            assert_eq!(
                envelope_for(action),
                EnvelopeKind::Compact(CompactData::Null),
                "{action} 的 data 是 nil"
            );
        }
        for action in ["delete_panel", "set_panel_target"] {
            assert_eq!(
                envelope_for(action),
                EnvelopeKind::Compact(CompactData::EmptyObject),
                "{action} 的 data 是空 map"
            );
        }
        assert_eq!(
            envelope_for("set_group_kick"),
            EnvelopeKind::Compact(CompactData::AsIs)
        );
    }

    /// A 发送类 / C 查询类 / Rust 扩展都不进 Compact。
    #[test]
    fn everything_else_stays_full() {
        for action in [
            "send_group_msg",
            "send_private_msg",
            "get_group_info",
            "get_group_member_list",
            "mark_msg_as_read",
            "get_status",
            "unknown_action",
        ] {
            assert_eq!(envelope_for(action), EnvelopeKind::Full, "{action}");
        }
    }
}
