//! handler 注册表：显式单清单，替代 Go 的 `init()` 文件自注册（design §3.2）。
//!
//! handler 是异步 trait（[`Handler::handle`] 返回 [`HandlerFuture`]）：runtime 直接 await，
//! 每条 action 不再经一次 `spawn_blocking` 线程池调度；纯同步实现用 [`ready`] 包一层。
//!
//! 为什么不用 `inventory` / `linkme`：链接期 section 在部分平台/静态链接下有坑，且顺序
//! 不可测。这里用 `HashMap<&'static str, Arc<dyn Handler>>` + 别名指向同一 `Arc`，
//! [`Registry::actions`] 给出稳定排序的全集，供 M4 与 Go 的注册清单核对。
//!
//! Go 的注册全集（`handlers/*.go` 的 `init()`）见 [`KNOWN_ACTIONS`]；别名见
//! [`ACTION_ALIASES`]。M4 只实现 action 逻辑，注册清单照抄本模块。

use std::collections::HashMap;
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;

use gensokyo_core::{Error, Result};

use crate::action::{ActionRequest, ActionResponse};

/// [`Handler::handle`] 的返回类型：一个装箱的 future。
///
/// 生命周期同时绑定 `&'a self` 与 `&'a ActionRequest`，handler 可以零拷贝借用请求参数；
/// `Send` 让 runtime 能把分发放进任意 tokio 任务。
///
/// 为什么手写 `Pin<Box<dyn Future>>` 而不用 `async fn`：trait 里的 `async fn`（AFIT）不是
/// dyn-compatible，而注册表必须持有 `Arc<dyn Handler>`。装箱的代价是一次分配——比原先每条
/// 消息一次 `spawn_blocking` 线程池调度便宜一个数量级。
pub type HandlerFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ActionResponse<Value>>> + Send + 'a>>;

/// action 处理入口（异步）。
///
/// 参数/响应用 [`serde_json::Value`] 兜底：注册表不关心每个 action 的具体形状，各 handler
/// 自己反序列化成 [`crate::action`] 里的类型化参数结构。
///
/// `handle` 返回 future 而不是直接返回值：action 需要 await QQ OpenAPI 时，同步签名会逼
/// 调用方 `block_on`，或者让 runtime 把整条分发塞进 `spawn_blocking`。纯同步的 handler 用
/// [`ready`] 或 `Box::pin(async move { … })` 包一层即可，不必自造执行器。
pub trait Handler: Send + Sync + Debug {
    /// 唯一实现名，用于日志与 action 覆盖率统计。
    fn name(&self) -> &'static str;

    /// 执行 action。
    ///
    /// # Errors
    /// 参数缺失/非法、上游失败等可预期错误返回 [`Error`]；协议层只负责形状，不实现业务。
    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a>;
}

/// 同步 handler 的快捷出口：把已算好的结果包成已就绪的 [`HandlerFuture`]。
///
/// 返回值是 `'static` future；trait object 的生命周期参数是协变的，因此可以直接当作任意
/// `HandlerFuture<'a>` 返回，实现方的签名不必跟着变。
pub fn ready(response: Result<ActionResponse<Value>>) -> HandlerFuture<'static> {
    Box::pin(std::future::ready(response))
}

/// action 名 → handler。别名指向同一 `Arc`，因此别名不会带来第二份实现。
#[derive(Debug, Default)]
pub struct Registry {
    handlers: HashMap<&'static str, Arc<dyn Handler>>,
}

impl Registry {
    /// 空注册表。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册 action 名；返回被覆盖的旧 handler，调用方可据此发现重复注册而不是静默覆盖。
    pub fn register(
        &mut self,
        action: &'static str,
        handler: Arc<dyn Handler>,
    ) -> Option<Arc<dyn Handler>> {
        self.handlers.insert(action, handler)
    }

    /// 为已注册的 action 增加别名（指向同一实现）。
    ///
    /// # Errors
    /// 目标尚未注册时返回 [`Error::Protocol`]——别名表写错必须在装配期炸出来。
    pub fn alias(&mut self, alias: &'static str, target: &str) -> Result<()> {
        let handler = self
            .handlers
            .get(target)
            .ok_or_else(|| Error::Protocol(format!("别名 {alias} 的目标 {target} 未注册")))?
            .clone();
        self.handlers.insert(alias, handler);
        Ok(())
    }

    /// 注册 [`ACTION_ALIASES`] 里的全部兼容别名。
    ///
    /// # Errors
    /// 任一别名目标未注册时返回 [`Error::Protocol`]。
    pub fn register_default_aliases(&mut self) -> Result<()> {
        for (alias, target) in ACTION_ALIASES {
            self.alias(alias, target)?;
        }
        Ok(())
    }

    /// 按 action 名查找（含别名）。
    #[must_use]
    pub fn get(&self, action: &str) -> Option<&Arc<dyn Handler>> {
        self.handlers.get(action)
    }

