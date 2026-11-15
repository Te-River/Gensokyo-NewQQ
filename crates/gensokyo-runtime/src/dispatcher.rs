//! action 分发：注册表查询、错误码映射、panic 隔离、`echo` 回传、deferred 计划执行。
//!
//! 错误码沿用 OneBot V11 语义（与 Go 的 `callapi` 回退路径一致）：
//! - `1400`：请求 JSON 或参数非法（[`gensokyo_core::Error::Protocol`]）；
//! - `1404`：未知 action（含 [`gensokyo_core::Error::Unsupported`]）；
//! - `1500`：内部错误——配置缺失、存储/上游/IO 失败，以及 handler panic；panic 对外只回
//!   通用文案，全文只进日志。
//!
//! # deferred 计划
//!
//! `send_*` 的 handler 只回计划（`data.deferred == true`）。分发器识别到该标记后按载荷键分派：
//! 单条 [`gensokyo_actions::SendRequest`] → [`crate::deferred::execute`]（合成 `MessageToCreate`，
//! 含媒体上传与 reply 还原，唤醒计划落体 `is_wakeup`，其终局再经 [`Dispatcher::with_hub`] 挂上的
//! 总线回推 `c2c_wakeup_resp` notice，对齐 Go `sendWakeupNotice`）；`message_sse` 键 →
//! [`crate::deferred::execute_sse`]（SSE 形态体直发 C2C 端点）；`forward` 键 →
//! [`crate::deferred::execute_forward`]（合并转发逐条重发，成功 ID 进 `message_ids`、失败
//! 进带**原 node 序号**的 `failed` 清单——规划层解析失败的 node 也在册，整体 retcode 恒
//! ok——Go 的回执语义）。计划本身（含 `deferred`
//! 标记）不会回给客户端。
//!
//! 执行失败按错误类型分档：请求侧问题（媒体来源 / 大小 / 类型、reply 无法还原）是
//! `retcode 1400`，其余（未装配 [`gensokyo_actions::ActionContext`]、计划损坏、下载 / 上传 /
//! 上游失败）一律 `retcode 1500`——与 handler 阶段的 [`retcode_for`] 不同，这里不把
//! [`CoreError::Unsupported`] 当成 `1404`：计划已经交给了 runtime，未知能力是服务端问题。
//!
//! handler 是异步 trait（`gensokyo_protocol::Handler`），[`dispatch_json`] 直接 await：
//! 每条消息不再额外经一次 `spawn_blocking` 线程池调度；handler 需要上游 I/O 时自己 await。
//! panic 隔离用 `futures::FutureExt::catch_unwind`，并把 `handle` 的**调用本身**一起包进被
//! 捕获的 future——在构造 future 时 panic 与在 poll 时 panic 同样回 1500。
//! 注意：release 构建是 `panic = "abort"`，`catch_unwind` 在 abort 口径下不生效，这里的隔离
//! 退化为**进程退出**（含 SSM / echo / 延迟队列的内存态）；运维后果见 `docs/rust-build.md`。

use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use gensokyo_actions::{ActionContext, ForwardRequest, SendRequest, SendTarget, SseRequest};
use gensokyo_core::Error as CoreError;
use gensokyo_core::Ob11Id;
use gensokyo_core::telemetry::metrics;
use gensokyo_protocol::Registry;
use gensokyo_protocol::action::{ActionRequest, ActionResponse, Echo};
use gensokyo_storage::{BotStatsStore, EchoStore};
use serde_json::{Value, json};

use crate::deferred::DeferredDeps;
use crate::event_hub::EventHub;
use crate::media::MediaRuntime;

/// 请求 JSON 或参数非法。
pub const RETCODE_BAD_REQUEST: i32 = 1400;
/// 未知 action。
pub const RETCODE_UNKNOWN_ACTION: i32 = 1404;
/// 内部错误（含 handler panic 与 deferred 执行失败）。
pub const RETCODE_INTERNAL: i32 = 1500;

/// 把 action 请求喂给注册表。
#[derive(Debug, Clone)]
pub struct Dispatcher {
    registry: Arc<Registry>,
    actions: Option<Arc<ActionContext>>,
    media: Arc<MediaRuntime>,
    echo: Option<Arc<EchoStore>>,
    botstats: Option<Arc<BotStatsStore>>,
    hub: Option<EventHub>,
    forward_interval: Duration,
    url_mint: Option<Arc<crate::url_pipeline::UrlMint>>,
}

