//! deferred 计划执行：注册表装配、`send_*` 分派、上游失败与非 deferred 透传。
//!
//! 用 [`FakeQqApi`] 记录上游入参并注入结果，全程不碰网络。

mod support;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use gensokyo_actions::{ActionContext, QqApi, QqFuture, RetractRequest};
use gensokyo_core::Error;
use gensokyo_protocol::Registry;
use gensokyo_protocol::action::ActionRequest;
use gensokyo_qq::dto::{
    BotInGroupState, GroupInfo, GroupMember, GroupMemberList, JoinRequestList, MessageResponse,
    MessageToCreate,
};
use gensokyo_runtime::{AppState, RETCODE_BAD_REQUEST, RETCODE_INTERNAL};
use gensokyo_storage::{EchoStore, IdMapOptions, IdMapStore};
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
    /// 指定业务码的上游失败（错误分类分支用：`40034025` 事件过期 / `22009` 频控）。
    Code(i64, &'static str),
}

/// 记录发送入参、按序注入结果的 `QqApi` 替身。
///
/// 槽位是队列（合并转发一条计划要打多次上游）：测试按序 `push_*`，取走即消费。
#[derive(Debug, Default)]
struct FakeQqApi {
    sent: Mutex<Vec<Sent>>,
    deleted: Mutex<Vec<RetractRequest>>,
    group: Mutex<VecDeque<Outcome>>,
    c2c: Mutex<VecDeque<Outcome>>,
}

impl FakeQqApi {
    fn sent(&self) -> Vec<Sent> {
        self.sent.lock().expect("发送记录锁未中毒").clone()
    }

    fn deleted(&self) -> Vec<RetractRequest> {
        self.deleted.lock().expect("撤回记录锁未中毒").clone()
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
        Outcome::Code(code, message) => Err(Error::Upstream {
            code,
            message: message.to_owned(),
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

    fn delete_msg(&self, target: RetractRequest) -> QqFuture<'_, ()> {
        Box::pin(async move {
            self.deleted.lock().expect("撤回记录锁未中毒").push(target);
            Ok(())
        })
    }
}

/// 装配「内置 action + actions 全集」的 `AppState` 与共享替身。
fn state(settings: &str) -> (TempDir, Arc<FakeQqApi>, AppState) {
    let (dir, api, state, _echo) = state_with_echo(settings);
    (dir, api, state)
}

/// 同 [`state`]，并返回内存 echo 存储的句柄：生产 `boot` 必然装配 echo（`boot.rs:150`），
/// numeric 模式的虚拟行号派生就靠它，回执断言需要同一份句柄算期望值。
fn state_with_echo(settings: &str) -> (TempDir, Arc<FakeQqApi>, AppState, Arc<EchoStore>) {
    let (manager, dir) = config(settings);
    let api = Arc::new(FakeQqApi::default());
    let api_dyn: Arc<dyn QqApi> = api.clone();
    let echo = Arc::new(EchoStore::new());
    // 与生产 `boot::attach_echo` 同口径：echo 同时挂进状态与 `ActionContext`
    let context =
        Arc::new(ActionContext::new(api_dyn, Arc::clone(&manager)).with_echo(Arc::clone(&echo)));
    let state = AppState::with_actions(manager, context).with_echo(Arc::clone(&echo));
    (dir, api, state, echo)
}

/// `set_group_ban` 成功用的替身：记录入参并回 `Ok(())`（默认替身恒 `Unsupported`，
/// 钉不了成功帧）。其余端点仍走 [`FakeQqApi`] 的失败口径。
#[derive(Debug, Default)]
struct BanOkApi {
    bans: Mutex<Vec<(String, String, i64)>>,
}

impl QqApi for BanOkApi {
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
        group_openid: &str,
        member_openid: &str,
        duration_secs: i64,
    ) -> QqFuture<'_, ()> {
        let group_openid = group_openid.to_owned();
        let member_openid = member_openid.to_owned();
        Box::pin(async move {
            self.bans.lock().expect("禁言记录锁未中毒").push((
                group_openid,
                member_openid,
                duration_secs,
            ));
            Ok(())
        })
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
        _group_openid: String,
        _msg: MessageToCreate,
    ) -> QqFuture<'_, MessageResponse> {
        Box::pin(async { Err(Error::Unsupported("fake: send_group_message".to_owned())) })
    }

    fn send_c2c_message(
        &self,
        _user_openid: String,
        _msg: MessageToCreate,
    ) -> QqFuture<'_, MessageResponse> {
        Box::pin(async { Err(Error::Unsupported("fake: send_c2c_message".to_owned())) })
    }

    fn delete_msg(&self, _target: RetractRequest) -> QqFuture<'_, ()> {
        Box::pin(async { Err(Error::Unsupported("fake: delete_msg".to_owned())) })
    }
}

/// 生产注册表 + 禁言成功的替身：`set_group_ban` 能走到 ok 分支。
fn ban_ok_state(settings: &str) -> (TempDir, Arc<BanOkApi>, AppState) {
    let (manager, dir) = config(settings);
    let api = Arc::new(BanOkApi::default());
    let api_dyn: Arc<dyn QqApi> = api.clone();
    let context = Arc::new(ActionContext::new(api_dyn, Arc::clone(&manager)));
    let state = AppState::with_actions(manager, context);
    (dir, api, state)
}

