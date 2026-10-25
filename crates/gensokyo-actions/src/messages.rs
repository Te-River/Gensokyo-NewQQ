//! 消息类 action：`delete_msg` / `get_msg` / `get_forward_msg`。
//!
//! # 覆盖范围
//!
//! | action | Go 对应 | 上游端点 | 结果 |
//! |--------|---------|----------|------|
//! | `delete_msg` | `handlers/delete_msg.go` | 消息撤回端点 | 未在 `gensokyo-qq` 落地 → `1500` |
//! | `get_msg` | 无（Go 未注册） | 官方无按 message_id 查询端点 | `1500` |
//! | `get_forward_msg` | 无（Go 未注册） | 官方无合并转发查询端点 | `1500` |
//!
//! 三个 action 的**参数翻译与 ID 还原**已经完整实现，缺的只是上游端点：`gensokyo-qq` 的公开
//! 方法补齐后，handler 不用改。
//!
//! # message_id 口径（对齐 Go `DeleteMsg`）
//!
//! `string_ob11 = true` 时请求里的 `message_id` 就是真实消息 ID，原样透传；否则它是 idmap 分配的
//! 虚拟号，需要用 [`gensokyo_storage::IdMapStore::retrieve_row_by_cache`] 还原——未装配 idmap
//! 或映射不存在时回 `1400`，不做「拿虚拟号直接撤回」的猜测。
//!
//! # 刻意的差异（相对 Go）
//!
//! - **`delete_msg` 必须给 `group_id` 或 `user_id`**：Go 在两者都缺时静默回 `ok` 且什么都不做；
//!   这里回 `1400`，避免把「参数漏了」当成「撤回成功」。
//! - **撤回失败会上抛**：Go 只把错误打进日志、仍然回 `ok`；这里回 `1500` + 上游错误文案。
//! - **`get_msg` / `get_forward_msg` 是 OneBot 标准 action**：Go 的注册表里没有它们，因此没有可
//!   对照的 Go 语义，参数形状取 OneBot v11 约定（`message_id` 必填 / `id` 或 `message_id` 二选一）。

use std::sync::Arc;

use serde_json::Value;

use gensokyo_core::{Error, Result};
use gensokyo_protocol::MessageContent;
use gensokyo_protocol::action::{ActionRequest, ActionResponse, GetMsgResult};
use gensokyo_protocol::{Extra, Handler, HandlerFuture};

use crate::params;
use crate::qqapi::{ActionContext, RetractRequest};
use crate::util::{
    bad_request, ob11_id, ok_data, params_object, resolve_identity_text, upstream_error,
};

/// `delete_msg`：按 message_id 撤回（群聊 / C2C 二选一）。
#[derive(Debug, Clone)]
pub struct DeleteMsgHandler {
    context: Arc<ActionContext>,
}

impl DeleteMsgHandler {
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
        let requested = match params::required_id(object, "message_id") {
            Ok(requested) => requested,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let message_id = match resolve_message_id(&self.context, &requested) {
            Ok(message_id) => message_id,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let target = match retract_target(&self.context, object, message_id) {
            Ok(target) => target,
            Err(err) => return Ok(bad_request(request, &err)),
        };

        if let Err(err) = self.context.api().delete_msg(target).await {
            return Ok(upstream_error(request, &err));
        }
        // OneBot 的 delete_msg 成功回执没有业务数据
        Ok(ActionResponse::ok(Value::Null).with_echo(request.echo.clone()))
    }
}

impl Handler for DeleteMsgHandler {
    fn name(&self) -> &'static str {
        "delete_msg"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `get_msg`：按 message_id 取消息内容。
#[derive(Debug, Clone)]
pub struct GetMsgHandler {
    context: Arc<ActionContext>,
}

impl GetMsgHandler {
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
        let requested = match params::required_id(object, "message_id") {
            Ok(requested) => requested,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let message_id = match resolve_message_id(&self.context, &requested) {
            Ok(message_id) => message_id,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let detail = match self.context.api().get_msg(&message_id).await {
            Ok(detail) => detail,
            Err(err) => return Ok(upstream_error(request, &err)),
        };

        // 上游给了消息段就用数组形态，否则回退纯文本
        let message = if detail.segments.is_empty() {
            MessageContent::Text(detail.text)
        } else {
            MessageContent::Array(detail.segments)
        };
        let data = GetMsgResult {
            message_id: Some(ob11_id(&requested, self.context.string_ob11())),
            message: Some(message),
            extra: Extra::new(),
        };
        ok_data(request, &data)
    }
}

impl Handler for GetMsgHandler {
    fn name(&self) -> &'static str {
        "get_msg"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `get_forward_msg`：按转发 ID 取合并转发节点。
#[derive(Debug, Clone)]
pub struct GetForwardMsgHandler {
    context: Arc<ActionContext>,
}

impl GetForwardMsgHandler {
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
        // OneBot v11 的字段是 `id`；部分客户端沿用 `message_id`，两者同义
        let forward_id = match params::id_text(object, "id") {
            Ok(Some(id)) => id,
            Ok(None) => match params::required_id(object, "message_id") {
                Ok(id) => id,
                Err(err) => return Ok(bad_request(request, &err)),
            },
            Err(err) => return Ok(bad_request(request, &err)),
        };

