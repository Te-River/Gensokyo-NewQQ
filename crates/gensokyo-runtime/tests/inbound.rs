//! 入站转换单测：QQ 事件 `t` + `d` → OneBot `Event` 字段。
//!
//! 全部走公开 API（[`gensokyo_runtime::Inbound`]），无外网、无真实 QQ 连接。

mod support;

use gensokyo_core::Ob11Id;
use gensokyo_protocol::Event;
use gensokyo_protocol::event::MessageType;
use gensokyo_runtime::Inbound;
use serde_json::{Value, json};
use tempfile::TempDir;

/// 字符串上报 + 关闭增强字段，字段断言最直观。
const BASE: &str = r#"  app_id: 12345
  string_ob11: true
  native_ob11: true
  enable_ws_server: false
  http_address: ""
"#;

fn inbound(settings: &str) -> (Inbound, TempDir) {
    let (state, dir) = support::state(settings);
    (Inbound::from_state(&state), dir)
}

fn group_message_payload() -> Value {
    json!({
        "id": "M1",
        "group_id": "G1",
        "content": "hi",
        "timestamp": "1700000000",
        "author": {"id": "U1", "username": "alice", "member_role": "admin"}
    })
}

#[test]
fn group_at_message_becomes_group_message() {
    let (inbound, _dir) = inbound(BASE);
    let events = inbound
        .convert(
            "GROUP_AT_MESSAGE_CREATE",
            Some(&group_message_payload()),
            None,
        )
        .expect("转换失败");
    assert_eq!(events.len(), 1);

    let Event::Message(message) = &events[0] else {
        panic!("应为 message 事件，实际 {events:?}");
    };
    assert_eq!(message.message_type, MessageType::Group);
    assert_eq!(message.sub_type.as_deref(), Some("normal"));
    assert_eq!(message.self_id, Ob11Id::num(12345));
    assert_eq!(message.group_id.as_ref(), Some(&Ob11Id::str("G1")));
    assert_eq!(message.user_id, Ob11Id::str("U1"));
    assert_eq!(message.message_id, Ob11Id::str("M1"));
    assert_eq!(message.raw_message, "hi");
    assert_eq!(message.text(), "hi");
    assert_eq!(message.to_me, Some(true));
    assert_eq!(message.time, 1_700_000_000);
    let sender = message.sender.as_ref().expect("sender 不应缺失");
    assert_eq!(sender.nickname.as_deref(), Some("alice"));
    assert_eq!(sender.role.as_deref(), Some("admin"));
    assert_eq!(sender.user_id.as_ref(), Some(&Ob11Id::str("U1")));
    assert!(message.extra.is_empty(), "native_ob11 下不应有增强字段");
}

#[test]
fn full_group_message_is_not_to_me() {
    let (inbound, _dir) = inbound(BASE);
    let events = inbound
        .convert("GROUP_MESSAGE_CREATE", Some(&group_message_payload()), None)
        .expect("转换失败");
    let Event::Message(message) = &events[0] else {
        panic!("应为 message 事件");
    };
    assert_eq!(message.to_me, Some(false));
    assert_eq!(
        message.extra.get("is_full_group_message"),
        Some(&Value::Bool(true))
    );
}

#[test]
fn numeric_mode_without_idmap_falls_back_to_raw_strings() {
    let (inbound, _dir) = inbound(&BASE.replace("string_ob11: true", "string_ob11: false"));
    let events = inbound
        .convert(
            "GROUP_AT_MESSAGE_CREATE",
            Some(&group_message_payload()),
            None,
        )
        .expect("转换失败");
    let Event::Message(message) = &events[0] else {
        panic!("应为 message 事件");
    };
    // 没有 idmap 时不能伪造数字：原样字符串（模块文档承诺的口径）。
    assert_eq!(message.group_id.as_ref(), Some(&Ob11Id::str("G1")));
    assert_eq!(message.user_id, Ob11Id::str("U1"));
    assert_eq!(message.self_id, Ob11Id::num(12345), "self_id 恒为数字");
}

#[test]
fn enhanced_fields_are_present_without_native_ob11() {
    let (inbound, _dir) = inbound(&BASE.replace("native_ob11: true", "native_ob11: false"));
    let events = inbound
        .convert(
            "GROUP_AT_MESSAGE_CREATE",
            Some(&group_message_payload()),
            None,
        )
        .expect("转换失败");
    let Event::Message(message) = &events[0] else {
        panic!("应为 message 事件");
    };
    assert_eq!(
        message.extra.get("real_user_id"),
        Some(&Value::String("U1".to_owned()))
    );
    assert_eq!(
        message.extra.get("real_group_id"),
        Some(&Value::String("G1".to_owned()))
    );
    assert_eq!(
        message.extra.get("real_message_type"),
        Some(&Value::String("group".to_owned()))
    );
}