impl Dispatcher {
    /// 绑定一个注册表（未装配 action 依赖：deferred 计划会以 `1500` 失败）。
    #[must_use]
    pub fn new(registry: Arc<Registry>) -> Self {
        Self {
            registry,
            actions: None,
            media: Arc::new(MediaRuntime::default()),
            echo: None,
            botstats: None,
            hub: None,
            forward_interval: crate::deferred::DEFAULT_FORWARD_SEND_INTERVAL,
            url_mint: None,
        }
    }

    /// 挂上 action 依赖容器，`send_*` 的 deferred 计划才会真正执行。
    #[must_use]
    pub fn with_actions(mut self, actions: Option<Arc<ActionContext>>) -> Self {
        self.actions = actions;
        self
    }

    /// 覆盖媒体下载器（单测注入假传输；生产用 [`Dispatcher::new`] 的默认 HTTP 实现）。
    #[must_use]
    pub fn with_media(mut self, media: Arc<MediaRuntime>) -> Self {
        self.media = media;
        self
    }

    /// 挂上 echo 存储：deferred 发送成功后回写 `message_id`，`[CQ:reply]` 也靠它反查虚拟号。
    #[must_use]
    pub fn with_echo(mut self, echo: Option<Arc<EchoStore>>) -> Self {
        self.echo = echo;
        self
    }

    /// 挂上事件总线：wakeup 发送到达终局后经它回推 `c2c_wakeup_resp` notice（对齐 Go
    /// `sendWakeupNotice`）。传 `None` 或未挂时静默跳过，发送与回执口径不变。
    #[must_use]
    pub fn with_hub(mut self, hub: Option<EventHub>) -> Self {
        self.hub = hub;
        self
    }

    /// 挂上 botstats 统计库：每次 deferred 发送成功的当日计数 +1（Go `botstats.RecordMessageSent`）。
    ///
    /// 未挂时只递增内存指标 [`metrics::MESSAGES_SENT`]——统计库打不开不阻断发送。
    #[must_use]
    pub fn with_botstats(mut self, botstats: Option<Arc<BotStatsStore>>) -> Self {
        self.botstats = botstats;
        self
    }

    /// 挂上出站 URL 管线的短链 mint（P56-T4；`transfer_url` 开启时 deferred 执行链才消费）。
    /// 未挂时短链分支走 [`crate::url_pipeline::transform`] 的 Rust-only 防御路径。
    #[must_use]
    pub fn with_url_mint(mut self, url_mint: Option<Arc<crate::url_pipeline::UrlMint>>) -> Self {
        self.url_mint = url_mint;
        self
    }

