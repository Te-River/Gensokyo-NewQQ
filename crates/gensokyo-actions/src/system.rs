//! 运行控制与凭证 action：`set_restart` / `clean_cache` / `get_cookies` /
//! `get_csrf_token` / `get_credentials`。
//!
//! # 覆盖范围
//!
//! | action | Go 对应 | 上游端点 | 结果 |
//! |--------|---------|----------|------|
//! | `set_restart` | 无（Go 未注册） | 官方无进程控制端点 | 记录日志 + `{"code": 0}`，**不重启** |
//! | `clean_cache` | 无（Go 未注册） | 官方无缓存管理端点 | 记录日志 + `{"code": 0}`，**不清缓存** |
//! | `get_cookies` | 无（Go 未注册） | 官方机器人无网页登录态 | `{"cookies": ""}` |
//! | `get_csrf_token` | 无（Go 未注册） | 同上 | `{"token": ""}` |
//! | `get_credentials` | 无（Go 未注册） | 同上 | `{"cookies": "", "csrf_token": ""}` |
//!
//! # 为什么不真的重启 / 清缓存
//!
//! 重启与缓存清理属于宿主进程（`gensokyo-bin`）的职责：本 crate 既没有进程控制权，也没有
//! `gensokyo-media` 的临时文件目录句柄。两个 action 只写一行审计日志再回 `{"code": 0}`——
//! 返回失败会让「把 set_restart 当优雅退出用」的客户端退出码变成错误，谎报「已重启」又会掩盖
//! 真正没重启的事实，因此日志里明确写「仅记录，未执行」。
//!
//! 日志走 `stderr`：本 crate 没有声明 `tracing` 依赖（新增依赖边会让 `Cargo.lock` 失效，
//! 而 `--locked` 构建不允许），与 `gensokyo-protocol` 的调试输出同一取舍。订阅器的选型在
//! `gensokyo-core::telemetry`，接入后这两处换成 `tracing::info!` 即可。
//!
//! # 为什么凭证返回空值而不是 1500
//!
//! 官方机器人用 AppID + Secret 换 `access_token`，没有 QQ 网页端的 cookies / csrf；Go 的
//! `handlers/` 里也没有这三个 action。给固定空值让客户端的启动自检能过，而不是把「本来就没有
//! 的东西」报成故障——但对 `set_restart` / `clean_cache` 这类**有副作用预期**的调用，日志里必须
//! 明说没有执行，不能让客户端读成「已完成」。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use gensokyo_core::Result;
use gensokyo_protocol::action::{ActionRequest, ActionResponse};
use gensokyo_protocol::{Handler, HandlerFuture};

use crate::util::ok_data;

/// `set_restart` / `clean_cache` 的回执：`{"code": 0}`（go-cqhttp 同形）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeData {
    /// 业务码；这两个 action 恒为 0。
    pub code: i32,
}

impl CodeData {
    /// 成功回执。
    const OK: Self = Self { code: 0 };
}

/// `get_cookies` 的响应数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookiesData {
    /// 网页登录态 cookies；官方机器人恒为空串。
    pub cookies: String,
}

/// `get_csrf_token` 的响应数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsrfTokenData {
    /// csrf token；官方机器人恒为空串。
    pub token: String,
}

/// `get_credentials` 的响应数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialsData {
    /// 网页登录态 cookies；恒为空串。
    pub cookies: String,
    /// csrf token；恒为空串。
    pub csrf_token: String,
}

/// `set_restart`：只记录，不重启。
#[derive(Debug, Clone, Copy, Default)]
pub struct SetRestartHandler;

impl SetRestartHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        // 参数（`delay` 等）有意忽略：没有可调度的重启动作，校验只会制造无意义的 1400
        eprintln!("[gensokyo-actions] set_restart: 收到重启请求（仅记录，未重启进程）");
        ok_data(request, &CodeData::OK)
    }
}