#[test]
fn c2c_message_becomes_private_message() {
    let (inbound, _dir) = inbound(BASE);
    let payload = json!({
        "id": "M2",
        "content": "私聊",
        "timestamp": 1_700_000_100,
        "author": {"id": "U2", "username": "bob"}
    });
    let events = inbound
        .convert("C2C_MESSAGE_CREATE", Some(&payload), None)
        .expect("转换失败");
    let Event::Message(message) = &events[0] else {
        panic!("应为 message 事件");
    };
    assert_eq!(message.message_type, MessageType::Private);
    assert_eq!(message.sub_type.as_deref(), Some("friend"));
    assert_eq!(message.group_id, None);
    assert_eq!(message.to_me, None);
    assert_eq!(message.user_id, Ob11Id::str("U2"));
    assert_eq!(message.raw_message, "私聊");
    assert_eq!(message.time, 1_700_000_100);
}

#[test]
fn friend_add_emits_request_then_notice() {
    let (inbound, _dir) = inbound(BASE);
    let payload = json!({
        "openid": "U3",
        "timestamp": "1700000200",
        "scene": 3,
        "scene_param": "from-search"
    });
    let events = inbound
        .convert("FRIEND_ADD", Some(&payload), Some("EV1"))
        .expect("转换失败");
    assert_eq!(events.len(), 2, "对齐 Go：request + notice");

    let Event::Request(request) = &events[0] else {
        panic!("首条应为 request");
    };
    assert_eq!(request.request_type, "friend");
    assert_eq!(request.user_id.as_ref(), Some(&Ob11Id::str("U3")));
    assert_eq!(request.comment, "from-search");
    assert_eq!(request.flag, "EV1", "d.event_id 缺失时用信封 id");

    let Event::Notice(notice) = &events[1] else {
        panic!("第二条应为 notice");
    };
    assert_eq!(notice.notice_type, "friend_add");
    assert_eq!(notice.user_id.as_ref(), Some(&Ob11Id::str("U3")));
    assert_eq!(
        notice.extra.get("scene_param"),
        Some(&Value::String("from-search".to_owned()))
    );
}

#[test]
fn group_add_robot_emits_request_then_group_increase() {
    let (inbound, _dir) = inbound(BASE);
    let payload = json!({
        "group_openid": "G9",
        "op_member_openid": "U9",
        "timestamp": "1700000300"
    });
    let events = inbound
        .convert("GROUP_ADD_ROBOT", Some(&payload), None)
        .expect("转换失败");
    assert_eq!(events.len(), 2);

    let Event::Request(request) = &events[0] else {
        panic!("首条应为 request");
    };
    assert_eq!(request.request_type, "group");
    assert_eq!(request.sub_type.as_deref(), Some("invite"));
    assert_eq!(request.group_id.as_ref(), Some(&Ob11Id::str("G9")));

    let Event::Notice(notice) = &events[1] else {
        panic!("第二条应为 notice");
    };
    assert_eq!(notice.notice_type, "group_increase");
    assert_eq!(notice.sub_type.as_deref(), Some("invite"));
    assert_eq!(notice.user_id.as_ref(), Some(&Ob11Id::str("U9")));
    assert_eq!(notice.operator_id, Some(Ob11Id::num(0)));
}

#[test]
fn group_management_events_map_to_their_notices() {
    let (inbound, _dir) = inbound(BASE);
    let payload = json!({
        "group_openid": "G7",
        "op_member_openid": "U7",
        "timestamp": 1_700_000_400
    });
    let cases = [
        ("GROUP_DEL_ROBOT", "group_decrease", Some("kick_me")),
        ("GROUP_MSG_REJECT", "group_reject", Some("create")),
        ("GROUP_MSG_RECEIVE", "group_receive", Some("create")),
    ];
    for (kind, notice_type, sub_type) in cases {
        let events = inbound
            .convert(kind, Some(&payload), None)
            .expect("转换失败");
        assert_eq!(events.len(), 1, "{kind} 应只产出一条 notice");
        let Event::Notice(notice) = &events[0] else {
            panic!("{kind} 应为 notice");
        };
        assert_eq!(notice.notice_type, notice_type);
        assert_eq!(notice.sub_type.as_deref(), sub_type);
        assert_eq!(notice.group_id.as_ref(), Some(&Ob11Id::str("G7")));
        assert_eq!(notice.time, 1_700_000_400);
    }
}