        let nodes = match self.context.api().get_forward_msg(&forward_id).await {
            Ok(nodes) => nodes,
            Err(err) => return Ok(upstream_error(request, &err)),
        };
        ok_data(request, &nodes)
    }
}

impl Handler for GetForwardMsgHandler {
    fn name(&self) -> &'static str {
        "get_forward_msg"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// 请求里的 `message_id` → 上游真实消息 ID。
///
/// `string_ob11` 为真时原样透传；否则按 idmap 虚拟号还原（Go `RetrieveRowByCachev2` 语义）。
fn resolve_message_id(context: &ActionContext, requested: &str) -> Result<String> {
    if context.string_ob11() {
        return Ok(requested.to_owned());
    }
    let vuin = requested.parse::<i64>().map_err(|_| {
        Error::Protocol(format!(
            "message_id 在 string_ob11=false 时必须是数字虚拟号: {requested}"
        ))
    })?;
    let store = context.idmap().ok_or_else(|| {
        Error::Protocol("未装配 idmap，无法把 message_id 还原成真实 ID".to_owned())
    })?;
    store
        .retrieve_row_by_cache(vuin)
        .map_err(|err| Error::Protocol(format!("反查 message_id 失败: {err}")))
}

/// 撤回目标：`group_id` 优先，其次 `user_id`，两者都缺是参数错误。
fn retract_target(
    context: &ActionContext,
    object: &serde_json::Map<String, Value>,
    message_id: String,
) -> Result<RetractRequest> {
    if let Some(group_id) = params::id_text(object, "group_id")? {
        let group = resolve_identity_text(context, "group_id", &group_id, "group_openid")?;
        return Ok(RetractRequest {
            group_openid: Some(group.openid),
            user_openid: None,
            message_id,
        });
    }
    if let Some(user_id) = params::id_text(object, "user_id")? {
        let user = resolve_identity_text(context, "user_id", &user_id, "user_openid")?;
        return Ok(RetractRequest {
            group_openid: None,
            user_openid: Some(user.openid),
            message_id,
        });
    }
    Err(Error::Protocol(
        "delete_msg 需要 group_id 或 user_id".to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{DeleteMsgHandler, GetForwardMsgHandler, GetMsgHandler};
    use crate::handler::RETCODE_BAD_REQUEST;
    use crate::qqapi::MsgDetail;
    use crate::testing::{Call, block_on, context, failed, ok_data, request};
    use crate::util::RETCODE_UPSTREAM;
    use gensokyo_core::Error;
    use gensokyo_protocol::Handler;
    use gensokyo_protocol::Status;

    const GROUP: &str = "0123456789abcdef0123456789abcdef";
    const MEMBER: &str = "fedcba9876543210fedcba9876543210";

    #[test]
    fn delete_msg_forwards_retract_target() {
        let (context, api) = context();
        *api.delete.lock().unwrap() = Some(Ok(()));
        let handler = DeleteMsgHandler::new(context);
        let response = block_on(handler.handle(&request(
            "delete_msg",
            json!({"message_id": "m-1", "group_id": GROUP}),
        )))
        .expect("handler 不应返回 Err");
        let data = ok_data(response);

        assert_eq!(data, json!(null));
        assert_eq!(
            api.calls(),
            vec![Call::DeleteMsg(
                Some(GROUP.to_owned()),
                None,
                "m-1".to_owned()
            )]
        );
        assert_eq!(handler.name(), "delete_msg");
    }

    #[test]
    fn delete_msg_supports_c2c_targets() {
        let (context, api) = context();
        *api.delete.lock().unwrap() = Some(Ok(()));
        let handler = DeleteMsgHandler::new(context);
        let response = block_on(handler.handle(&request(
            "delete_msg",
            json!({"message_id": "m-2", "user_id": MEMBER}),
        )))
        .expect("handler 不应返回 Err");
        let _ = ok_data(response);

        assert_eq!(
            api.calls(),
            vec![Call::DeleteMsg(
                None,
                Some(MEMBER.to_owned()),
                "m-2".to_owned()
            )]
        );
    }

    #[test]
    fn delete_msg_requires_message_id_and_target() {
        let (context, api) = context();
        let handler = DeleteMsgHandler::new(context);
        for params in [json!({"group_id": GROUP}), json!({"message_id": "m-1"})] {
            let response = block_on(handler.handle(&request("delete_msg", params)))
                .expect("参数错误也应是 Ok(失败响应)");
            let (status, retcode, data) = failed(response);
            assert_eq!(status, Status::Failed);
            assert_eq!(retcode, RETCODE_BAD_REQUEST);
            assert_eq!(data, None);
        }
        assert!(api.calls().is_empty(), "参数错误不应触达上游");
    }

    #[test]
    fn delete_msg_unsupported_endpoint_maps_to_1500() {
        let (context, api) = context();
        *api.delete.lock().unwrap() = Some(Err(Error::Unsupported(
            "delete_msg: gensokyo-qq 尚未暴露消息撤回端点".to_owned(),
        )));
        let handler = DeleteMsgHandler::new(context);
        let response = block_on(handler.handle(&request(
            "delete_msg",
            json!({"message_id": "m-1", "group_id": GROUP}),
        )))
        .expect("上游错误也应是 Ok(失败响应)");
        let (status, retcode, data) = failed(response);
        assert_eq!(status, Status::Failed);
        assert_eq!(retcode, RETCODE_UPSTREAM);
        assert_eq!(data, None);
    }

    #[test]
    fn get_msg_prefers_segments_over_text() {
        let (context, api) = context();
        *api.msg.lock().unwrap() = Some(Ok(MsgDetail {
            message_id: "m-1".to_owned(),
            text: "hi".to_owned(),
            segments: gensokyo_protocol::cqcode::parse("hi[CQ:face,id=1]"),
        }));
        let handler = GetMsgHandler::new(context);
        let response = block_on(handler.handle(&request("get_msg", json!({"message_id": "m-1"}))))
            .expect("handler 不应返回 Err");
        let data = ok_data(response);

        assert_eq!(api.calls(), vec![Call::GetMsg("m-1".to_owned())]);
        assert_eq!(data["message_id"], json!("m-1"));
        assert_eq!(data["message"][0]["type"], json!("text"));
        assert_eq!(data["message"][1]["type"], json!("face"));
        assert_eq!(handler.name(), "get_msg");
    }

    #[test]
    fn get_msg_falls_back_to_text_without_segments() {
        let (context, api) = context();
        *api.msg.lock().unwrap() = Some(Ok(MsgDetail {
            message_id: "m-1".to_owned(),
            text: "纯文本".to_owned(),
            segments: Vec::new(),
        }));
        let handler = GetMsgHandler::new(context);
        let response = block_on(handler.handle(&request("get_msg", json!({"message_id": "m-1"}))))
            .expect("handler 不应返回 Err");
        let data = ok_data(response);

        assert_eq!(data["message"], json!("纯文本"));
    }

    #[test]
    fn get_msg_requires_message_id_and_maps_upstream_errors() {
        let (context, api) = context();
        let handler = GetMsgHandler::new(context);

        let response = block_on(handler.handle(&request("get_msg", json!({}))))
            .expect("参数错误也应是 Ok(失败响应)");
        assert_eq!(failed(response).1, RETCODE_BAD_REQUEST);

        *api.msg.lock().unwrap() = Some(Err(Error::Unsupported(
            "get_msg: 官方 v2 无按 message_id 查询消息的端点".to_owned(),
        )));
        let response = block_on(handler.handle(&request("get_msg", json!({"message_id": "m-1"}))))
            .expect("上游错误也应是 Ok(失败响应)");
        assert_eq!(failed(response).1, RETCODE_UPSTREAM);
    }

    #[test]
    fn get_forward_msg_accepts_id_and_message_id() {
        let (context, api) = context();
        *api.forward.lock().unwrap() = Some(Ok(vec![gensokyo_protocol::Segment::text("node")]));
        let handler = GetForwardMsgHandler::new(context);

        let response = block_on(handler.handle(&request("get_forward_msg", json!({"id": "f-1"}))))
            .expect("handler 不应返回 Err");
        let data = ok_data(response);
        assert_eq!(data[0]["type"], json!("text"));

        *api.forward.lock().unwrap() = Some(Ok(Vec::new()));
        let response =
            block_on(handler.handle(&request("get_forward_msg", json!({"message_id": "f-2"}))))
                .expect("handler 不应返回 Err");
        let _ = ok_data(response);

        assert_eq!(
            api.calls(),
            vec![
                Call::GetForwardMsg("f-1".to_owned()),
                Call::GetForwardMsg("f-2".to_owned()),
            ]
        );
        assert_eq!(handler.name(), "get_forward_msg");
    }

    #[test]
    fn get_forward_msg_requires_an_id() {
        let (context, api) = context();
        let handler = GetForwardMsgHandler::new(context);
        let response = block_on(handler.handle(&request("get_forward_msg", json!({}))))
            .expect("参数错误也应是 Ok(失败响应)");
        let (status, retcode, _) = failed(response);
        assert_eq!(status, Status::Failed);
        assert_eq!(retcode, RETCODE_BAD_REQUEST);
        assert!(api.calls().is_empty());
    }
}
