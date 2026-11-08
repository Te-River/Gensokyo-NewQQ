//! M1h-a 内置 action：`get_status` / `get_version_info` / `get_login_info`。
//!
//! 字段与 Go `handlers/get_status.go`、`handlers/get_version_info.go`、
//! `handlers/get_login_info.go` 对齐。
//!
//! `get_status.stat` 的 `message_received` / `message_sent` / `last_message_time` 读 botstats
//! （Go `handlers/get_status.go:49` 的 `botstats.GetStats()`）；统计库未装配或读取失败时三项
//! 按 0 上报，其余字段照旧（Go 在错误分支同样保留零值并打日志）。
//!
//! 与 Go 的差异：
//! - `get_version_info.runtime_version` 留空：Go 用 `runtime.Version()`，Rust 没有等价的
//!   运行时 API，后续由 buildinfo 注入。

use std::sync::Arc;

use gensokyo_config::ConfigManager;
use gensokyo_protocol::action::{ActionRequest, ActionResponse};
use gensokyo_protocol::registry::ready;
use gensokyo_protocol::{Handler, HandlerFuture, Registry};
use gensokyo_storage::BotStats;
use serde_json::{Value, json};

use crate::state::BotStatsSlot;

/// `get_status`（Go `handlers/get_status.go`）。
#[derive(Debug, Default)]
pub struct GetStatus {
    botstats: BotStatsSlot,
}

impl GetStatus {
    /// 绑定 botstats 单元；统计在请求时读取（Go 同样在请求时读包级 `db`）。
    #[must_use]
    pub fn new(botstats: BotStatsSlot) -> Self {
        Self { botstats }
    }

    /// 当日统计；未装配或读取失败时 `None`（字段按 0 上报）。
    fn stats(&self) -> Option<BotStats> {
        let store = self.botstats.get()?;
        match store.get_stats() {
            Ok(stats) => Some(stats),
            Err(err) => {
                tracing::warn!(error = %err, "get_status 读取 botstats 失败，统计按 0 上报");
                None
            }
        }
    }
}

impl Handler for GetStatus {
    fn name(&self) -> &'static str {
        "get_status"
    }

    fn handle<'a>(&'a self, _request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        let stats = self.stats();
        ready(Ok(ActionResponse::ok(json!({
            "app_initialized": true,
            "app_enabled": true,
            "plugins_good": true,
            "app_good": true,
            "online": true,
            "good": true,
            "stat": {
                "packet_received": 0,
                "packet_sent": 0,
                "packet_lost": 0,
                "message_received": stats.map_or(0, |stats| stats.message_received),
                "message_sent": stats.map_or(0, |stats| stats.message_sent),
                "disconnect_times": 0,
                "lost_times": 0,
                "last_message_time": stats.map_or(0, |stats| stats.last_message_time),
            },
        }))))
    }
}

/// `get_version_info`（Go `handlers/get_version_info.go`）。
#[derive(Debug, Default)]
pub struct GetVersionInfo;

impl Handler for GetVersionInfo {
    fn name(&self) -> &'static str {
        "get_version_info"
    }

    fn handle<'a>(&'a self, _request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        let version = env!("CARGO_PKG_VERSION");
        ready(Ok(ActionResponse::ok(json!({
            "app_full_name": "gensokyo",
            "app_name": "gensokyo",
            "app_version": version,
            "coolq_directory": "",
            "coolq_edition": "pro",
            "go-cqhttp": true,
            "plugin_build_configuration": version,
            "plugin_build_number": 0,
            "plugin_version": version,
            "protocol_name": 4,
            "protocol_version": "v11",
            "runtime_os": std::env::consts::OS,
            "runtime_version": "",
            "version": version,
        }))))
    }
}

/// `get_login_info`（Go `handlers/get_login_info.go`）：`use_uin` 为真取 `uin`，否则取
/// `app_id`；昵称取 `custom_bot_name`。
#[derive(Debug)]
pub struct GetLoginInfo {
    config: Arc<ConfigManager>,
}

impl GetLoginInfo {
    /// 绑定配置快照管理器（每次请求现读快照，热重载即时生效）。
    #[must_use]
    pub fn new(config: Arc<ConfigManager>) -> Self {
        Self { config }
    }
}

impl Handler for GetLoginInfo {
    fn name(&self) -> &'static str {
        "get_login_info"
    }

    fn handle<'a>(&'a self, _request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        let settings = self.config.snapshot();
        #[allow(clippy::cast_possible_wrap)] // 与 Go `int64(globalBotID)` 的截断口径一致
        let user_id = if settings.get_use_uin() {
            settings.get_uin()
        } else {
            settings.get_app_id() as i64
        };
        ready(Ok(ActionResponse::ok(json!({
            "user_id": user_id,
            "nickname": settings.get_custom_bot_name(),
        }))))
    }
}