/// 生产注册表下的 Compact **成功**整串（D-a 口径修正）：活体 Go 的 `set_group_ban` ok 帧是
/// `{"data":null,"echo":"e1","message":"","retcode":0,"status":"ok"}`——`sendActionResult.Data`
/// 无 `omitempty`，成功也恒带 `data`（E01/E11/E17/E21/E33/E43 六次复现）。真实 handler、真实
/// 信封表、真实 `dispatch_json` 出口；无 echo 时该键整个消失，`data` 照在。
#[tokio::test]
async fn compact_success_receipt_carries_null_data_like_go() {
    let (_dir, api, state) = ban_ok_state("app_id: 12345\n");
    let dispatcher = state.dispatcher();

    let body = gensokyo_runtime::dispatch_json(
        dispatcher.clone(),
        format!(
            r#"{{"action":"set_group_ban","params":{{"group_id":"{OPENID}","user_id":"{USER_OPENID}","duration":600}},"echo":"e1"}}"#
        )
        .into_bytes(),
    )
    .await;
    assert_eq!(
        body,
        r#"{"data":null,"echo":"e1","message":"","retcode":0,"status":"ok"}"#
    );

    // 无请求回执：`echo` 按 omitempty 缺席，`data:null` 不受影响
    let body = gensokyo_runtime::dispatch_json(
        dispatcher,
        format!(
            r#"{{"action":"set_group_ban","params":{{"group_id":"{OPENID}","user_id":"{USER_OPENID}","duration":0}}}}"#
        )
        .into_bytes(),
    )
    .await;
    assert_eq!(
        body,
        r#"{"data":null,"message":"","retcode":0,"status":"ok"}"#
    );

    // 成功确实打到了上游：替身按序记录 (group_openid, member_openid, duration)；
    // 两个 ID 都是 32 位 OpenID，直通不反查（本用例刻意不装配 idmap）
    assert_eq!(
        api.bans.lock().expect("禁言记录锁未中毒").as_slice(),
        [
            (OPENID.to_owned(), USER_OPENID.to_owned(), 600_i64),
            (OPENID.to_owned(), USER_OPENID.to_owned(), 0_i64),
        ]
    );
}

/// 同 [`state_with_echo`]，并装配 idmap：numeric 模式的虚拟号要它才能反查回 OpenID。
fn state_with_idmap(
    settings: &str,
) -> (
    TempDir,
    Arc<FakeQqApi>,
    AppState,
    Arc<EchoStore>,
    Arc<IdMapStore>,
) {
    let (manager, dir) = config(settings);
    let api = Arc::new(FakeQqApi::default());
    let api_dyn: Arc<dyn QqApi> = api.clone();
    let idmap = Arc::new(
        IdMapStore::open(
            IdMapOptions::new(
                dir.path().join("openid-map.redb"),
                dir.path().join("msgid-map.redb"),
            )
            .with_app_id("12345"),
        )
        .expect("idmap 应能打开"),
    );
    let echo = Arc::new(EchoStore::new());
    // 与生产 `boot::attach_echo` 同口径：echo 同时挂进状态与 `ActionContext`，撤回才能反查回执虚拟号
    let context = Arc::new(
        ActionContext::new(api_dyn, Arc::clone(&manager))
            .with_idmap(Arc::clone(&idmap))
            .with_echo(Arc::clone(&echo)),
    );
    let state = AppState::with_actions(manager, context).with_echo(Arc::clone(&echo));
    (dir, api, state, echo, idmap)
}

