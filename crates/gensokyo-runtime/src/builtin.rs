//! M1h-a 内置 action：`get_status` / `get_version_info` / `get_login_info`。
//!
//! 字段与 Go `handlers/get_status.go`、`handlers/get_version_info.go`、
//! `handlers/get_login_info.go` 对齐。
//!
//! 与 Go 的差异：
//! - `get_status.stat` 的 `message_received` / `message_sent` / `last_message_time` 在 Go
//!   读 `botstats`；M1h-a 还没有统计源，统一返回 0（不伪造数据），统计接线留到 runtime
//!   任务树落地时补。
//! - `get_version_info.runtime_version` 留空：Go 用 `runtime.Version()`，Rust 没有等价的
//!   运行时 API，后续由 buildinfo 注入。

use std::sync::Arc;

use gensokyo_config::ConfigManager;
use gensokyo_protocol::action::{ActionRequest, ActionResponse};
use gensokyo_protocol::registry::ready;
use gensokyo_protocol::{Handler, HandlerFuture, Registry};
use serde_json::{Value, json};

/// `get_status`（Go `handlers/get_status.go`）。
#[derive(Debug, Default)]
pub struct GetStatus;

impl Handler for GetStatus {
    fn name(&self) -> &'static str {
        "get_status"
    }

    fn handle<'a>(&'a self, _request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
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
                "message_received": 0,
                "message_sent": 0,
                "disconnect_times": 0,
                "lost_times": 0,
                "last_message_time": 0,
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
pub fn register_all(registry: &mut Registry, config: Arc<ConfigManager>) {
    let handlers: [(&'static str, Arc<dyn Handler>); 3] = [
        ("get_status", Arc::new(GetStatus)),
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

    use super::{GetLoginInfo, GetStatus, GetVersionInfo, register_all};

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
        let response = GetStatus
            .handle(&request("get_status"))
            .await
            .expect("get_status 应成功");
        assert_eq!(response.retcode, 0);
        assert_eq!(response.data.as_ref().expect("data")["online"], true);
        assert_eq!(response.data.as_ref().expect("data")["good"], true);
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
        register_all(&mut registry, config);
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
