//! `downtime_message` 兜底回复（P26，Go `Processor/Processor.go:374-399`）。
//!
//! 触发口径：`!http_only_bot` + 没有任何 OneBot 下游订阅者 + `downtime_message` 非空，
//! 且事件是 @ 机器人的群消息或单聊消息。回复形态与 Go 一致：被动消息
//! （`msg_id` = 原始消息 ID、`msg_seq = 1`、`msg_type = 0`），失败只 warn。

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gensokyo_actions::qqapi::{QqApi, QqFuture};
use gensokyo_actions::ActionContext;
use gensokyo_core::{Error, Ob11Id};
use gensokyo_protocol::event::{MessageEvent, MessageType};
use gensokyo_protocol::{Event, MessageContent};
use gensokyo_qq::dto::{MessageResponse, MessageToCreate};
use gensokyo_runtime::AppState;
use gensokyo_runtime::downtime::Target;

/// 一次 `send_*` 调用的记录。
type Call = (Target, MessageToCreate);

/// 记录 `send_*` 的假 QqApi；其余端点走 trait 的默认实现（一律「未实现」）。
#[derive(Debug, Default)]
struct RecordingApi {
    sent: Mutex<Vec<Call>>,
}

impl RecordingApi {
    fn sent(&self) -> Vec<Call> {
        self.sent.lock().expect("调用记录锁").clone()
    }

    fn record(&self, target: Target, msg: MessageToCreate) {
        self.sent.lock().expect("调用记录锁").push((target, msg));
    }
}

impl QqApi for RecordingApi {
    fn get_group_info(&self, _group_openid: &str) -> QqFuture<'_, gensokyo_qq::GroupInfo> {
        Box::pin(async { Err(Error::Unsupported("未实现".to_owned())) })
    }

    fn get_group_member_info(
        &self,
        _group_openid: &str,
        _member_openid: &str,
    ) -> QqFuture<'_, gensokyo_qq::GroupMember> {
        Box::pin(async { Err(Error::Unsupported("未实现".to_owned())) })
    }

    fn get_group_member_list(
        &self,
        _group_openid: &str,
        _limit: u32,
    ) -> QqFuture<'_, gensokyo_qq::GroupMemberList> {
        Box::pin(async { Err(Error::Unsupported("未实现".to_owned())) })
    }

    fn set_group_ban(
        &self,
        _group_openid: &str,
        _member_openid: &str,
        _duration_secs: i64,
    ) -> QqFuture<'_, ()> {
        Box::pin(async { Err(Error::Unsupported("未实现".to_owned())) })
    }

    fn get_group_bot_state(&self, _group_openid: &str) -> QqFuture<'_, gensokyo_qq::BotInGroupState> {
        Box::pin(async { Err(Error::Unsupported("未实现".to_owned())) })
    }

    fn get_group_join_request_list(
        &self,
        _group_openid: &str,
        _cursor: Option<&str>,
        _limit: Option<u32>,
    ) -> QqFuture<'_, gensokyo_qq::JoinRequestList> {
        Box::pin(async { Err(Error::Unsupported("未实现".to_owned())) })
    }

    fn send_group_message(
        &self,
        group_openid: String,
        msg: MessageToCreate,
    ) -> QqFuture<'_, MessageResponse> {
        Box::pin(async move {
            self.record(Target::Group(group_openid), msg);
            Ok(MessageResponse::default())
        })
    }

    fn send_c2c_message(
        &self,
        user_openid: String,
        msg: MessageToCreate,
    ) -> QqFuture<'_, MessageResponse> {
        Box::pin(async move {
            self.record(Target::Private(user_openid), msg);
            Ok(MessageResponse::default())
        })
    }
}

/// 两格缩进的 `settings:` 串（见 `support::config` 的坑位说明）。
fn settings(http_only_bot: bool, downtime_message: &str) -> String {
    format!(
        "  string_ob11: true\n  http_only_bot: {http_only_bot}\n  downtime_message: \"{downtime_message}\"\n"
    )
}

fn state_for(http_only_bot: bool, downtime_message: &str) -> (AppState, Arc<RecordingApi>, tempfile::TempDir) {
    let (config, dir) = support::config(&settings(http_only_bot, downtime_message));
    let api = Arc::new(RecordingApi::default());
    let actions = Arc::new(ActionContext::new(api.clone(), Arc::clone(&config)));
    let state = AppState::with_actions(config, actions);
    (state, api, dir)
}

fn group_event(message_id: Ob11Id, group_id: Ob11Id, to_me: Option<bool>) -> Event {
    Event::Message(MessageEvent {
        time: 1_700_000_000,
        self_id: Ob11Id::num(12345),
        message_type: MessageType::Group,
        sub_type: Some("normal".to_owned()),
        message_id,
        user_id: Ob11Id::str("U1"),
        group_id: Some(group_id),
        message: MessageContent::Text("hi".to_owned()),
        raw_message: "hi".to_owned(),
        font: Some(0),
        sender: None,
        anonymous: None,
        message_seq: Some(0),
        to_me,
        extra: Default::default(),
    })
}

