//! Rust 入站转换的 Go 金样回放（任务 #58b）。
//!
//! 期望值来自同仓 Go 侧的 `Processor/testdata/inbound/`（#58a 入库，本测试**只读消费**）：
//! 每个 `<EVENT>.json` 是一组 OneBot V11 上报 payload，`manifest.json` 给出每类的预期条数。
//! 回放链路与真实入站一致：`d` 负载 → `{"id","op","s","t","d"}` 信封 →
//! [`Inbound::convert_json`] → 事件序列化 → 归一化 `time` → 与金样逐元素相等。
//!
//! # 输入信封为什么在 Rust 侧
//!
//! 金样文件存的是**产出**（OneBot payload），QQ 入站 `d` 只以固定字面量存在于生成器
//! `Processor/golden_inbound_gen_test.go` 的 `goldenCases()[].envelope`。notice 的数字 ID 是
//! MD5 派生的 vUIN（不可逆），无法从产出反推输入，因此这里按生成器逐字镜像一份
//! [`CASES`]。改 Go 侧入站输出后必须重新生成金样，本测试才会跟着变红/变绿：
//!
//! ```text
//! go test -tags goldengen ./Processor -run TestGoldenGen -count=1
//! ```
//!
//! # 新增事件类型
//!
//! 生成器 `goldenCases()` 加一条 → 重跑上面的命令 → manifest 里多一项。此时本测试会报
//! 「harness 缺少输入信封」，在 [`CASES`] 补一条同样的 `envelope` 字面量即可（无需改断言）。
//!
//! # `time` 归一化
//!
//! Go 侧构造时用 `time.Now()`，不可复现：两侧都把**任意层级**键名为 `time` 的字段置 0。
//! 嵌套的 `data.timestamp` 是输入原文（不是构造时间），不参与归一化。

use std::sync::Arc;

use gensokyo_config::ConfigManager;
use gensokyo_runtime::Inbound;
use gensokyo_storage::{IdMapOptions, IdMapStore};
use serde_json::{Value, json};

/// 每个事件类型的 QQ 入站 `d` 原文，与 `Processor/golden_inbound_gen_test.go` 的
/// `goldenCases()[].envelope` 逐字一致。
const CASES: &[(&str, &str)] = &[
    (
        "GROUP_AT_MESSAGE_CREATE",
        r#"{"id":"MSG_AT_1","content":"hi","group_id":"GROUP_OPENID_1","author":{"id":"USER_OPENID_1","username":"tester","member_role":"member"},"timestamp":"2024-01-01T00:00:00+08:00","message_scene":{"ext":["msg_idx=REFIDX_1"]}}"#,
    ),
    (
        "GROUP_MESSAGE_CREATE",
        r#"{"id":"MSG_FULL_1","content":"hello group","group_id":"GROUP_OPENID_1","author":{"id":"USER_OPENID_1","username":"tester","member_role":"member"},"timestamp":"2024-01-01T00:00:00+08:00"}"#,
    ),
    (
        "C2C_MESSAGE_CREATE",
        r#"{"id":"MSG_C2C_1","content":"hello","author":{"id":"USER_OPENID_1","username":"tester"},"timestamp":"2024-01-01T00:00:00+08:00"}"#,
    ),
    (
        "GROUP_MEMBER_ADD",
        r#"{"id":"EVENT_MEMBER_ADD_1","group_openid":"GROUP_OPENID_1","member_openid":"USER_OPENID_2","timestamp":1704067200}"#,
    ),
    (
        "GROUP_MEMBER_REMOVE",
        r#"{"id":"EVENT_MEMBER_REMOVE_1","group_openid":"GROUP_OPENID_1","member_openid":"USER_OPENID_2","timestamp":1704067200}"#,
    ),
    (
        "GROUP_JOIN_REQUEST",
        r#"{"id":"EVENT_JOIN_1","group_openid":"GROUP_OPENID_1","join_request_id":"JOIN_REQUEST_1","member_openid":"USER_OPENID_3","username":"applicant","apply_at":"2024-01-01T00:00:00+08:00","verify_info":{"method":"admin_confirm","verify_message":"let me in"}}"#,
    ),
    (
        "INTERACTION_CREATE",
        r#"{"id":"INTERACTION_1","event_id":"EVENT_INTERACTION_1","type":11,"scene":"group","chat_type":1,"timestamp":"2024-01-01T00:00:00+08:00","group_openid":"GROUP_OPENID_1","group_member_openid":"USER_OPENID_1","data":{"name":"button","type":1,"resolved":{"button_data":"cmd_1","button_id":"BTN_1","user_id":"USER_OPENID_1","feature_id":"FEATURE_1","message_id":"MSG_AT_1"}}}"#,
    ),
    (
        "FRIEND_ADD",
        r#"{"event_id":"EVENT_FRIEND_ADD_1","openid":"USER_OPENID_4","timestamp":1704067200,"scene":1,"scene_param":"callback_data_1"}"#,
    ),
    (
        "FRIEND_DEL",
        r#"{"event_id":"EVENT_FRIEND_DEL_1","openid":"USER_OPENID_4","timestamp":1704067200}"#,
    ),
    (
        "GROUP_ADD_ROBOT",
        r#"{"id":"EVENT_ADD_ROBOT_1","event_id":"EVENT_ADD_ROBOT_1","group_openid":"GROUP_OPENID_5","op_member_openid":"USER_OPENID_5","timestamp":1704067200}"#,
    ),
    (
        "GROUP_DEL_ROBOT",
        r#"{"id":"EVENT_DEL_ROBOT_1","event_id":"EVENT_DEL_ROBOT_1","group_openid":"GROUP_OPENID_5","op_member_openid":"USER_OPENID_5","timestamp":1704067200}"#,
    ),
    (
        "GROUP_MSG_REJECT",
        r#"{"event_id":"EVENT_MSG_REJECT_1","group_openid":"GROUP_OPENID_5","op_member_openid":"USER_OPENID_5","timestamp":1704067200}"#,
    ),
    (
        "GROUP_MSG_RECEIVE",
        r#"{"event_id":"EVENT_MSG_RECEIVE_1","group_openid":"GROUP_OPENID_5","op_member_openid":"USER_OPENID_5","timestamp":1704067200}"#,
    ),
    (
        "C2C_MSG_REJECT",
        r#"{"event_id":"EVENT_C2C_REJECT_1","openid":"USER_OPENID_6","timestamp":1704067200}"#,
    ),
    (
        "C2C_MSG_RECEIVE",
        r#"{"event_id":"EVENT_C2C_RECEIVE_1","openid":"USER_OPENID_6","timestamp":1704067200}"#,
    ),
];

