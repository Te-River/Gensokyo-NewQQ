//! 逐 action 交叉表：Go `ParamsContent` 强类型整包解码（活体 B01/B02/F02/H04）。
//!
//! Go 在收包层 `json.Unmarshal` 就把坏类型的 params 挡在分发层（1400「参数解析失败」，
//! `wsclient/ws.go:253-258` → `callapi.ReplyUnparseableMessage`），请求进不了 handler；
//! Rust 的同构校验在 `gensokyo_protocol::strict_params::check_params`（runtime 的
//! `Dispatcher::route` 调用 handler 之前执行）。本文件钉两件事：
//! 1. **契约表单测**——每个 action 的已声明字段传错类型都给出 Go 同款定位文案；
//! 2. **无上游副作用证明**——用记录一切调用的 `RecordingApi` 走真实 handler：校验通过
//!    时上游确实被调用（对照组），而 handler 层的既有拒绝（如 `approve:"yes"`）零调用。
//!    wire 级「1400 + 零上游」的整串断言在 runtime 的 `tests/deferred.rs`（同 mock 口径）。

use std::sync::{Arc, Mutex};

use gensokyo_actions::{ActionContext, QqApi, QqFuture, register_all};
use gensokyo_config::ConfigManager;
use gensokyo_core::Error;
use gensokyo_protocol::Registry;
use gensokyo_protocol::action::{ActionRequest, Status};
use gensokyo_protocol::strict_params::check_params;
use gensokyo_qq::dto::{
    ApprovalJoinRequest, BotInGroupState, GroupInfo, GroupMember, GroupMemberList, JoinRequestList,
    MessageResponse, MessageToCreate,
};
use serde_json::{Value, json};

const GROUP: &str = "0123456789abcdef0123456789abcdef";
const MEMBER: &str = "fedcba9876543210fedcba9876543210";

/// 记录每一次上游方法名（其余端点走 trait 默认实现报 Unsupported，不会静默成功）。
#[derive(Debug, Default)]
struct RecordingApi {
    calls: Mutex<Vec<String>>,
}

impl RecordingApi {
    fn record(&self, method: &str) {
        self.calls.lock().unwrap().push(method.to_owned());
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

fn not_provided<T>(method: &str) -> Result<T, Error> {
    Err(Error::Unsupported(format!("未配置 {method}")))
}

impl QqApi for RecordingApi {
    fn get_group_info(&self, _group_openid: &str) -> QqFuture<'_, GroupInfo> {
        Box::pin(async { not_provided("get_group_info") })
    }

    fn get_group_member_info(
        &self,
        _group_openid: &str,
        _member_openid: &str,
    ) -> QqFuture<'_, GroupMember> {
        Box::pin(async { not_provided("get_group_member_info") })
    }

    fn get_group_member_list(
        &self,
        _group_openid: &str,
        _limit: u32,
    ) -> QqFuture<'_, GroupMemberList> {
        Box::pin(async { not_provided("get_group_member_list") })
    }

    // 禁言：记录调用并成功——duration 为合法 JSON 数字时的对照组。
    fn set_group_ban(
        &self,
        _group_openid: &str,
        _member_openid: &str,
        _duration_secs: i64,
    ) -> QqFuture<'_, ()> {
        let this = self;
        Box::pin(async move {
            this.record("set_group_ban");
            Ok(())
        })
    }

    // 入群审批：记录调用并成功——approve 为合法布尔时的对照组。
    fn approve_join_request(
        &self,
        _group_openid: &str,
        _member_openid: &str,
        _request: &ApprovalJoinRequest,
    ) -> QqFuture<'_, ()> {
        let this = self;
        Box::pin(async move {
            this.record("approve_join_request");
            Ok(())
        })
    }

    fn get_group_bot_state(&self, _group_openid: &str) -> QqFuture<'_, BotInGroupState> {
        Box::pin(async { not_provided("get_group_bot_state") })
    }

    fn get_group_join_request_list(
        &self,
        _group_openid: &str,
        _cursor: Option<&str>,
        _limit: Option<u32>,
    ) -> QqFuture<'_, JoinRequestList> {
        Box::pin(async { not_provided("get_group_join_request_list") })
    }

    fn send_group_message(
        &self,
        _group_openid: String,
        _msg: MessageToCreate,
    ) -> QqFuture<'_, MessageResponse> {
        Box::pin(async { not_provided("send_group_message") })
    }

    fn send_c2c_message(
        &self,
        _user_openid: String,
        _msg: MessageToCreate,
    ) -> QqFuture<'_, MessageResponse> {
        Box::pin(async { not_provided("send_c2c_message") })
    }
}

fn config() -> Arc<ConfigManager> {
    static MANAGER: std::sync::OnceLock<Arc<ConfigManager>> = std::sync::OnceLock::new();
    MANAGER
        .get_or_init(|| {
            let path = std::env::temp_dir().join(format!(
                "gensokyo-actions-strict-{}.yml",
                std::process::id()
            ));
            std::fs::write(&path, "{}\n").expect("写临时 config.yml 失败");
            let manager = Arc::new(ConfigManager::new(&path).expect("加载临时 config.yml 失败"));
            let _ = std::fs::remove_file(&path);
            manager
        })
        .clone()
}

