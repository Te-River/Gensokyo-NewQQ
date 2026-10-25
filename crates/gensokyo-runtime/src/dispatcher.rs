//! action 分发：注册表查询、错误码映射、panic 隔离、`echo` 回传。
//!
//! 错误码沿用 OneBot V11 语义（与 Go 的 `callapi` 回退路径一致）：
//! - `1400`：请求 JSON 或参数非法（[`gensokyo_core::Error::Protocol`]）；
//! - `1404`：未知 action（含 [`gensokyo_core::Error::Unsupported`]）；
//! - `1500`：内部错误——配置缺失、存储/上游/IO 失败，以及 handler panic；panic 对外只回
//!   通用文案，全文只进日志。
//!
//! handler 是异步 trait（`gensokyo_protocol::Handler`），[`dispatch_json`] 直接 await：
//! 每条消息不再额外经一次 `spawn_blocking` 线程池调度；handler 需要上游 I/O 时自己 await。
//! panic 隔离用 `futures::FutureExt::catch_unwind`，并把 `handle` 的**调用本身**一起包进被
//! 捕获的 future——在构造 future 时 panic 与在 poll 时 panic 同样回 1500。

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use futures::FutureExt;
use gensokyo_core::Error as CoreError;
use gensokyo_protocol::Registry;
use gensokyo_protocol::action::{ActionRequest, ActionResponse, Echo};
use serde_json::Value;

/// 请求 JSON 或参数非法。
pub const RETCODE_BAD_REQUEST: i32 = 1400;
/// 未知 action。
pub const RETCODE_UNKNOWN_ACTION: i32 = 1404;
/// 内部错误（含 handler panic）。
pub const RETCODE_INTERNAL: i32 = 1500;

/// 把 action 请求喂给注册表。
#[derive(Debug, Clone)]
pub struct Dispatcher {
    registry: Arc<Registry>,
}

impl Dispatcher {
    /// 绑定一个注册表。
    #[must_use]
    pub fn new(registry: Arc<Registry>) -> Self {
        Self { registry }
    }

    /// 分发一条请求，**不**返回 `Err`：所有失败都表达为 `status=failed` + `retcode`。
    ///
    /// panic 隔离：handler 的 panic 不会带走 WS 连接处理任务或 HTTP 请求任务，更不会拖垮
    /// 进程（worktree 的 panic=abort 配置若启用则退化为进程退出，属预期）。
    pub async fn dispatch(&self, request: &ActionRequest<Value>) -> ActionResponse<Value> {
        let Some(handler) = self.registry.get(&request.action) else {
            tracing::debug!(action = %request.action, "未知 action");
            return failure(
                RETCODE_UNKNOWN_ACTION,
                format!("未知 action: {}", request.action),
                request.echo.clone(),
            );
        };

        // 把 `handle` 的调用也放进 async 块再捕获：handler 在构造 future 时 panic（例如参数
        // 解包的 `expect`）同样被隔离成 1500，而不是沿调用栈炸到传输层的连接任务。
        let guarded = AssertUnwindSafe(async { handler.handle(request).await }).catch_unwind();

        match guarded.await {
            Ok(Ok(mut response)) => {
                // handler 未显式回填 echo 时由分发器兜底，保证 OneBot 客户端能对上请求。
                if response.echo.is_none() {
                    response.echo = request.echo.clone();
                }
                response
            }
            Ok(Err(err)) => failure(retcode_for(&err), err.to_string(), request.echo.clone()),
            Err(payload) => {
                let message = panic_message(payload.as_ref());
                // 全文只进日志：panic 文本可能带内部路径或凭据，不能回给 OneBot 客户端。
                tracing::error!(action = %request.action, panic = %message, "handler panic 已被隔离");
                failure(RETCODE_INTERNAL, "handler panic", request.echo.clone())
            }
        }
    }
}

/// 解析一条线上 JSON（WS 文本帧 / HTTP body），分发并返回响应 JSON 文本。
///
/// 解析失败按 OneBot 语义回 `retcode=1400`；响应序列化失败时返回固定形状的兜底 JSON
/// （`ActionResponse<Value>` 的序列化理论上不会失败，但响应不能因为 codec 问题凭空消失）。
pub async fn dispatch_json(dispatcher: Dispatcher, payload: Vec<u8>) -> String {
    let request = match serde_json::from_slice::<ActionRequest<Value>>(&payload) {
        Ok(request) => request,
        Err(err) => {
            tracing::debug!(error = %err, "action 请求 JSON 解析失败");
            return encode(&failure(
                RETCODE_BAD_REQUEST,
                format!("invalid JSON: {err}"),
                None,
            ));
        }
    };

    // handler 是异步的：直接 await，不再经 `spawn_blocking` 走一次线程池调度。
    encode(&dispatcher.dispatch(&request).await)
}

fn failure(retcode: i32, message: impl Into<String>, echo: Option<Echo>) -> ActionResponse<Value> {
    ActionResponse::<Value>::failed(retcode, message).with_echo(echo)
}

fn retcode_for(err: &CoreError) -> i32 {
    match err {
        CoreError::Unsupported(_) => RETCODE_UNKNOWN_ACTION,
        // 只有请求侧参数错误才是 1400；配置缺失是服务端自身的问题，按 1500 上报。
        CoreError::Protocol(_) => RETCODE_BAD_REQUEST,
        _ => RETCODE_INTERNAL,
    }
}

