//! 文本类 action 的 [`Handler`] 适配器：只做规划，不发送。
//!
//! 成功响应的 `data` 是 [`SendRequest`] 的 JSON 加一个 `"deferred": true` 标记。它只是
//! handler → runtime 的内部握手载荷：runtime 注册这些 action 后，用
//! `serde_json::from_value::<SendRequest>(data)` 还原计划并接管真正的 QQ 调用（媒体上传 →
//! `MessageToCreate` → 发送 → echo / 懒消息回写），最终回给 OneBot 客户端的是发送结果，
//! **不会把计划本身直接回给客户端**。`data` 不含 `message_id` 等线上字段；反序列化时
//! `deferred` 标记（以及其它未知字段）被忽略。
//!
//! 参数缺失或非法时返回 `status = failed` + `retcode = 1400`（[`RETCODE_BAD_REQUEST`]，
//! 与 `gensokyo_runtime::dispatcher::RETCODE_BAD_REQUEST` 同值），`echo` 原样回传。
//!
//! 别名（`send_to_group`）不在这里注册：由 `gensokyo_protocol::Registry` 的别名表指向
//! [`SendGroupHandler`]。go-cqhttp 的 `send_group_msg_async` / `send_private_msg_async` /
//! `send_msg_async` 不是别名而是**独立注册名**（Go 里各自 `RegisterHandler` 到同一个函数），
//! 因此这里是三个独立 handler（[`SendGroupAsyncHandler`] 等），共用同一套规划函数。
//!
//! 六个 handler 只做规划、不碰 I/O，但仍按异步 trait 返回 `Box::pin(async move { … })`：
//! 后续 action 要 await QQ API 时无需再改注册表契约。

use gensokyo_core::{Error, Result};
use gensokyo_protocol::action::{ActionRequest, ActionResponse};
use gensokyo_protocol::{Handler, HandlerFuture};
use serde_json::Value;

use crate::send::{SendRequest, plan_send_group_msg, plan_send_msg, plan_send_private_msg};

/// 请求 JSON 或参数非法（与 runtime 的 `RETCODE_BAD_REQUEST` 同值）。
pub const RETCODE_BAD_REQUEST: i32 = 1400;

/// 规划结果的 JSON 标记：真正的发送由 runtime 接管。
pub const DEFERRED_FIELD: &str = "deferred";

/// `send_group_msg`（别名 `send_to_group`）。
#[derive(Debug, Clone, Copy, Default)]
pub struct SendGroupHandler;

impl SendGroupHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Handler for SendGroupHandler {
    fn name(&self) -> &'static str {
        "send_group_msg"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(async move { respond(request, plan_send_group_msg) })
    }
}

/// `send_private_msg`。
#[derive(Debug, Clone, Copy, Default)]
pub struct SendPrivateHandler;

impl SendPrivateHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Handler for SendPrivateHandler {
    fn name(&self) -> &'static str {
        "send_private_msg"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(async move { respond(request, plan_send_private_msg) })
    }
}

/// `send_msg`（群聊 / 私聊由 `message_type` 决定）。
#[derive(Debug, Clone, Copy, Default)]
pub struct SendMsgHandler;

impl SendMsgHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Handler for SendMsgHandler {
    fn name(&self) -> &'static str {
        "send_msg"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(async move { respond(request, plan_send_msg) })
    }
}

/// `send_group_msg_async`（go-cqhttp 扩展名）。
///
/// Go 的 `handlers/send_group_msg_async.go` 只是把该 action 注册到 `HandleSendGroupMsg`，语义与
/// 同步版完全相同；「异步」指调用方不等结果，服务端仍按同一路径发送。本 crate 的 deferred 计划
/// 本就是「先回收据、发送由 runtime 接管」，因此直接复用 [`plan_send_group_msg`]，响应形状与
/// [`SendGroupHandler`] 一致。
#[derive(Debug, Clone, Copy, Default)]
pub struct SendGroupAsyncHandler;

impl SendGroupAsyncHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Handler for SendGroupAsyncHandler {
    fn name(&self) -> &'static str {
        "send_group_msg_async"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(async move { respond(request, plan_send_group_msg) })
    }
}

/// `send_private_msg_async`（go-cqhttp 扩展名，Go 复用 `HandleSendPrivateMsg`）。
#[derive(Debug, Clone, Copy, Default)]
pub struct SendPrivateAsyncHandler;

impl SendPrivateAsyncHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Handler for SendPrivateAsyncHandler {
    fn name(&self) -> &'static str {
        "send_private_msg_async"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(async move { respond(request, plan_send_private_msg) })
    }
}

/// `send_msg_async`（go-cqhttp 扩展名，Go 复用 `HandleSendMsg`）。
#[derive(Debug, Clone, Copy, Default)]
pub struct SendMsgAsyncHandler;

impl SendMsgAsyncHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Handler for SendMsgAsyncHandler {
    fn name(&self) -> &'static str {
        "send_msg_async"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(async move { respond(request, plan_send_msg) })
    }
}

