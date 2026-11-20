//! P83 出站管理 CQ 码（`member` / `set_group` / `remove`）的运行期端到端用例。
//!
//! 覆盖设计 §6.2 的 ④（参数反转义）、⑨（上游失败）、⑪（多码顺序 + 中间失败）、
//! ⑫（私聊 `set_group` 留字面 / `remove` 走 C2C）、⑬（`member` add 取 event_id /
//! remove 清空）。链路走 `Dispatcher` → `send_*` handler → deferred → `cq_admin`，
//! 断言落在 `FakeQqApi.calls` 的**入参记录**上，全程不触网。
//!
//! # 装置说明
//!
//! `gensokyo-actions/src/testing.rs` 的 `FakeQqApi` 挂在 `#[cfg(test)] mod testing`
//! （`crates/gensokyo-actions/src/lib.rs:82`）下，集成测试 target 链接不到它；这里按它的
//! 形状复刻一份：`Call` 记录 + 槽位按序注入 + 未配置即 panic（「忘了准备 fixture」立刻炸，
//! 而不是静默变成 1500）。

mod support;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use gensokyo_actions::{ActionContext, KickResult, QqApi, QqFuture, RetractRequest};
use gensokyo_core::{Error, Result};
use gensokyo_protocol::action::{ActionRequest, Status};
use gensokyo_qq::dto::{
    ApprovalJoinRequest, BotInGroupState, GroupInfo, GroupMember, GroupMemberList, JoinRequestList,
    MemberBlacklistRequest, MemberBlacklistResponse, MessageResponse, MessageToCreate,
};
use gensokyo_runtime::{AppState, Inbound, RETCODE_INTERNAL};
use gensokyo_storage::echo::echo_key_event_id_v2;
use gensokyo_storage::{EchoStore, IdMapOptions, IdMapStore};
use serde_json::{Value, json};
use tempfile::TempDir;

use support::config;

/// 群 OpenID（32 位，直通上游、不需 idmap）。
const GROUP: &str = "0123456789abcdef0123456789abcdef";
/// 私聊目标用户 OpenID。
const USER: &str = "fedcba9876543210fedcba9876543210";
/// 被管理成员 OpenID（32 位直通）。
const MEMBER_A: &str = "aaaabbbbccccddddaaaabbbbccccdddd";
/// 第二个被管理成员 OpenID。
const MEMBER_B: &str = "11112222333344441111222233334444";

/// 一次上游调用的入参记录（只收录管理码会触达的七个方法）。
#[derive(Debug, Clone, PartialEq)]
enum Call {
    /// `set_group_ban(group_openid, member_openid, duration_secs)`
    Ban(String, String, i64),
    /// `set_group_kick(group_openid, member_openids, add_blacklist)`
    Kick(String, Vec<String>, bool),
    /// `delete_msg(group_openid, user_openid, message_id)`
    DeleteMsg(Option<String>, Option<String>, String),
    /// `approve_join_request(group_openid, member_openid, request)`
    JoinApproval(String, String, ApprovalJoinRequest),
    /// `update_member_blacklist(group_openid, request)`
    BlacklistUpdate(String, MemberBlacklistRequest),
    /// `send_group_message(group_openid, msg)`
    SendGroup(String, MessageToCreate),
    /// `send_c2c_message(user_openid, msg)`
    SendC2c(String, MessageToCreate),
}

/// 记录入参、按队列注入结果的 `QqApi` 替身。
///
/// 槽位为空即 panic：既防「忘配置」，也把「不该发生的调用」变成明确的测试失败。
#[derive(Debug, Default)]
struct FakeQqApi {
    calls: Mutex<Vec<Call>>,
    ban: Mutex<VecDeque<Result<()>>>,
    kick: Mutex<VecDeque<Result<KickResult>>>,
    delete: Mutex<VecDeque<Result<()>>>,
    join_approval: Mutex<VecDeque<Result<()>>>,
    blacklist: Mutex<VecDeque<Result<MemberBlacklistResponse>>>,
    group_send: Mutex<VecDeque<Result<MessageResponse>>>,
    c2c_send: Mutex<VecDeque<Result<MessageResponse>>>,
}

impl FakeQqApi {
    fn record(&self, call: Call) {
        self.calls.lock().expect("调用记录锁未中毒").push(call);
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("调用记录锁未中毒").clone()
    }

