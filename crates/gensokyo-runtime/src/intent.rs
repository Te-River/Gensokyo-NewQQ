//! `text_intent` → botgo intent 位（Go `main.go:316-353`）。
//!
//! Go 用「handler 名字」间接订阅：`config.text_intent` 里的每个名字先经
//! `getHandlerByName` 取到回调，再交给 `websocket.RegisterHandlers` 换出一组 `dto.Intent`
//! 位。这里把同一层映射搬成纯函数，供启动日志与配置校验使用；**Rust 侧不维持 botgo WS
//! 会话**（QQ 侧连接仍在 Go 主干），所以这些位目前只影响日志与配置诊断。
//!
//! | `text_intent` 名字 | intent 位 | 备注 |
//! |--------------------|-----------|------|
//! | `ReadyHandler` / `ErrorNotifyHandler` | 0 | 连接生命周期回调，不订阅事件 |
//! | `InteractionHandler` | `1<<26` | `IntentInteraction` |
//! | `GroupATMessageEventHandler` | `1<<25` | `IntentGroupMessages` |
//! | `C2CMessageEventHandler` | `1<<25` | 同上 |
//! | `GroupMessageEventHandler` | `1<<25` | 同上 |
//! | `GroupJoinRequestEventHandler` | `1<<25` | `EventGroupJoinRequest` 属于 `IntentGroupMessages` |
//! | `GroupMemberAddEventHandler` | `1<<24` | `IntentGroupMembers` |
//! | `GroupMemberRemoveEventHandler` | `1<<24` | 同上 |
//! | `GroupAddRobotEventHandler` / `GroupDelRobotEventHandler` | 0 | 见下方「Go 的坑」 |
//! | `GroupMsgRejectHandler` / `GroupMsgReceiveHandler` | 0 | 同上 |
//! | `FriendAddEventHandler` / `FriendDelEventHandler` | 0 | 同上 |
//! | `C2CMsgRejectHandler` / `C2CMsgReceiveHandler` | 0 | 同上 |
//!
//! # Go 的坑（照搬，不修正）
//!
//! `RegisterHandlers` 只对 `InteractionEventHandler`、`GroupMemberAdd/Remove`、
//! `GroupJoinRequest` 以及三个消息 handler 执行 `i |= ...`；另外 8 个关系链 / 群开关
//! handler 只登记回调、**不贡献任何位**，它们的事件实际走 `IntentGroupMessages`。因此
//! `text_intent` 里只写 `FriendAddEventHandler` 会得到 `intent = 0`、一个事件也收不到。
//! 本模块保持同一语义，不做「自动补齐」，避免与 Go 主干的订阅行为分叉。
//!
//! # 自动订阅与两个开关
//!
//! - `discover_unknown_events=true`：额外订阅 Go `main.go:335` 的 16 个未使用位
//!   （6/7/8/11/13/14/15/16/17/18/19/20/21/22/23/31）。
//! - `global_group_msg_rre_to_message=true`（Go 的缩写拼写，Rust 字段名
//!   `global_group_msg_reject_recive_event_to_message`）：自动启用 `GroupMsgRejectHandler` /
//!   `GroupMsgReceiveHandler`（Go `main.go:342-351`）；两者位为 0，只影响 `enabled`。
//! - `suppress_disallowed_intents=true`：按 Go `main.go:913-955` 摘位——启用了
//!   `GroupMember*` 就摘 `IntentGroupMembers`；只启用 `GroupMessageEventHandler` 而没有
//!   任何其他共用 `IntentGroupMessages` 的 handler 时才摘该位。
//!
//! # 与 Go 的差异
//!
//! `text_intent` 为空时 Go 在启动阶段 panic（`main.go:182-184`）；这里只 `warn` 并返回
//! 空位集——运行期不该因为这个配置让整进程退出。

use gensokyo_config::Settings;

/// botgo `IntentGroupMembers`：群成员变动事件（`GROUP_MEMBER_ADD/REMOVE`）。
pub const INTENT_GROUP_MEMBERS: i64 = 1 << 24;
/// botgo `IntentGroupMessages`：群/私聊消息与 8 个关系链事件。
pub const INTENT_GROUP_MESSAGES: i64 = 1 << 25;
/// botgo `IntentInteraction`：频道互动事件。
pub const INTENT_INTERACTION: i64 = 1 << 26;