impl Handler for SetRestartHandler {
    fn name(&self) -> &'static str {
        "set_restart"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `clean_cache`：只记录，不清缓存。
#[derive(Debug, Clone, Copy, Default)]
pub struct CleanCacheHandler;

impl CleanCacheHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        eprintln!("[gensokyo-actions] clean_cache: 收到清理缓存请求（仅记录，未清理任何数据）");
        ok_data(request, &CodeData::OK)
    }
}

impl Handler for CleanCacheHandler {
    fn name(&self) -> &'static str {
        "clean_cache"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `get_cookies`：官方机器人没有网页登录态，恒为空串。
#[derive(Debug, Clone, Copy, Default)]
pub struct GetCookiesHandler;

impl GetCookiesHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        ok_data(
            request,
            &CookiesData {
                cookies: String::new(),
            },
        )
    }
}

impl Handler for GetCookiesHandler {
    fn name(&self) -> &'static str {
        "get_cookies"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `get_csrf_token`：官方机器人没有网页登录态，恒为空串。
#[derive(Debug, Clone, Copy, Default)]
pub struct GetCsrfTokenHandler;

impl GetCsrfTokenHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        ok_data(
            request,
            &CsrfTokenData {
                token: String::new(),
            },
        )
    }
}

impl Handler for GetCsrfTokenHandler {
    fn name(&self) -> &'static str {
        "get_csrf_token"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `get_credentials`：`get_cookies` + `get_csrf_token` 的合并形态。
#[derive(Debug, Clone, Copy, Default)]
pub struct GetCredentialsHandler;

impl GetCredentialsHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        ok_data(
            request,
            &CredentialsData {
                cookies: String::new(),
                csrf_token: String::new(),
            },
        )
    }
}

impl Handler for GetCredentialsHandler {
    fn name(&self) -> &'static str {
        "get_credentials"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        CleanCacheHandler, GetCookiesHandler, GetCredentialsHandler, GetCsrfTokenHandler,
        SetRestartHandler,
    };
    use crate::testing::{block_on, ok_data, request, request_without_params};
    use gensokyo_protocol::Handler;

    #[test]
    fn restart_and_clean_cache_ack_with_code_zero() {
        let restart = SetRestartHandler::new();
        for req in [
            request("set_restart", json!({"delay": 1000})),
            request_without_params("set_restart"),
        ] {
            let response = block_on(restart.handle(&req)).expect("固定回执不应失败");
            assert_eq!(ok_data(response), json!({"code": 0}));
        }
        assert_eq!(restart.name(), "set_restart");

        let clean = CleanCacheHandler::new();
        let response = block_on(clean.handle(&request_without_params("clean_cache")))
            .expect("固定回执不应失败");
        assert_eq!(ok_data(response), json!({"code": 0}));
        assert_eq!(clean.name(), "clean_cache");
    }

    #[test]
    fn credentials_are_empty_strings() {
        let cookies =
            block_on(GetCookiesHandler::new().handle(&request_without_params("get_cookies")))
                .expect("凭证回执不应失败");
        assert_eq!(ok_data(cookies), json!({"cookies": ""}));

        let csrf =
            block_on(GetCsrfTokenHandler::new().handle(&request_without_params("get_csrf_token")))
                .expect("凭证回执不应失败");
        assert_eq!(ok_data(csrf), json!({"token": ""}));

        let credentials = block_on(
            GetCredentialsHandler::new().handle(&request_without_params("get_credentials")),
        )
        .expect("凭证回执不应失败");
        assert_eq!(
            ok_data(credentials),
            json!({"cookies": "", "csrf_token": ""})
        );
    }

    #[test]
    fn handler_names_match_the_registered_actions() {
        assert_eq!(GetCookiesHandler::new().name(), "get_cookies");
        assert_eq!(GetCsrfTokenHandler::new().name(), "get_csrf_token");
        assert_eq!(GetCredentialsHandler::new().name(), "get_credentials");
    }
}