    /// 上游成功回执（`id` 只是占位，管理码用例不关心回执 ID）。
    fn ok_receipt() -> Result<MessageResponse> {
        Ok(MessageResponse {
            id: "m-1".to_owned(),
            timestamp: "2026-11-15T00:00:00Z".to_owned(),
        })
    }
}

/// 取走一次预置结果；未配置即 panic。
fn take<T>(slot: &Mutex<VecDeque<Result<T>>>) -> Result<T> {
    slot.lock()
        .expect("响应槽锁未中毒")
        .pop_front()
        .expect("测试未配置该方法的响应")
}

impl QqApi for FakeQqApi {
    /// 管理码用例不查群信息：明确报「未实现」而不是 panic。
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

    fn set_group_ban(
        &self,
        group_openid: &str,
        member_openid: &str,
        duration_secs: i64,
    ) -> QqFuture<'_, ()> {
        let group_openid = group_openid.to_owned();
        let member_openid = member_openid.to_owned();
        Box::pin(async move {
            self.record(Call::Ban(group_openid, member_openid, duration_secs));
            take(&self.ban)
        })
    }

    fn set_group_kick(
        &self,
        group_openid: &str,
        member_openids: &[String],
        add_blacklist: bool,
    ) -> QqFuture<'_, KickResult> {
        let group_openid = group_openid.to_owned();
        let member_openids = member_openids.to_vec();
        Box::pin(async move {
            self.record(Call::Kick(group_openid, member_openids, add_blacklist));
            take(&self.kick)
        })
    }

    fn delete_msg(&self, target: RetractRequest) -> QqFuture<'_, ()> {
        Box::pin(async move {
            self.record(Call::DeleteMsg(
                target.group_openid,
                target.user_openid,
                target.message_id,
            ));
            take(&self.delete)
        })
    }

    fn approve_join_request(
        &self,
        group_openid: &str,
        member_openid: &str,
        request: &ApprovalJoinRequest,
    ) -> QqFuture<'_, ()> {
        let group_openid = group_openid.to_owned();
        let member_openid = member_openid.to_owned();
        let request = request.clone();
        Box::pin(async move {
            self.record(Call::JoinApproval(group_openid, member_openid, request));
            take(&self.join_approval)
        })
    }

    fn update_member_blacklist(
        &self,
        group_openid: &str,
        request: &MemberBlacklistRequest,
    ) -> QqFuture<'_, MemberBlacklistResponse> {
        let group_openid = group_openid.to_owned();
        let request = request.clone();
        Box::pin(async move {
            self.record(Call::BlacklistUpdate(group_openid, request));
            take(&self.blacklist)
        })
    }

    fn send_group_message(
        &self,
        group_openid: String,
        msg: MessageToCreate,
    ) -> QqFuture<'_, MessageResponse> {
        Box::pin(async move {
            self.record(Call::SendGroup(group_openid, msg));
            take(&self.group_send)
        })
    }

    fn send_c2c_message(
        &self,
        user_openid: String,
        msg: MessageToCreate,
    ) -> QqFuture<'_, MessageResponse> {
        Box::pin(async move {
            self.record(Call::SendC2c(user_openid, msg));
            take(&self.c2c_send)
        })
    }
}

/// 装配「内置 action + actions 全集」的 `AppState`、共享替身、echo 与 app_id。
///
/// `settings` 必须两格缩进（`support::config` 的既有坑）：`  app_id: 12345` 才真正落进
/// `settings:` 段；app_id 由同一份快照取值，`echo_key_event_id_v2` 的键才不会拼错。
fn state() -> (TempDir, Arc<FakeQqApi>, AppState, Arc<EchoStore>, String) {
    let (manager, dir) = config("  app_id: 12345\n");
    let app_id = manager.snapshot().get_app_id().to_string();
    let api = Arc::new(FakeQqApi::default());
    let qq: Arc<dyn QqApi> = api.clone();
    let context = Arc::new(ActionContext::new(qq, Arc::clone(&manager)));
    let echo = Arc::new(EchoStore::new());
    let state = AppState::with_actions(manager, context).with_echo(Arc::clone(&echo));
    (dir, api, state, echo, app_id)
}

fn request(action: &str, params: Value, echo: Option<Value>) -> ActionRequest<Value> {
    ActionRequest {
        action: action.to_owned(),
        params: Some(params),
        echo,
    }
}