fn encode(response: &ActionResponse<Value>) -> String {
    serde_json::to_string(response).unwrap_or_else(|err| {
        tracing::error!(error = %err, "action 响应序列化失败");
        r#"{"status":"failed","retcode":1500,"data":null}"#.to_owned()
    })
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<非字符串 panic 负载>".to_owned())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gensokyo_core::Error;
    use gensokyo_protocol::Registry;
    use gensokyo_protocol::action::{ActionRequest, ActionResponse};
    use gensokyo_protocol::registry::{Handler, HandlerFuture};
    use serde_json::{Value, json};

    use super::{
        Dispatcher, RETCODE_BAD_REQUEST, RETCODE_INTERNAL, RETCODE_UNKNOWN_ACTION, dispatch_json,
    };

    #[derive(Debug)]
    struct Fixed;

    impl Handler for Fixed {
        fn name(&self) -> &'static str {
            "fixed"
        }

        fn handle<'a>(&'a self, _request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
            Box::pin(async { Ok(ActionResponse::ok(json!({ "pong": true }))) })
        }
    }

    /// 在 `handle` 本体 panic（future 构造阶段）：分发器必须连这一步也隔离。
    #[derive(Debug)]
    struct Panics;

    impl Handler for Panics {
        fn name(&self) -> &'static str {
            "panics"
        }

        fn handle<'a>(&'a self, _request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
            panic!("boom")
        }
    }

    #[derive(Debug)]
    struct Unimplemented;

    impl Handler for Unimplemented {
        fn name(&self) -> &'static str {
            "unimplemented"
        }

        fn handle<'a>(&'a self, _request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
            Box::pin(async { Err(Error::Unsupported("send_group_msg".to_owned())) })
        }
    }

    #[derive(Debug)]
    struct BadParams;

    impl Handler for BadParams {
        fn name(&self) -> &'static str {
            "bad_params"
        }

        fn handle<'a>(&'a self, _request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
            Box::pin(async { Err(Error::Protocol("group_id 缺失".to_owned())) })
        }
    }

    #[derive(Debug)]
    struct BadConfig;

    impl Handler for BadConfig {
        fn name(&self) -> &'static str {
            "bad_config"
        }

        fn handle<'a>(&'a self, _request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
            Box::pin(async { Err(Error::Config("settings 缺失".to_owned())) })
        }
    }

    fn dispatcher() -> Dispatcher {
        let mut registry = Registry::new();
        let _ = registry.register("fixed", Arc::new(Fixed));
        let _ = registry.register("panics", Arc::new(Panics));
        let _ = registry.register("unimplemented", Arc::new(Unimplemented));
        let _ = registry.register("bad_params", Arc::new(BadParams));
        let _ = registry.register("bad_config", Arc::new(BadConfig));
        Dispatcher::new(Arc::new(registry))
    }

    fn request(action: &str, echo: Option<Value>) -> ActionRequest<Value> {
        ActionRequest {
            action: action.to_owned(),
            params: None,
            echo,
        }
    }

    #[tokio::test]
    async fn unknown_action_is_1404_and_echo_is_kept() {
        let response = dispatcher()
            .dispatch(&request("nope", Some(json!("e1"))))
            .await;
        assert_eq!(response.retcode, RETCODE_UNKNOWN_ACTION);
        assert_eq!(response.echo, Some(json!("e1")));
        assert_eq!(response.status, gensokyo_protocol::action::Status::Failed);
    }

    #[tokio::test]
    async fn handler_panic_is_isolated_as_1500() {
        let response = dispatcher()
            .dispatch(&request("panics", Some(json!("e2"))))
            .await;
        assert_eq!(response.retcode, RETCODE_INTERNAL);
        assert_eq!(response.echo, Some(json!("e2")));
        // 对外只回通用文案，panic 全文留在日志里。
        assert_eq!(response.message.as_deref(), Some("handler panic"));
    }

    #[tokio::test]
    async fn handler_errors_map_to_onebot_retcodes() {
        let dispatcher = dispatcher();
        assert_eq!(
            dispatcher
                .dispatch(&request("unimplemented", None))
                .await
                .retcode,
            RETCODE_UNKNOWN_ACTION
        );
        assert_eq!(
            dispatcher
                .dispatch(&request("bad_params", None))
                .await
                .retcode,
            RETCODE_BAD_REQUEST
        );
        assert_eq!(
            dispatcher
                .dispatch(&request("bad_config", None))
                .await
                .retcode,
            RETCODE_INTERNAL
        );
    }

    #[tokio::test]
    async fn success_response_gets_echo_filled_in() {
        let response = dispatcher()
            .dispatch(&request("fixed", Some(json!({"id": 7}))))
            .await;
        assert_eq!(response.retcode, 0);
        assert_eq!(response.data, Some(json!({ "pong": true })));
        assert_eq!(response.echo, Some(json!({"id": 7})));
    }

    #[tokio::test]
    async fn invalid_json_is_1400() {
        let payload = b"not an action".to_vec();
        let body = dispatch_json(dispatcher(), payload).await;
        let value: Value = serde_json::from_str(&body).expect("响应应为 JSON");
        assert_eq!(value["retcode"], RETCODE_BAD_REQUEST);
        assert_eq!(value["status"], "failed");
        assert_eq!(value["data"], Value::Null);
    }

    #[tokio::test]
    async fn valid_json_roundtrips_through_json() {
        let payload = br#"{"action":"fixed","echo":"wire"}"#.to_vec();
        let body = dispatch_json(dispatcher(), payload).await;
        let value: Value = serde_json::from_str(&body).expect("响应应为 JSON");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["retcode"], 0);
        assert_eq!(value["echo"], "wire");
    }
}
