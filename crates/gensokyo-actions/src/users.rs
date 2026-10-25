//! 用户类 action：`get_friend_list` / `get_stranger_info` / `send_like`。
//!
//! # 覆盖范围
//!
//! | action | Go 对应 | 上游端点 | 结果 |
//! |--------|---------|----------|------|
//! | `get_friend_list` | `handlers/get_friend_list.go` | 官方无好友列表端点 | `1500` |
//! | `get_stranger_info` | 无（Go 未注册） | 官方无陌生人资料端点 | `1500` |
//! | `send_like` | 无（Go 未注册） | 官方无点赞端点 | `1500` |
//!
//! # 与 Go 的差异
//!
//! Go 的 `get_friend_list` 不走上游：它直接读 idmap 数据库里的全部用户
//! （`idmap.ListAllUsers`）。`gensokyo-storage` 的 [`gensokyo_storage::IdMapStore`] 目前没有
//! 枚举接口，所以这里只能如实回 `1500`（[`gensokyo_core::Error::Unsupported`]），不伪造空列表。
//! 「返回空列表」会让调用方以为「机器人没有好友」，比失败更难排查。
//!
//! `get_stranger_info` / `send_like` 是 OneBot v11 标准 action，Go 未注册；参数形状按 OneBot
//! 约定，端点缺失时同样回 `1500`。
//!
//! # 响应 ID
//!
//! `user_id` 回填请求值（虚拟号形态由配置 `string_ob11` 决定），真实 `user_openid` 放在附加字段
//! `user_openid` 里——与 [`crate::group`] 的成员信息同一口径。

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use gensokyo_core::{Error, Ob11Id, Result};
use gensokyo_protocol::action::{ActionRequest, ActionResponse};
use gensokyo_protocol::{Extra, Handler, HandlerFuture};

use crate::params;
use crate::qqapi::ActionContext;
use crate::util::{
    bad_request, ob11_id, ok_data, params_object, params_or_empty, resolve_identity_text,
    upstream_error, virtual_id_text,
};

/// `send_like` 未传 `times` 时的默认次数（与 OneBot v11 的 1 次一致）。
pub const DEFAULT_LIKE_TIMES: u32 = 1;

/// 单次 `send_like` 的次数上限（官方每天每人最多 10 次）。超过按上限截断。
pub const MAX_LIKE_TIMES: u32 = 10;

/// `get_friend_list` 的条目（OneBot v11 字段；`protocol` 尚未定义该类型，先落在本 crate）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendInfo {
    /// 好友虚拟号（无 idmap 时为 `"0"`）。
    pub user_id: Ob11Id,
    /// 昵称。
    pub nickname: String,
    /// 备注（官方无此字段，恒为空串）。
    pub remark: String,
}

/// `get_stranger_info` 的响应数据（OneBot v11 字段）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrangerInfoData {
    /// 用户虚拟号（回填请求值）。
    pub user_id: Ob11Id,
    /// 昵称。
    pub nickname: String,
    /// 性别。
    pub sex: String,
    /// 年龄。
    pub age: i64,
    /// 真实 `user_openid`（附加字段）。
    #[serde(flatten)]
    pub extra: Extra,
}

/// `get_friend_list`。
#[derive(Debug, Clone)]
pub struct GetFriendListHandler {
    context: Arc<ActionContext>,
}

impl GetFriendListHandler {
    /// 构造。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        // 没有必填参数；仍拒绝非对象的 params
        if let Err(err) = params_or_empty(request) {
            return Ok(bad_request(request, &err));
        }
        let friends = match self.context.api().get_friend_list().await {
            Ok(friends) => friends,
            Err(err) => return Ok(upstream_error(request, &err)),
        };

        let string_mode = self.context.string_ob11();
        let data: Vec<FriendInfo> = friends
            .iter()
            .map(|friend| FriendInfo {
                user_id: ob11_id(
                    &virtual_id_text(&self.context, &friend.user_openid),
                    string_mode,
                ),
                nickname: friend.nickname.clone(),
                remark: friend.remark.clone(),
            })
            .collect();
        ok_data(request, &data)
    }
}