/// 把三个内置 action 注册进 `registry`；重复注册会打 `warn` 并以本次实现覆盖。
///
/// `botstats` 是 `get_status` 在**请求时**读的统计单元，注册期可以是空的
/// （[`BotStatsSlot`]）。生产路径由 [`AppState`](crate::AppState) 把自己的那一份交给这里，
/// 之后 [`AppState::with_botstats`](crate::AppState::with_botstats) 填进去即可；
/// 手工构造注册表（`AppState::new`）的调用方拿不到状态自己的单元，传
/// `BotStatsSlot::default()` 时该 `get_status` 恒报 0——这是给测试的降级路径。
pub fn register_all(registry: &mut Registry, config: Arc<ConfigManager>, botstats: BotStatsSlot) {
    let handlers: [(&'static str, Arc<dyn Handler>); 3] = [
        ("get_status", Arc::new(GetStatus::new(botstats))),
        ("get_version_info", Arc::new(GetVersionInfo)),
        ("get_login_info", Arc::new(GetLoginInfo::new(config))),
    ];
    for (action, handler) in handlers {
        if registry.register(action, handler).is_some() {
            tracing::warn!(action, "内置 action 被重复注册，已用内置实现覆盖");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gensokyo_config::ConfigManager;
    use gensokyo_protocol::Registry;
    use gensokyo_protocol::action::ActionRequest;
    use gensokyo_protocol::registry::Handler;
    use serde_json::{Value, json};
    use tempfile::TempDir;

    use super::{BotStatsSlot, GetLoginInfo, GetStatus, GetVersionInfo, register_all};

    fn manager(settings: &str) -> (TempDir, Arc<ConfigManager>) {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        // 统一缩进到 `settings:` 下，避免每个夹具都手写两格缩进
        let body = settings
            .lines()
            .map(|line| format!("  {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, format!("version: 1\nsettings:\n{body}\n")).expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        (dir, manager)
    }

    fn request(action: &str) -> ActionRequest<Value> {
        ActionRequest {
            action: action.to_owned(),
            params: None,
            echo: None,
        }
    }

    #[tokio::test]
    async fn get_status_reports_online_and_good() {
        let response = GetStatus::default()
            .handle(&request("get_status"))
            .await
            .expect("get_status 应成功");
        assert_eq!(response.retcode, 0);
        assert_eq!(response.data.as_ref().expect("data")["online"], true);
        assert_eq!(response.data.as_ref().expect("data")["good"], true);
    }

    /// 统计单元为空（未装配 botstats）时统计字段按 0 上报，不 panic。
    #[tokio::test]
    async fn get_status_without_botstats_reports_zero_stat() {
        let response = GetStatus::default()
            .handle(&request("get_status"))
            .await
            .expect("get_status 应成功");
        let data = response.data.expect("data");
        assert_eq!(data["stat"]["message_received"], 0);
        assert_eq!(data["stat"]["message_sent"], 0);
        assert_eq!(data["stat"]["last_message_time"], 0);
    }

    /// 装配 botstats 后，统计三项按 Go `GetStats` 的口径上报（其余统计字段保持 0）。
    #[tokio::test]
    async fn get_status_reads_botstats_at_request_time() {
        let dir = tempfile::tempdir().expect("临时目录");
        let store = Arc::new(
            gensokyo_storage::BotStatsStore::open(dir.path().join("bot-stats.redb"))
                .expect("打开 botstats"),
        );
        store.record_received().expect("记收");
        store.record_sent().expect("记发");
        let slot = BotStatsSlot::default();
        // 注册早于注入：注册期单元是空的，注入后请求即可读到（Go 包级 `db` 的同一顺序）。
        let handler = GetStatus::new(Arc::clone(&slot));
        let _ = slot.set(Arc::clone(&store));

        let response = handler
            .handle(&request("get_status"))
            .await
            .expect("应成功");
        let stats = &response.data.expect("data")["stat"];
        assert_eq!(stats["message_received"], 1);
        assert_eq!(stats["message_sent"], 1);
        assert!(
            stats["last_message_time"].as_i64().unwrap_or(0) > 0,
            "写入后应有最后消息时刻"
        );
        assert_eq!(stats["packet_sent"], 0, "官方 API 无包统计，保持 0");
    }

    #[tokio::test]
    async fn get_version_info_matches_go_shape() {
        let response = GetVersionInfo
            .handle(&request("get_version_info"))
            .await
            .expect("get_version_info 应成功");
        let data = response.data.expect("data");
        assert_eq!(data["app_name"], "gensokyo");
        assert_eq!(data["app_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(data["protocol_version"], "v11");
        assert_eq!(data["protocol_name"], 4);
    }

    #[tokio::test]
    async fn get_login_info_prefers_uin_when_use_uin() {
        let (_dir, config) =
            manager("use_uin: true\nuin: 10001\napp_id: 12345\ncustom_bot_name: Bot\n");
        let response = GetLoginInfo::new(config)
            .handle(&request("get_login_info"))
            .await
            .expect("get_login_info 应成功");
        assert_eq!(response.data.as_ref().expect("data")["user_id"], 10001);
        assert_eq!(response.data.as_ref().expect("data")["nickname"], "Bot");
    }

    #[tokio::test]
    async fn get_login_info_falls_back_to_app_id() {
        let (_dir, config) = manager("use_uin: false\nuin: 10001\napp_id: 12345\n");
        let response = GetLoginInfo::new(config)
            .handle(&request("get_login_info"))
            .await
            .expect("get_login_info 应成功");
        assert_eq!(response.data.as_ref().expect("data")["user_id"], 12345);
    }

    #[tokio::test]
    async fn register_all_installs_three_actions() {
        let (_dir, config) = manager("app_id: 12345\n");
        let mut registry = Registry::new();
        register_all(&mut registry, config, BotStatsSlot::default());
        assert_eq!(
            registry.actions(),
            vec!["get_login_info", "get_status", "get_version_info"]
        );

        let response = registry
            .get("get_status")
            .expect("应已注册")
            .handle(&ActionRequest {
                action: "get_status".to_owned(),
                params: Some(json!({})),
                echo: None,
            })
            .await
            .expect("应成功");
        assert_eq!(response.status, gensokyo_protocol::action::Status::Ok);
    }
}