fn private_event(message_id: Ob11Id, user_id: Ob11Id) -> Event {
    Event::Message(MessageEvent {
        time: 1_700_000_000,
        self_id: Ob11Id::num(12345),
        message_type: MessageType::Private,
        sub_type: Some("friend".to_owned()),
        message_id,
        user_id,
        group_id: None,
        message: MessageContent::Text("hi".to_owned()),
        raw_message: "hi".to_owned(),
        font: Some(0),
        sender: None,
        anonymous: None,
        message_seq: Some(0),
        to_me: None,
        extra: Default::default(),
    })
}

/// 等兜底任务把调用记下来（发送在 `tokio::spawn` 里，不阻塞发布方）。
async fn wait_sent(api: &RecordingApi) -> Vec<Call> {
    for _ in 0..100 {
        let sent = api.sent();
        if !sent.is_empty() {
            return sent;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    api.sent()
}

#[tokio::test]
async fn http_only_bot_suppresses_the_fallback() {
    let (state, api, _dir) = state_for(true, "bot-offline");
    assert!(
        state
            .downtime()
            .notice(&group_event(
                Ob11Id::str("M1"),
                Ob11Id::str("G1"),
                Some(true)
            ))
            .is_none(),
        "http_only_bot = true 时不该兜底"
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(api.sent().is_empty());
}

#[tokio::test]
async fn missing_downstream_sends_one_passive_group_reply() {
    let (state, api, _dir) = state_for(false, "bot-offline");
    let event = group_event(Ob11Id::str("M1"), Ob11Id::str("G1"), Some(true));

    let notice = state.downtime().notice(&event).expect("应该要兜底");
    assert_eq!(notice.target, Target::Group("G1".to_owned()));
    assert_eq!(notice.message.content.as_deref(), Some("bot-offline"));
    assert_eq!(notice.message.msg_id.as_deref(), Some("M1"), "对齐 Go 的 MsgID: v.ID");
    assert_eq!(notice.message.msg_seq, Some(1));
    assert_eq!(notice.message.msg_type, Some(0));
    assert!(notice.message.event_id.is_none() && !notice.message.is_wakeup);

    assert_eq!(state.downtime().publish(event), 0, "没有订阅者，送达数 0");
    let sent = wait_sent(&api).await;
    assert_eq!(sent.len(), 1, "只回一条");
    assert_eq!(sent[0], (notice.target.clone(), notice.message.clone()));
}

#[tokio::test]
async fn downstream_subscriber_suppresses_the_fallback() {
    let (state, api, _dir) = state_for(false, "bot-offline");
    let _ws = state.hub().subscribe();
    let event = group_event(Ob11Id::str("M1"), Ob11Id::str("G1"), Some(true));

    assert!(state.downtime().notice(&event).is_none());
    assert_eq!(state.downtime().publish(event), 1);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(api.sent().is_empty());
}

#[tokio::test]
async fn empty_downtime_message_is_silent() {
    let (state, api, _dir) = state_for(false, "");
    assert!(
        state
            .downtime()
            .notice(&group_event(
                Ob11Id::num(1),
                Ob11Id::num(2),
                Some(true)
            ))
            .is_none()
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(api.sent().is_empty());
}

#[tokio::test]
async fn full_group_message_has_no_fallback() {
    // Go 的 switch 只匹配 `*dto.WSGroupATMessageData`：全量群消息（to_me 缺省）不在这条兜底里。
    let (state, _api, _dir) = state_for(false, "bot-offline");
    let event = group_event(Ob11Id::str("M1"), Ob11Id::str("G1"), None);
    assert!(state.downtime().notice(&event).is_none());
}

#[tokio::test]
async fn private_message_replies_to_the_author() {
    let (state, api, _dir) = state_for(false, "bot-offline");
    let event = private_event(Ob11Id::str("M2"), Ob11Id::str("U9"));

    let notice = state.downtime().notice(&event).expect("C2C 也要兜底");
    assert_eq!(notice.target, Target::Private("U9".to_owned()));
    state.downtime().publish(event);
    let sent = wait_sent(&api).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1.msg_id.as_deref(), Some("M2"));
}

#[tokio::test]
async fn background_subscription_is_not_a_downstream() {
    // post_url 上报这类后台订阅者不算「有下游可送」。
    let (state, api, _dir) = state_for(false, "bot-offline");
    let background = state.hub().subscribe_background();
    let event = group_event(Ob11Id::str("M1"), Ob11Id::str("G1"), Some(true));

    assert_eq!(state.hub().downstream_count(), 0);
    assert!(state.downtime().notice(&event).is_some());

    state.downtime().publish(event);
    assert_eq!(wait_sent(&api).await.len(), 1);
    drop(background);
}

#[tokio::test]
async fn numeric_ids_are_resolved_through_idmap() {
    let (state, _api, dir) = state_for(false, "bot-offline");
    let state = support::with_temp_stores(state, &dir);
    let idmap = state.idmap().expect("应注入 idmap");

    let (raw_group, group_vuin) = idmap.retrieve_virtual_value("GROUP-OPENID").expect("虚拟群号");
    let message_vuin = idmap.store_cache("MSG-RAW").expect("虚拟消息 ID");

    let event = group_event(Ob11Id::num(message_vuin), Ob11Id::num(group_vuin), Some(true));
    let notice = state.downtime().notice(&event).expect("数字形态也要兜底");
    assert_eq!(notice.target, Target::Group(raw_group));
    assert_eq!(notice.message.msg_id.as_deref(), Some("MSG-RAW"));
}