fn request(action: &str, params: Value, echo: Option<Value>) -> ActionRequest<Value> {
    ActionRequest {
        action: action.to_owned(),
        params: Some(params),
        echo,
        post_type: None,
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
    let (_dir, api, state, echo) = state_with_echo("app_id: 12345\n");
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
    // deferred 计划不泄漏：最终 data 只有 message_id；numeric 模式（默认 string_ob11=false）
    // 回的是 echo 派生的虚拟行号数字（Go `SendResponse`）。
    let expected = echo.store_cache_in_memory("m-1").expect("派生行号");
    assert_eq!(
        response.data,
        Some(json!({ "message_id": expected })),
        "numeric 模式的 message_id 必须是 JSON 数字"
    );

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
async fn at_segments_with_openid_are_passed_through() {
    let (_dir, api, state) = state("app_id: 12345\n");
    api.push_group(api.ok("m-2"));

    // Go `cqAtNumRe = \[CQ:at,qq=(\d+)\]`：只匹配纯数字虚拟号。
    // 32 位 openid 不是受支持用法（插件侧只见虚拟号），原样透传。
    let mention = format!("[CQ:at,qq={USER_OPENID}] hi");
    let expected = mention.clone();
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

/// 生产注册表下的 Compact 失败整串（D-a + D-b）：Go `set_group_whole_ban` 命中群类型后
/// 固定 `sendActionResult(errWholeBanUnsupported, 100)`，WS 实发帧带 `data:null`——
/// `Data` 无 `omitempty`，五键字母序。真实 handler、真实信封表、真实 `dispatch_json` 出口。
/// 缺 `group_id` 的分支钉 D-b：Rust 的用户级参数校验回 `1400`，经白名单原样放行、
/// 不被改写成 `100`（Go `handlers/set_group_whole_ban.go:24,28` 的两个 `sendParamRetCode`）。
/// `enable:"yes"` 分支钉 B01（活体对照）：Go 的 `ParamsContent.Enable` 是 `bool`，整包
/// 反序列化在收包层失败 → 分发层 1400「参数解析失败」（`callapi.ReplyUnparseableMessage`）；
/// Rust 由 `strict_params` 在 route() 同构拒绝，**不进 handler、零上游调用**。
#[tokio::test]
async fn whole_ban_receipt_carries_null_data_like_go() {
    let (_dir, api, state) = state("app_id: 12345\n");
    let dispatcher = state.dispatcher();

    let body = gensokyo_runtime::dispatch_json(
        dispatcher.clone(),
        br#"{"action":"set_group_whole_ban","params":{"group_id":"610636458","enable":"yes"},"echo":"e1"}"#
            .to_vec(),
    )
    .await;
    assert_eq!(
        body,
        r#"{"data":null,"echo":"e1","message":"参数解析失败: cannot unmarshal string into Go struct field .Alias.params.enable of type bool","retcode":1400,"status":"failed"}"#
    );
    // 零上游副作用证明：FakeQqApi 记录器必须一条调用都没有（等价 mock try_recv 空）
    assert!(api.sent().is_empty(), "enable:\"yes\" 不得触达任何发送端点");

    let body = gensokyo_runtime::dispatch_json(
        dispatcher,
        br#"{"action":"set_group_whole_ban","params":{},"echo":"e2"}"#.to_vec(),
    )
    .await;
    assert_eq!(
        body,
        r#"{"data":null,"echo":"e2","message":"协议错误: 缺少 group_id","retcode":1400,"status":"failed"}"#
    );
}

/// F02/H04（活体最高危）：`duration:"0"` 字符串数字在 Go 让整包 `json.Unmarshal` 失败
/// → 分发层 1400「参数解析失败」，请求根本进不了 handler；修复前 Rust 宽容解码并**真发出**
/// `restrict_chat_setting`（op=add 禁言副作用！）。本用例钉两件事：
/// 1. wire 整串 = Full 信封 1400 + data:null + 「参数解析失败: 」文案档；
/// 2. **零上游调用**——`api.calls()`（FakeQqApi 记录器，等价 mock transport try_recv 空）
///    必须为空，坏类型请求在到达 openapi 之前就被拒。
#[tokio::test]
async fn ban_duration_string_number_is_1400_with_zero_upstream_calls() {
    let (_dir, api, state) = state("app_id: 12345\n");
    let dispatcher = state.dispatcher();

    for payload in [
        r#"{"action":"set_group_ban","params":{"group_id":"610636458","user_id":"7","duration":"0"},"echo":"d1"}"#,
        r#"{"action":"set_group_ban","params":{"group_id":"610636458","user_id":"7","duration":"abc"},"echo":"d2"}"#,
    ] {
        let body =
            gensokyo_runtime::dispatch_json(dispatcher.clone(), payload.as_bytes().to_vec()).await;
        let value: serde_json::Value = serde_json::from_str(&body).expect("响应应为 JSON");
        assert_eq!(value["retcode"], 1400, "{body}");
        assert_eq!(value["status"], "failed", "{body}");
        assert_eq!(value["data"], serde_json::Value::Null, "{body}");
        let message = value["message"].as_str().unwrap_or_default();
        assert!(message.starts_with("参数解析失败: "), "{body}");
        assert!(
            message.contains(".Alias.params.duration of type int"),
            "应给出 Go 同款字段定位: {body}"
        );
    }
    assert!(
        api.sent().is_empty(),
        "duration 字符串数字必须被分发层拒绝，禁止任何发送类上游调用"
    );
}

/// 生产注册表下的 Compact **参数错 / 上游失败**整串（D-a + D-b/D-c 口径）：`create_panel` 缺
/// `scope` 在 Go 是 `sendActionResult("scope 参数缺失", 100)`（`handlers/create_panel.go:27`）——
/// 管理类参数错一律 100，且 WS 实发帧恒带 `data:null`（`Data` 无 `omitempty`）。Rust 侧 handler
/// 回 `1400`，不在任何放行表内，经 runtime 的 B 类改写落 `100`；帧形与 Go 逐字节同构（五键字母序）。
#[tokio::test]
async fn compact_upstream_failure_receipt_is_100_with_null_data() {
    let (_dir, _api, state) = state("app_id: 12345\n");
    let body = gensokyo_runtime::dispatch_json(
        state.dispatcher(),
        br#"{"action":"create_panel","params":{"panel":{}},"echo":"e1"}"#.to_vec(),
    )
    .await;
    let value: serde_json::Value = serde_json::from_str(&body).expect("响应应为 JSON");
    assert_eq!(value["retcode"], 100, "{body}");
    assert_eq!(value["status"], "failed");
    assert_eq!(
        value["data"],
        serde_json::Value::Null,
        "Compact 失败必须带 data:null: {body}"
    );
    assert_eq!(value["echo"], "e1");
    assert!(value["message"].as_str().is_some(), "message 恒在: {body}");
}

/// 审批策略关联群换算失败的整串（D-b 分支型放行）：Go 只在
/// `resolveJoinApprovalGroupOpenIDs` 失败这一个出口写 `sendParamRetCode`
/// （`handlers/join_approval_strategy.go:34,73`），其余失败都是 `100`。Rust 的文案照 Go 逐字
/// （`panels.rs` `strategy_group_openids`），runtime 按前缀放行 → 线上帧 `1400` + `data:null`。
#[tokio::test]
async fn join_approval_unmapped_group_keeps_1400_with_null_data() {
    let (_dir, _api, state) = state("app_id: 12345\n");
    let dispatcher = state.dispatcher();
    for action in [
        "join_approval_strategy_create",
        "join_approval_strategy_update",
    ] {
        let payload = format!(
            r#"{{"action":"{action}","params":{{"group_openids":["123456"],"op":"add","strategy_id":"s1"}},"echo":"e1"}}"#
        );
        let body = gensokyo_runtime::dispatch_json(dispatcher.clone(), payload.into_bytes()).await;
        let value: serde_json::Value = serde_json::from_str(&body).expect("响应应为 JSON");
        assert_eq!(value["retcode"], 1400, "{action}: {body}");
        assert_eq!(value["status"], "failed");
        assert_eq!(value["data"], serde_json::Value::Null, "{action}: {body}");
        assert_eq!(value["echo"], "e1");
        let message = value["message"].as_str().expect("message 恒在");
        assert!(
            message.starts_with("协议错误: group_openids=123456 无法反查群 OpenID"),
            "{action} 应照 Go 文案给出换算失败原因: {body}"
        );
    }
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

/// 40034025（`event_id` 失效）走 Go 的清空重发一次：重发成功即本次成功。
/// （`event_id` 被清空这一步由 `deferred` 的单元测试直接盯消息体断言。）
#[tokio::test]
async fn event_expired_resends_once_through_the_action_chain() {
    let (_dir, api, state, echo) = state_with_echo("app_id: 12345\n");
    api.push_group(Outcome::Code(40_034_025, "请求参数event_id无效"));
    api.push_group(api.ok("m-fallback"));

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            json!({ "group_id": OPENID, "message": "hello" }),
            None,
        ))
        .await;

    assert_eq!(response.retcode, 0, "重发成功应回 0");
    let expected = echo.store_cache_in_memory("m-fallback").expect("派生行号");
    assert_eq!(response.data, Some(json!({ "message_id": expected })));
    let sent = api.sent();
    assert_eq!(sent.len(), 2, "Go：40034025 重发一次");
    assert!(
        matches!(&sent[0], Sent::Group(openid, message)
            if openid == OPENID && message.content.as_deref() == Some("hello")),
        "首打记录: {:?}",
        sent[0]
    );
    assert!(
        matches!(&sent[1], Sent::Group(_, message) if message.content.as_deref() == Some("hello")),
        "重发记录: {:?}",
        sent[1]
    );
}

/// 重发仍失败：终局是上游原码（`retcode=1500` + 文案带 40034025），且只重发一次、不入 SSM 队列。
#[tokio::test]
async fn event_expired_final_failure_reports_the_upstream_code() {
    let (_dir, api, state) = state("app_id: 12345\n");
    api.push_group(Outcome::Code(40_034_025, "请求参数event_id无效"));
    api.push_group(Outcome::Code(40_034_025, "请求参数event_id无效"));

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            json!({ "group_id": OPENID, "message": "hello" }),
            Some(json!("e-expired")),
        ))
        .await;

    assert_eq!(response.retcode, RETCODE_INTERNAL);
    assert_eq!(response.echo, Some(json!("e-expired")));
    assert!(
        response
            .message
            .as_deref()
            .is_some_and(|message| message.contains("40034025")),
        "终局错误应带原码: {:?}",
        response.message
    );
    assert_eq!(api.sent().len(), 2, "重发只有一次，不构成循环");
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
    let (_dir, api, state, echo) = state_with_echo("app_id: 12345\n");
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
    let expected = echo.store_cache_in_memory("w-1").expect("派生行号");
    assert_eq!(response.data, Some(json!({ "message_id": expected })));

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

/// B：wakeup 的线上回执与 Go 字面量逐字节一致（无 echo / data），失败才是 1500 信封。
#[tokio::test]
async fn wakeup_receipt_is_the_go_literal() {
    let (_dir, api, state, _echo) = state_with_echo("app_id: 12345\n");
    api.push_c2c(api.ok("w-2"));
    let payload = serde_json::to_vec(&json!({
        "action": "send_private_msg_wakeup",
        "params": { "user_id": USER_OPENID, "message": "召回一下" },
        "echo": "e-w",
    }))
    .expect("请求应可序列化");
    let text = gensokyo_runtime::dispatcher::dispatch_json(state.dispatcher(), payload).await;
    assert_eq!(text, r#"{"status": "ok", "retcode": 0}"#);

    // 参数错误仍走 1500/1400 的 Full 信封（带 message，便于定位）
    let payload = serde_json::to_vec(&json!({
        "action": "send_private_msg_wakeup",
        "params": { "user_id": USER_OPENID },
        "echo": "e-w2",
    }))
    .expect("请求应可序列化");
    let text = gensokyo_runtime::dispatcher::dispatch_json(state.dispatcher(), payload).await;
    assert!(text.contains(r#""retcode":1400"#), "{text}");
    assert!(text.contains(r#""echo":"e-w2""#), "{text}");
}

#[tokio::test]
async fn raw_send_replies_with_params_message_id() {
    let (_dir, api, state, echo) = state_with_echo("app_id: 12345\n");
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
    let expected = echo.store_cache_in_memory("r-1").expect("派生行号");
    assert_eq!(response.data, Some(json!({ "message_id": expected })));

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
    let (_dir, api, state, echo, idmap) = state_with_idmap("app_id: 12345\n");
    let (_, user_vuin) = idmap
        .retrieve_virtual_value(USER_OPENID)
        .expect("分配用户虚拟号");
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

    // P86d-2：回执改走 `SendC2CResponse` 口径——`message_id` 是 echo 派生数字（numeric 模式），
    // 顶层 `user_id` 是数字虚拟号（Go 私聊回执恒数字结构）
    let message_id = echo.store_cache_in_memory("s-1").expect("派生行号");
    assert_eq!(
        serde_json::to_string(&response).expect("序列化失败"),
        format!(
            r#"{{"data":{{"message_id":{message_id}}},"echo":"e-s","message":"","retcode":0,"status":"ok","user_id":{user_vuin}}}"#
        ),
        "SSE 回执（Go `SendC2CResponse`）"
    );

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
    let (_dir, api, state, echo, idmap) =
        state_with_idmap("  app_id: 12345\n  forward_msg_limit: 5\n");
    let (_, group_vuin) = idmap.retrieve_virtual_value(OPENID).expect("分配群虚拟号");
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
    // 顶层 `group_id` 是数字虚拟号：Go 转发直接调 `SendResponse`、不经 `string_ob11` 分派
    let whole = serde_json::to_value(&response).expect("序列化失败");
    assert_eq!(whole["group_id"], json!(group_vuin), "实际回执: {whole}");
    let data = response.data.expect("data");
    // P86d-2：`message_ids` 元素过 `message_id_value`——numeric 模式是 echo 派生虚拟号
    let first_id = echo.store_cache_in_memory("f-1").expect("派生行号");
    let second_id = echo.store_cache_in_memory("f-3").expect("派生行号");
    assert_eq!(
        data["message_ids"],
        json!([first_id, second_id]),
        "已发不回滚"
    );
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
    let (_dir, api, state, echo, idmap) =
        state_with_idmap("  app_id: 12345\n  forward_msg_limit: 5\n");
    let (_, user_vuin) = idmap
        .retrieve_virtual_value(USER_OPENID)
        .expect("分配用户虚拟号");
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
    // 私聊转发是 Rust 扩展 action（Go 未注册）：顶层 ID 照私聊回执口径给数字虚拟号
    let whole = serde_json::to_value(&response).expect("序列化失败");
    assert_eq!(whole["user_id"], json!(user_vuin));
    assert_eq!(whole["group_id"], Value::Null, "私聊回执不带 group_id");
    let data = response.data.expect("data");
    let first_id = echo.store_cache_in_memory("p-1").expect("派生行号");
    let second_id = echo.store_cache_in_memory("p-2").expect("派生行号");
    assert_eq!(data["message_ids"], json!([first_id, second_id]));
    let sent = api.sent();
    assert_eq!(sent.len(), 2);
    assert!(matches!(
        &sent[1],
        Sent::C2c(openid, message)
            if openid == USER_OPENID && message.content.as_deref() == Some("第二条")
    ));
}

// ---- P86d-2：SSE / 合并转发回执的 `string_ob11` 双模式渲染 ----

/// `string_ob11` 下 SSE 回执的 `message_id` 与单条发送同口径：回上游原串（Go
/// `SendC2CResponse` 恒派生数字，Rust 给的是超集，已登记），`user_id` 仍是数字虚拟号。
#[tokio::test]
async fn sse_string_mode_receipt_keeps_upstream_id() {
    let (_dir, api, state, _echo, idmap) =
        state_with_idmap("  app_id: 12345\n  string_ob11: true\n");
    let (_, user_vuin) = idmap
        .retrieve_virtual_value(USER_OPENID)
        .expect("分配用户虚拟号");
    api.push_c2c(api.ok("s-str"));

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_private_msg_sse",
            json!({
                "user_id": USER_OPENID,
                "message": { "content": "片段", "state": 1 }
            }),
            None,
        ))
        .await;

    assert_eq!(
        serde_json::to_string(&response).expect("序列化失败"),
        format!(
            r#"{{"data":{{"message_id":"s-str"}},"echo":null,"message":"","retcode":0,"status":"ok","user_id":{user_vuin}}}"#
        ),
        "string 模式的 SSE 回执"
    );
}

/// 合并转发的 `message_ids` 元素按 `string_ob11` 双模式渲染，但顶层 `group_id` 恒数字：
/// Go 的转发 handler 直接调 `SendResponse`（`send_group_forward_msg.go:75`），不过
/// `send_group_msg.go:1020` 的 `string_ob11` 分派。`failed` 清单是 Rust 独有增强，两种模式都保留。
#[tokio::test]
async fn forward_receipts_follow_mode_for_message_ids_but_group_id_is_numeric() {
    for (settings, string_mode) in [
        ("  app_id: 12345\n  forward_msg_limit: 5\n", false),
        (
            "  app_id: 12345\n  forward_msg_limit: 5\n  string_ob11: true\n",
            true,
        ),
    ] {
        let (_dir, api, state, echo, idmap) = state_with_idmap(settings);
        let (_, group_vuin) = idmap.retrieve_virtual_value(OPENID).expect("分配群虚拟号");
        api.push_group(api.ok("m-f1"));
        api.push_group(Outcome::Fail);
        let dispatcher = state
            .dispatcher()
            .with_forward_interval(std::time::Duration::ZERO);

        let response = dispatcher
            .dispatch(&request(
                "send_group_forward_msg",
                json!({
                    "group_id": OPENID,
                    "messages": [
                        {"data": {"content": "一"}},
                        {"data": {"content": "二"}}
                    ]
                }),
                None,
            ))
            .await;

        let whole = serde_json::to_value(&response).expect("序列化失败");
        assert_eq!(
            whole["group_id"],
            json!(group_vuin),
            "转发回执的 group_id 恒数字（{settings:?}）"
        );
        let data = response.data.expect("data");
        let expected = if string_mode {
            json!("m-f1")
        } else {
            json!(echo.store_cache_in_memory("m-f1").expect("派生行号"))
        };
        assert_eq!(
            data["message_ids"],
            json!([expected]),
            "string 模式回原串 / numeric 模式派生数字（{settings:?}）"
        );
        assert_eq!(
            data["failed"].as_array().map(Vec::len),
            Some(1),
            "failed 清单两种模式都保留（{settings:?}）"
        );
    }
}

/// 正文被 CQ 管线吃空的转发 node 在**规划层**就被剔除（`gensokyo-actions/src/send.rs:692`：
/// 空正文 + 无 plan 不入队），因此 `message_ids` 里永远拿不到空回执 `0`——该规则由单条用例
/// [`empty_receipt_is_numeric_zero_in_both_modes`] 覆盖。整次转发无 node 可发时计划里也没有
/// 收件人（`ForwardRequest` 无顶层 `target`），顶层 ID 留空是登记在案的已知边界。
#[tokio::test]
async fn forward_with_only_empty_nodes_reports_no_ids() {
    for settings in [
        "  app_id: 12345\n  forward_msg_limit: 5\n",
        "  app_id: 12345\n  forward_msg_limit: 5\n  string_ob11: true\n",
    ] {
        let (_dir, api, state) = state(settings);
        let dispatcher = state
            .dispatcher()
            .with_forward_interval(std::time::Duration::ZERO);

        let response = dispatcher
            .dispatch(&request(
                "send_group_forward_msg",
                json!({
                    "group_id": OPENID,
                    "messages": [{"data": {"content": "[CQ:card]"}}]
                }),
                None,
            ))
            .await;

        assert_eq!(response.retcode, 0, "纯码 node 仍回执成功（{settings:?}）");
        assert!(
            api.sent().is_empty(),
            "纯码 node 不应触达上游（{settings:?}）"
        );
        let whole = serde_json::to_value(&response).expect("序列化失败");
        let data = response.data.expect("data");
        assert_eq!(
            data["message_ids"],
            json!([]),
            "空正文 node 不入队（{settings:?}）"
        );
        assert_eq!(
            data["failed"],
            json!([]),
            "空正文 node 不入失败册（{settings:?}）"
        );
        assert_eq!(
            whole["group_id"],
            Value::Null,
            "已知边界：无可发 node 时计划里没有收件人（{settings:?}）"
        );
    }
}

// ---- P86a：回执 message_id 的类型与空回执（Go `SendResponse` 口径） ----

/// `string_ob11 = true`：`data.message_id` 是上游原 ID 字符串（Go `SendResponseSB`）。
#[tokio::test]
async fn string_ob11_receipt_keeps_upstream_message_id() {
    let (_dir, api, state) = state("  app_id: 12345\n  string_ob11: true\n");
    api.push_group(api.ok("m-str"));

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            json!({ "group_id": OPENID, "message": "hello" }),
            None,
        ))
        .await;

    assert_eq!(response.retcode, 0);
    assert_eq!(
        response.data,
        Some(json!({ "message_id": "m-str" })),
        "string_ob11=true 时应回上游原 ID 字符串"
    );
}

/// 空消息回执（CQ 管线吃空正文）：两种模式下都是数字 `0`（`message_parser.go:375` / `:599`）。
#[tokio::test]
async fn empty_receipt_is_numeric_zero_in_both_modes() {
    for settings in ["app_id: 12345\n", "  app_id: 12345\n  string_ob11: true\n"] {
        let (_dir, api, state) = state(settings);
        let response = state
            .dispatcher()
            .dispatch(&request(
                "send_group_msg",
                json!({ "group_id": OPENID, "message": "[CQ:card]" }),
                None,
            ))
            .await;

        assert_eq!(response.retcode, 0, "纯码空消息仍回执成功");
        assert!(api.sent().is_empty(), "纯码空消息不应触达上游");
        assert_eq!(
            response.data,
            Some(json!({ "message_id": 0 })),
            "空回执必须是数字 0（配置 {settings:?}）"
        );
    }
}

// ---- P86b：响应外层信封（Go `ServerResponse` / `ServerResponseSB` 口径） ----

/// 群发送 numeric：顶层 `group_id` 是数字虚拟号；整串键序与 Go 的 map 字母序一致。
#[tokio::test]
async fn group_envelope_is_alphabetical_in_numeric_mode() {
    let (_dir, api, state, echo, idmap) = state_with_idmap("app_id: 12345\n");
    let (_, group_vuin) = idmap.retrieve_virtual_value(OPENID).expect("分配群虚拟号");
    api.push_group(api.ok("m-g1"));

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            json!({ "group_id": group_vuin, "message": "hello" }),
            None,
        ))
        .await;

    let message_id = echo.store_cache_in_memory("m-g1").expect("派生行号");
    assert_eq!(
        serde_json::to_string(&response).expect("序列化失败"),
        format!(
            r#"{{"data":{{"message_id":{message_id}}},"echo":null,"group_id":{group_vuin},"message":"","retcode":0,"status":"ok"}}"#
        ),
        "numeric 模式的群回执（Go `SendResponse`）"
    );
    match &api.sent()[0] {
        Sent::Group(openid, _) => assert_eq!(openid, OPENID, "虚拟号要反查回群 OpenID"),
        other => panic!("应为群消息: {other:?}"),
    }
}

