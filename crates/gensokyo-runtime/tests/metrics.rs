//! `/metrics` 端点（Go `main.go:1141` `MetricsHandler`）的集成测试。
//!
//! 覆盖：a) 200 + Prometheus 文本含全部指标名；d) 未装配 botstats 时的降级。
//! 计数器递增（b）与 botstats 往返（c）各自在 `gensokyo-core` / `gensokyo-storage` 的
//! 单元测试里，不需要起服务。

mod support;

use std::net::SocketAddr;

use gensokyo_runtime::{
    AppState, DEFAULT_SHUTDOWN_TIMEOUT, Runtime, RuntimeHandle, RuntimeOptions,
};
use tempfile::TempDir;

/// 只开 HTTP API（`127.0.0.1:0` 取随机端口），其余 listener 全关。
const METRICS_SETTINGS: &str = "  http_address: \"127.0.0.1:0\"\n";

fn options() -> RuntimeOptions {
    RuntimeOptions {
        config_watch: false,
        gateway: false,
        post_url: false,
        ..RuntimeOptions::default()
    }
}

/// 起一个只带内置 action 的运行时；不注入 botstats（降级路径即默认路径）。
async fn start() -> (RuntimeHandle, SocketAddr, TempDir) {
    let (manager, dir) = support::config(METRICS_SETTINGS);
    let state = support::with_temp_idmap(AppState::with_builtin_handlers(manager), &dir);
    assert!(state.botstats().is_none(), "本用例刻意不装配 botstats");
    let handle = Runtime::start_with(state, options())
        .await
        .expect("启动失败");
    let addr = handle.http_addr().expect("HTTP 应已监听");
    (handle, addr, dir)
}

#[tokio::test]
async fn metrics_endpoint_serves_prometheus_text() {
    let (handle, addr, _dir) = start().await;

    let (status, response) = support::get(addr, "/metrics").await;
    assert_eq!(status, 200, "GET /metrics 应返回 200: {response}");
    assert!(
        response
            .to_ascii_lowercase()
            .contains("content-type: text/plain; version=0.0.4"),
        "Content-Type 应与 Go 一致: {response}"
    );

    let body = support::response_body(&response);
    for name in [
        "gensokyo_uptime_seconds",
        "gensokyo_messages_received_total",
        "gensokyo_messages_sent_total",
        "gensokyo_errors_total",
        "gensokyo_slow_events_total",
        "gensokyo_memory_allocated_bytes",
    ] {
        assert!(body.contains(&format!("# HELP {name} ")), "{name} 缺 HELP");
        assert!(body.contains(&format!("# TYPE {name} ")), "{name} 缺 TYPE");
    }

    // 样本行：uptime 是可解析的浮点，收发计数是可解析的整数。
    let sample = |name: &str| -> String {
        body.lines()
            .find_map(|line| {
                line.strip_prefix(name)?
                    .strip_prefix(' ')
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| panic!("{name} 缺样本行\n{body}"))
    };
    let uptime: f64 = sample("gensokyo_uptime_seconds")
        .parse()
        .expect("uptime 应是浮点");
    assert!(uptime >= 0.0, "uptime 不应为负: {uptime}");
    for name in [
        "gensokyo_messages_received_total",
        "gensokyo_messages_sent_total",
        "gensokyo_errors_total",
        "gensokyo_slow_events_total",
    ] {
        let value: u64 = sample(name)
            .parse()
            .unwrap_or_else(|err| panic!("{name}: {err}"));
        assert!(value < u64::MAX, "{name} 应是整数计数");
    }

    handle.shutdown_with_timeout(DEFAULT_SHUTDOWN_TIMEOUT).await;
}

/// 统计库未装配（生产装配前 / 打不开）时端点照旧可用，只是不落盘。
#[tokio::test]
async fn metrics_endpoint_degrades_without_botstats() {
    let (handle, addr, _dir) = start().await;

    let (status, response) = support::get(addr, "/metrics").await;
    assert_eq!(status, 200, "未装配 botstats 不应影响端点: {response}");
    let body = support::response_body(&response);
    assert!(
        body.contains("gensokyo_messages_received_total "),
        "内存计数不受统计库缺失影响\n{body}"
    );

    handle.shutdown_with_timeout(DEFAULT_SHUTDOWN_TIMEOUT).await;
}

/// 装配 botstats 后端点照旧，且状态里能拿到句柄（生产装配走 `AppState::with_botstats`；
/// `Runtime::attach_botstats` 尚未接线，见 HANDOFF）。
#[tokio::test]
async fn metrics_endpoint_survives_attached_botstats() {
    let (manager, dir) = support::config(METRICS_SETTINGS);
    let store = gensokyo_storage::BotStatsStore::open(dir.path().join("bot-stats.redb"))
        .expect("打开临时 botstats");
    let state = support::with_temp_idmap(AppState::with_builtin_handlers(manager), &dir)
        .with_botstats(std::sync::Arc::new(store));
    assert!(state.botstats().is_some(), "注入后应有句柄");
    let handle = Runtime::start_with(state, options())
        .await
        .expect("启动失败");
    let addr = handle.http_addr().expect("HTTP 应已监听");

    let (status, response) = support::get(addr, "/metrics").await;
    assert_eq!(status, 200, "装配 botstats 不影响端点: {response}");
    assert!(
        support::response_body(&response).contains("gensokyo_messages_received_total "),
        "样本行仍在"
    );

    handle.shutdown_with_timeout(DEFAULT_SHUTDOWN_TIMEOUT).await;
}
