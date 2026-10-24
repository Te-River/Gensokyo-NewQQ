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
//! [`SendGroupHandler`]。

use gensokyo_core::{Error, Result};
use gensokyo_protocol::Handler;
use gensokyo_protocol::action::{ActionRequest, ActionResponse};
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

    fn handle(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        respond(request, plan_send_group_msg)
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

    fn handle(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        respond(request, plan_send_private_msg)
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

    fn handle(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        respond(request, plan_send_msg)
    }
}

/// 三个 handler 的公共流程：规划 → `failed(1400)` 或 `ok(deferred)`，两路都回传 echo。
fn respond(
    request: &ActionRequest<Value>,
    planner: fn(&Value) -> Result<SendRequest>,
) -> Result<ActionResponse<Value>> {
    let null = Value::Null;
    let params = request.params.as_ref().unwrap_or(&null);
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
    use serde_json::{Value, json};

    use super::{
        DEFERRED_FIELD, RETCODE_BAD_REQUEST, SendGroupHandler, SendMsgHandler, SendPrivateHandler,
    };
    use gensokyo_protocol::Handler;
    use gensokyo_protocol::Status;
    use gensokyo_protocol::action::{ActionRequest, ActionResponse};

    use crate::send::{SendRequest, SendTarget};

    fn request(action: &str, params: Value) -> ActionRequest<Value> {
        serde_json::from_value(json!({"action": action, "params": params, "echo": "e-1"}))
            .expect("请求应可反序列化")
    }

    fn ok_data(response: ActionResponse<Value>) -> Value {
        assert_eq!(response.status, Status::Ok);
        assert_eq!(response.retcode, 0);
        assert_eq!(response.echo, Some(json!("e-1")));
        response.data.expect("成功响应应带 data")
    }

    #[test]
    fn send_group_handler_returns_deferred_plan() {
        let response = SendGroupHandler::new()
            .handle(&request(
                "send_group_msg",
                json!({"group_id": 1, "message": "hi"}),
            ))
            .expect("handler 不应返回 Err");

        let data = ok_data(response);
        assert_eq!(data[DEFERRED_FIELD], json!(true));
        assert_eq!(data["target"], json!({"group": "1"}));
        assert_eq!(data["message"]["text"], json!("hi"));
        assert_eq!(data["message"]["plan"], json!([]));
        assert_eq!(SendGroupHandler.name(), "send_group_msg");
    }

    #[test]
    fn deferred_data_round_trips_without_message_id() {
        let data = ok_data(
            SendGroupHandler::new()
                .handle(&request(
                    "send_group_msg",
                    json!({"group_id": 1, "message": "hi"}),
                ))
                .expect("handler 不应返回 Err"),
        );

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
        let response = SendPrivateHandler::new()
            .handle(&request(
                "send_private_msg",
                json!({"user_id": "abc", "message": "[CQ:reply,id=9]hi"}),
            ))
            .expect("handler 不应返回 Err");

        let data = ok_data(response);
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

        let group = ok_data(
            handler
                .handle(&request(
                    "send_msg",
                    json!({"message_type": "group", "group_id": 2, "message": "g"}),
                ))
                .expect("群聊分派应成功"),
        );
        assert_eq!(group["target"], json!({"group": "2"}));

        let private = ok_data(
            handler
                .handle(&request(
                    "send_msg",
                    json!({"message_type": "private", "user_id": 3, "message": "p"}),
                ))
                .expect("私聊分派应成功"),
        );
        assert_eq!(private["target"], json!({"private": "3"}));
        assert_eq!(SendMsgHandler.name(), "send_msg");
    }

    #[test]
    fn missing_params_fail_with_1400() {
        let response = SendGroupHandler::new()
            .handle(&ActionRequest {
                action: "send_group_msg".to_owned(),
                params: None,
                echo: Some(json!({"id": 7})),
            })
            .expect("参数错误也应是 Ok(失败响应)");

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
            let response = handler
                .handle(&request(handler.name(), params))
                .expect("参数错误也应是 Ok(失败响应)");

            assert_eq!(response.status, Status::Failed);
            assert_eq!(response.retcode, RETCODE_BAD_REQUEST);
            assert_eq!(response.data, None);
            assert_eq!(response.echo, Some(json!("e-1")));
            assert!(response.message.is_some());
        }
    }
}