    /// 覆盖合并转发逐条重发的间隔（缺省 [`DEFAULT_FORWARD_SEND_INTERVAL`]，Go 的 500ms；
    /// 测试传 `Duration::ZERO` 免真睡，评审 ③）。
    #[must_use]
    pub fn with_forward_interval(mut self, interval: Duration) -> Self {
        self.forward_interval = interval;
        self
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
            Ok(Ok(response)) => self.finalize(response, request).await,
            Ok(Err(err)) => failure(retcode_for(&err), err.to_string(), request.echo.clone()),
            Err(payload) => {
                let message = panic_message(payload.as_ref());
                // 全文只进日志：panic 文本可能带内部路径或凭据，不能回给 OneBot 客户端。
                tracing::error!(action = %request.action, panic = %message, "handler panic 已被隔离");
                failure(RETCODE_INTERNAL, "handler panic", request.echo.clone())
            }
        }
    }

    /// 收尾：deferred 计划就地执行并替换成发送结果，最后兜底 `echo`。
    async fn finalize(
        &self,
        mut response: ActionResponse<Value>,
        request: &ActionRequest<Value>,
    ) -> ActionResponse<Value> {
        let deferred = response
            .data
            .as_ref()
            .filter(|data| crate::deferred::is_deferred(data))
            .cloned();
        if let Some(data) = deferred {
            // 计划只是 handler → runtime 的内部握手载荷，不能泄漏给客户端。
            let echo = response.echo.clone();
            response = match self.run_deferred(data, echo.as_ref()).await {
                Ok((outcome, target)) => {
                    self.record_message_sent();
                    let mut sent = ActionResponse::ok(outcome).with_echo(echo);
                    // 顶层 ID 按收件人填（Go `SendResponse` / `SendC2CResponse` 用同一
                    // `message.Params` 构造）：空消息回执没打上游，同样要带。
                    if let (Some(context), Some(target)) = (self.actions.as_ref(), target.as_ref())
                    {
                        let (group_id, user_id) = receipt_ids(context, target);
                        sent.group_id = group_id;
                        sent.user_id = user_id;
                    }
                    sent
                }
                Err((retcode, message)) => failure(retcode, message, echo),
            };
        }

        // handler 未显式回填 echo 时由分发器兜底，保证 OneBot 客户端能对上请求。
        if response.echo.is_none() {
            response.echo = request.echo.clone();
        }
        response
    }

    /// 执行 deferred 计划；失败带上该用的 `retcode`（见 [`deferred_retcode`]）。
    ///
    /// 三种握手形状按载荷键判别：`message_sse` → [`crate::deferred::execute_sse`]、
    /// `forward` → [`crate::deferred::execute_forward`]（合并转发逐条重发）、其余 →
    /// [`crate::deferred::execute`] 单条。返回的 JSON 即给客户端的 `data`，第二项只有单条
    /// 发送分支给得出来（收件人；SSE / 合并转发回执没有顶层 ID）。
    ///
    /// `echo` 是请求回执原值：规划层看不到它，这里补进单条计划的
    /// [`gensokyo_actions::EchoContext::echo_key`]（Go `message.Echo` 直接当 `msg_id` 键查表）。
    ///
    /// 未装配 [`ActionContext`]、计划反序列化失败都算服务端问题（`1500`）；消息侧问题由
    /// [`crate::deferred::execute`] 的 [`CoreError`] 决定。
    async fn run_deferred(
        &self,
        data: Value,
        echo: Option<&Value>,
    ) -> Result<(Value, Option<SendTarget>), (i32, String)> {
        let Some(context) = self.actions.as_ref() else {
            return Err((
                RETCODE_INTERNAL,
                "deferred 计划需要装配 ActionContext（QqApi）".to_owned(),
            ));
        };
        let deps = DeferredDeps {
            media: self.media.as_ref(),
            hosting: crate::hosting::shared(),
            echo: self.echo.as_deref(),
            hub: self.hub.as_ref(),
            forward_send_interval: self.forward_interval,
            limiter: Some(crate::queue::RateLimiter::shared()),
            url_mint: self.url_mint.as_deref(),
        };
        if data.get("forward").is_some() {
            let request = serde_json::from_value::<ForwardRequest>(data).map_err(bad_plan)?;
            let outcome = crate::deferred::execute_forward(context, &request, &deps).await;
            // 部分失败不回滚也不改 retcode（Go 的转发回执恒 ok）：失败逐条进 `failed` 清单
            let failed: Vec<Value> = outcome
                .failed
                .into_iter()
                .map(|item| json!({ "index": item.index, "error": item.error }))
                .collect();
            return Ok((
                json!({
                    "message_ids": outcome.message_ids,
                    "failed": failed,
                }),
                None,
            ));
        }
        if data.get("message_sse").is_some() {
            let request = serde_json::from_value::<SseRequest>(data).map_err(bad_plan)?;
            let message_id = crate::deferred::execute_sse(context, &request, &deps)
                .await
                .map_err(|err| (deferred_retcode(&err), err.to_string()))?;
            return Ok((json!({ "message_id": message_id }), None));
        }
        let mut plan: SendRequest = serde_json::from_value(data).map_err(bad_plan)?;
        // 规划层拿不到请求回执，这里补进去：Go 用 `message.Echo` 的字符串形态查 `msg_id` 表
        if plan.echo_context.echo_key.is_none() {
            plan.echo_context.echo_key = echo.and_then(Value::as_str).map(str::to_owned);
        }
        let message_id = crate::deferred::execute(context, &plan, &deps)
            .await
            .map_err(|err| (deferred_retcode(&err), err.to_string()))?;
        Ok((
            json!({
                "message_id": message_id_value(context.string_ob11(), self.echo.as_deref(), &message_id),
            }),
            Some(plan.target),
        ))
    }

    /// 一条 deferred 计划发送成功后记账（Go `botstats.RecordMessageSent` 的等价物）。
    ///
    /// 内存指标与 botstats 一起递增；统计库打不开 / 未装配时只丢掉落盘部分，发送不受影响。
    /// 合并转发按**一条计划**记 1 次（Go 在 `message_parser` 里逐条解析时记账，转发分支会记
    /// 多次），这是与 Go 唯一的口径差异。
    fn record_message_sent(&self) {
        metrics::MESSAGES_SENT.inc();
        let Some(stats) = self.botstats.as_deref() else {
            return;
        };
        if let Err(err) = stats.record_sent() {
            tracing::warn!(error = %err, "botstats 写入失败，本次发送只计内存指标");
        }
    }
}

