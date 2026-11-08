//! deferred 计划执行：注册表装配、`send_*` 分派、上游失败与非 deferred 透传。
//!
//! 用 [`FakeQqApi`] 记录上游入参并注入结果，全程不碰网络。

mod support;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use gensokyo_actions::{ActionContext, QqApi, QqFuture};
use gensokyo_core::Error;
use gensokyo_protocol::Registry;
use gensokyo_protocol::action::ActionRequest;
use gensokyo_qq::dto::{
    BotInGroupState, GroupInfo, GroupMember, GroupMemberList, JoinRequestList, MessageResponse,
    MessageToCreate,
};
use gensokyo_runtime::{AppState, RETCODE_INTERNAL};
use serde_json::{Value, json};
use tempfile::TempDir;

use support::config;

/// 群 OpenID（32 位，直通上游、不需要 idmap）。
const OPENID: &str = "0123456789abcdef0123456789abcdef";
/// 用户 OpenID（私聊用例）。
const USER_OPENID: &str = "fedcba9876543210fedcba9876543210";

/// 一次发送的入参记录。
#[derive(Debug, Clone, PartialEq)]
enum Sent {
    Group(String, MessageToCreate),
    C2c(String, MessageToCreate),
}

/// 一次发送的上游结果。
#[derive(Debug, Clone)]
enum Outcome {
    Ok(MessageResponse),
    Fail,
}

/// 记录发送入参、按序注入结果的 `QqApi` 替身。
///
/// 槽位是队列（合并转发一条计划要打多次上游）：测试按序 `push_*`，取走即消费。
#[derive(Debug, Default)]
struct FakeQqApi {
    sent: Mutex<Vec<Sent>>,
    group: Mutex<VecDeque<Outcome>>,
    c2c: Mutex<VecDeque<Outcome>>,
}

impl FakeQqApi {
    fn sent(&self) -> Vec<Sent> {
        self.sent.lock().expect("发送记录锁未中毒").clone()
    }

    fn push_group(&self, outcome: Outcome) {
        self.group
            .lock()
            .expect("响应槽锁未中毒")
            .push_back(outcome);
    }

    fn push_c2c(&self, outcome: Outcome) {
        self.c2c.lock().expect("响应槽锁未中毒").push_back(outcome);
    }

    fn ok(&self, id: &str) -> Outcome {
        Outcome::Ok(MessageResponse {
            id: id.to_owned(),
            timestamp: "2026-10-25T00:00:00Z".to_owned(),
        })
    }
}

/// 取走下一次预置结果；未配置即 panic（「忘了准备 fixture」要立刻炸）。
fn take(slot: &Mutex<VecDeque<Outcome>>) -> Result<MessageResponse, Error> {
    match slot
        .lock()
        .expect("响应槽锁未中毒")
        .pop_front()
        .expect("测试未配置发送结果")
    {
        Outcome::Ok(response) => Ok(response),
        Outcome::Fail => Err(Error::Upstream {
            code: 40_054_005,
            message: "msgseq 去重".to_owned(),
        }),
    }
}