#[test]
fn friend_del_becomes_friend_decrease_notice() {
    let (inbound, _dir) = inbound(BASE);
    let payload = json!({"openid": "U4", "timestamp": "1700000500"});
    let events = inbound
        .convert("FRIEND_DEL", Some(&payload), None)
        .expect("转换失败");
    assert_eq!(events.len(), 1, "Go ProcessFriendDel 只上报 notice");

    let Event::Notice(notice) = &events[0] else {
        panic!("应为 notice，实际 {events:?}");
    };
    assert_eq!(notice.notice_type, "friend_decrease");
    assert_eq!(notice.sub_type, None);
    assert_eq!(notice.group_id, None);
    assert_eq!(notice.user_id.as_ref(), Some(&Ob11Id::str("U4")));
    assert_eq!(notice.operator_id, None);
    assert_eq!(notice.self_id, Ob11Id::num(12345));
    assert_eq!(notice.time, 1_700_000_500);
    assert!(notice.extra.is_empty(), "native_ob11 下不应有增强字段");
}

#[test]
fn c2c_switch_events_become_their_notices() {
    let (inbound, _dir) = inbound(BASE);
    let payload = json!({"openid": "U6", "timestamp": 1_700_000_700});
    for (kind, notice_type) in [
        ("C2C_MSG_REJECT", "c2c_msg_reject"),
        ("C2C_MSG_RECEIVE", "c2c_msg_receive"),
    ] {
        let events = inbound
            .convert(kind, Some(&payload), None)
            .expect("转换失败");
        assert_eq!(events.len(), 1, "{kind} 应只产出一条 notice");
        let Event::Notice(notice) = &events[0] else {
            panic!("{kind} 应为 notice");
        };
        assert_eq!(notice.notice_type, notice_type);
        assert_eq!(notice.group_id, None);
        assert_eq!(notice.user_id.as_ref(), Some(&Ob11Id::str("U6")));
        assert_eq!(notice.time, 1_700_000_700);
    }
}

#[test]
fn group_member_add_becomes_group_increase_notice() {
    let (inbound, _dir) = inbound(BASE);
    let payload = json!({
        "event_id": "EV9",
        "group_openid": "G9",
        "member_openid": "U9",
        "timestamp": 1_700_000_600
    });
    let events = inbound
        .convert("GROUP_MEMBER_ADD", Some(&payload), None)
        .expect("转换失败");
    assert_eq!(events.len(), 1);

    let Event::Notice(notice) = &events[0] else {
        panic!("应为 notice，实际 {events:?}");
    };
    assert_eq!(notice.notice_type, "group_increase");
    assert_eq!(notice.sub_type.as_deref(), Some("approve"));
    assert_eq!(notice.group_id.as_ref(), Some(&Ob11Id::str("G9")));
    assert_eq!(notice.user_id.as_ref(), Some(&Ob11Id::str("U9")));
    assert_eq!(notice.operator_id, Some(Ob11Id::num(0)));
    assert_eq!(notice.time, 1_700_000_600);
    assert_eq!(
        notice.extra.get("message"),
        Some(&json!("[CQ:member,type=add,group_id=G9,user_id=U9]"))
    );
    assert_eq!(notice.extra.get("event_id"), Some(&json!("EV9")));
    // Go 对这两个事件不区分 native_ob11：real_* 只被 omitempty 省略空值。
    assert_eq!(notice.extra.get("real_user_id"), Some(&json!("U9")));
    assert_eq!(notice.extra.get("real_group_id"), Some(&json!("G9")));
}

#[test]
fn group_member_remove_becomes_group_decrease_notice() {
    let (inbound, _dir) = inbound(BASE);
    let payload = json!({
        "event_id": "EV10",
        "group_openid": "G9",
        "member_openid": "U9",
        "timestamp": "1700000800"
    });
    let events = inbound
        .convert("GROUP_MEMBER_REMOVE", Some(&payload), None)
        .expect("转换失败");

    let Event::Notice(notice) = &events[0] else {
        panic!("应为 notice，实际 {events:?}");
    };
    assert_eq!(notice.notice_type, "group_decrease");
    assert_eq!(notice.sub_type.as_deref(), Some("leave"));
    assert_eq!(notice.operator_id, Some(Ob11Id::num(0)));
    assert_eq!(notice.time, 1_700_000_800);
    assert_eq!(
        notice.extra.get("message"),
        Some(&json!("[CQ:member,type=remove,group_id=G9,user_id=U9]"))
    );
    assert!(
        notice.extra.get("event_id").is_none(),
        "Go 只给入群方向保留 event_id"
    );
}