/// 金样目录：同仓 Go 侧 `Processor/testdata/inbound/`。
fn golden_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Processor/testdata/inbound")
}

/// 按 manifest 的配置矩阵构造入站转换器：`string_ob11` / `native_ob11` 打开，
/// `app_id = 12345`，idmap 用 `hash_id` 与 Go 的 `hash_id: true` 对齐（vUIN 由 MD5 派生）。
fn inbound(dir: &std::path::Path) -> Inbound {
    let path = dir.join("config.yml");
    std::fs::write(
        &path,
        "version: 1\nsettings:\n  app_id: 12345\n  string_ob11: true\n  native_ob11: true\n",
    )
    .expect("写测试配置失败");
    let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
    let store = Arc::new(
        IdMapStore::open(
            IdMapOptions::new(dir.join("openid-map.redb"), dir.join("msgid-map.redb"))
                .with_app_id("12345")
                .with_hash_id(true),
        )
        .expect("打开 idmap 存储失败"),
    );
    Inbound::new(manager, Some(store))
}

/// 递归把任意层级键名为 `time` 的字段置 0（Go 侧构造时间不可复现）。
fn zero_time(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if key == "time" {
                    *child = Value::from(0);
                } else {
                    zero_time(child);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                zero_time(item);
            }
        }
        _ => {}
    }
}

/// 回放：输入信封 → 事件序列 → 归一化 → 与金样逐元素比对。
///
/// 一次跑完 15 类事件，把全部差异收集后一起报出（失败信息带事件名、下标与两侧 pretty
/// JSON），避免「修一条、跑一遍」的串行排查。
#[test]
fn golden_inbound_matches_go() {
    let dir = tempfile::tempdir().expect("临时目录");
    let inbound = inbound(dir.path());
    let golden = golden_dir();

    let manifest: Value = serde_json::from_slice(
        &std::fs::read(golden.join("manifest.json")).expect("读 manifest 失败"),
    )
    .expect("解析 manifest 失败");
    let events = manifest["events"]
        .as_array()
        .expect("manifest.events 应为数组");

    let mut failures: Vec<String> = Vec::new();
    for entry in events {
        let event = entry["event"].as_str().expect("manifest.events[].event");
        let expected_len = usize::try_from(
            entry["payloads"]
                .as_u64()
                .expect("manifest.events[].payloads"),
        )
        .expect("payloads 应能转 usize");

        let Some((_, input)) = CASES.iter().find(|(name, _)| *name == event) else {
            failures.push(format!("{event}: harness 的 CASES 缺少该事件的输入信封"));
            continue;
        };
        let input: Value = serde_json::from_str(input).expect("CASES 里的输入信封应是合法 JSON");
        // Go 的 handler 把 WS payload 的 id 写进 data.EventID；这里同样按 `d.id` 填信封 id。
        let envelope = json!({
            "id": input.get("id").cloned().unwrap_or(Value::Null),
            "op": 0,
            "s": 1,
            "t": event,
            "d": input,
        });
        let raw = serde_json::to_vec(&envelope).expect("信封序列化失败");
        let produced = inbound
            .convert_json(&raw)
            .unwrap_or_else(|err| panic!("{event} 转换失败: {err}"));

        let mut actual: Vec<Value> = produced
            .iter()
            .map(|item| serde_json::to_value(item).expect("事件序列化失败"))
            .collect();
        for item in &mut actual {
            zero_time(item);
        }

        let mut expected: Value = serde_json::from_slice(
            &std::fs::read(golden.join(format!("{event}.json")))
                .unwrap_or_else(|err| panic!("读 {event}.json 失败: {err}")),
        )
        .expect("解析金样失败");
        zero_time(&mut expected);
        let expected = expected.as_array().cloned().unwrap_or_default();

        if actual.len() != expected_len {
            failures.push(format!(
                "{event}: 条数不符，金样 {expected_len} 条，Rust 产出 {} 条\n实际: {}",
                actual.len(),
                serde_json::to_string_pretty(&actual).unwrap_or_default()
            ));
            continue;
        }
        for (index, (want, got)) in expected.iter().zip(actual.iter()).enumerate() {
            if want != got {
                failures.push(format!(
                    "{event}: 第 {index} 条不一致\n金样: {}\nRust: {}",
                    serde_json::to_string_pretty(want).unwrap_or_default(),
                    serde_json::to_string_pretty(got).unwrap_or_default()
                ));
            }
        }
        println!("ok {event} ({} 条)", expected.len());
    }

    assert!(
        failures.is_empty(),
        "入站事件与 Go 金样不一致（{} 类）:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