impl QqApi for FakeQqApi {
    fn get_group_info(&self, _group_openid: &str) -> QqFuture<'_, GroupInfo> {
        Box::pin(async { Err(Error::Unsupported("fake: get_group_info".to_owned())) })
    }

    fn get_group_member_info(
        &self,
        _group_openid: &str,
        _member_openid: &str,
    ) -> QqFuture<'_, GroupMember> {
        Box::pin(async { Err(Error::Unsupported("fake: get_group_member_info".to_owned())) })
    }

    fn get_group_member_list(
        &self,
        _group_openid: &str,
        _limit: u32,
    ) -> QqFuture<'_, GroupMemberList> {
        Box::pin(async { Err(Error::Unsupported("fake: get_group_member_list".to_owned())) })
    }

    fn set_group_ban(
        &self,
        _group_openid: &str,
        _member_openid: &str,
        _duration_secs: i64,
    ) -> QqFuture<'_, ()> {
        Box::pin(async { Err(Error::Unsupported("fake: set_group_ban".to_owned())) })
    }

    fn get_group_bot_state(&self, _group_openid: &str) -> QqFuture<'_, BotInGroupState> {
        Box::pin(async { Err(Error::Unsupported("fake: get_group_bot_state".to_owned())) })
    }

    fn get_group_join_request_list(
        &self,
        _group_openid: &str,
        _cursor: Option<&str>,
        _limit: Option<u32>,
    ) -> QqFuture<'_, JoinRequestList> {
        Box::pin(async {
            Err(Error::Unsupported(
                "fake: get_group_join_request_list".to_owned(),
            ))
        })
    }

    fn send_group_message(
        &self,
        group_openid: String,
        msg: MessageToCreate,
    ) -> QqFuture<'_, MessageResponse> {
        Box::pin(async move {
            self.sent
                .lock()
                .expect("发送记录锁未中毒")
                .push(Sent::Group(group_openid, msg));
            take(&self.group)
        })
    }

    fn send_c2c_message(
        &self,
        user_openid: String,
        msg: MessageToCreate,
    ) -> QqFuture<'_, MessageResponse> {
        Box::pin(async move {
            self.sent
                .lock()
                .expect("发送记录锁未中毒")
                .push(Sent::C2c(user_openid, msg));
            take(&self.c2c)
        })
    }
}

/// 装配「内置 action + actions 全集」的 `AppState` 与共享替身。
fn state(settings: &str) -> (TempDir, Arc<FakeQqApi>, AppState) {
    let (manager, dir) = config(settings);
    let api = Arc::new(FakeQqApi::default());
    let api_dyn: Arc<dyn QqApi> = api.clone();
    let context = Arc::new(ActionContext::new(api_dyn, Arc::clone(&manager)));
    let state = AppState::with_actions(manager, context);
    (dir, api, state)
}

fn request(action: &str, params: Value, echo: Option<Value>) -> ActionRequest<Value> {
    ActionRequest {
        action: action.to_owned(),
        params: Some(params),
        echo,
    }
}

#[tokio::test]
async fn registry_contains_all_actions_and_aliases() {
    let (_dir, _api, state) = state("app_id: 12345\n");
    let actions = state.registry().actions();

    assert_eq!(
        actions.len(),
        gensokyo_actions::IMPLEMENTED_ACTIONS.len() + 3 + 3,
        "41 个 actions 注册名 + 3 个 Go 兼容别名 + 3 个内置 action"
    );
    for action in gensokyo_actions::IMPLEMENTED_ACTIONS {
        assert!(actions.contains(action), "缺少 action {action}");
    }
    for alias in ["get_group_ban", "get_group_whole_ban", "send_to_group"] {
        assert!(actions.contains(&alias), "缺少别名 {alias}");
    }
}

#[tokio::test]
async fn send_group_msg_sends_text_and_returns_message_id() {
    let (_dir, api, state) = state("app_id: 12345\n");
    api.push_group(api.ok("m-1"));

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            json!({ "group_id": OPENID, "message": "hello" }),
            Some(json!("echo-1")),
        ))
        .await;

    assert_eq!(response.retcode, 0);
    assert_eq!(response.echo, Some(json!("echo-1")));
    // deferred 计划不泄漏：最终 data 只有 message_id。
    assert_eq!(response.data, Some(json!({ "message_id": "m-1" })));

    let sent = api.sent();
    assert_eq!(sent.len(), 1);
    match &sent[0] {
        Sent::Group(openid, message) => {
            assert_eq!(openid, OPENID);
            assert_eq!(message.content.as_deref(), Some("hello"));
            assert!(message.media.is_none());
        }
        other => panic!("send_group_msg 应发群消息: {other:?}"),
    }
}