impl Handler for GetFriendListHandler {
    fn name(&self) -> &'static str {
        "get_friend_list"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `get_stranger_info`。
#[derive(Debug, Clone)]
pub struct GetStrangerInfoHandler {
    context: Arc<ActionContext>,
}

impl GetStrangerInfoHandler {
    /// 构造。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = match params_object(request) {
            Ok(object) => object,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let requested = match params::required_id(object, "user_id") {
            Ok(requested) => requested,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let user = match resolve_identity_text(&self.context, "user_id", &requested, "user_openid")
        {
            Ok(user) => user,
            Err(err) => return Ok(bad_request(request, &err)),
        };

        let profile = match self.context.api().get_stranger_info(&user.openid).await {
            Ok(profile) => profile,
            Err(err) => return Ok(upstream_error(request, &err)),
        };

        let mut extra = Extra::new();
        extra.insert("user_openid".to_owned(), Value::String(user.openid));
        let data = StrangerInfoData {
            user_id: ob11_id(&user.requested, self.context.string_ob11()),
            nickname: profile.nickname,
            sex: profile.sex,
            age: profile.age,
            extra,
        };
        ok_data(request, &data)
    }
}

impl Handler for GetStrangerInfoHandler {
    fn name(&self) -> &'static str {
        "get_stranger_info"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `send_like`。
#[derive(Debug, Clone)]
pub struct SendLikeHandler {
    context: Arc<ActionContext>,
}

impl SendLikeHandler {
    /// 构造。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = match params_object(request) {
            Ok(object) => object,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let requested = match params::required_id(object, "user_id") {
            Ok(requested) => requested,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let times = match params::optional_i64(object, "times") {
            Ok(None) => DEFAULT_LIKE_TIMES,
            Ok(Some(times)) if times < 1 => {
                return Ok(bad_request(
                    request,
                    &Error::Protocol(format!("times 必须 ≥ 1: {times}")),
                ));
            }
            // 官方每天每人上限 10 次：超出按上限截断，不把请求打回
            Ok(Some(times)) => u32::try_from(times).unwrap_or(u32::MAX).min(MAX_LIKE_TIMES),
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let user = match resolve_identity_text(&self.context, "user_id", &requested, "user_openid")
        {
            Ok(user) => user,
            Err(err) => return Ok(bad_request(request, &err)),
        };

        if let Err(err) = self.context.api().send_like(&user.openid, times).await {
            return Ok(upstream_error(request, &err));
        }
        // OneBot 的 send_like 成功回执没有业务数据
        Ok(ActionResponse::ok(Value::Null).with_echo(request.echo.clone()))
    }
}

impl Handler for SendLikeHandler {
    fn name(&self) -> &'static str {
        "send_like"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        DEFAULT_LIKE_TIMES, GetFriendListHandler, GetStrangerInfoHandler, MAX_LIKE_TIMES,
        SendLikeHandler,
    };
    use crate::handler::RETCODE_BAD_REQUEST;
    use crate::qqapi::{FriendEntry, StrangerProfile};
    use crate::testing::{
        Call, block_on, context, failed, ok_data, request, request_without_params,
    };
    use crate::util::RETCODE_UPSTREAM;
    use gensokyo_core::Error;
    use gensokyo_protocol::Handler;
    use gensokyo_protocol::Status;

    const MEMBER: &str = "fedcba9876543210fedcba9876543210";

    #[test]
    fn friend_list_maps_entries() {
        let (context, api) = context();
        *api.friends.lock().unwrap() = Some(Ok(vec![FriendEntry {
            user_openid: MEMBER.to_owned(),
            nickname: "小明".to_owned(),
            remark: String::new(),
        }]));
        let handler = GetFriendListHandler::new(context);
        let response = block_on(handler.handle(&request_without_params("get_friend_list")))
            .expect("handler 不应返回 Err");
        let data = ok_data(response);

        assert_eq!(api.calls(), vec![Call::FriendList]);
        assert_eq!(data[0]["nickname"], json!("小明"));
        assert_eq!(data[0]["remark"], json!(""));
        assert_eq!(data[0]["user_id"], json!("0"), "无 idmap 时回退 0");
        assert_eq!(handler.name(), "get_friend_list");
    }

    #[test]
    fn friend_list_unsupported_maps_to_1500() {
        let (context, api) = context();
        *api.friends.lock().unwrap() = Some(Err(Error::Unsupported(
            "get_friend_list: 官方无好友列表端点，且 idmap 暂无枚举接口".to_owned(),
        )));
        let handler = GetFriendListHandler::new(context);
        let response = block_on(handler.handle(&request("get_friend_list", json!({}))))
            .expect("上游错误也应是 Ok(失败响应)");
        let (status, retcode, data) = failed(response);
        assert_eq!(status, Status::Failed);
        assert_eq!(retcode, RETCODE_UPSTREAM);
        assert_eq!(data, None);
    }

