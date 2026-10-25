//! 本地上报能力 action：`can_send_image` / `can_send_record`。
//!
//! 两个 action 都是纯本地判断（固定回 `{"yes": true}`），不触达上游：官方 v2 的图片 / 语音
//! 都走 `POST /v2/groups/{gid}/files` 上传，既没有「当前是否允许发媒体」的查询接口，也没有
//! 配额端点。Go 的注册表里没有这两个 action；OneBot 客户端常用它们做启动自检，给固定 `yes`
//! 比「未实现」更符合客户端预期。
//!
//! 媒体开关（图床选择、大小压缩）属于发送链的判断，见 [`crate::send`]。
//!
//! # 与 Go 的差异
//!
//! Go 没有对应 handler，因此没有可对照的语义；返回值取 OneBot v11 的 `{ "yes": bool }`。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use gensokyo_core::Result;
use gensokyo_protocol::action::{ActionRequest, ActionResponse};
use gensokyo_protocol::{Handler, HandlerFuture};

use crate::util::ok_data;

/// `can_send_image` / `can_send_record` 的响应数据。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanSendData {
    /// 是否允许发送该类媒体。
    pub yes: bool,
}

impl CanSendData {
    /// 固定能力：官方媒体上传只要 token 有效就能发，本地判断恒为真。
    const YES: Self = Self { yes: true };
}

/// `can_send_image`。
#[derive(Debug, Clone, Copy, Default)]
pub struct CanSendImageHandler;

impl CanSendImageHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        ok_data(request, &CanSendData::YES)
    }
}

impl Handler for CanSendImageHandler {
    fn name(&self) -> &'static str {
        "can_send_image"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `can_send_record`。
#[derive(Debug, Clone, Copy, Default)]
pub struct CanSendRecordHandler;

impl CanSendRecordHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        ok_data(request, &CanSendData::YES)
    }
}

impl Handler for CanSendRecordHandler {
    fn name(&self) -> &'static str {
        "can_send_record"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{CanSendImageHandler, CanSendRecordHandler};
    use crate::testing::{block_on, ok_data, request, request_without_params};
    use gensokyo_protocol::Handler;

    #[test]
    fn can_send_image_is_a_local_yes() {
        let handler = CanSendImageHandler::new();
        for req in [
            request_without_params("can_send_image"),
            request("can_send_image", json!({"ignored": 1})),
        ] {
            let response = block_on(handler.handle(&req)).expect("本地判断不应失败");
            assert_eq!(ok_data(response), json!({"yes": true}));
        }
        assert_eq!(handler.name(), "can_send_image");
    }

    #[test]
    fn can_send_record_is_a_local_yes() {
        let handler = CanSendRecordHandler::new();
        let response = block_on(handler.handle(&request_without_params("can_send_record")))
            .expect("本地判断不应失败");
        assert_eq!(ok_data(response), json!({"yes": true}));
        assert_eq!(handler.name(), "can_send_record");
    }
}