/// 六个 handler 的公共流程：规划 → `failed(1400)` 或 `ok(deferred)`，两路都回传 echo。
fn respond(
    request: &ActionRequest<Value>,
    planner: fn(&Value) -> Result<SendRequest>,
) -> Result<ActionResponse<Value>> {
    let null = Value::Null;
    let params = request.params.as_ref().unwrap_or(&null);
    respond_to_params(request, params, planner)
}

/// 用调用方给的 `params` 走同一条规划流程。
///
/// `.handle_quick_operation` 把 `context` / `operation` 还原成发送参数后复用这里，响应形状与
/// 同步发送 action 完全一致（[`DEFERRED_FIELD`] + [`SendRequest`]）。
pub(crate) fn respond_to_params(
    request: &ActionRequest<Value>,
    params: &Value,
    planner: fn(&Value) -> Result<SendRequest>,
) -> Result<ActionResponse<Value>> {
    let planned = match planner(params) {
        Ok(planned) => planned,
        Err(err) => {
            return Ok(ActionResponse::failed(RETCODE_BAD_REQUEST, err.to_string())
                .with_echo(request.echo.clone()));
        }
    };
    Ok(ActionResponse::ok(deferred_data(&planned)?).with_echo(request.echo.clone()))
}

/// `SendRequest` 的 JSON + `"deferred": true`。
fn deferred_data(planned: &SendRequest) -> Result<Value> {
    let mut data = serde_json::to_value(planned)
        .map_err(|err| Error::Protocol(format!("SendRequest 序列化失败: {err}")))?;
    if let Value::Object(object) = &mut data {
        object.insert(DEFERRED_FIELD.to_owned(), Value::Bool(true));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    use serde_json::{Value, json};

    use super::{
        DEFERRED_FIELD, RETCODE_BAD_REQUEST, SendGroupAsyncHandler, SendGroupHandler,
        SendMsgAsyncHandler, SendMsgHandler, SendPrivateAsyncHandler, SendPrivateHandler,
    };
    use gensokyo_protocol::Handler;
    use gensokyo_protocol::Status;
    use gensokyo_protocol::action::{ActionRequest, ActionResponse};

    use crate::send::{SendRequest, SendTarget};

    /// 单测里的最小 `block_on`：三个 handler 都不碰 I/O，一次 poll 即 `Ready`。
    ///
    /// 不给本 crate 加 `futures` / `tokio` 的 dev-dependency——那会改动 `Cargo.lock`。
    fn block_on<F: Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let mut context = Context::from_waker(Waker::noop());
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    fn request(action: &str, params: Value) -> ActionRequest<Value> {
        serde_json::from_value(json!({"action": action, "params": params, "echo": "e-1"}))
            .expect("请求应可反序列化")
    }

    fn request_without_params(action: &str) -> ActionRequest<Value> {
        ActionRequest {
            action: action.to_owned(),
            params: None,
            echo: Some(json!("e-1")),
        }
    }

    fn ok_data(response: ActionResponse<Value>) -> Value {
        assert_eq!(response.status, Status::Ok);
        assert_eq!(response.retcode, 0);
        assert_eq!(response.echo, Some(json!("e-1")));
        response.data.expect("成功响应应带 data")
    }

    #[test]
    fn send_group_handler_returns_deferred_plan() {
        let req = request("send_group_msg", json!({"group_id": 1, "message": "hi"}));
        let response = block_on(SendGroupHandler::new().handle(&req));
        let data = ok_data(response.expect("handler 不应返回 Err"));

        assert_eq!(data[DEFERRED_FIELD], json!(true));
        assert_eq!(data["target"], json!({"group": "1"}));
        assert_eq!(data["message"]["text"], json!("hi"));
        assert_eq!(data["message"]["plan"], json!([]));
        assert_eq!(SendGroupHandler.name(), "send_group_msg");
    }

    #[test]
    fn deferred_data_round_trips_without_message_id() {
        let req = request("send_group_msg", json!({"group_id": 1, "message": "hi"}));
        let response = block_on(SendGroupHandler::new().handle(&req));
        let data = ok_data(response.expect("handler 不应返回 Err"));

        assert_eq!(data[DEFERRED_FIELD], json!(true));
        assert!(
            data.get("message_id").is_none(),
            "计划里不应出现 message_id: {data}"
        );

        let restored: SendRequest =
            serde_json::from_value(data).expect("runtime 应能从 deferred 载荷还原计划");
        assert_eq!(restored.target, SendTarget::Group("1".to_owned()));
        assert_eq!(restored.message.text, "hi");
    }

    #[test]
    fn send_private_handler_returns_deferred_plan() {
        let req = request(
            "send_private_msg",
            json!({"user_id": "abc", "message": "[CQ:reply,id=9]hi"}),
        );
        let response = block_on(SendPrivateHandler::new().handle(&req));
        let data = ok_data(response.expect("handler 不应返回 Err"));

        assert_eq!(data[DEFERRED_FIELD], json!(true));
        assert_eq!(data["target"], json!({"private": "abc"}));
        assert_eq!(data["message"]["text"], json!("hi"));
        assert_eq!(
            data["message"]["plan"],
            json!([{"type": "reply", "data": {"id": "9"}}])
        );
        assert_eq!(SendPrivateHandler.name(), "send_private_msg");
    }

    #[test]
    fn send_msg_handler_dispatches_both_targets() {
        let handler = SendMsgHandler::new();

        let group_req = request(
            "send_msg",
            json!({"message_type": "group", "group_id": 2, "message": "g"}),
        );
        let group = ok_data(block_on(handler.handle(&group_req)).expect("群聊分派应成功"));
        assert_eq!(group["target"], json!({"group": "2"}));

        let private_req = request(
            "send_msg",
            json!({"message_type": "private", "user_id": 3, "message": "p"}),
        );
        let private = ok_data(block_on(handler.handle(&private_req)).expect("私聊分派应成功"));
        assert_eq!(private["target"], json!({"private": "3"}));
        assert_eq!(SendMsgHandler.name(), "send_msg");
    }

    #[test]
    fn async_variants_reuse_the_sync_plan() {
        // (async handler, 同步 handler, params)：形状必须与同步版逐字段一致
        let async_group = SendGroupAsyncHandler::new();
        let sync_group = SendGroupHandler::new();
        let params = json!({"group_id": 1, "message": "hi"});
        let async_data = ok_data(
            block_on(async_group.handle(&request("send_group_msg_async", params.clone())))
                .expect("异步规划不应失败"),
        );
        let sync_data = ok_data(
            block_on(sync_group.handle(&request("send_group_msg", params)))
                .expect("同步规划不应失败"),
        );
        assert_eq!(async_data, sync_data);
        assert_eq!(async_data[DEFERRED_FIELD], json!(true));
        assert_eq!(async_group.name(), "send_group_msg_async");

        let async_private = SendPrivateAsyncHandler::new();
        let private_params = json!({"user_id": "abc", "message": "hi"});
        let async_data = ok_data(
            block_on(
                async_private.handle(&request("send_private_msg_async", private_params.clone())),
            )
            .expect("异步规划不应失败"),
        );
        let sync_data = ok_data(
            block_on(
                SendPrivateHandler::new().handle(&request("send_private_msg", private_params)),
            )
            .expect("同步规划不应失败"),
        );
        assert_eq!(async_data, sync_data);
        assert_eq!(async_private.name(), "send_private_msg_async");

        let async_msg = SendMsgAsyncHandler::new();
        let msg_params = json!({"message_type": "group", "group_id": 2, "message": "g"});
        let async_data = ok_data(
            block_on(async_msg.handle(&request("send_msg_async", msg_params.clone())))
                .expect("异步规划不应失败"),
        );
        let sync_data = ok_data(
            block_on(SendMsgHandler::new().handle(&request("send_msg", msg_params)))
                .expect("同步规划不应失败"),
        );
        assert_eq!(async_data, sync_data);
        assert_eq!(async_msg.name(), "send_msg_async");
    }

    #[test]
    fn async_variants_fail_with_1400_like_the_sync_ones() {
        for handler in [
            &SendGroupAsyncHandler::new() as &dyn Handler,
            &SendPrivateAsyncHandler::new(),
            &SendMsgAsyncHandler::new(),
        ] {
            let response = block_on(handler.handle(&request_without_params(handler.name())))
                .expect("参数错误也应是 Ok(失败响应)");
            assert_eq!(response.status, Status::Failed);
            assert_eq!(response.retcode, RETCODE_BAD_REQUEST);
            assert_eq!(response.data, None);
        }
    }

    #[test]
    fn missing_params_fail_with_1400() {
        let req = ActionRequest {
            action: "send_group_msg".to_owned(),
            params: None,
            echo: Some(json!({"id": 7})),
        };
        let response = block_on(SendGroupHandler::new().handle(&req));
        let response = response.expect("参数错误也应是 Ok(失败响应)");

        assert_eq!(response.status, Status::Failed);
        assert_eq!(response.retcode, RETCODE_BAD_REQUEST);
        assert_eq!(response.data, None);
        assert_eq!(response.echo, Some(json!({"id": 7})));
        assert!(
            response
                .message
                .as_deref()
                .is_some_and(|message| message.contains("params"))
        );
    }

    #[test]
    fn invalid_params_fail_with_1400_and_keep_echo() {
        for (handler, params) in [
            (
                &SendGroupHandler::new() as &dyn Handler,
                json!({"group_id": 1, "message": ""}),
            ),
            (&SendPrivateHandler::new(), json!({"message": "x"})),
            (
                &SendMsgHandler::new(),
                json!({"message_type": "guild", "group_id": 1, "message": "x"}),
            ),
        ] {
            let req = request(handler.name(), params);
            let response = block_on(handler.handle(&req));
            let response = response.expect("参数错误也应是 Ok(失败响应)");

            assert_eq!(response.status, Status::Failed);
            assert_eq!(response.retcode, RETCODE_BAD_REQUEST);
            assert_eq!(response.data, None);
            assert_eq!(response.echo, Some(json!("e-1")));
            assert!(response.message.is_some());
        }
    }
}
