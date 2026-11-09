//! 入站转换单测：QQ 事件 `t` + `d` → OneBot `Event` 字段。
//!
//! 全部走公开 API（[`gensokyo_runtime::Inbound`]），无外网、无真实 QQ 连接。

mod support;

use std::sync::Arc;

use gensokyo_core::Ob11Id;
use gensokyo_protocol::Event;
use gensokyo_protocol::MessageContent;
use gensokyo_protocol::event::MessageType;
use gensokyo_runtime::Inbound;
use gensokyo_storage::{IdMapOptions, IdMapStore};
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
    // Go M1-B：string 形态不做 idmap 转换，`sender.user_id` 恒为 0（数字）
    assert_eq!(sender.user_id.as_ref(), Some(&Ob11Id::num(0)));
    assert_eq!(
        sender.tiny_id.as_deref(),
        Some(""),
        "tiny_id 恒上报（空串）"
    );
    assert_eq!(sender.sex.as_deref(), Some("0"));
    assert_eq!(sender.level.as_deref(), Some("0"));
    assert_eq!(sender.age, None, "Go `Sender.Age` 带 omitempty，0 不上报");
    assert_eq!(
        message.extra.get("platform"),
        Some(&Value::String("qq".to_owned())),
        "string 形态的群消息恒带 platform"
    );
    assert_eq!(
        message.message_seq,
        Some(0),
        "message_seq 非 omitempty，缺省上报 0"
    );
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
    assert_eq!(
        message.to_me, None,
        "Go `ToMe` 带 omitempty：全量群消息不上报该字段"
    );
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

/// 段数组模式（`array_value = true`）：`string_ob11` 只决定 ID 类型，不改变消息形态。
fn array_settings(string_ob11: bool) -> String {
    format!(
        "  app_id: 12345\n  string_ob11: {string_ob11}\n  native_ob11: true\n  array: true\n  enable_ws_server: false\n  http_address: \"\"\n"
    )
}

fn message_payload(content: &str) -> Value {
    json!({
        "id": "M1",
        "group_id": "G1",
        "content": content,
        "timestamp": "1700000000",
        "author": {"id": "U1", "username": "alice"}
    })
}

/// 转换一条群消息，拆出 `message` 段数组（JSON 形态）并带上 `raw_message`。
fn segments_of(settings: &str, content: &str) -> (Vec<Value>, String) {
    let (inbound, _dir) = inbound(settings);
    let events = inbound
        .convert(
            "GROUP_AT_MESSAGE_CREATE",
            Some(&message_payload(content)),
            None,
        )
        .expect("转换失败");
    let Event::Message(message) = &events[0] else {
        panic!("应为 message 事件，实得 {events:?}");
    };
    let MessageContent::Array(segments) = &message.message else {
        panic!("array_value=true 应上报段数组，实得 {:?}", message.message);
    };
    (
        segments
            .iter()
            .map(|segment| serde_json::to_value(segment).expect("段序列化失败"))
            .collect(),
        message.raw_message.clone(),
    )
}

#[test]
fn array_value_parses_plain_text_into_text_segment() {
    let (segments, raw) = segments_of(&array_settings(true), "hi");
    assert_eq!(
        segments,
        vec![json!({"type": "text", "data": {"text": "hi"}})]
    );
    assert_eq!(raw, "hi", "raw_message 恒为字符串");
}

#[test]
fn array_value_parses_at_and_multiple_segments() {
    let (segments, raw) = segments_of(&array_settings(true), "a[CQ:at,qq=10086]b[CQ:face,id=1]c");
    assert_eq!(
        segments,
        vec![
            json!({"type": "text", "data": {"text": "a"}}),
            json!({"type": "at", "data": {"qq": "10086"}}),
            json!({"type": "text", "data": {"text": "b"}}),
            json!({"type": "face", "data": {"id": "1"}}),
            json!({"type": "text", "data": {"text": "c"}}),
        ]
    );
    assert_eq!(raw, "a[CQ:at,qq=10086]b[CQ:face,id=1]c");
}

