//! 单测公共装置：`FakeQqApi`、最小 `block_on`、临时配置快照与响应断言。
//!
//! 只在 `cargo test` 下编译（[`crate::testing`] 带 `#[cfg(test)]`），不进产物。
//! `group` 的测试自带一份只覆盖 M1k 六个方法的替身；这里这份面向群管理 / 消息 / 用户三组
//! action，实现全部新方法并记录入参。

use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{Value, json};

use gensokyo_config::ConfigManager;
use gensokyo_core::{Error, Result};
use gensokyo_protocol::action::{ActionRequest, ActionResponse};
use gensokyo_protocol::{Segment, Status};
use gensokyo_qq::dto::{
    BotInGroupState, GroupInfo, GroupMember, GroupMemberList, JoinRequestList, MessageResponse,
    MessageToCreate,
};

use crate::qqapi::{
    ActionContext, FriendEntry, KickResult, MsgDetail, QqApi, QqFuture, RetractRequest,
    StrangerProfile,
};

/// 一次上游调用的入参记录。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Call {
    /// `set_group_kick(group_openid, member_openids, add_blacklist)`
    Kick(String, Vec<String>, bool),
    /// `set_group_leave(group_openid)`
    Leave(String),
    /// `get_group_list()`
    GroupList,
    /// `delete_msg(group_openid, user_openid, message_id)`
    DeleteMsg(Option<String>, Option<String>, String),
    /// `get_msg(message_id)`
    GetMsg(String),
    /// `get_forward_msg(forward_id)`
    GetForwardMsg(String),
    /// `get_friend_list()`
    FriendList,
    /// `get_stranger_info(user_openid)`
    StrangerInfo(String),
    /// `send_like(user_openid, times)`
    SendLike(String, u32),
    /// `send_group_message(group_openid, msg)`
    SendGroup(String, MessageToCreate),
    /// `send_c2c_message(user_openid, msg)`
    SendC2c(String, MessageToCreate),
}

/// 记录入参、按槽位注入结果的 `QqApi` 替身。
///
/// 槽位是 `Option<Result<T>>`：测试写 `Some(...)` 后由 handler 取走（一次调用一次取值），
/// 未配置槽位直接 panic——「忘了准备 fixture」要立刻炸，而不是静默变成 1500。
#[derive(Debug, Default)]
pub(crate) struct FakeQqApi {
    calls: Mutex<Vec<Call>>,
    pub(crate) kick: Mutex<Option<Result<KickResult>>>,
    pub(crate) leave: Mutex<Option<Result<()>>>,
    pub(crate) group_list: Mutex<Option<Result<Vec<GroupInfo>>>>,
    pub(crate) delete: Mutex<Option<Result<()>>>,
    pub(crate) msg: Mutex<Option<Result<MsgDetail>>>,
    pub(crate) forward: Mutex<Option<Result<Vec<Segment>>>>,
    pub(crate) friends: Mutex<Option<Result<Vec<FriendEntry>>>>,
    pub(crate) stranger: Mutex<Option<Result<StrangerProfile>>>,
    pub(crate) like: Mutex<Option<Result<()>>>,
    pub(crate) group_send: Mutex<Option<Result<MessageResponse>>>,
    pub(crate) c2c_send: Mutex<Option<Result<MessageResponse>>>,
}

impl FakeQqApi {
    fn record(&self, call: Call) {
        self.calls.lock().expect("调用记录锁未中毒").push(call);
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("调用记录锁未中毒").clone()
    }
}

/// 取走一次预置结果；未配置即 panic。
fn take<T>(slot: &Mutex<Option<Result<T>>>) -> Result<T> {
    slot.lock()
        .expect("响应槽锁未中毒")
        .take()
        .expect("测试未配置该方法的响应")
}

/// M1k 的六个真实端点：新测试用不到，明确报「未实现」而不是 panic。
fn not_provided<T>(method: &str) -> Result<T> {
    Err(Error::Unsupported(format!("FakeQqApi 未提供 {method}")))
}

impl QqApi for FakeQqApi {
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

    fn set_group_ban(
        &self,
        _group_openid: &str,
        _member_openid: &str,
        _duration_secs: i64,
    ) -> QqFuture<'_, ()> {
        Box::pin(async { not_provided("set_group_ban") })
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

