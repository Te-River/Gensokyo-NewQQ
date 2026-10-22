//! handler 注册表：显式单清单，替代 Go 的 `init()` 文件自注册（design §3.2）。
//!
//! 清单由各 action 的真实注册代码在 M1 填充；这里只给出 trait、future 别名与容器，
//! 保证"编译期即可检查每个 action 是否有实现"这一形态可落地。

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use gensokyo_core::Result;

use crate::action::{ActionRequest, ActionResponse};

/// action 处理可读取的运行时上下文。
///
/// M0 占位；M1 起填充 config 快照、storage、QQ OpenAPI 句柄。
#[derive(Debug, Default)]
pub struct ActionContext;

/// handler 返回的 future。
pub type ActionFuture =
    Pin<Box<dyn Future<Output = Result<ActionResponse<String>>> + Send + 'static>>;

/// 一个 action 的实现。
///
/// `P` 在 M0 固定为 `String`（原始 JSON 参数），M1 由各 action 的类型化参数结构替换。
pub trait Handler: Send + Sync + Debug {
    /// 唯一实现名，用于日志与 action 覆盖率统计。
    fn name(&self) -> &'static str;

    /// 执行 action。
    fn handle<'a>(&'a self, ctx: &'a ActionContext, request: ActionRequest<String>)
    -> ActionFuture;
}

/// action 名 → handler。别名指向同一 `Arc`，因此别名不会带来第二份实现。
#[derive(Debug, Default)]
pub struct Registry {
    handlers: BTreeMap<&'static str, Arc<dyn Handler>>,
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

    /// 按 action 名查找（含别名）。
    #[must_use]
    pub fn get(&self, action: &str) -> Option<&Arc<dyn Handler>> {
        self.handlers.get(action)
    }

    /// 已注册的 action 名数量。
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
    pub fn actions(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.handlers.keys().copied()
    }
}