/// 群发送 string 模式：`group_id` 是请求原串（`ServerResponseSB.GroupID` 是 string）。
#[tokio::test]
async fn group_envelope_is_alphabetical_in_string_mode() {
    let (_dir, api, state) = state("  app_id: 12345\n  string_ob11: true\n");
    api.push_group(api.ok("m-g2"));

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            json!({ "group_id": OPENID, "message": "hello" }),
            None,
        ))
        .await;

    assert_eq!(
        serde_json::to_string(&response).expect("序列化失败"),
        format!(
            r#"{{"data":{{"message_id":"m-g2"}},"echo":null,"group_id":"{OPENID}","message":"","retcode":0,"status":"ok"}}"#
        ),
        "string 模式的群回执（Go `SendResponseSB`）"
    );
}

/// 私聊 numeric：顶层 `user_id` 是数字虚拟号，`group_id` 不出现；带 echo 时 echo 落在 data 后。
#[tokio::test]
async fn private_envelope_carries_numeric_user_id() {
    let (_dir, api, state, echo, idmap) = state_with_idmap("app_id: 12345\n");
    let (_, user_vuin) = idmap
        .retrieve_virtual_value(USER_OPENID)
        .expect("分配用户虚拟号");
    api.push_c2c(api.ok("m-p1"));

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_private_msg",
            json!({ "user_id": user_vuin, "message": "hi" }),
            Some(json!("e-p1")),
        ))
        .await;

    let message_id = echo.store_cache_in_memory("m-p1").expect("派生行号");
    assert_eq!(
        serde_json::to_string(&response).expect("序列化失败"),
        format!(
            r#"{{"data":{{"message_id":{message_id}}},"echo":"e-p1","message":"","retcode":0,"status":"ok","user_id":{user_vuin}}}"#
        ),
        "私聊回执（Go `SendC2CResponse`）"
    );
}