#[test]
fn group_join_request_becomes_group_add_request() {
    let (inbound, _dir) = inbound(BASE);
    let payload = json!({
        "join_request_id": "JR1",
        "group_openid": "G5",
        "member_openid": "U5",
        "username": "carol",
        "apply_at": "2023-11-14T22:13:20Z",
        "verify_info": {"method": "verify_message", "verify_message": "让我进来"},
        "auto_approved": {"strategy_id": "s1"}
    });
    let events = inbound
        .convert("GROUP_JOIN_REQUEST", Some(&payload), None)
        .expect("转换失败");
    assert_eq!(events.len(), 1);

    let Event::Request(request) = &events[0] else {
        panic!("应为 request，实际 {events:?}");
    };
    assert_eq!(request.request_type, "group");
    assert_eq!(request.sub_type.as_deref(), Some("add"));
    assert_eq!(request.flag, "JR1", "flag 为 join_request_id，供审批回传");
    assert_eq!(request.comment, "让我进来");
    assert_eq!(request.user_id.as_ref(), Some(&Ob11Id::str("U5")));
    assert_eq!(request.group_id.as_ref(), Some(&Ob11Id::str("G5")));
    assert_eq!(request.time, 1_700_000_000, "apply_at 按 RFC3339 解析");
    assert_eq!(request.extra.get("username"), Some(&json!("carol")));
    assert_eq!(request.extra.get("auto_approved_bool"), Some(&json!(true)));
}

#[test]
fn group_join_request_accepts_legacy_verify_string_and_numbers() {
    let (inbound, _dir) = inbound(BASE);
    let payload = json!({
        "join_request_id": "JR2",
        "group_openid": "G5",
        "member_openid": "U5",
        "apply_at": "1700000900",
        "verify_info": "旧格式",
        "auto_approved": false
    });
    let events = inbound
        .convert("GROUP_JOIN_REQUEST", Some(&payload), None)
        .expect("转换失败");
    let Event::Request(request) = &events[0] else {
        panic!("应为 request");
    };
    assert_eq!(request.comment, "旧格式");
    assert_eq!(request.time, 1_700_000_900);
    assert_eq!(request.extra.get("auto_approved_bool"), Some(&json!(false)));
    assert!(
        request.extra.get("username").is_none(),
        "username 为空时不补字段"
    );
}

#[test]
fn relation_events_carry_enhanced_ids_without_native_ob11() {
    let (inbound, _dir) = inbound(&BASE.replace("native_ob11: true", "native_ob11: false"));

    let user_op = json!({"openid": "U8", "timestamp": 1_700_001_000});
    let events = inbound
        .convert("FRIEND_DEL", Some(&user_op), None)
        .expect("转换失败");
    let Event::Notice(notice) = &events[0] else {
        panic!("应为 notice");
    };
    assert_eq!(notice.extra.get("real_user_id"), Some(&json!("U8")));

    let join = json!({
        "join_request_id": "JR3",
        "group_openid": "G8",
        "member_openid": "U8",
        "apply_at": 1_700_001_100
    });
    let events = inbound
        .convert("GROUP_JOIN_REQUEST", Some(&join), None)
        .expect("转换失败");
    let Event::Request(request) = &events[0] else {
        panic!("应为 request");
    };
    assert_eq!(request.extra.get("real_user_id"), Some(&json!("U8")));
    assert_eq!(request.extra.get("real_group_id"), Some(&json!("G8")));
    assert_eq!(request.comment, "", "verify_info 缺失时 comment 为空");
    assert_eq!(request.time, 1_700_001_100);
}