    #[test]
    fn stranger_info_maps_profile_and_keeps_openid() {
        let (context, api) = context();
        *api.stranger.lock().unwrap() = Some(Ok(StrangerProfile {
            user_openid: MEMBER.to_owned(),
            nickname: "路人".to_owned(),
            sex: "unknown".to_owned(),
            age: 0,
        }));
        let handler = GetStrangerInfoHandler::new(context);
        let response =
            block_on(handler.handle(&request("get_stranger_info", json!({"user_id": MEMBER}))))
                .expect("handler 不应返回 Err");
        let data = ok_data(response);

        assert_eq!(api.calls(), vec![Call::StrangerInfo(MEMBER.to_owned())]);
        assert_eq!(data["user_id"], json!(MEMBER));
        assert_eq!(data["nickname"], json!("路人"));
        assert_eq!(data["sex"], json!("unknown"));
        assert_eq!(data["age"], json!(0));
        assert_eq!(data["user_openid"], json!(MEMBER));
        assert_eq!(handler.name(), "get_stranger_info");
    }

    #[test]
    fn stranger_info_requires_user_id() {
        let (context, api) = context();
        let handler = GetStrangerInfoHandler::new(context);
        let response = block_on(handler.handle(&request("get_stranger_info", json!({}))))
            .expect("参数错误也应是 Ok(失败响应)");
        let (status, retcode, _) = failed(response);
        assert_eq!(status, Status::Failed);
        assert_eq!(retcode, RETCODE_BAD_REQUEST);
        assert!(api.calls().is_empty());
    }

    #[test]
    fn stranger_info_unsupported_maps_to_1500() {
        let (context, api) = context();
        *api.stranger.lock().unwrap() = Some(Err(Error::Unsupported(
            "get_stranger_info: 官方无陌生人资料端点".to_owned(),
        )));
        let handler = GetStrangerInfoHandler::new(context);
        let response =
            block_on(handler.handle(&request("get_stranger_info", json!({"user_id": MEMBER}))))
                .expect("上游错误也应是 Ok(失败响应)");
        assert_eq!(failed(response).1, RETCODE_UPSTREAM);
    }

    #[test]
    fn send_like_defaults_and_clamps_times() {
        let (context, api) = context();
        let handler = SendLikeHandler::new(context);

        *api.like.lock().unwrap() = Some(Ok(()));
        let response = block_on(handler.handle(&request("send_like", json!({"user_id": MEMBER}))))
            .expect("handler 不应返回 Err");
        let data = ok_data(response);
        assert_eq!(data, json!(null));

        *api.like.lock().unwrap() = Some(Ok(()));
        let response = block_on(handler.handle(&request(
            "send_like",
            json!({"user_id": MEMBER, "times": 99}),
        )))
        .expect("handler 不应返回 Err");
        let _ = ok_data(response);

        assert_eq!(
            api.calls(),
            vec![
                Call::SendLike(MEMBER.to_owned(), DEFAULT_LIKE_TIMES),
                Call::SendLike(MEMBER.to_owned(), MAX_LIKE_TIMES),
            ]
        );
        assert_eq!(handler.name(), "send_like");
    }

    #[test]
    fn send_like_rejects_zero_times() {
        let (context, api) = context();
        let handler = SendLikeHandler::new(context);
        let response = block_on(handler.handle(&request(
            "send_like",
            json!({"user_id": MEMBER, "times": 0}),
        )))
        .expect("参数错误也应是 Ok(失败响应)");
        let (status, retcode, _) = failed(response);
        assert_eq!(status, Status::Failed);
        assert_eq!(retcode, RETCODE_BAD_REQUEST);
        assert!(api.calls().is_empty());
    }

    #[test]
    fn send_like_unsupported_maps_to_1500() {
        let (context, api) = context();
        *api.like.lock().unwrap() = Some(Err(Error::Unsupported(
            "send_like: 官方无点赞端点".to_owned(),
        )));
        let handler = SendLikeHandler::new(context);
        let response = block_on(handler.handle(&request("send_like", json!({"user_id": MEMBER}))))
            .expect("上游错误也应是 Ok(失败响应)");
        assert_eq!(failed(response).1, RETCODE_UPSTREAM);
    }
}