/// 私聊 string 模式：`user_id` 仍是**数字**虚拟号——照抄 Go 的内部不一致（`SendC2CResponse`
/// 恒用数字结构），`message_id` 才是字符串。
#[tokio::test]
async fn private_envelope_keeps_numeric_user_id_in_string_mode() {
    let (_dir, api, state, _echo, idmap) =
        state_with_idmap("  app_id: 12345\n  string_ob11: true\n");
    let (_, user_vuin) = idmap
        .retrieve_virtual_value(USER_OPENID)
        .expect("分配用户虚拟号");
    api.push_c2c(api.ok("m-p2"));

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_private_msg",
            json!({ "user_id": user_vuin, "message": "hi" }),
            None,
        ))
        .await;

    assert_eq!(
        serde_json::to_string(&response).expect("序列化失败"),
        format!(
            r#"{{"data":{{"message_id":"m-p2"}},"echo":null,"message":"","retcode":0,"status":"ok","user_id":{user_vuin}}}"#
        ),
        "string 模式私聊的 user_id 仍是数字（Go 缺陷照抄）"
    );
}

/// 空消息回执也带顶层 ID：Go 在判空前就用同一 `message.Params` 构造 `group_id` / `user_id`。
#[tokio::test]
async fn empty_receipt_still_carries_target_id() {
    let (_dir, api, state, _echo, idmap) = state_with_idmap("app_id: 12345\n");
    let (_, group_vuin) = idmap.retrieve_virtual_value(OPENID).expect("分配群虚拟号");
    let (_, user_vuin) = idmap
        .retrieve_virtual_value(USER_OPENID)
        .expect("分配用户虚拟号");

    let group = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            json!({ "group_id": group_vuin, "message": "[CQ:card]" }),
            None,
        ))
        .await;
    assert_eq!(
        serde_json::to_string(&group).expect("序列化失败"),
        format!(
            r#"{{"data":{{"message_id":0}},"echo":null,"group_id":{group_vuin},"message":"","retcode":0,"status":"ok"}}"#
        ),
        "群空回执"
    );

    let private = state
        .dispatcher()
        .dispatch(&request(
            "send_private_msg",
            json!({ "user_id": user_vuin, "message": "[CQ:card]" }),
            None,
        ))
        .await;
    assert_eq!(
        serde_json::to_string(&private).expect("序列化失败"),
        format!(
            r#"{{"data":{{"message_id":0}},"echo":null,"message":"","retcode":0,"status":"ok","user_id":{user_vuin}}}"#
        ),
        "私聊空回执"
    );
    assert!(api.sent().is_empty(), "纯码空消息不应触达上游");
}