#[tokio::test]
async fn at_segments_become_v2_mentions() {
    let (_dir, api, state) = state("app_id: 12345\n");
    api.push_group(api.ok("m-2"));

    let mention = format!("[CQ:at,qq={USER_OPENID}] hi");
    let expected = format!("<@{USER_OPENID}> hi");
    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            json!({ "group_id": OPENID, "message": mention }),
            None,
        ))
        .await;

    assert_eq!(response.retcode, 0);
    let sent = api.sent();
    match &sent[0] {
        Sent::Group(_, message) => assert_eq!(message.content.as_deref(), Some(expected.as_str())),
        other => panic!("应发群消息: {other:?}"),
    }
}

#[tokio::test]
async fn send_private_msg_and_send_msg_dispatch_by_target() {
    let (_dir, api, state) = state("app_id: 12345\n");
    let dispatcher = state.dispatcher();

    // send_private_msg → C2C。
    api.push_c2c(api.ok("c-1"));
    let response = dispatcher
        .dispatch(&request(
            "send_private_msg",
            json!({ "user_id": USER_OPENID, "message": "hi" }),
            None,
        ))
        .await;
    assert_eq!(response.retcode, 0);

    // send_msg(message_type=group) → 群。
    api.push_group(api.ok("g-1"));
    let response = dispatcher
        .dispatch(&request(
            "send_msg",
            json!({ "message_type": "group", "group_id": OPENID, "message": "yo" }),
            None,
        ))
        .await;
    assert_eq!(response.retcode, 0);

    // send_msg(message_type=private) → C2C。
    api.push_c2c(api.ok("c-2"));
    let response = dispatcher
        .dispatch(&request(
            "send_msg",
            json!({ "message_type": "private", "user_id": USER_OPENID, "message": "yo" }),
            None,
        ))
        .await;
    assert_eq!(response.retcode, 0);

    let sent = api.sent();
    assert_eq!(sent.len(), 3);
    assert!(matches!(&sent[0], Sent::C2c(openid, _) if openid == USER_OPENID));
    assert!(matches!(&sent[1], Sent::Group(openid, _) if openid == OPENID));
    assert!(matches!(&sent[2], Sent::C2c(openid, _) if openid == USER_OPENID));
}

#[tokio::test]
async fn upstream_failure_is_1500_and_echo_kept() {
    let (_dir, api, state) = state("app_id: 12345\n");
    api.push_group(Outcome::Fail);

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            json!({ "group_id": OPENID, "message": "hello" }),
            Some(json!("e-500")),
        ))
        .await;

    assert_eq!(response.status, gensokyo_protocol::action::Status::Failed);
    assert_eq!(response.retcode, RETCODE_INTERNAL);
    assert!(response.data.is_none(), "失败响应不应带 data");
    assert_eq!(response.echo, Some(json!("e-500")));
    assert!(
        response
            .message
            .as_deref()
            .is_some_and(|message| message.contains("msgseq 去重")),
        "上游错误文案应回传: {:?}",
        response.message
    );
}

#[tokio::test]
async fn non_deferred_action_is_returned_unchanged() {
    let (_dir, api, state) = state("app_id: 12345\n");

    let response = state
        .dispatcher()
        .dispatch(&request("get_status", json!({}), None))
        .await;

    assert_eq!(response.retcode, 0);
    assert_eq!(response.data.as_ref().expect("data")["online"], true);
    assert!(api.sent().is_empty(), "非 deferred action 不应触达上游发送");
}

#[tokio::test]
async fn deferred_without_action_context_is_1500() {
    let (manager, _dir) = config("app_id: 12345\n");
    let api = Arc::new(FakeQqApi::default());
    let api_dyn: Arc<dyn QqApi> = api.clone();
    let context = Arc::new(ActionContext::new(api_dyn, Arc::clone(&manager)));

    // 只装注册表、不挂 ActionContext：deferred 计划无法执行，必须 1500 而不是泄漏计划。
    let mut registry = Registry::new();
    gensokyo_runtime::builtin::register_all(
        &mut registry,
        Arc::clone(&manager),
        gensokyo_runtime::BotStatsSlot::default(),
    );
    gensokyo_actions::register_all(&mut registry, context);
    let state = AppState::new(manager, Arc::new(registry));

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            json!({ "group_id": OPENID, "message": "hello" }),
            None,
        ))
        .await;

    assert_eq!(response.retcode, RETCODE_INTERNAL);
    assert!(response.data.is_none(), "失败响应不应带 data");
    assert!(api.sent().is_empty());
}

