//! `text_intent` → botgo intent 位（[`gensokyo_runtime::IntentPlan`]）。
//!
//! 口径来源：Go `main.go:316-353`（名字 → handler → `RegisterHandlers` 换位）与
//! `main.go:333-340` / `913-955`（`discover_unknown_events` / 摘位策略）。

mod support;

use gensokyo_runtime::IntentPlan;

/// botgo `IntentGroupMembers`。
const GROUP_MEMBERS: i64 = 1 << 24;
/// botgo `IntentGroupMessages`。
const GROUP_MESSAGES: i64 = 1 << 25;
/// botgo `IntentInteraction`。
const INTERACTION: i64 = 1 << 26;

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

fn base(text_intent: &str, extra: &str) -> String {
    format!(
        r#"  app_id: 12345
  string_ob11: true
  native_ob11: true
  enable_ws_server: false
  http_address: ""
  text_intent: [{text_intent}]
{extra}"#
    )
}

fn plan_for(settings: &str) -> IntentPlan {
    let (manager, _dir) = support::config(settings);
    IntentPlan::from_settings(&manager.snapshot())
}

#[test]
fn handler_names_map_to_botgo_intent_bits() {
    // 17 个名字的完整映射；8 个关系链/群开关 handler 在位图里贡献 0（Go 的既有行为）。
    let cases = [
        ("ReadyHandler", 0),
        ("ErrorNotifyHandler", 0),
        ("InteractionHandler", INTERACTION),
        ("GroupATMessageEventHandler", GROUP_MESSAGES),
        ("C2CMessageEventHandler", GROUP_MESSAGES),
        ("GroupAddRobotEventHandler", 0),
        ("GroupDelRobotEventHandler", 0),
        ("GroupMsgRejectHandler", 0),
        ("GroupMsgReceiveHandler", 0),
        ("FriendAddEventHandler", 0),
        ("FriendDelEventHandler", 0),
        ("C2CMsgRejectHandler", 0),
        ("C2CMsgReceiveHandler", 0),
        ("GroupMessageEventHandler", GROUP_MESSAGES),
        ("GroupMemberAddEventHandler", GROUP_MEMBERS),
        ("GroupMemberRemoveEventHandler", GROUP_MEMBERS),
        ("GroupJoinRequestEventHandler", GROUP_MESSAGES),
    ];
    assert_eq!(cases.len(), 17);
    for (name, bits) in cases {
        let plan = IntentPlan::from_names(&names(&[name]));
        assert_eq!(plan.bits(), bits, "{name} 的位");
        assert!(plan.unknown_names().is_empty(), "{name} 不应算未知");
        assert_eq!(plan.enabled_handlers(), &[name][..]);
    }
}

#[test]
fn unknown_names_are_reported_and_ignored() {
    let plan = IntentPlan::from_names(&names(&[
        "GroupMemberAddEventHandler",
        "NopeHandler",
        "GroupMemberAddEventHandler",
    ]));
    assert_eq!(plan.bits(), GROUP_MEMBERS);
    assert_eq!(
        plan.unknown_names().to_vec(),
        vec!["NopeHandler".to_owned()]
    );
    // 重名只登记一次。
    assert_eq!(
        plan.enabled_handlers(),
        &["GroupMemberAddEventHandler"][..],
        "重复名字不应重复登记"
    );
}

#[test]
fn subscription_bits_combine_with_bitwise_or() {
    let plan = IntentPlan::from_names(&names(&[
        "GroupATMessageEventHandler",
        "GroupMemberRemoveEventHandler",
        "InteractionHandler",
    ]));
    assert_eq!(plan.bits(), GROUP_MESSAGES | GROUP_MEMBERS | INTERACTION);
    assert!(plan.contains(GROUP_MESSAGES));
    assert!(plan.contains(GROUP_MEMBERS));
    assert!(plan.contains(INTERACTION));
    assert!(!plan.contains(1 << 30), "未订阅的位不能出现");
}

#[test]
fn empty_text_intent_yields_no_bits_instead_of_panicking() {
    let plan = plan_for(&base("", ""));
    assert_eq!(plan.bits(), 0);
    assert!(plan.enabled_handlers().is_empty());
}

#[test]
fn discover_unknown_events_adds_the_sixteen_probe_bits() {
    let plan = plan_for(&base(
        "GroupATMessageEventHandler",
        "  discover_unknown_events: true\n",
    ));
    assert!(plan.contains(GROUP_MESSAGES));
    for bit in [6, 7, 8, 11, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 31] {
        assert!(plan.contains(1_i64 << bit), "第 {bit} 位应被探测订阅");
    }
    assert!(!plan.contains(GROUP_MEMBERS), "未使用位之外不应凭空多出");
    assert!(!plan.contains(1));
}

#[test]
fn auto_subscribe_enables_reject_and_receive_handlers_without_bits() {
    let plan = plan_for(&base(
        "GroupATMessageEventHandler",
        "  global_group_msg_rre_to_message: true\n",
    ));
    assert_eq!(plan.bits(), GROUP_MESSAGES, "自动订阅不改变位集");
    assert_eq!(
        plan.enabled_handlers(),
        &[
            "GroupATMessageEventHandler",
            "GroupMsgRejectHandler",
            "GroupMsgReceiveHandler"
        ][..]
    );
}

#[test]
fn suppress_disallowed_intents_drops_group_members_bit() {
    let plan = plan_for(&base(
        "GroupMemberAddEventHandler",
        "  suppress_disallowed_intents: true\n",
    ));
    assert_eq!(plan.bits(), 0);
    assert_eq!(plan.suppressed(), &["group_members"][..]);
}

#[test]
fn suppress_disallowed_intents_drops_lone_group_messages_bit() {
    let plan = plan_for(&base(
        "GroupMessageEventHandler",
        "  suppress_disallowed_intents: true\n",
    ));
    assert_eq!(plan.bits(), 0);
    assert_eq!(plan.suppressed(), &["group_messages"][..]);
}

#[test]
fn suppress_keeps_group_messages_bit_when_other_handlers_share_it() {
    let plan = plan_for(&base(
        "GroupMessageEventHandler, GroupATMessageEventHandler",
        "  suppress_disallowed_intents: true\n",
    ));
    assert_eq!(plan.bits(), GROUP_MESSAGES);
    assert!(plan.suppressed().is_empty());
}

#[test]
fn suppress_without_the_switch_leaves_bits_untouched() {
    let plan = plan_for(&base("GroupMemberAddEventHandler", ""));
    assert_eq!(plan.bits(), GROUP_MEMBERS);
    assert!(plan.suppressed().is_empty());
}