/// 非发送类 action 的信封里没有 `group_id` / `user_id`，但 `message` / `echo` 恒在。
#[tokio::test]
async fn non_send_actions_keep_the_plain_envelope() {
    let (_dir, _api, state) = state("app_id: 12345\n");

    let response = state
        .dispatcher()
        .dispatch(&request("get_status", json!({}), Some(json!("e-s"))))
        .await;

    let value = serde_json::to_value(&response).expect("序列化失败");
    assert!(
        value.get("group_id").is_none(),
        "非发送类不带 group_id: {value}"
    );
    assert!(
        value.get("user_id").is_none(),
        "非发送类不带 user_id: {value}"
    );
    assert_eq!(value["message"], json!(""), "成功回执的 message 是空串");
    assert_eq!(value["echo"], json!("e-s"), "echo 原样回传");
}

// ---- 回执虚拟号闭环：`send_*` 发出的 message_id 必须能被 `delete_msg` 反查回真实 ID ----

/// 群聊：用发送回执里的虚拟号立刻撤回，必须还原成上游真实 ID 并成功。
///
/// 回归自活体实测（`delete_msg` 回 `1400 反查 message_id 失败: 键不存在: 虚拟消息 ID=…`）：
/// 回执虚拟号登记在 echo 行号反表，撤回原来只查 idmap，两条来源对不上。
#[tokio::test]
async fn group_receipt_message_id_can_be_retracted() {
    let (_dir, api, state, _echo, _idmap) = state_with_idmap("app_id: 12345\n");
    api.push_group(api.ok("REAL-GROUP-MID"));

    let sent = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            json!({ "group_id": OPENID, "message": "hi" }),
            Some(json!("e-send")),
        ))
        .await;
    assert_eq!(sent.retcode, 0);
    let message_id = sent.data.expect("成功回执应带 data")["message_id"].clone();
    assert!(
        message_id.is_i64(),
        "numeric 模式的回执虚拟号必须是数字: {message_id}"
    );

    let deleted = state
        .dispatcher()
        .dispatch(&request(
            "delete_msg",
            json!({ "message_id": message_id, "group_id": OPENID }),
            Some(json!("e-del")),
        ))
        .await;
    assert_eq!(
        deleted.retcode, 0,
        "撤回自己刚发的消息不应失败: {:?}",
        deleted.message
    );
    assert_eq!(
        api.deleted(),
        vec![RetractRequest {
            group_openid: Some(OPENID.to_owned()),
            user_openid: None,
            message_id: "REAL-GROUP-MID".to_owned(),
        }],
        "撤回必须打到上游真实消息 ID"
    );
}