#[test]
fn array_value_degrades_unknown_codes_to_text() {
    // `bogus` 不在码表；`remove` 是出站专用（inbound=false）。两者都折回 text 原文，
    // 且相邻 text 合并成一段（与 Go 文本段「连续切片」的粒度一致）。
    let content = "a[CQ:bogus,foo=1]b[CQ:remove,user_id=9]c";
    let (segments, raw) = segments_of(&array_settings(true), content);
    assert_eq!(
        segments,
        vec![json!({"type": "text", "data": {"text": content}})]
    );
    assert_eq!(raw, content);
}

#[test]
fn array_value_off_keeps_string_message() {
    let (inbound, _dir) = inbound(BASE);
    let events = inbound
        .convert(
            "GROUP_AT_MESSAGE_CREATE",
            Some(&message_payload("hi")),
            None,
        )
        .expect("转换失败");
    let Event::Message(message) = &events[0] else {
        panic!("应为 message 事件，实得 {events:?}");
    };
    let MessageContent::Text(text) = &message.message else {
        panic!(
            "array_value 关闭时应为字符串形态，实得 {:?}",
            message.message
        );
    };
    assert_eq!(text, "hi");
    assert_eq!(message.raw_message, "hi");
}

#[test]
fn add_at_group_prefix_only_reaches_raw_message_in_array_mode() {
    let settings = format!("{}  add_at_group: true\n", array_settings(true));
    let (inbound, _dir) = inbound(&settings);
    let events = inbound
        .convert("GROUP_MESSAGE_CREATE", Some(&message_payload("hi")), None)
        .expect("转换失败");
    let Event::Message(message) = &events[0] else {
        panic!("应为 message 事件，实得 {events:?}");
    };
    assert_eq!(message.raw_message, "[CQ:at,qq=12345] hi");
    let MessageContent::Array(segments) = &message.message else {
        panic!("array_value=true 应上报段数组，实得 {:?}", message.message);
    };
    let segments: Vec<Value> = segments
        .iter()
        .map(|segment| serde_json::to_value(segment).expect("段序列化失败"))
        .collect();
    assert_eq!(
        segments,
        vec![json!({"type": "text", "data": {"text": "hi"}})],
        "Go 数组路径读 data.Content，add_at_group 前缀不进 message"
    );
}

#[test]
fn array_value_is_independent_of_string_ob11() {
    for string_ob11 in [true, false] {
        let (segments, _raw) = segments_of(&array_settings(string_ob11), "hi[CQ:at,qq=7]");
        assert_eq!(
            segments,
            vec![
                json!({"type": "text", "data": {"text": "hi"}}),
                json!({"type": "at", "data": {"qq": "7"}}),
            ],
            "string_ob11={string_ob11} 时消息段形态不变"
        );
    }
}

#[test]
fn array_value_applies_to_private_message() {
    let (inbound, _dir) = inbound(&array_settings(true));
    let events = inbound
        .convert(
            "C2C_MESSAGE_CREATE",
            Some(&json!({
                "id": "M2",
                "content": "x[CQ:at,qq=7]",
                "timestamp": "1700000000",
                "author": {"id": "U1"}
            })),
            None,
        )
        .expect("转换失败");
    let Event::Message(message) = &events[0] else {
        panic!("应为 message 事件，实得 {events:?}");
    };
    assert!(message.message.is_array(), "单聊同样走段数组");
    assert_eq!(message.raw_message, "x[CQ:at,qq=7]");
}

// ---- 入站正文：mention 反解 / remove_at / remove_prefix（Go `RevertTransformedText`）----

/// 群场景里机器人的 OpenID（来自 `mentions[].is_you`，与 Ready 的 `bot_id` 形态不同）。
const BOT_MENTION: &str = "BEDAB0A652B5271EF7A8E9EB9ED48B42";
/// 机器人真实 ID（`GET /users/@me`）；纯数字，也能被 `<@([0-9A-Fa-f]+)>` 匹配。
const BOT_ID: &str = "7000012345678901234";
/// 消息发送者：被 `is_you` 误标时也不允许当作 @bot。
const SENDER_OPENID: &str = "D8FB64288B16F160289247BA495B7233";
/// 可反查的他人。
const OTHER_OPENID: &str = "AAAA1111AAAA1111AAAA1111AAAA1111";
/// 故意不入 idmap 的他人。
const UNMAPPED_OPENID: &str = "BBBB2222BBBB2222BBBB2222BBBB2222";

