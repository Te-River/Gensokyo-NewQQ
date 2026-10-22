//! 群管理子集：路径、分页参数与响应解析。

mod support;

use std::sync::Arc;

use gensokyo_qq::{HttpMethod, RestrictChatSetting};
use serde_json::{Value, json};
use support::{FakeTransport, ManualClock, ok_json, test_client, token_response};

const START: u64 = 1_700_000_000;

fn setup() -> (
    Arc<FakeTransport>,
    Arc<ManualClock>,
    gensokyo_qq::OpenApiClient<FakeTransport>,
) {
    let transport = Arc::new(FakeTransport::new());
    let clock = Arc::new(ManualClock::new(START));
    let client = test_client(&transport, &clock);
    (transport, clock, client)
}

fn body_of(request: &gensokyo_qq::HttpRequest) -> Value {
    serde_json::from_slice(&request.body).expect("请求体应是 JSON")
}

#[tokio::test]
async fn gets_group_info() {
    let (transport, _clock, client) = setup();
    transport.push_response(token_response("tok", 7200));
    transport.push_response(ok_json(
        r#"{"group_openid":"G","group_name":"群","group_member_num":42}"#,
    ));

    let info = client.get_group_info("G").await.unwrap();
    assert_eq!(info.group_openid, "G");
    assert_eq!(info.group_name, "群");
    assert_eq!(info.group_member_num, 42);

    let request = &transport.requests()[1];
    assert_eq!(request.method, HttpMethod::Get);
    assert_eq!(request.url, "https://api.bot.qq.com/v2/groups/G/info");
    assert!(request.body.is_empty());
}

#[tokio::test]
async fn gets_bot_state() {
    let (transport, _clock, client) = setup();
    transport.push_response(token_response("tok", 7200));
    transport.push_response(ok_json(
        r#"{"member_role":"admin","allow_proactive_msg":true,"recv_msg_setting":"all"}"#,
    ));

    let state = client.get_bot_state("G").await.unwrap();
    assert_eq!(state.member_role, "admin");
    assert!(state.allow_proactive_msg);
    assert_eq!(
        transport.requests()[1].url,
        "https://api.bot.qq.com/v2/groups/G/bot_state"
    );
}

#[tokio::test]
async fn lists_join_requests_with_cursor_and_limit() {
    let (transport, _clock, client) = setup();
    transport.push_response(token_response("tok", 7200));
    transport.push_response(ok_json(
        r#"{
            "list": [
                {"join_request_id":"jr1","username":"甲","apply_at":1729500000,
                 "verify_info":{"method":"admin_review_qa","verify_message":"答案"}},
                {"join_request_id":"jr2","apply_at":"2026-10-22T10:00:00+08:00",
                 "verify_info":"旧格式验证消息","auto_approved":true}
            ],
            "next_cursor":"next"
        }"#,
    ));

    let list = client
        .get_join_request_list("G", Some("a b"), Some(50))
        .await
        .unwrap();
    assert_eq!(list.list.len(), 2);
    assert_eq!(list.next_cursor, "next");
    assert_eq!(list.list[0].apply_at, "1729500000");
    assert_eq!(list.list[0].verify_info.method, "admin_review_qa");
    assert_eq!(list.list[0].verify_info.message, "答案");
    assert_eq!(list.list[1].apply_at, "2026-10-22T10:00:00+08:00");
    assert_eq!(list.list[1].verify_info.message, "旧格式验证消息");
    assert_eq!(
        list.list[1].auto_approved.as_ref().map(|a| a.approved),
        Some(true)
    );

    assert_eq!(
        transport.requests()[1].url,
        "https://api.bot.qq.com/v2/groups/G/join_request_list?cursor=a%20b&limit=50"
    );
}

#[tokio::test]
async fn omits_paging_query_when_not_provided() {
    let (transport, _clock, client) = setup();
    transport.push_response(token_response("tok", 7200));
    transport.push_response(ok_json(r#"{"list":[],"next_cursor":""}"#));

    client.get_join_request_list("G", None, None).await.unwrap();
    assert_eq!(
        transport.requests()[1].url,
        "https://api.bot.qq.com/v2/groups/G/join_request_list"
    );
}

#[tokio::test]
async fn omits_limit_query_when_zero() {
    let (transport, _clock, client) = setup();
    transport.push_response(token_response("tok", 7200));
    transport.push_response(ok_json(r#"{"list":[],"next_cursor":""}"#));

    client
        .get_join_request_list("G", None, Some(0))
        .await
        .unwrap();
    assert_eq!(
        transport.requests()[1].url,
        "https://api.bot.qq.com/v2/groups/G/join_request_list",
        "limit=0 不应出现在 query 里"
    );
}

#[tokio::test]
async fn sets_restrict_chat_setting_with_members_only() {
    let (transport, _clock, client) = setup();
    transport.push_response(token_response("tok", 7200));
    transport.push_response(ok_json("{}"));

    let setting = RestrictChatSetting {
        global_rule: None,
        members: vec![gensokyo_qq::dto::group::MemberMuteState {
            op: Some("add".to_owned()),
            member_openid: "M".to_owned(),
            mute_expire_at: "2026-10-22T12:00:00+08:00".to_owned(),
            ..Default::default()
        }],
    };
    client
        .set_restrict_chat_setting("G", &setting)
        .await
        .unwrap();

    let request = &transport.requests()[1];
    assert_eq!(request.method, HttpMethod::Post);
    assert_eq!(
        request.url,
        "https://api.bot.qq.com/v2/groups/G/restrict_chat_setting"
    );
    assert_eq!(
        body_of(request),
        json!({"members": [{
            "op": "add",
            "member_openid": "M",
            "mute_expire_at": "2026-10-22T12:00:00+08:00"
        }]})
    );
}

#[tokio::test]
async fn lists_group_members() {
    let (transport, _clock, client) = setup();
    transport.push_response(token_response("tok", 7200));
    transport.push_response(ok_json(
        r#"{"members":[{"member_openid":"M1","username":"成员","member_role":"member"}],"next_cursor":""}"#,
    ));

    let members = client.get_group_members("G", None, Some(30)).await.unwrap();
    assert_eq!(members.members.len(), 1);
    assert_eq!(members.members[0].username, "成员");
    assert_eq!(
        transport.requests()[1].url,
        "https://api.bot.qq.com/v2/groups/G/members?limit=30"
    );
}

#[tokio::test]
async fn gets_member_info_with_encoded_openid() {
    let (transport, _clock, client) = setup();
    transport.push_response(token_response("tok", 7200));
    transport.push_response(ok_json(r#"{"member_openid":"M/1","member_role":"owner"}"#));

    let member = client.get_member_info("G", "M/1").await.unwrap();
    assert_eq!(member.member_openid, "M/1");
    assert_eq!(member.member_role, "owner");
    assert_eq!(
        transport.requests()[1].url,
        "https://api.bot.qq.com/v2/groups/G/members/M%2F1"
    );
}