    /// 已注册的 action 名数量（含别名）。
    #[must_use]
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    /// 是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }

    /// 全部 action 名（字典序，便于与 Go 的 `RegisterHandler` 全集对照）。
    #[must_use]
    pub fn actions(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self.handlers.keys().copied().collect();
        names.sort_unstable();
        names
    }

    /// 尚未注册的期望 action 名（`expected` 通常传 [`KNOWN_ACTIONS`]）。
    #[must_use]
    pub fn missing(&self, expected: &[&'static str]) -> Vec<&'static str> {
        expected
            .iter()
            .copied()
            .filter(|action| !self.handlers.contains_key(action))
            .collect()
    }
}

/// Go `handlers/*.go` 中 `callapi.RegisterHandler` 注册的全部 action 名（M1f 盘点）。
///
/// 顺序与 Go 源码文件顺序无关，按字典序排列便于 diff。
pub const KNOWN_ACTIONS: &[&str] = &[
    ".handle_quick_operation",
    ".handle_quick_operation_async",
    "create_panel",
    "delete_group_msg",
    "delete_media_image",
    "delete_msg",
    "delete_panel",
    "get_avatar",
    "get_custom_menu",
    "get_friend_list",
    "get_group_ban",
    "get_group_bot_state",
    "get_group_info",
    "get_group_join_request_list",
    "get_group_list",
    "get_group_member_blacklist",
    "get_group_member_info",
    "get_group_member_list",
    "get_group_whole_ban",
    "get_login_info",
    "get_media_image_list",
    "get_online_clients",
    "get_panel",
    "get_panel_list",
    "get_robot_share_link",
    "get_status",
    "get_version_info",
    "join_approval_strategy_create",
    "join_approval_strategy_delete",
    "join_approval_strategy_execute",
    "join_approval_strategy_list",
    "join_approval_strategy_update",
    "join_approval_strategy_whitelist",
    "mark_msg_as_read",
    "put_interaction",
    "send_group_forward_msg",
    "send_group_msg",
    "send_group_msg_async",
    "send_group_msg_raw",
    "send_msg",
    "send_msg_async",
    "send_private_msg",
    "send_private_msg_async",
    "send_private_msg_sse",
    "send_private_msg_wakeup",
    "send_to_group",
    "set_custom_menu",
    "set_group_add_request",
    "set_group_ban",
    "set_group_card",
    "set_group_kick",
    "set_group_member_blacklist",
    "set_group_whole_ban",
    "set_panel",
    "set_panel_target",
    "upload_media_image",
];

/// Go 里指向同一实现的兼容别名（alias → canonical）。
pub const ACTION_ALIASES: &[(&str, &str)] = &[
    // handlers/send_group_msg.go 同时注册 send_to_group
    ("send_to_group", "send_group_msg"),
    // handlers/set_group_ban.go 的旧 action 名
    ("get_group_ban", "set_group_ban"),
    // handlers/set_group_whole_ban.go 的旧 action 名
    ("get_group_whole_ban", "set_group_whole_ban"),
];

/// 批量注册：`register_actions!(registry, "a" => H, alias "b" => "a", ...);`
///
/// 宏只做两件事：`Registry::register` 与 `Registry::alias`，不含任何链接期魔法。
/// 参数必须是注册表变量标识符（不是表达式），避免 `&mut` 与方法调用的优先级陷阱。
#[macro_export]
macro_rules! register_actions {
    ($registry:ident $(,)?) => {};
    ($registry:ident, alias $alias:literal => $target:literal $(, $($rest:tt)*)?) => {{
        $registry
            .alias($alias, $target)
            .expect("别名目标必须已注册");
        $crate::register_actions!($registry $(, $($rest)*)?);
    }};
    ($registry:ident, $action:literal => $handler:expr $(, $($rest:tt)*)?) => {{
        $registry.register($action, ::std::sync::Arc::new($handler));
        $crate::register_actions!($registry $(, $($rest)*)?);
    }};
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll, Waker};

    use serde_json::Value;

    use super::{ACTION_ALIASES, Handler, HandlerFuture, KNOWN_ACTIONS, Registry, ready};
    use crate::action::{ActionRequest, ActionResponse};

    /// 单测里的最小 `block_on`。
    ///
    /// 刻意不引 tokio / futures：protocol 只依赖 serde，加 dev-dependency 会动 `Cargo.lock`。
    /// `Handler` 的 future 不做 I/O，`Pending` 只可能来自下面 `YieldOnce` 这种自己唤醒自己的
    /// 实现，所以自旋一轮即可推进。
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

    #[derive(Debug)]
    struct Stub(&'static str);

    impl Handler for Stub {
        fn name(&self) -> &'static str {
            self.0
        }

        fn handle<'a>(&'a self, _request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
            ready(Ok(ActionResponse::ok(Value::Null)))
        }
    }

    /// 首次 poll 返回 `Pending`、被唤醒后第二次 poll 才 `Ready`。
    #[derive(Debug)]
    struct YieldOnce(bool);