#[test]
fn all_twelve_botgo_events_convert() {
    // botgo/dto/websocket_event.go:42-57 的 12 类自定义事件，逐条确认不会掉进未知分支。
    let (inbound, _dir) = inbound(BASE);
    let cases = [
        (
            "GROUP_MESSAGE_CREATE",
            json!({"id": "M1", "group_id": "G1", "content": "x"}),
            "post_type=message",
        ),
        (
            "GROUP_ADD_ROBOT",
            json!({"group_openid": "G1", "op_member_openid": "U1"}),
            "post_type=request",
        ),
        (
            "GROUP_DEL_ROBOT",
            json!({"group_openid": "G1", "op_member_openid": "U1"}),
            "post_type=notice",
        ),
        (
            "GROUP_MSG_REJECT",
            json!({"group_openid": "G1", "op_member_openid": "U1"}),
            "post_type=notice",
        ),
        (
            "GROUP_MSG_RECEIVE",
            json!({"group_openid": "G1", "op_member_openid": "U1"}),
            "post_type=notice",
        ),
        (
            "FRIEND_ADD",
            json!({"openid": "U1", "scene": 1}),
            "post_type=request",
        ),
        ("FRIEND_DEL", json!({"openid": "U1"}), "post_type=notice"),
        (
            "C2C_MSG_REJECT",
            json!({"openid": "U1"}),
            "post_type=notice",
        ),
        (
            "C2C_MSG_RECEIVE",
            json!({"openid": "U1"}),
            "post_type=notice",
        ),
        (
            "GROUP_MEMBER_ADD",
            json!({"group_openid": "G1", "member_openid": "U1"}),
            "post_type=notice",
        ),
        (
            "GROUP_MEMBER_REMOVE",
            json!({"group_openid": "G1", "member_openid": "U1"}),
            "post_type=notice",
        ),
        (
            "GROUP_JOIN_REQUEST",
            json!({"group_openid": "G1", "member_openid": "U1", "join_request_id": "JR"}),
            "post_type=request",
        ),
    ];
    assert_eq!(cases.len(), 12);
    for (kind, payload, expected) in cases {
        let events = inbound
            .convert(kind, Some(&payload), None)
            .unwrap_or_else(|err| panic!("{kind} 转换失败: {err}"));
        assert!(!events.is_empty(), "{kind} 不应落到未知事件分支");
        let actual = format!("post_type={}", events[0].post_type());
        assert_eq!(actual, expected, "{kind} 的首条事件类型");
    }
}

#[test]
fn lifecycle_and_heartbeat_meta_events_match_go_shape() {
    let (inbound, _dir) = inbound(BASE);

    let Event::MetaEvent(connect) = inbound.lifecycle_connect() else {
        panic!("应为 meta_event");
    };
    assert_eq!(connect.meta_event_type, "lifecycle");
    assert_eq!(connect.sub_type.as_deref(), Some("connect"));
    assert_eq!(connect.self_id, Ob11Id::num(12345));

    let Event::MetaEvent(heartbeat) = inbound.heartbeat(5000) else {
        panic!("应为 meta_event");
    };
    assert_eq!(heartbeat.meta_event_type, "heartbeat");
    assert_eq!(heartbeat.interval, Some(5000));
    let status = heartbeat.status.expect("心跳应带 status");
    assert!(status.online && status.good);
}

#[test]
fn invalid_json_is_an_error_not_a_panic() {
    let (inbound, _dir) = inbound(BASE);
    assert!(inbound.convert_json(b"not json").is_err());
    assert!(inbound.convert_json(b"{]").is_err());
    // 合法 JSON 但不是对象：serde 反序列化到 Envelope 失败 → Err，不是 panic。
    // （空数组 `[]` 会被 serde 按字段位置解成全默认值，因此不算错误。）
    assert!(inbound.convert_json(b"\"oops\"").is_err());
}

#[test]
fn unknown_or_incomplete_events_yield_nothing() {
    let (inbound, _dir) = inbound(BASE);
    assert!(
        inbound
            .convert_json(br#"{"t":"GUILD_CREATE","d":{}}"#)
            .expect("结构合法")
            .is_empty()
    );
    // 缺 t / 缺 d 都不能炸。
    assert!(
        inbound
            .convert_json(br#"{"d":{}}"#)
            .expect("结构合法")
            .is_empty()
    );
    assert!(
        inbound
            .convert_json(br#"{"t":"GROUP_AT_MESSAGE_CREATE"}"#)
            .expect("结构合法")
            .is_empty()
    );
    // 已知 t 但 d 字段类型不对：返回 Err（webhook 回 400），同样不 panic。
    assert!(
        inbound
            .convert("GROUP_AT_MESSAGE_CREATE", Some(&json!("oops")), None)
            .is_err()
    );
}