// ---- R3b：唤醒 / 原始直发 / SSE / 合并转发的真实端点接线 ----

#[tokio::test]
async fn wakeup_send_marks_is_wakeup_on_c2c_endpoint() {
    let (_dir, api, state) = state("app_id: 12345\n");
    api.push_c2c(api.ok("w-1"));

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_private_msg_wakeup",
            json!({ "user_id": USER_OPENID, "message": "召回一下" }),
            Some(json!("e-w")),
        ))
        .await;

    assert_eq!(response.retcode, 0);
    assert_eq!(response.echo, Some(json!("e-w")));
    assert_eq!(response.data, Some(json!({ "message_id": "w-1" })));

    let sent = api.sent();
    match &sent[0] {
        Sent::C2c(openid, message) => {
            assert_eq!(openid, USER_OPENID);
            assert!(message.is_wakeup, "召回应置 is_wakeup");
            assert_eq!(message.msg_id, None, "主动召回不带 msg_id");
            assert_eq!(
                message.event_id, None,
                "主动召回同样不带 event_id（三者互斥）"
            );
            assert_eq!(message.content.as_deref(), Some("召回一下"));
        }
        other => panic!("wakeup 应发 C2C: {other:?}"),
    }
}

#[tokio::test]
async fn raw_send_replies_with_params_message_id() {
    let (_dir, api, state) = state("app_id: 12345\n");
    api.push_group(api.ok("r-1"));

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg_raw",
            json!({ "group_id": OPENID, "message": "收到", "message_id": "REAL-MID-7" }),
            None,
        ))
        .await;

    assert_eq!(response.retcode, 0);
    assert_eq!(response.data, Some(json!({ "message_id": "r-1" })));

    let sent = api.sent();
    match &sent[0] {
        Sent::Group(openid, message) => {
            assert_eq!(openid, OPENID);
            assert_eq!(
                message.msg_id.as_deref(),
                Some("REAL-MID-7"),
                "params.message_id 应成为被动回复目标（Go raw 口径）"
            );
            assert_eq!(message.content.as_deref(), Some("收到"));
        }
        other => panic!("raw 应发群消息: {other:?}"),
    }
}

#[tokio::test]
async fn sse_send_posts_stream_body_to_c2c_endpoint() {
    let (_dir, api, state) = state("app_id: 12345\n");
    api.push_c2c(api.ok("s-1"));

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_private_msg_sse",
            json!({
                "user_id": USER_OPENID,
                "message": {
                    "content": "第一段",
                    "state": 1,
                    "prompt_keyboard": ["继续"],
                    "action_button": 3,
                    "callback_data": "cb"
                }
            }),
            Some(json!("e-s")),
        ))
        .await;

    assert_eq!(response.retcode, 0);
    assert_eq!(response.data, Some(json!({ "message_id": "s-1" })));

    let sent = api.sent();
    match &sent[0] {
        Sent::C2c(openid, message) => {
            assert_eq!(
                openid, USER_OPENID,
                "SSE 与普通 C2C 同端点（Go PostC2CMessageSSE）"
            );
            assert_eq!(message.msg_type, Some(2));
            assert_eq!(
                message
                    .markdown
                    .as_ref()
                    .and_then(|markdown| markdown.content.as_deref()),
                Some("第一段"),
                "正文原样进 markdown.content（Go 不解析 CQ）"
            );
            let stream = message.stream.as_ref().expect("SSE 应带 stream");
            assert_eq!((stream.state, stream.index), (1, 0));
            assert_eq!(message.msg_seq, Some(10), "Go 口径 msg_seq = index + 10");
            assert_eq!(
                message
                    .extra
                    .get("prompt_keyboard")
                    .and_then(|value| value
                        .pointer("/keyboard/content/rows/0/buttons/0/render_data/label")),
                Some(&json!("继续"))
            );
            assert_eq!(
                message.extra.get("action_button"),
                Some(&json!({ "template_id": 3, "callback_data": "cb" }))
            );
        }
        other => panic!("SSE 应走 C2C 端点: {other:?}"),
    }
}