/// Go `main.go:335` 的「未知事件探测」位序号（使用时左移）。
const DISCOVER_UNKNOWN_BITS: [i64; 16] =
    [6, 7, 8, 11, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 31];

/// 与 `GroupMessageEventHandler` 共用 `IntentGroupMessages` 的 11 个 handler
/// （Go `main.go:931-943` 的 `hasAnyHandler` 名单）。
const GROUP_MESSAGES_SHARED: [&str; 11] = [
    "GroupATMessageEventHandler",
    "C2CMessageEventHandler",
    "GroupAddRobotEventHandler",
    "GroupDelRobotEventHandler",
    "GroupMsgRejectHandler",
    "GroupMsgReceiveHandler",
    "FriendAddEventHandler",
    "FriendDelEventHandler",
    "C2CMsgRejectHandler",
    "C2CMsgReceiveHandler",
    "GroupJoinRequestEventHandler",
];

/// 17 个 `text_intent` 名字 → 订阅位（`getHandlerByName` + `RegisterHandlers` 的结果）。
const HANDLERS: [(&str, i64); 17] = [
    ("ReadyHandler", 0),
    ("ErrorNotifyHandler", 0),
    ("InteractionHandler", INTENT_INTERACTION),
    ("GroupATMessageEventHandler", INTENT_GROUP_MESSAGES),
    ("C2CMessageEventHandler", INTENT_GROUP_MESSAGES),
    ("GroupAddRobotEventHandler", 0),
    ("GroupDelRobotEventHandler", 0),
    ("GroupMsgRejectHandler", 0),
    ("GroupMsgReceiveHandler", 0),
    ("FriendAddEventHandler", 0),
    ("FriendDelEventHandler", 0),
    ("C2CMsgRejectHandler", 0),
    ("C2CMsgReceiveHandler", 0),
    ("GroupMessageEventHandler", INTENT_GROUP_MESSAGES),
    ("GroupMemberAddEventHandler", INTENT_GROUP_MEMBERS),
    ("GroupMemberRemoveEventHandler", INTENT_GROUP_MEMBERS),
    ("GroupJoinRequestEventHandler", INTENT_GROUP_MESSAGES),
];

/// 一份 `text_intent` 的解析结果：位集 + 已启用 / 未知 / 被摘掉的标注。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IntentPlan {
    bits: i64,
    enabled: Vec<&'static str>,
    unknown: Vec<String>,
    suppressed: Vec<&'static str>,
}

impl IntentPlan {
    /// 只按名字解析（不读配置开关）；未知名字 `warn` 后忽略，与 Go 的 `Unknown handler` 一致。
    #[must_use]
    pub fn from_names(names: &[String]) -> Self {
        let mut plan = Self::default();
        for name in names {
            let Some((known, bits)) = handler_bits(name) else {
                tracing::warn!(
                    handler = name.as_str(),
                    "text_intent 中有未知 handler 名，已忽略"
                );
                plan.unknown.push(name.clone());
                continue;
            };
            plan.bits |= bits;
            plan.enable(known);
        }
        plan
    }

    /// 按配置快照解析：`text_intent` + `discover_unknown_events` + 自动订阅 + 摘位策略。
    #[must_use]
    pub fn from_settings(settings: &Settings) -> Self {
        let mut plan = Self::from_names(settings.get_text_intent());
        if settings.get_text_intent().is_empty() {
            // Go 在这里 panic；Rust 侧只告警，空位集由调用方自行判断。
            tracing::warn!("text_intent 为空（Go 会 panic），本次不订阅任何 intent 位");
        }

        if settings.get_discover_unknown_events() {
            for bit in DISCOVER_UNKNOWN_BITS {
                plan.bits |= 1_i64 << bit;
            }
            tracing::debug!(
                bits = plan.bits,
                "discover_unknown_events：额外订阅 16 个未使用位"
            );
        }

        if settings.get_global_group_msg_reject_recive_event_to_message() {
            // YAML 沿用 Go 的缩写：global_group_msg_rre_to_message；两个开关 handler 本身不贡献位。
            for name in ["GroupMsgRejectHandler", "GroupMsgReceiveHandler"] {
                plan.enable(name);
                tracing::info!(
                    handler = name,
                    "自动订阅 intent（global_group_msg_rre_to_message）"
                );
            }
        }

        if settings.get_suppress_disallowed_intents() {
            plan.suppress_disallowed();
        }
        plan
    }