const AT_BASE: &str = "  app_id: 12345\n  string_ob11: true\n  native_ob11: true\n";

/// 带 idmap 的入站：预置 `seed` 里 OpenID 的 vUIN（`convert_other_at` 是**只查不分配**）。
fn at_inbound(settings: &str, seed: &[&str]) -> (Inbound, Vec<String>, TempDir) {
    let (manager, dir) = support::config(settings);
    let store = Arc::new(
        IdMapStore::open(
            IdMapOptions::new(
                dir.path().join("openid-map.redb"),
                dir.path().join("msgid-map.redb"),
            )
            .with_app_id("12345"),
        )
        .expect("打开 idmap 存储失败"),
    );
    let vids = seed
        .iter()
        .map(|openid| {
            let (_, vuin) = store.retrieve_virtual_value(openid).expect("预置映射失败");
            vuin.to_string()
        })
        .collect();
    (Inbound::new(manager, Some(store)), vids, dir)
}

fn at_payload(content: &str, mentions: Value) -> Value {
    json!({
        "id": "M1",
        "group_id": "G1",
        "content": content,
        "timestamp": "1700000000",
        "author": {"id": SENDER_OPENID, "username": "alice"},
        "mentions": mentions,
    })
}

/// 转换一条群消息，返回 (`raw_message`, `message` 的纯文本形态)。
fn at_text(inbound: &Inbound, kind: &str, payload: &Value) -> (String, String) {
    let events = inbound
        .convert(kind, Some(payload), None)
        .expect("转换失败");
    let Event::Message(message) = &events[0] else {
        panic!("应为 message 事件，实得 {events:?}");
    };
    (message.raw_message.clone(), message.text())
}

/// 段数组形态的 `message` 段（JSON 视图）。
fn array_segments(inbound: &Inbound, kind: &str, payload: &Value) -> (Vec<Value>, String) {
    let events = inbound
        .convert(kind, Some(payload), None)
        .expect("转换失败");
    let Event::Message(message) = &events[0] else {
        panic!("应为 message 事件，实得 {events:?}");
    };
    let MessageContent::Array(segments) = &message.message else {
        panic!("array_value=true 应上报段数组，实得 {:?}", message.message);
    };
    (
        segments
            .iter()
            .map(|segment| serde_json::to_value(segment).expect("段序列化失败"))
            .collect(),
        message.raw_message.clone(),
    )
}

#[test]
fn mention_of_other_becomes_cq_at() {
    let settings = format!("{AT_BASE}  convert_other_at: true\n  remove_at: true\n");
    let (inbound, vids, _dir) = at_inbound(&settings, &[OTHER_OPENID]);
    let payload = at_payload(
        &format!("<@{BOT_MENTION}> <@{OTHER_OPENID}> 你好"),
        json!([{"id": BOT_MENTION, "is_you": true}]),
    );
    let (raw, text) = at_text(&inbound, "GROUP_AT_MESSAGE_CREATE", &payload);
    assert_eq!(raw, format!("[CQ:at,qq={}] 你好", vids[0]));
    assert_eq!(text, " 你好", "纯文本形态只剩 at 之后的正文");
}

#[test]
fn self_mention_uses_self_id_when_remove_at_is_off() {
    let settings = format!("{AT_BASE}  remove_at: false\n");
    let (inbound, _vids, _dir) = at_inbound(&settings, &[]);
    let payload = at_payload(
        &format!("<@{BOT_MENTION}> 你好"),
        json!([{"id": BOT_MENTION, "is_you": true}]),
    );
    let (raw, _) = at_text(&inbound, "GROUP_AT_MESSAGE_CREATE", &payload);
    assert_eq!(
        raw, "[CQ:at,qq=12345] 你好",
        "关闭 remove_at 时自身 @ 还原成 self_id（`use_uin ? uin : app_id`）"
    );
}