#[tokio::test]
async fn group_forward_resends_nodes_and_reports_failed_index() {
    let (_dir, api, state) = state("  app_id: 12345\n  forward_msg_limit: 5\n");
    api.push_group(api.ok("f-1"));
    api.push_group(Outcome::Fail);
    api.push_group(api.ok("f-3"));
    // 零间隔注入（评审 ③）：转发循环不在测试里真睡
    let dispatcher = state
        .dispatcher()
        .with_forward_interval(std::time::Duration::ZERO);

    // node 布局刻意把坏形状放在**中间**（评审 ①）：node 2 被静默跳过后，失败清单里的
    // 序号仍须是原 node 位置而不是计划下标；node 3 段解析失败也要入册（评审 ②）
    let response = dispatcher
        .dispatch(&request(
            "send_group_forward_msg",
            json!({
                "group_id": OPENID,
                "messages": [
                    {"data": {"content": "第一条"}},
                    {"data": {"content": "第二条"}},
                    {"bogus": true},
                    {"data": {"content": [123]}},
                    {"data": {"content": [{"type": "text", "data": {"text": "第五条"}}]}}
                ]
            }),
            Some(json!("e-f")),
        ))
        .await;

    assert_eq!(
        response.retcode, 0,
        "部分失败不改整体 retcode（Go 回执语义）"
    );
    let data = response.data.expect("data");
    assert_eq!(data["message_ids"], json!(["f-1", "f-3"]), "已发不回滚");
    // node 1 发送失败 + node 3 段解析失败（日志：规划层 tracing::warn 打点）；
    // node 2 坏形状按 Go 静默跳过、不入册
    let failed = data["failed"].as_array().expect("failed 数组").clone();
    assert_eq!(failed.len(), 2, "实际回执: {data}");
    assert_eq!(failed[0]["index"], json!(1), "发送失败报原 node 序号");
    assert!(
        failed[0]["error"]
            .as_str()
            .unwrap_or_default()
            .contains("msgseq 去重"),
        "失败原因要带上游文案: {data}"
    );
    assert_eq!(failed[1]["index"], json!(3), "解析失败也报原 node 序号");
    assert!(
        !failed[1]["error"].as_str().unwrap_or_default().is_empty(),
        "解析失败要带 serde 错误文本: {data}"
    );

    let sent = api.sent();
    assert_eq!(
        sent.len(),
        3,
        "坏形状 / 解析失败 node 都不打上游，其余逐条重发"
    );
    match &sent[2] {
        Sent::Group(_, message) => assert_eq!(message.content.as_deref(), Some("第五条")),
        other => panic!("应为群消息: {other:?}"),
    }
}

#[tokio::test]
async fn private_forward_loops_c2c_endpoint() {
    let (_dir, api, state) = state("  app_id: 12345\n  forward_msg_limit: 5\n");
    api.push_c2c(api.ok("p-1"));
    api.push_c2c(api.ok("p-2"));
    let dispatcher = state
        .dispatcher()
        .with_forward_interval(std::time::Duration::ZERO);

    let response = dispatcher
        .dispatch(&request(
            "send_private_forward_msg",
            json!({
                "user_id": USER_OPENID,
                "messages": [
                    {"data": {"content": "第一条"}},
                    {"data": {"content": "第二条"}}
                ]
            }),
            None,
        ))
        .await;

    assert_eq!(response.retcode, 0);
    assert_eq!(
        response.data.expect("data")["message_ids"],
        json!(["p-1", "p-2"])
    );
    let sent = api.sent();
    assert_eq!(sent.len(), 2);
    assert!(matches!(
        &sent[1],
        Sent::C2c(openid, message)
            if openid == USER_OPENID && message.content.as_deref() == Some("第二条")
    ));
}
