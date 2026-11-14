//! 用户类 action：`get_friend_list` / `get_stranger_info` / `send_like`。
//!
//! # 覆盖范围
//!
//! | action | Go 对应 | 上游端点 | 结果 |
//! |--------|---------|----------|------|
//! | `get_friend_list` | `handlers/get_friend_list.go` | 无上游：直读 idmap `user_info` 表 | `0` |
//! | `get_stranger_info` | 无（Go 未注册） | 官方无陌生人资料端点 | `1500` |
//! | `send_like` | 无（Go 未注册） | 官方无点赞端点 | `1500` |
//!
//! # 与 Go 的差异
//!
//! `get_friend_list` 与 Go 同样不走上游：直读 idmap 本地 `user_info` 表
//! （Go `idmap.ListAllUsers`，`idmap/service.go:2290-2314`，Rust 侧为
//! [`gensokyo_storage::IdMapStore::list_all_users`]）。按表 key 字节序升序枚举、无去重，
//! 仅过滤空 `user_id`（Go `get_friend_list.go:36-40`），条目字段按 Go 声明序 `nickname`、
//! `remark`、`user_id` 输出。空表 / 未装配 / 枚举出错时 Go 是 `append(nil, …)` 得 nil 切片
//! ⇒ `"data":null`（不是 `[]`），且恒 `retcode=0 status="ok"`、错误只记日志
//! （`get_friend_list.go:30-32,45-47`）——不套 [`crate::group_admin`] 的失败分支。
//! **写侧（Go `StoreUserInfo`，入站 C2C 时写 `ProcessC2CMessage.go:231-241`）尚未移植
//! （P64b）**：本表当前只有 M1e 导入器的数据，全新 Rust 部署在导入前恒回 `data: null`。
//!
//! `get_stranger_info` / `send_like` 是 OneBot v11 标准 action，Go 未注册；参数形状按 OneBot
//! 约定，端点缺失时同样回 `1500`。
//!
//! # 响应 ID
//!
//! `get_friend_list` 的 `user_id` 是表内原串，Go 不做虚拟号转换，这里恒按 JSON 字符串输出
//! （不读 `string_ob11`）。`get_stranger_info` / `send_like` 的 `user_id` 回填请求值（虚拟号
//! 形态由配置 `string_ob11` 决定），真实 `user_openid` 放在附加字段 `user_openid` 里——与
//! [`crate::group`] 的成员信息同一口径。

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
    upstream_error,
};

/// `send_like` 未传 `times` 时的默认次数（与 OneBot v11 的 1 次一致）。
pub const DEFAULT_LIKE_TIMES: u32 = 1;

/// 单次 `send_like` 的次数上限（官方每天每人最多 10 次）。超过按上限截断。
pub const MAX_LIKE_TIMES: u32 = 10;