#[test]
fn self_mention_strip_follows_event_and_remove_at() {
    let content = format!("<@{BOT_MENTION}> 你好");
    let mentions = json!([{"id": BOT_MENTION, "is_you": true}]);
    for (kind, remove_at, want) in [
        ("GROUP_AT_MESSAGE_CREATE", true, "你好"),
        ("GROUP_AT_MESSAGE_CREATE", false, "[CQ:at,qq=12345] 你好"),
        // 全量群消息的 @bot 恒剥离，与 remove_at 无关
        ("GROUP_MESSAGE_CREATE", false, "你好"),
        ("GROUP_MESSAGE_CREATE", true, "你好"),
    ] {
        let settings = format!("{AT_BASE}  remove_at: {remove_at}\n");
        let (inbound, _vids, _dir) = at_inbound(&settings, &[]);
        let (raw, _) = at_text(&inbound, kind, &at_payload(&content, mentions.clone()));
        assert_eq!(raw, want, "kind={kind} remove_at={remove_at}");
    }
}

#[test]
fn mention_matching_bot_id_is_stripped_without_mention_flags() {
    let settings = format!("{AT_BASE}  remove_at: true\n");
    let (inbound, _vids, _dir) = at_inbound(&settings, &[]);
    let inbound = inbound.with_bot_id(BOT_ID);
    let payload = at_payload(&format!("<@{BOT_ID}> 你好"), json!([]));
    let (raw, _) = at_text(&inbound, "GROUP_AT_MESSAGE_CREATE", &payload);
    assert_eq!(raw, "你好", "mentions 缺失时 BotID 直等仍判为自身");
}

#[test]
fn mention_author_is_never_treated_as_self() {
    // Go `zzzz_inbound_at_order_test.go` ⑦：`is_you` 误标发送者时，发送者的 @ 既不能被
    // 当成 @bot 剥掉，也不能被映射成 bot 的 ID。
    let settings = format!("{AT_BASE}  convert_other_at: true\n  remove_at: true\n");
    let (inbound, vids, _dir) = at_inbound(&settings, &[SENDER_OPENID]);
    let payload = at_payload(
        &format!("<@{BOT_MENTION}> at检测 <@{SENDER_OPENID}>"),
        json!([
            {"id": SENDER_OPENID, "is_you": true},
            {"id": BOT_MENTION, "bot": true},
        ]),
    );
    let (raw, _) = at_text(&inbound, "GROUP_AT_MESSAGE_CREATE", &payload);
    assert_eq!(raw, format!("at检测 [CQ:at,qq={}]", vids[0]));
}

#[test]
fn unresolvable_mention_keeps_raw_text_in_place() {
    let settings = format!("{AT_BASE}  convert_other_at: true\n  remove_at: true\n");
    let (inbound, _vids, _dir) = at_inbound(&settings, &[]);
    let payload = at_payload(
        &format!("<@{BOT_MENTION}> <@{UNMAPPED_OPENID}> 中"),
        json!([{"id": BOT_MENTION, "is_you": true}]),
    );
    let (raw, _) = at_text(&inbound, "GROUP_AT_MESSAGE_CREATE", &payload);
    assert_eq!(raw, format!("<@{UNMAPPED_OPENID}> 中"), "反查失败原位保留");
}

#[test]
fn all_switches_off_keeps_content_verbatim() {
    // 他人 OpenID 已进 idmap，但 convert_other_at 关闭：不得查表、不得生成 at。
    let (inbound, _vids, _dir) = at_inbound(AT_BASE, &[OTHER_OPENID]);
    let content = format!("<@{OTHER_OPENID}> at检测 <@1234>");
    let (raw, _) = at_text(
        &inbound,
        "GROUP_AT_MESSAGE_CREATE",
        &at_payload(&content, json!([])),
    );
    assert_eq!(raw, content, "三个开关全关时正文零变化");
    assert!(!raw.contains("[CQ:at"), "convert_other_at 关闭时不得生成 at 码");
}

