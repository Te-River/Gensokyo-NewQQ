//! botstats 生产装配、入站打点与 `get_status` 读取（P12b）。
//!
//! 覆盖 P12 交接的三项待办：
//! - a) 群 AT / 群全量 / C2C 各一条 → [`metrics::MESSAGES_RECEIVED`] 递增 + 当日接收数落库；
//! - b) 重开统计库（新 `BotStatsStore`）后计数仍在；
//! - d) `get_status.stat` 的 `message_received` / `message_sent` / `last_message_time` 读 botstats；
//! - e) 未装配 botstats 时零 panic：入站只计内存指标，`get_status` 统计字段报 0。
//!
//! 统计库全部落在各自的临时目录：`Runtime::start*` 的 `attach_botstats` 只在状态没有句柄时才按
//! 默认路径（cwd）建库，`cargo test` 因此不会在 crate 目录留下 `bot-stats.redb`。

mod support;

use std::sync::Arc;

use gensokyo_core::telemetry::metrics;
use gensokyo_protocol::action::ActionRequest;
use gensokyo_runtime::{AppState, Inbound};
use gensokyo_storage::{BotStatsStore, DEFAULT_BOTSTATS_PATH};
use serde_json::{Value, json};
use tempfile::TempDir;

/// 带临时 idmap / botstats 的运行时状态（`Runtime::start*` 的 `attach_*` 会原样保留）。
fn state_with_stores(settings: &str) -> (AppState, TempDir) {
    let (manager, dir) = support::config(settings);
    let state = support::with_temp_stores(AppState::with_builtin_handlers(manager), &dir);
    (state, dir)
}

fn group_at_payload() -> Value {
    json!({
        "id": "m-at",
        "group_id": "g0123456789abcdef0123456789abcdef",
        "content": "hello",
        "author": {"id": "u0123456789abcdef0123456789abcdef"}
    })
}

fn group_full_payload() -> Value {
    json!({
        "id": "m-full",
        "group_id": "g0123456789abcdef0123456789abcdef",
        "content": "everyone",
        "author": {"id": "u0123456789abcdef0123456789abcdef"}
    })
}

fn c2c_payload() -> Value {
    json!({
        "id": "m-c2c",
        "content": "hi",
        "author": {"id": "u0123456789abcdef0123456789abcdef"}
    })
}

fn get_status_request() -> ActionRequest<Value> {
    ActionRequest {
        action: "get_status".to_owned(),
        params: None,
        echo: None,
    }
}

/// a) 三个消息事件各记一次：内存指标递增，当日接收数落库，发出计数不动。
#[test]
fn inbound_message_events_count_once_each() {
    let (state, _dir) = state_with_stores("  app_id: 102000001\n");
    let inbound = Inbound::from_state(&state);
    let before = metrics::MESSAGES_RECEIVED.get();

    for (kind, payload) in [
        ("GROUP_AT_MESSAGE_CREATE", group_at_payload()),
        ("GROUP_MESSAGE_CREATE", group_full_payload()),
        ("C2C_MESSAGE_CREATE", c2c_payload()),
    ] {
        let events = inbound
            .convert(kind, Some(&payload), None)
            .unwrap_or_else(|err| panic!("{kind} 应能转换: {err}"));
        assert_eq!(events.len(), 1, "{kind} 应产出一条事件");
    }

    // 全局计数器被同进程的用例并发递增，只能断言下界。
    assert!(
        metrics::MESSAGES_RECEIVED.get() >= before + 3,
        "三个消息事件应各计一次: {} -> {}",
        before,
        metrics::MESSAGES_RECEIVED.get()
    );

    let stats = state
        .botstats()
        .expect("应已装配 botstats")
        .get_stats()
        .expect("读统计");
    assert_eq!(stats.message_received, 3, "当日接收数应落库");
    assert!(stats.last_message_time > 0, "写入后应记下最后消息时刻");
    assert_eq!(stats.message_sent, 0, "入站不改发出计数");
}