/// ④ 参数反转义端到端：CQ 实体还原后才进上游 payload。
///
/// `reason=a&#44;b` 在 `cqcode` 里反转义成 `a,b`（`cqcode.rs:78-125`），
/// 经 `AdminOp::SetGroupOp::AddRequest.reason` 落到 `ApprovalJoinRequest.reject_reason`。
#[tokio::test]
async fn add_request_reason_unescapes_cq_entities_end_to_end() {
    let (_dir, api, state, _echo, _app) = state();
    api.join_approval.lock().expect("槽锁").push_back(Ok(()));

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            json!({
                "group_id": GROUP,
                "message": format!(
                    "[CQ:set_group,action=add_request,user_id={MEMBER_A},flag=f-1,reason=a&#44;b]"
                ),
            }),
            None,
        ))
        .await;

    assert_eq!(
        response.retcode, 0,
        "审批成功应回 0: {:?}",
        response.message
    );
    assert_eq!(
        api.calls(),
        vec![Call::JoinApproval(
            GROUP.to_owned(),
            MEMBER_A.to_owned(),
            ApprovalJoinRequest {
                op: "approve".to_owned(),
                join_request_id: "f-1".to_owned(),
                reject_reason: "a,b".to_owned(),
                add_to_member_blacklist: false,
            },
        )],
        "reason 必须在进上游前还原成 a,b"
    );
}

/// ⑨ 上游失败注入：管理码的上游错误按 crate 统一口径回 `1500`，零发送。
#[tokio::test]
async fn admin_upstream_failure_is_1500() {
    let (_dir, api, state, _echo, _app) = state();
    api.ban
        .lock()
        .expect("槽锁")
        .push_back(Err(Error::Upstream {
            code: 40_034_025,
            message: "请求参数event_id无效".to_owned(),
        }));

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            json!({
                "group_id": GROUP,
                "message": format!("[CQ:set_group,action=ban,user_id={MEMBER_A},duration=60]"),
            }),
            Some(json!("e-9")),
        ))
        .await;

    assert_eq!(response.status, Status::Failed);
    assert_eq!(response.retcode, RETCODE_INTERNAL, "上游失败统一 1500");
    assert_eq!(response.echo, Some(json!("e-9")));
    assert!(
        response
            .message
            .as_deref()
            .is_some_and(|message| message.contains("请求参数event_id无效")),
        "上游错误文案应回传: {:?}",
        response.message
    );
    assert_eq!(
        api.calls(),
        vec![Call::Ban(GROUP.to_owned(), MEMBER_A.to_owned(), 60)],
        "失败发生在 ban 本身，不该有其它调用"
    );
}

/// ⑪ 多码按到达顺序串行；中间失败不中断后者，整体取第一个错误。
#[tokio::test]
async fn multi_code_keeps_arrival_order_and_survives_middle_failure() {
    let (_dir, api, state, _echo, _app) = state();
    api.kick
        .lock()
        .expect("槽锁")
        .push_back(Err(Error::Upstream {
            code: 40_054_005,
            message: "kick 被拒".to_owned(),
        }));
    api.ban.lock().expect("槽锁").push_back(Ok(()));
    api.blacklist
        .lock()
        .expect("槽锁")
        .push_back(Ok(MemberBlacklistResponse::default()));

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            json!({
                "group_id": GROUP,
                "message": format!(
                    "[CQ:set_group,action=kick,user_id={MEMBER_A}]\
                     [CQ:set_group,action=ban,user_id={MEMBER_B},duration=60]\
                     [CQ:set_group,action=blacklist_add,user_id={MEMBER_A}]"
                ),
            }),
            None,
        ))
        .await;

    assert_eq!(
        response.retcode, RETCODE_INTERNAL,
        "首错（kick）决定整体 retcode"
    );
    assert!(
        response
            .message
            .as_deref()
            .is_some_and(|message| message.contains("kick 被拒")),
        "整体文案应是第一个错误: {:?}",
        response.message
    );
    assert_eq!(
        api.calls(),
        vec![
            Call::Kick(GROUP.to_owned(), vec![MEMBER_A.to_owned()], false),
            Call::Ban(GROUP.to_owned(), MEMBER_B.to_owned(), 60),
            Call::BlacklistUpdate(
                GROUP.to_owned(),
                MemberBlacklistRequest {
                    op: "add".to_owned(),
                    member_openids: vec![MEMBER_A.to_owned()],
                },
            ),
        ],
        "调用序列必须严格按到达顺序，且 kick 失败后 ban / blacklist 仍执行"
    );
}