#[test]
fn remove_prefix_value_drops_first_slash() {
    let settings = format!("{AT_BASE}  remove_prefix: true\n");
    let (inbound, _vids, _dir) = at_inbound(&settings, &[]);
    let (raw, _) = at_text(
        &inbound,
        "GROUP_AT_MESSAGE_CREATE",
        &at_payload("/指令 a/b", json!([])),
    );
    assert_eq!(raw, "指令 a/b", "只去第一个 `/`（Go `strings.Index` 口径）");
}

#[test]
fn string_form_trims_surrounding_whitespace() {
    let (inbound, _vids, _dir) = at_inbound(AT_BASE, &[]);
    let (raw, _) = at_text(
        &inbound,
        "GROUP_AT_MESSAGE_CREATE",
        &at_payload("  你好  ", json!([])),
    );
    assert_eq!(raw, "你好");
}

#[test]
fn lone_slash_and_space_survives_trim_and_prefix_removal() {
    // 字符串路径把 "/ " 归一成 " "（Go `message_parser.go:1832`），既不再 trim 也不去 `/`。
    let settings = format!("{AT_BASE}  remove_prefix: true\n  remove_at: true\n");
    let (inbound, _vids, _dir) = at_inbound(&settings, &[]);
    let (raw, _) = at_text(
        &inbound,
        "GROUP_AT_MESSAGE_CREATE",
        &at_payload("/ ", json!([])),
    );
    assert_eq!(raw, " ");
}

#[test]
fn array_form_emits_at_segment_in_place() {
    let settings = "  app_id: 12345\n  string_ob11: true\n  native_ob11: true\n  array: true\n  convert_other_at: true\n  remove_at: true\n";
    let (inbound, vids, _dir) = at_inbound(settings, &[OTHER_OPENID]);
    let payload = at_payload(
        &format!("<@{BOT_MENTION}> <@{OTHER_OPENID}> 你好"),
        json!([{"id": BOT_MENTION, "is_you": true}]),
    );
    let (segments, raw) = array_segments(&inbound, "GROUP_AT_MESSAGE_CREATE", &payload);
    assert_eq!(
        segments,
        vec![
            json!({"type": "at", "data": {"qq": vids[0]}}),
            json!({"type": "text", "data": {"text": " 你好"}}),
        ],
        "at 段按原位顺序落段，不前移"
    );
    assert_eq!(raw, format!("[CQ:at,qq={}] 你好", vids[0]));
}

#[test]
fn array_form_keeps_whitespace_when_remove_at_is_off() {
    // Go 段路径（`ConvertToSegmentedMessage`）没有初始 trim：`remove_at=false` 的 AT 事件
    // 把 "  你好  " 原样报成 text 段，而字符串路径（先 trim）报 "你好"。
    let settings = "  app_id: 12345\n  string_ob11: true\n  native_ob11: true\n  array: true\n";
    let (inbound, _vids, _dir) = at_inbound(settings, &[]);
    let (segments, raw) = array_segments(
        &inbound,
        "GROUP_AT_MESSAGE_CREATE",
        &at_payload("  你好  ", json!([])),
    );
    assert_eq!(
        segments,
        vec![json!({"type": "text", "data": {"text": "  你好  "}})]
    );
    assert_eq!(raw, "你好");
}

#[test]
fn private_message_converts_other_mentions() {
    // 单聊在 Go 里同样走 `RevertTransformedText`（`ProcessC2CMessage.go:104`）。
    let settings = format!("{AT_BASE}  convert_other_at: true\n");
    let (inbound, vids, _dir) = at_inbound(&settings, &[OTHER_OPENID]);
    let events = inbound
        .convert(
            "C2C_MESSAGE_CREATE",
            Some(&json!({
                "id": "M2",
                "content": format!("<@{OTHER_OPENID}> hi"),
                "timestamp": "1700000000",
                "author": {"id": SENDER_OPENID}
            })),
            None,
        )
        .expect("转换失败");
    let Event::Message(message) = &events[0] else {
        panic!("应为 message 事件，实得 {events:?}");
    };
    assert_eq!(message.raw_message, format!("[CQ:at,qq={}] hi", vids[0]));
}