    impl Future for YieldOnce {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            if self.0 {
                Poll::Ready(())
            } else {
                self.0 = true;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }

    /// 真的 await 一次的 handler：证明 trait 允许 await 点，而不只是“包一层立即就绪”。
    #[derive(Debug)]
    struct Yielder;

    impl Handler for Yielder {
        fn name(&self) -> &'static str {
            "yielder"
        }

        fn handle<'a>(&'a self, _request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
            Box::pin(async move {
                YieldOnce(false).await;
                Ok(ActionResponse::ok(Value::Null))
            })
        }
    }

    fn registry_with(stub: &'static str) -> Registry {
        let mut registry = Registry::new();
        registry.register("send_group_msg", Arc::new(Stub(stub)));
        registry
    }

    #[test]
    fn alias_points_at_the_same_implementation() {
        let mut registry = registry_with("send_group_msg");
        registry
            .alias("send_to_group", "send_group_msg")
            .expect("别名注册失败");

        let canonical = registry.get("send_group_msg").expect("应有实现");
        let alias = registry.get("send_to_group").expect("别名应可查找");
        assert!(Arc::ptr_eq(canonical, alias), "别名必须是同一份实现");
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn alias_to_unknown_target_is_an_error() {
        let mut registry = Registry::new();
        assert!(registry.alias("x", "y").is_err());
        assert!(registry.get("x").is_none());
    }

    #[test]
    fn actions_are_sorted_and_complete() {
        let mut registry = Registry::new();
        registry.register("zeta", Arc::new(Stub("zeta")));
        registry.register("alpha", Arc::new(Stub("alpha")));
        registry.alias("beta", "alpha").expect("别名注册失败");
        assert_eq!(registry.actions(), vec!["alpha", "beta", "zeta"]);
    }

    #[test]
    fn duplicate_registration_returns_previous_handler() {
        let mut registry = registry_with("first");
        let previous = registry.register("send_group_msg", Arc::new(Stub("second")));
        assert_eq!(previous.expect("应返回旧 handler").name(), "first");
        assert_eq!(
            registry.get("send_group_msg").expect("应有实现").name(),
            "second"
        );
    }

    #[test]
    fn missing_reports_unregistered_names() {
        let registry = registry_with("send_group_msg");
        let missing = registry.missing(KNOWN_ACTIONS);
        assert!(missing.contains(&"get_status"));
        assert!(!missing.contains(&"send_group_msg"));
        assert_eq!(missing.len(), KNOWN_ACTIONS.len() - 1);
    }

    #[test]
    fn default_alias_table_is_consistent() {
        for (alias, target) in ACTION_ALIASES {
            assert!(
                KNOWN_ACTIONS.contains(alias),
                "别名 {alias} 应在 Go 注册全集里"
            );
            assert!(
                KNOWN_ACTIONS.contains(target),
                "别名目标 {target} 应在 Go 注册全集里"
            );
        }
        let mut registry = registry_with("send_group_msg");
        registry.register("set_group_ban", Arc::new(Stub("set_group_ban")));
        registry.register("set_group_whole_ban", Arc::new(Stub("set_group_whole_ban")));
        registry
            .register_default_aliases()
            .expect("默认别名应全部可解析");
        for (alias, _) in ACTION_ALIASES {
            assert!(registry.get(alias).is_some(), "别名 {alias} 未生效");
        }
    }

    #[test]
    fn register_macro_handles_actions_and_aliases() {
        let mut registry = Registry::new();
        register_actions!(
            registry,
            "send_group_msg" => Stub("send_group_msg"),
            "set_group_ban" => Stub("set_group_ban"),
            alias "send_to_group" => "send_group_msg",
        );
        assert_eq!(registry.len(), 3);
        assert_eq!(
            registry.get("send_to_group").expect("别名应存在").name(),
            "send_group_msg"
        );
    }

    #[test]
    fn handler_trait_is_object_safe_and_dispatches() {
        let registry = registry_with("send_group_msg");
        let handler = registry.get("send_group_msg").expect("应有实现");
        let request: ActionRequest<Value> =
            serde_json::from_str(r#"{"action":"send_group_msg","params":{"group_id":1}}"#)
                .expect("反序列化失败");
        let response = block_on(handler.handle(&request)).expect("handler 应成功");
        assert_eq!(response.status, crate::action::Status::Ok);
    }

    #[test]
    fn handler_future_may_yield_before_completing() {
        let mut registry = Registry::new();
        registry.register("yielder", Arc::new(Yielder));
        let handler = registry.get("yielder").expect("应有实现");
        let request: ActionRequest<Value> =
            serde_json::from_str(r#"{"action":"yielder"}"#).expect("反序列化失败");

        let response = block_on(handler.handle(&request)).expect("yield 之后仍应成功");
        assert_eq!(response.status, crate::action::Status::Ok);
    }
}