/// ⑫-a 私聊 `set_group` 不执行，规范化字面留在正文（正文保留，不静默丢弃）。
#[tokio::test]
async fn private_set_group_stays_literal_and_body_is_kept() {
    let (_dir, api, state, _echo, _app) = state();
    api.c2c_send
        .lock()
        .expect("槽锁")
        .push_back(FakeQqApi::ok_receipt());

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_private_msg",
            json!({
                "user_id": USER,
                "message": format!(
                    "hi[CQ:set_group,action=ban,user_id={MEMBER_A},duration=60]"
                ),
            }),
            None,
        ))
        .await;

    assert_eq!(
        response.retcode, 0,
        "私聊留字面不是错误: {:?}",
        response.message
    );
    let calls = api.calls();
    assert_eq!(
        calls.len(),
        1,
        "只应有 C2C 发送，零管理码上游调用: {calls:?}"
    );
    match &calls[0] {
        Call::SendC2c(openid, message) => {
            assert_eq!(openid, USER);
            let content = message.content.as_deref().expect("正文应保留");
            assert!(
                content.starts_with("hi[CQ:set_group"),
                "正文前缀应保留: {content:?}"
            );
            assert!(
                content.contains("action=ban") && content.contains(MEMBER_A),
                "管理码应按 CQ 字面回写: {content:?}"
            );
        }
        other => panic!("应发 C2C 消息: {other:?}"),
    }
}

/// ⑫-b 私聊 `remove` 走 C2C 撤回（`user_openid` 分派，不是 Go 的硬编码群撤回）。
///
/// **当前失败 = 产品缺陷（P83）**：handler 把字符串 `[CQ:remove,…]` 解析进 `admin_ops`，
/// 但 deferred 计划经 `Segment` 的 serde 往返（`gensokyo-protocol/src/segment.rs:336-350` 只写
/// `data` / `type`，`pub(crate) raw` 不上线）后，runtime 重新解析只能看到**段形态**
/// `remove`；按设计 §0.2 / Q5「段路径 remove 不进 admin_ops」，它被回退成字面 ⇒
/// `delete_msg` 从未被调用，正文多出一条规范化 CQ 字面并按普通 C2C 发出。
/// 期望：`DeleteMsg(None, Some(<目标用户>), <msg_id>)` 且零发送。
#[tokio::test]
async fn private_remove_retracts_through_c2c() {
    let (_dir, api, state, _echo, _app) = state();
    api.delete.lock().expect("槽锁").push_back(Ok(()));
    // 若实现退化成「字面留在正文」，会多出一次 C2C 发送；预置回执让用例以断言而不是
    // 替身 panic 收场（panic 会把真实差异藏起来）。
    api.c2c_send
        .lock()
        .expect("槽锁")
        .push_back(FakeQqApi::ok_receipt());

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_private_msg",
            json!({
                "user_id": USER,
                "message": format!("[CQ:remove,user_id={MEMBER_A},msg_id=m-real-1]"),
            }),
            None,
        ))
        .await;

    assert_eq!(
        response.retcode, 0,
        "撤回成功应回 0: {:?}",
        response.message
    );
    assert_eq!(
        api.calls(),
        vec![Call::DeleteMsg(
            None,
            Some(USER.to_owned()),
            "m-real-1".to_owned()
        )],
        "私聊 remove 必须以 user_openid 走 C2C 撤回"
    );
}