/// 计划反序列化的失败形状：握手载荷坏了属于服务端问题（`1500`）。
fn bad_plan(err: serde_json::Error) -> (i32, String) {
    (
        RETCODE_INTERNAL,
        format!("deferred 计划反序列化失败: {err}"),
    )
}

/// 回执里的 `message_id`：Go `SendResponse` 的两套模式（`handlers/message_parser.go:307-376`
/// 群、`:575-600` 私聊）。
///
/// - 空消息回执 [`crate::deferred::EMPTY_MESSAGE_ID`]：**两种模式都是数字 `0`**（`:375` / `:599`，
///   上游 `resp=nil` 的分支绕过 string 分派）；
/// - `string_ob11 = true`：上游原 ID 字符串（`:419`，且不登记 echo 缓存）；
/// - 否则：真实 ID 经 echo 行号派生出虚拟号（`:311-324`，`memory_msgid` 缺省 true 走
///   `echo.StoreCacheInMemory`）。
///
/// 派生失败（未装配 echo / 存储报错）时先按整数解析（上游直接回数字 ID 的部署也走这条）；
/// 仍不行就保留原字符串并告警，**不伪造 `0`**——把真实 ID 换成 0 会让客户端的撤回 / 引用
/// 指向错误对象。
fn message_id_value(string_ob11: bool, echo: Option<&EchoStore>, raw: &str) -> Value {
    if raw == crate::deferred::EMPTY_MESSAGE_ID {
        return json!(0);
    }
    if string_ob11 {
        return json!(raw);
    }
    let derived = echo.and_then(|echo| match echo.store_cache_in_memory(raw) {
        Ok(row) => Some(row),
        Err(err) => {
            tracing::warn!(
                message_id = %raw,
                error = %err,
                "回执 message_id 派生虚拟号失败，退回整数解析"
            );
            None
        }
    });
    if let Some(row) = derived {
        return json!(row);
    }
    match raw.parse::<i64>() {
        Ok(row) => json!(row),
        Err(_) => {
            tracing::warn!(
                message_id = %raw,
                "回执 message_id 无法转成数字（未装配 echo 或非数字 ID），按原字符串回传"
            );
            json!(raw)
        }
    }
}

/// 回执顶层的 `group_id` / `user_id`（Go `ServerResponse.GroupID` / `UserID`）。
///
/// 值只取自 deferred 计划的收件人**请求原串**，不来自 `data`、不来自反查结果：
///
/// - 群聊 numeric：请求原串是十进制数字（numeric 部署下客户端发的就是虚拟号）直接用，否则
///   走 idmap 反查虚拟号（Go `StoreIDv2`，`handlers/message_parser.go:252`），未装配 / 失败
///   时留空——那边是 `omitempty` 吞掉 0，这里不写字段，**不伪造 0**；
/// - 群聊 string（`string_ob11`）：请求原串，`ServerResponseSB.GroupID` 是 `string`
///   （`:429` 赋值、`:211` 声明）；
/// - 私聊：两种模式都是数字虚拟号（Go `SendC2CResponse` 恒用数字结构 `ServerResponse`，
///   `:605-609`）——这是 Go 自身的不一致，照抄以保 parity。
fn receipt_ids(context: &ActionContext, target: &SendTarget) -> (Option<Ob11Id>, Option<Ob11Id>) {
    match target {
        SendTarget::Group(raw) if context.string_ob11() => (Some(Ob11Id::Str(raw.clone())), None),
        SendTarget::Group(raw) => (target_virtual_id(context, raw), None),
        SendTarget::Private(raw) => (None, target_virtual_id(context, raw)),
    }
}

/// 收件人原串 → 数字虚拟号：纯数字原串本身就是虚拟号，其余走 idmap 反查（查不到会分配）。
fn target_virtual_id(context: &ActionContext, raw: &str) -> Option<Ob11Id> {
    if let Ok(value) = raw.parse::<i64>() {
        return Some(Ob11Id::Num(value));
    }
    let idmap = context.idmap()?;
    match idmap.retrieve_virtual_value(raw) {
        Ok((_, vuin)) => Some(Ob11Id::Num(vuin)),
        Err(err) => {
            tracing::warn!(
                target = %raw,
                error = %err,
                "回执顶层 ID 反查虚拟号失败，省略该字段"
            );
            None
        }
    }
}