/// b) 重开统计库后计数仍在（redb 持久化，Go bbolt 同语义）。
#[test]
fn counters_survive_reopen() {
    let (manager, _config_dir) = support::config("  app_id: 102000001\n");
    let dir = tempfile::tempdir().expect("临时目录");
    let path = dir.path().join(DEFAULT_BOTSTATS_PATH);

    {
        let store = Arc::new(BotStatsStore::open(&path).expect("打开统计库失败"));
        let state = AppState::with_builtin_handlers(manager).with_botstats(Arc::clone(&store));
        Inbound::from_state(&state)
            .convert("C2C_MESSAGE_CREATE", Some(&c2c_payload()), None)
            .expect("C2C 应能转换");
    }

    let reopened = BotStatsStore::open(&path).expect("重开统计库失败");
    let stats = reopened.get_stats().expect("读统计");
    assert_eq!(stats.message_received, 1, "重开后计数应仍在");
    assert!(stats.last_message_time > 0);
}

/// d) `get_status.stat` 三项读 botstats，其余统计字段与 Go 同形状（恒 0）。
#[tokio::test]
async fn get_status_reports_botstats_fields() {
    let (state, _dir) = state_with_stores("  app_id: 102000001\n");
    let store = state.botstats().expect("应已装配 botstats");
    store.record_received().expect("记收");
    store.record_received().expect("记收");
    store.record_sent().expect("记发");

    let response = state.dispatcher().dispatch(&get_status_request()).await;
    assert_eq!(response.retcode, 0);
    let data = response.data.expect("data");
    let stat = &data["stat"];
    assert_eq!(stat["message_received"], 2);
    assert_eq!(stat["message_sent"], 1);
    assert!(
        stat["last_message_time"].as_i64().unwrap_or(0) > 0,
        "写入后应有最后消息时刻"
    );
    for field in [
        "packet_received",
        "packet_sent",
        "packet_lost",
        "disconnect_times",
        "lost_times",
    ] {
        assert_eq!(stat[field], 0, "Go 对 {field} 恒报 0（官方 API 无此统计）");
    }
}

/// e) 未装配 botstats：入站只计内存指标、`get_status` 统计报 0，全程不 panic。
#[tokio::test]
async fn missing_botstats_degrades_without_panic() {
    let (manager, _dir) = support::config("  app_id: 102000001\n");
    let state = AppState::with_builtin_handlers(manager);
    assert!(state.botstats().is_none(), "本用例刻意不装配 botstats");

    let before = metrics::MESSAGES_RECEIVED.get();
    Inbound::from_state(&state)
        .convert("GROUP_AT_MESSAGE_CREATE", Some(&group_at_payload()), None)
        .expect("群 AT 应能转换");
    assert!(
        metrics::MESSAGES_RECEIVED.get() > before,
        "统计库缺失不影响内存指标"
    );

    let response = state.dispatcher().dispatch(&get_status_request()).await;
    assert_eq!(response.retcode, 0);
    let data = response.data.expect("data");
    assert_eq!(data["stat"]["message_received"], 0);
    assert_eq!(data["stat"]["message_sent"], 0);
    assert_eq!(data["stat"]["last_message_time"], 0);
}

/// `disable_error_chan=true` 时 Go 不记账（`main.go:762/784/1134` 的 `if !GetDisableErrorChan()`），
/// 这里照搬：只影响统计，不影响事件上报。
#[test]
fn disable_error_chan_skips_counting_like_go() {
    let (state, _dir) = state_with_stores("  app_id: 102000001\n  disable_error_chan: true\n");
    let events = Inbound::from_state(&state)
        .convert("C2C_MESSAGE_CREATE", Some(&c2c_payload()), None)
        .expect("C2C 应能转换");
    assert_eq!(events.len(), 1, "上报不受统计开关影响");

    let stats = state
        .botstats()
        .expect("应已装配")
        .get_stats()
        .expect("读统计");
    assert_eq!(stats.message_received, 0);
    assert_eq!(stats.last_message_time, 0);
}