/// ⑫-c 群聊 `remove` 不带 `msg_id`：反查该用户在该群的最新消息（Go `cqcode.go:350-369`）。
///
/// 键由入站写入（Go `ProcessGroupMessage.go:179` / `ProcessGroupNormalMessage.go:168`），
/// 所以本用例先把一条 `GROUP_AT_MESSAGE_CREATE` 喂给入站、再发 `[CQ:remove]`：写入缺失时
/// 会落在「没有可撤回的最新消息」，撤回不进上游。
#[tokio::test]
async fn group_remove_without_msg_id_uses_the_latest_inbound_message() {
    const RAW_MSG_ID: &str = "m-real-latest";
    let (manager, dir) = config("  app_id: 12345\n");
    let idmap = Arc::new(
        IdMapStore::open(
            IdMapOptions::new(
                dir.path().join("openid-map.redb"),
                dir.path().join("msgid-map.redb"),
            )
            .with_app_id("12345"),
        )
        .expect("打开临时 idmap 失败"),
    );
    let api = Arc::new(FakeQqApi::default());
    let qq: Arc<dyn QqApi> = api.clone();
    let context =
        Arc::new(ActionContext::new(qq, Arc::clone(&manager)).with_idmap(Arc::clone(&idmap)));
    let state = AppState::with_actions(manager, context).with_idmap(Arc::clone(&idmap));

    Inbound::from_state(&state)
        .convert(
            "GROUP_AT_MESSAGE_CREATE",
            Some(&json!({
                "id": RAW_MSG_ID,
                "group_id": GROUP,
                "content": "hello",
                "author": {"id": MEMBER_A},
            })),
            None,
        )
        .expect("群消息应可转换");
    assert_eq!(
        idmap
            .get_latest_msg_id(GROUP, MEMBER_A)
            .expect("入站应写下「群 × 用户」最新消息"),
        RAW_MSG_ID
    );

    api.delete.lock().expect("槽锁").push_back(Ok(()));
    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            json!({
                "group_id": GROUP,
                "message": format!("[CQ:remove,group_id={GROUP},user_id={MEMBER_A}]"),
            }),
            None,
        ))
        .await;

    assert_eq!(
        response.retcode, 0,
        "缺 msg_id 应反查到最新消息: {:?}",
        response.message
    );
    assert_eq!(
        api.calls(),
        vec![Call::DeleteMsg(
            Some(GROUP.to_owned()),
            None,
            RAW_MSG_ID.to_owned()
        )],
        "群撤回必须以最新真实 msg_id 走上游"
    );
}

/// ⑬-a `member add` 以该群事件 ID 作被动回复（echo 预置键命中）。
#[tokio::test]
async fn member_add_replies_with_the_group_event_id() {
    let (_dir, api, state, echo, app_id) = state();
    echo.add_event_id_v2(&app_id, GROUP, "event-1");
    assert_eq!(
        echo.get_event_id_by_key(&echo_key_event_id_v2(&app_id, GROUP)),
        "event-1",
        "预置键必须与 cq_admin 的查键（echo_key_event_id_v2）一致"
    );
    api.group_send
        .lock()
        .expect("槽锁")
        .push_back(FakeQqApi::ok_receipt());

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            json!({
                "group_id": GROUP,
                "message": format!("欢迎[CQ:member,type=add,user_id={MEMBER_A}]"),
            }),
            None,
        ))
        .await;

    assert_eq!(
        response.retcode, 0,
        "入群回复应成功: {:?}",
        response.message
    );
    let calls = api.calls();
    assert_eq!(
        calls.len(),
        1,
        "member 码零上游调用（只有本条发送）: {calls:?}"
    );
    match &calls[0] {
        Call::SendGroup(openid, message) => {
            assert_eq!(openid, GROUP);
            assert_eq!(message.content.as_deref(), Some("欢迎"));
            assert_eq!(
                message.event_id.as_deref(),
                Some("event-1"),
                "应以该群的入群 event_id 作被动回复"
            );
            assert!(message.msg_id.is_none(), "event_id 与 msg_id 互斥");
        }
        other => panic!("应发群消息: {other:?}"),
    }
}

/// ⑬-b `member remove` 清空被动身份（转主动推送）。
#[tokio::test]
async fn member_remove_clears_the_event_id() {
    let (_dir, api, state, echo, app_id) = state();
    echo.add_event_id_v2(&app_id, GROUP, "event-1");
    api.group_send
        .lock()
        .expect("槽锁")
        .push_back(FakeQqApi::ok_receipt());

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            json!({
                "group_id": GROUP,
                "message": format!("再见[CQ:member,type=remove,user_id={MEMBER_A}]"),
            }),
            None,
        ))
        .await;

    assert_eq!(
        response.retcode, 0,
        "退群消息应成功: {:?}",
        response.message
    );
    let calls = api.calls();
    assert_eq!(
        calls.len(),
        1,
        "member 码零上游调用（只有本条发送）: {calls:?}"
    );
    match &calls[0] {
        Call::SendGroup(openid, message) => {
            assert_eq!(openid, GROUP);
            assert_eq!(message.content.as_deref(), Some("再见"));
            assert!(
                message.event_id.is_none(),
                "member remove 必须清空 event_id: {:?}",
                message.event_id
            );
            assert!(
                message.msg_id.is_none(),
                "转主动推送时也不该带 msg_id: {:?}",
                message.msg_id
            );
        }
        other => panic!("应发群消息: {other:?}"),
    }
}