/// deferred 失败的 retcode：请求侧问题（媒体来源 / 大小 / 类型、reply 无法还原）是 `1400`，
/// 其余（上游 / 网络 / 存储 / 计划损坏）`1500`。
///
/// 与 [`retcode_for`] 的差别：这里不把 [`CoreError::Unsupported`] 映射成 `1404`——计划已经被
/// runtime 收下，未知能力是服务端没能兑现，不是「未知 action」。
fn deferred_retcode(err: &CoreError) -> i32 {
    match err {
        CoreError::Protocol(_) => RETCODE_BAD_REQUEST,
        _ => RETCODE_INTERNAL,
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
    use gensokyo_storage::EchoStore;
    use serde_json::{Value, json};

    use super::{
        Dispatcher, RETCODE_BAD_REQUEST, RETCODE_INTERNAL, RETCODE_UNKNOWN_ACTION, dispatch_json,
    };
    use crate::deferred::EMPTY_MESSAGE_ID;

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

    // 本用例跑在 dev/test（unwind）口径；release 的 panic=abort 下 catch_unwind 不生效，
    // handler panic 会直接退出进程（含 SSM / echo / 延迟队列的内存态，见 docs/rust-build.md）。
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

    /// deferred 失败分档：请求侧 `1400`，上游 / 网络 / 存储 / 未实现能力 `1500`。
    ///
    /// 与 [`super::retcode_for`] 的 `Unsupported → 1404` 不同：计划已进 runtime，
    /// 未知能力是服务端没兑现，不能回「未知 action」。
    #[test]
    fn deferred_failures_split_request_and_server_side() {
        assert_eq!(
            super::deferred_retcode(&Error::Protocol("媒体超限".to_owned())),
            RETCODE_BAD_REQUEST
        );
        assert_eq!(
            super::deferred_retcode(&Error::Upstream {
                code: 22009,
                message: "频控".to_owned(),
            }),
            RETCODE_INTERNAL
        );
        assert_eq!(
            super::deferred_retcode(&Error::Network("下载失败".to_owned())),
            RETCODE_INTERNAL
        );
        assert_eq!(
            super::deferred_retcode(&Error::Unsupported("upload_group_file".to_owned())),
            RETCODE_INTERNAL
        );
    }

    // ---- P86a：回执 message_id 的类型与空回执（Go `SendResponse` 口径） ----

    /// numeric 模式（`string_ob11=false`，默认）：真实 ID 经 echo 派生成虚拟行号，回执是数字。
    #[test]
    fn numeric_mode_returns_derived_virtual_row_as_number() {
        let echo = EchoStore::new();
        let expected = echo.store_cache_in_memory("real-1").expect("派生行号");

        let value = super::message_id_value(false, Some(&echo), "real-1");
        assert!(value.is_i64(), "numeric 模式必须是 JSON 数字: {value}");
        assert_eq!(value, json!(expected));
    }

    /// string 模式（`string_ob11=true`）：上游原 ID 原样回字符串（`message_parser.go:419`）。
    #[test]
    fn string_mode_returns_upstream_id_string() {
        let echo = EchoStore::new();
        assert_eq!(
            super::message_id_value(true, Some(&echo), "real-2"),
            json!("real-2")
        );
    }

    /// 空消息回执（上游 `resp=nil`）：两种模式都是数字 `0`（`:375` / `:599`）。
    #[test]
    fn empty_receipt_is_numeric_zero_in_both_modes() {
        for string_ob11 in [false, true] {
            let value = super::message_id_value(string_ob11, None, EMPTY_MESSAGE_ID);
            assert!(
                value.is_i64(),
                "string_ob11={string_ob11} 时空回执必须是数字: {value}"
            );
            assert_eq!(value, json!(0));
        }
    }

    /// 派生失败（未装配 echo）且 ID 非数字：保留原字符串，不伪造 `0`。
    #[test]
    fn underivable_id_keeps_the_string() {
        assert_eq!(
            super::message_id_value(false, None, "upstream-alpha"),
            json!("upstream-alpha")
        );
    }

    /// 派生失败但上游回的就是数字：退回整数解析，numeric 模式仍回数字。
    #[test]
    fn underivable_numeric_id_falls_back_to_integer_parse() {
        assert_eq!(
            super::message_id_value(false, None, "987654321"),
            json!(987_654_321)
        );
    }
}