fn registry_with(api: Arc<RecordingApi>) -> Registry {
    let context = Arc::new(ActionContext::new(api, config()));
    let mut registry = Registry::new();
    // 先注册全集再挂别名：register_default_aliases 要求目标已注册（get_group_ban → set_group_ban）
    register_all(&mut registry, context);
    registry.register_default_aliases().expect("别名注册失败");
    registry
}

fn request(action: &str, params: Value) -> ActionRequest<Value> {
    ActionRequest {
        action: action.to_owned(),
        params: Some(params),
        echo: Some(json!("e-1")),
        post_type: None,
    }
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(output) => return output,
            std::task::Poll::Pending => std::thread::yield_now(),
        }
    }
}

/// 契约表交叉断言：这些 (params) 全是 Go 整包反序列化失败的形状，`check_params` 必须
/// 给出「cannot unmarshal … of type …」且定位到正确字段（label 仅为测试说明）。
#[test]
fn declared_field_type_mismatch_table() {
    let cases: &[(&str, Value, &str)] = &[
        // B01 whole_ban enable:"yes"
        (
            "whole_ban.enable",
            json!({"group_id": GROUP, "enable": "yes"}),
            ".Alias.params.enable of type bool",
        ),
        // B02 add_request approve:"yes"
        (
            "add_request.approve",
            json!({"group_id": GROUP, "user_id": MEMBER, "approve": "yes"}),
            ".Alias.params.approve of type bool",
        ),
        // F02/H04 ban duration:"0" / "abc"
        (
            "ban.duration-string-0",
            json!({"group_id": GROUP, "user_id": MEMBER, "duration": "0"}),
            ".Alias.params.duration of type int",
        ),
        (
            "ban.duration-string-abc",
            json!({"group_id": GROUP, "user_id": MEMBER, "duration": "abc"}),
            ".Alias.params.duration of type int",
        ),
        // 分页 limit 字符串数字（get_group_join_request_list / blacklist / panel list 共用）
        (
            "join_request_list.limit",
            json!({"group_id": GROUP, "limit": "30"}),
            ".Alias.params.limit of type int",
        ),
        (
            "blacklist.cursor-number",
            json!({"group_id": GROUP, "cursor": 5}),
            ".Alias.params.cursor of type string",
        ),
        (
            "kick.add_blacklist-number",
            json!({"group_id": GROUP, "add_blacklist": 1}),
            ".Alias.params.add_blacklist of type bool",
        ),
        (
            "strategy.strategy_id-number",
            json!({"strategy_id": 7}),
            ".Alias.params.strategy_id of type string",
        ),
        (
            "whitelist.users-element",
            json!({"strategy_id": "s1", "whitelist_users": [1]}),
            ".Alias.params.whitelist_users of type []string",
        ),
        (
            "add_request.blacklist-string",
            json!({"add_to_member_blacklist": "true"}),
            ".Alias.params.add_to_member_blacklist of type bool",
        ),
    ];
    for (label, params, want) in cases {
        let err = check_params(Some(params)).expect_err(label);
        assert!(err.contains(want), "{label}: {err} 应含 {want}");
        assert!(err.starts_with("cannot unmarshal "), "{label}: {err}");
    }
}

/// 未声明字段（OneBot 常见扩展 auto_escape / reject_add_request、键盘按钮 click_limit
/// 形态）与 interface{} 字段维持宽容——Go 的 Alias 也只校验已声明字段。
#[test]
fn undeclared_fields_stay_lenient() {
    for params in [
        json!({"auto_escape": "true", "reject_add_request": "yes", "no_cache": 1}),
        json!({"keyboard": {"rows": [[{"click_limit": "3"}]]}, "enter_text": "x"}),
        json!({"message": [{"type": "image", "data": {"file": "a.png"}}]}),
    ] {
        assert!(check_params(Some(&params)).is_ok(), "{params}");
    }
}

/// 对照组：合法类型照常放行到上游（证明校验没有误伤正常请求、mock 确实在记录）。
#[tokio::test]
async fn valid_duration_reaches_upstream_as_control() {
    let api = Arc::new(RecordingApi::default());
    let registry = registry_with(Arc::clone(&api));
    let handler = registry.get("set_group_ban").expect("handler 应已注册");
    let response = block_on(handler.handle(&request(
        "set_group_ban",
        json!({"group_id": GROUP, "user_id": MEMBER, "duration": 60}),
    )))
    .expect("handler 不应返回 Err");
    assert_eq!(response.status, Status::Ok);
    assert_eq!(api.calls(), vec!["set_group_ban".to_owned()]);
}

/// handler 层的既有拒绝（approve:"yes" 经 optional_bool）同样零上游调用——即使某条链路
/// 忘了接分发层校验，副作用也不会不对称（defense-in-depth 证据）。
#[tokio::test]
async fn handler_level_rejection_has_zero_upstream_calls() {
    let api = Arc::new(RecordingApi::default());
    let registry = registry_with(Arc::clone(&api));
    let handler = registry
        .get("set_group_add_request")
        .expect("handler 应已注册");
    let response = block_on(handler.handle(&request(
        "set_group_add_request",
        json!({"group_id": GROUP, "user_id": MEMBER, "approve": "yes"}),
    )))
    .expect("参数错误也应是 Ok(失败响应)");
    assert_eq!(response.retcode, 1400);
    assert!(api.calls().is_empty(), "approve:\"yes\" 不得触达上游");
}