/// 私聊（C2C）：同一条反查链，`user_id` 分支。
#[tokio::test]
async fn private_receipt_message_id_can_be_retracted() {
    let (_dir, api, state, _echo, _idmap) = state_with_idmap("app_id: 12345\n");
    api.push_c2c(api.ok("REAL-C2C-MID"));

    let sent = state
        .dispatcher()
        .dispatch(&request(
            "send_private_msg",
            json!({ "user_id": USER_OPENID, "message": "hi" }),
            Some(json!("e-send")),
        ))
        .await;
    assert_eq!(sent.retcode, 0);
    let message_id = sent.data.expect("成功回执应带 data")["message_id"].clone();

    let deleted = state
        .dispatcher()
        .dispatch(&request(
            "delete_msg",
            json!({ "message_id": message_id, "user_id": USER_OPENID }),
            Some(json!("e-del")),
        ))
        .await;
    assert_eq!(
        deleted.retcode, 0,
        "撤回自己刚发的私聊消息不应失败: {:?}",
        deleted.message
    );
    assert_eq!(
        api.deleted(),
        vec![RetractRequest {
            group_openid: None,
            user_openid: Some(USER_OPENID.to_owned()),
            message_id: "REAL-C2C-MID".to_owned(),
        }]
    );
}

/// 从未出现过的虚拟号：两个来源都未命中，仍是 `1400` + 「键不存在」，且不触达上游。
#[tokio::test]
async fn delete_msg_unknown_message_id_still_fails_1400() {
    let (_dir, api, state, _echo, _idmap) = state_with_idmap("app_id: 12345\n");

    let deleted = state
        .dispatcher()
        .dispatch(&request(
            "delete_msg",
            json!({ "message_id": "987654321", "group_id": OPENID }),
            Some(json!("e-del")),
        ))
        .await;

    assert_eq!(deleted.retcode, RETCODE_BAD_REQUEST);
    let message = deleted.message.unwrap_or_default();
    assert!(
        message.contains("键不存在"),
        "未知虚拟号必须报键不存在，实际: {message}"
    );
    assert!(api.deleted().is_empty(), "反查失败不应触达上游");
}