    /// 订阅位集（Go `dto.Intent` 的 `int` 值）。
    #[must_use]
    pub fn bits(&self) -> i64 {
        self.bits
    }

    /// 是否订阅了某一位。
    #[must_use]
    pub fn contains(&self, bit: i64) -> bool {
        self.bits & bit != 0
    }

    /// 已启用的 handler 名（含自动订阅，保持 `text_intent` 顺序）。
    #[must_use]
    pub fn enabled_handlers(&self) -> &[&'static str] {
        &self.enabled
    }

    /// `text_intent` 里无法识别的名字。
    #[must_use]
    pub fn unknown_names(&self) -> &[String] {
        &self.unknown
    }

    /// 被 `suppress_disallowed_intents` 摘掉的位标注（`group_members` / `group_messages`）。
    #[must_use]
    pub fn suppressed(&self) -> &[&'static str] {
        &self.suppressed
    }

    /// 追加位（Go `intent |= ...`）。
    pub fn insert(&mut self, bits: i64) {
        self.bits |= bits;
    }

    /// 摘掉位（Go `intent &^= ...`）。
    pub fn remove(&mut self, bits: i64) {
        self.bits &= !bits;
    }

    fn enable(&mut self, name: &'static str) {
        if !self.enabled.contains(&name) {
            self.enabled.push(name);
        }
    }

    fn has(&self, name: &'static str) -> bool {
        self.enabled.contains(&name)
    }

    /// Go `main.go:913-955`：只摘「已启用对应 handler」的高风险位。
    fn suppress_disallowed(&mut self) {
        let group_members =
            self.has("GroupMemberAddEventHandler") || self.has("GroupMemberRemoveEventHandler");
        if group_members && self.contains(INTENT_GROUP_MEMBERS) {
            self.remove(INTENT_GROUP_MEMBERS);
            self.suppressed.push("group_members");
        }

        if self.has("GroupMessageEventHandler") && self.contains(INTENT_GROUP_MESSAGES) {
            if GROUP_MESSAGES_SHARED.iter().any(|name| self.has(name)) {
                tracing::info!(
                    "suppress_disallowed_intents: GroupMessageEventHandler 仅注册本地处理器，复用 IntentGroupMessages"
                );
            } else {
                self.remove(INTENT_GROUP_MESSAGES);
                self.suppressed.push("group_messages");
            }
        }

        if !self.suppressed.is_empty() {
            tracing::info!(suppressed = ?self.suppressed, bits = self.bits, "已屏蔽 intent 位");
        }
    }
}

/// 名字 → `(表内规范名, 位)`；`None` 表示 Go `getHandlerByName` 的 default 分支（未知名字）。
fn handler_bits(name: &str) -> Option<(&'static str, i64)> {
    HANDLERS
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(known, bits)| (*known, *bits))
}

#[cfg(test)]
mod tests {
    use super::{INTENT_GROUP_MEMBERS, INTENT_GROUP_MESSAGES, INTENT_INTERACTION, IntentPlan};

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn all_seventeen_names_are_known() {
        let all: Vec<String> = super::HANDLERS
            .iter()
            .map(|(name, _)| (*name).to_owned())
            .collect();
        let plan = IntentPlan::from_names(&all);
        assert!(plan.unknown_names().is_empty());
        assert_eq!(plan.enabled_handlers().len(), 17);
        assert_eq!(
            plan.bits(),
            INTENT_GROUP_MEMBERS | INTENT_GROUP_MESSAGES | INTENT_INTERACTION
        );
    }

    #[test]
    fn disabled_handlers_contribute_no_bits() {
        let plan = IntentPlan::from_names(&names(&[
            "FriendAddEventHandler",
            "GroupMsgRejectHandler",
            "ReadyHandler",
        ]));
        assert_eq!(plan.bits(), 0, "Go 里这 8 个关系链 handler 不贡献位");
        assert_eq!(plan.enabled_handlers().len(), 3);
    }

    #[test]
    fn unknown_names_are_collected_not_fatal() {
        let plan = IntentPlan::from_names(&names(&["GroupATMessageEventHandler", "Nope"]));
        assert_eq!(plan.unknown_names().to_vec(), vec!["Nope".to_owned()]);
        assert_eq!(plan.bits(), INTENT_GROUP_MESSAGES);
    }
}