    fn set_group_kick(
        &self,
        group_openid: &str,
        member_openids: &[String],
        add_blacklist: bool,
    ) -> QqFuture<'_, KickResult> {
        // 先把借用转成所有权：async 块只捕获 `&self`，返回的 future 才只依赖 self 的生命周期
        let group_openid = group_openid.to_owned();
        let member_openids = member_openids.to_vec();
        Box::pin(async move {
            self.record(Call::Kick(group_openid, member_openids, add_blacklist));
            take(&self.kick)
        })
    }

    fn set_group_leave(&self, group_openid: &str) -> QqFuture<'_, ()> {
        let group_openid = group_openid.to_owned();
        Box::pin(async move {
            self.record(Call::Leave(group_openid));
            take(&self.leave)
        })
    }

    fn get_group_list(&self) -> QqFuture<'_, Vec<GroupInfo>> {
        Box::pin(async move {
            self.record(Call::GroupList);
            take(&self.group_list)
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

    fn get_msg(&self, message_id: &str) -> QqFuture<'_, MsgDetail> {
        let message_id = message_id.to_owned();
        Box::pin(async move {
            self.record(Call::GetMsg(message_id));
            take(&self.msg)
        })
    }

    fn get_forward_msg(&self, forward_id: &str) -> QqFuture<'_, Vec<Segment>> {
        let forward_id = forward_id.to_owned();
        Box::pin(async move {
            self.record(Call::GetForwardMsg(forward_id));
            take(&self.forward)
        })
    }

    fn get_friend_list(&self) -> QqFuture<'_, Vec<FriendEntry>> {
        Box::pin(async move {
            self.record(Call::FriendList);
            take(&self.friends)
        })
    }

    fn get_stranger_info(&self, user_openid: &str) -> QqFuture<'_, StrangerProfile> {
        let user_openid = user_openid.to_owned();
        Box::pin(async move {
            self.record(Call::StrangerInfo(user_openid));
            take(&self.stranger)
        })
    }

    fn send_like(&self, user_openid: &str, times: u32) -> QqFuture<'_, ()> {
        let user_openid = user_openid.to_owned();
        Box::pin(async move {
            self.record(Call::SendLike(user_openid, times));
            take(&self.like)
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

/// 单测里的最小 `block_on`：handler 只 await 一次 FakeQqApi，一次 poll 即 `Ready`。
pub(crate) fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(output) => return output,
            std::task::Poll::Pending => std::thread::yield_now(),
        }
    }
}

/// 一份 `string_ob11 = true` 的配置快照：临时目录写一个最小 `config.yml` 后立即删除。
///
/// `string_ob11` 决定响应 ID 的形态；取 `true` 时 ID 一律字符串，测试断言不需要区分
/// 「虚拟号是否可解析成整数」。`app_id` 供 `get_avatar` 拼头像 URL（Go 用同一个配置项）。
pub(crate) fn config() -> Arc<ConfigManager> {
    static MANAGER: OnceLock<Arc<ConfigManager>> = OnceLock::new();
    MANAGER
        .get_or_init(|| {
            let path = std::env::temp_dir().join(format!(
                "gensokyo-actions-admin-test-{}.yml",
                std::process::id()
            ));
            std::fs::write(&path, "string_ob11: true\napp_id: 102001234\n")
                .expect("写临时 config.yml 失败");
            let manager = Arc::new(ConfigManager::new(&path).expect("加载临时 config.yml 失败"));
            let _ = std::fs::remove_file(&path);
            manager
        })
        .clone()
}

/// `ActionContext` + 与它共享的替身。
pub(crate) fn context() -> (Arc<ActionContext>, Arc<FakeQqApi>) {
    let api = Arc::new(FakeQqApi::default());
    let context = Arc::new(ActionContext::new(api.clone(), config()));
    (context, api)
}

/// 带 `echo = "e-1"` 的请求。
pub(crate) fn request(action: &str, params: Value) -> ActionRequest<Value> {
    serde_json::from_value(json!({"action": action, "params": params, "echo": "e-1"}))
        .expect("请求应可反序列化")
}

/// 完全不带 `params` 的请求（对齐 OneBot 客户端省略字段的写法）。
pub(crate) fn request_without_params(action: &str) -> ActionRequest<Value> {
    ActionRequest {
        action: action.to_owned(),
        params: None,
        echo: Some(json!("e-1")),
    }
}

/// 断言成功响应（`retcode = 0` + echo）并取出 `data`。
pub(crate) fn ok_data(response: ActionResponse<Value>) -> Value {
    assert_eq!(response.status, Status::Ok);
    assert_eq!(response.retcode, 0);
    assert_eq!(response.echo, Some(json!("e-1")));
    response.data.expect("成功响应应带 data")
}

/// 断言失败响应保留 echo，返回 `(status, retcode, data)`。
pub(crate) fn failed(response: ActionResponse<Value>) -> (Status, i32, Option<Value>) {
    assert_eq!(response.echo, Some(json!("e-1")));
    (response.status, response.retcode, response.data)
}