/// `get_friend_list` 的条目：字段声明序逐字对齐 Go `structs.FriendData`
/// （`structs/structs.go:3-7`）——`nickname`、`remark`、`user_id`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendInfo {
    /// 昵称。
    pub nickname: String,
    /// 备注。
    pub remark: String,
    /// 用户 ID：`user_info` 表内原串（Go 不做虚拟号转换，恒按 JSON 字符串输出）。
    pub user_id: Ob11Id,
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
        // 与 Go 同样直读 idmap、不触上游：未装配 / 枚举出错都只 warn 后继续
        // （Go `get_friend_list.go:30-32` 把 err 记日志吞掉，`data` 保持 nil）
        let rows = match self.context.idmap() {
            Some(store) => match store.list_all_users() {
                Ok(rows) => rows,
                Err(err) => {
                    tracing::warn!(error = %err, "get_friend_list: idmap 枚举失败，按空列表处理");
                    Vec::new()
                }
            },
            None => {
                tracing::warn!("get_friend_list: 未装配 idmap，按空列表处理");
                Vec::new()
            }
        };
        // Go 仅过滤空 `user_id`、其余字段原样透传（`get_friend_list.go:34-40`）；
        // 过滤后为空时 Go 是 `append(nil, …)` 得 nil 切片 ⇒ `"data":null`，不是 `[]`
        let friends: Vec<FriendInfo> = rows
            .into_iter()
            .filter(|row| !row.user_id.is_empty())
            .map(|row| FriendInfo {
                nickname: row.nickname,
                remark: row.remark,
                user_id: Ob11Id::Str(row.user_id),
            })
            .collect();
        if friends.is_empty() {
            return ok_data(request, &Value::Null);
        }
        ok_data(request, &friends)
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
    use crate::qqapi::StrangerProfile;
    use crate::testing::{
        Call, block_on, context, context_with_idmap, failed, ok_data, request,
        request_without_params,
    };
    use crate::util::RETCODE_UPSTREAM;
    use gensokyo_core::Error;
    use gensokyo_protocol::Handler;
    use gensokyo_protocol::Status;
    use gensokyo_storage::IdMapStore;
    use gensokyo_storage::importer::{ImportOptions, import_bytes};
    use std::sync::Arc;

    const MEMBER: &str = "fedcba9876543210fedcba9876543210";

    /// `user_info` 播种 dump：写侧 `StoreUserInfo` 属 P64b，公共写入路径目前只有 M1e 导入器。
    /// `end.sha256` 是前 4 行原始字节（含各行行尾 `\n`，共 371 字节，LF、无 BOM）的 SHA-256
    /// 小写十六进制；改动上面 4 行必须同步重算，否则导入直接以格式错失败（不会静默半库）。
    const USER_INFO_DUMP: &str = concat!(
        "{\"kind\":\"meta\",\"format\":\"gensokyo-idmap\",\"version\":1}\n",
        "{\"kind\":\"user_info\",\"key\":\"Bob:100002\",\"value\":{\"nickname\":\"Bob\",\"remark\":\"b-note\",\"user_id\":\"100002\",\"Private\":true}}\n",
        "{\"kind\":\"user_info\",\"key\":\"alice:200001\",\"value\":{\"nickname\":\"Alice\",\"remark\":\"a-note\",\"user_id\":\"200001\"}}\n",
        "{\"kind\":\"user_info\",\"key\":\"alice:\",\"value\":{\"nickname\":\"Ghost\",\"remark\":\"\",\"user_id\":\"\"}}\n",
        "{\"kind\":\"end\",\"lines\":4,\"sha256\":\"876211fa563c7b6a732c0b3178dd008929c86e8778d31dbf812f1cd3769a3fb3\"}\n",
    );

    /// 全新 idmap 存储（临时目录唯一后缀；与 `group_admin` 测试同口径）。
    fn fresh_store(tag: &str) -> Arc<IdMapStore> {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "gensokyo-actions-friend-{tag}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).expect("创建临时存储目录");
        let options = gensokyo_storage::IdMapOptions::new(
            dir.join("openid-map.redb"),
            dir.join("msgid-map.redb"),
        )
        .with_app_id("102001234");
        Arc::new(IdMapStore::open(options).expect("打开 idmap 存储失败"))
    }

    #[test]
    fn friend_list_reads_idmap_user_info_in_key_order() {
        let store = fresh_store("list");
        let report = import_bytes(&store, USER_INFO_DUMP.as_bytes(), &ImportOptions::default())
            .expect("导入 user_info 播种");
        assert_eq!(report.user_infos, 3);

        let (context, api) = context_with_idmap(store, true);
        let handler = GetFriendListHandler::new(context);
        let response = block_on(handler.handle(&request_without_params("get_friend_list")))
            .expect("handler 不应返回 Err");
        assert!(
            api.calls().is_empty(),
            "get_friend_list 不触上游（Go 同样只读库）"
        );
        // 外部 JSON 逐字节对 Go：key 字节序升序（`Bob:` < `alice:`），空 `user_id` 行被
        // 过滤（`get_friend_list.go:36-40`）；条目字段序 nickname,remark,user_id（Go 声明序
        // 与字母序恰好一致，`serde_json::Value` 按 BTreeMap 排序输出）；信封沿用本仓
        // ActionResponse 声明序口径（R3a 未重排，见 P64a 文档）
        assert_eq!(
            serde_json::to_string(&response).expect("序列化响应"),
            r#"{"status":"ok","retcode":0,"data":[{"nickname":"Bob","remark":"b-note","user_id":"100002"},{"nickname":"Alice","remark":"a-note","user_id":"200001"}],"echo":"e-1"}"#
        );
        assert_eq!(handler.name(), "get_friend_list");
    }

    #[test]
    fn friend_list_empty_table_outputs_data_null() {
        // Go 空态是 `append(nil, …)` 得 nil 切片 ⇒ `"data":null` 而非 `[]`
        // （`get_friend_list.go:35/43`）；retcode 恒 0、status ok
        let store = fresh_store("empty");
        let (context, _api) = context_with_idmap(store, true);
        let handler = GetFriendListHandler::new(context);
        let response = block_on(handler.handle(&request_without_params("get_friend_list")))
            .expect("handler 不应返回 Err");
        assert_eq!(
            serde_json::to_string(&response).expect("序列化响应"),
            r#"{"status":"ok","retcode":0,"data":null,"echo":"e-1"}"#
        );
    }

    #[test]
    fn friend_list_without_idmap_still_returns_ok() {
        // 未装配 idmap：Go 无此态（库恒存在），这里按同一降级口径——warn 后
        // retcode=0 + `"data":null`，不打 1500
        let (context, api) = context();
        let handler = GetFriendListHandler::new(context);
        let response = block_on(handler.handle(&request("get_friend_list", json!({}))))
            .expect("降级也应是 Ok(成功响应)");
        assert_eq!(response.status, Status::Ok);
        assert_eq!(response.retcode, 0);
        assert_eq!(ok_data(response), json!(null));
        assert!(api.calls().is_empty());
    }

    #[test]
    fn friend_list_rejects_non_object_params() {
        let store = fresh_store("badparams");
        let (context, api) = context_with_idmap(store, true);
        let handler = GetFriendListHandler::new(context);
        let response = block_on(handler.handle(&request("get_friend_list", json!([1, 2]))))
            .expect("参数错误也应是 Ok(失败响应)");
        assert_eq!(failed(response).1, RETCODE_BAD_REQUEST);
        assert!(api.calls().is_empty());
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
