//! QQ OpenAPI v2 客户端（对齐 `botgo/openapi/v2`）。
//!
//! 所有请求都经过 [`HttpTransport`]，因此可在无网络环境下用 FakeTransport 断言
//! URL / 方法 / 请求头 / 请求体。鉴权头格式为 `Authorization: QQBot <token>`。

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::dto::{
    ApprovalJoinRequest, BatchRemoveMembersRequest, BatchRemoveMembersResponse, BotUser,
    CreateJoinApprovalStrategyRequest, CreateJoinApprovalStrategyResponse, CreatePanelRequest,
    CreatePanelResponse, FileUpload, FileUploadMergeRequest, FileUploadPartFinishRequest,
    FileUploadPrepareRequest, FileUploadPrepareResponse, GenerateUrlLink, GenerateUrlLinkRequest,
    JoinApprovalStrategyList, MediaResponse, MemberBlacklistList, MemberBlacklistRequest,
    MemberBlacklistResponse, MenuResponse, MessageResponse, MessageToCreate, PanelList,
    PanelRecordDetail, PanelTargetRequest, PutMenuRequest, UpdateJoinApprovalStrategyRequest,
    UpdateJoinApprovalStrategyResponse, UpdatePanelRequest, UserInfo, VersionResponse,
    WhitelistUsersRequest, WhitelistUsersResponse,
};
use crate::error::{ApiError, Error, Result};
use crate::http::{HttpMethod, HttpRequest, HttpResponse, HttpTransport, encode_path_segment};
use crate::token::TokenManager;

/// 正式环境基址（`botgo/openapi/v2/resource.go` 的 `domain`）。
pub const DEFAULT_BASE_URL: &str = "https://api.bot.qq.com";
/// 沙箱基址；官方已把沙箱域名并入 `api.bot.qq.com`，保留常量以对齐 botgo。
pub const SANDBOX_BASE_URL: &str = "https://api.bot.qq.com";

/// 上游链路 ID 响应头。
const TRACE_ID_HEADER: &str = "X-Tps-trace-ID";
/// 鉴权 scheme（botgo `OnBeforeRequest` 硬编码为 `QQBot`）。
const AUTH_SCHEME: &str = "QQBot";
/// 请求日志的 target：短模块推导后控制台显示 `[openapi]`。
const OPENAPI_LOG_TARGET: &str = "gensokyo_qq::openapi";
/// 日志里 body 段的上限。Go 不截断，但本仓日志会进 WebUI 的 500 条环形缓冲，
/// 一条大响应能把它整段冲掉，所以这里刻意截断。
const LOG_BODY_LIMIT: usize = 4096;
/// 需要在日志里打码的 JSON key（token / 鉴权 / 应用凭据）。
const REDACT_KEYS: [&str; 8] = [
    "authorization",
    "token",
    "access_token",
    "refresh_token",
    "secret",
    "client_secret",
    "app_secret",
    "app_id",
];

/// QQ OpenAPI v2 客户端。
pub struct OpenApiClient<T: HttpTransport> {
    transport: Arc<T>,
    tokens: Arc<TokenManager<T>>,
    base_url: String,
}

impl<T: HttpTransport> fmt::Debug for OpenApiClient<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenApiClient")
            .field("base_url", &self.base_url)
            .field("tokens", &self.tokens)
            .finish_non_exhaustive()
    }
}

impl<T: HttpTransport> OpenApiClient<T> {
    /// 用共享的传输层与 token 管理器创建客户端。
    #[must_use]
    pub fn new(transport: Arc<T>, tokens: Arc<TokenManager<T>>) -> Self {
        Self {
            transport,
            tokens,
            base_url: DEFAULT_BASE_URL.to_owned(),
        }
    }

    /// 覆盖基址（测试或私有网关用）。
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_owned();
        self
    }

    /// 按沙箱开关选择基址。
    #[must_use]
    pub fn with_sandbox(self, sandbox: bool) -> Self {
        if sandbox {
            self.with_base_url(SANDBOX_BASE_URL)
        } else {
            self.with_base_url(DEFAULT_BASE_URL)
        }
    }

    /// 当前基址。
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// 共享的 token 管理器。
    #[must_use]
    pub fn token_manager(&self) -> &TokenManager<T> {
        &self.tokens
    }

    /// 当前机器人信息（`GET /users/@me`，对齐 botgo v2 `Me`）。
    ///
    /// Go 用它取机器人真实 ID（`main.go:254` 的 `api.Me` → `main.go:304` 的
    /// `handlers.BotID = me.ID`），再用于 AppID⇄BotID 文本替换。该路径在 botgo 里
    /// v1 / v2 同址（`openapi/v1/resource.go:43` / `v2/resource.go:82`），这里沿用
    /// 同一个基址与鉴权头，不另开客户端。
    pub async fn me(&self) -> Result<BotUser> {
        self.get_json("/users/@me").await
    }

    /// 用户资料（`GET /v2/users/{user_openid}/info`，官方 2026-11-16 新增）。
    pub async fn user_info(&self, user_openid: &str) -> Result<UserInfo> {
        let path = format!("/v2/users/{}/info", encode_path_segment(user_openid));
        self.get_json(&path).await
    }

    /// 发送群消息（`POST /v2/groups/{group_openid}/messages`）。
    pub async fn send_group_message(
        &self,
        group_openid: &str,
        message: &MessageToCreate,
    ) -> Result<MessageResponse> {
        message.validate()?;
        let path = format!("/v2/groups/{}/messages", encode_path_segment(group_openid));
        self.post_json(&path, message).await
    }

    /// 发送 C2C 消息（`POST /v2/users/{user_openid}/messages`）。
    pub async fn send_c2c_message(
        &self,
        user_openid: &str,
        message: &MessageToCreate,
    ) -> Result<MessageResponse> {
        message.validate()?;
        let path = format!("/v2/users/{}/messages", encode_path_segment(user_openid));
        self.post_json(&path, message).await
    }

    /// 上传群文件 / 媒体（`POST /v2/groups/{group_openid}/files`）。
    ///
    /// 请求体为 JSON（`file_type` + `url` / `file_data`），与 botgo
    /// `PostGroupMessage(RichMediaMessage)` 一致；botgo 的 multipart 只用于频道消息。
    pub async fn upload_group_file(
        &self,
        group_openid: &str,
        file: &FileUpload,
    ) -> Result<MediaResponse> {
        file.validate()?;
        let path = format!("/v2/groups/{}/files", encode_path_segment(group_openid));
        self.post_json(&path, file).await
    }

    /// 上传 C2C 文件 / 媒体（`POST /v2/users/{user_openid}/files`）。
    pub async fn upload_c2c_file(
        &self,
        user_openid: &str,
        file: &FileUpload,
    ) -> Result<MediaResponse> {
        file.validate()?;
        let path = format!("/v2/users/{}/files", encode_path_segment(user_openid));
        self.post_json(&path, file).await
    }

    /// 分片上传·预上传（`POST /v2/groups/{id}/upload_prepare` / `…/users/{id}/upload_prepare`）。
    pub async fn prepare_file_upload(
        &self,
        id: &str,
        is_group: bool,
        request: &FileUploadPrepareRequest,
    ) -> Result<FileUploadPrepareResponse> {
        let path = file_upload_path(id, is_group, "upload_prepare");
        self.post_json(&path, request).await
    }

    /// 分片上传·通知单片完成（`POST /v2/{groups|users}/{id}/upload_part_finish`）。
    pub async fn finish_file_upload_part(
        &self,
        id: &str,
        is_group: bool,
        request: &FileUploadPartFinishRequest,
    ) -> Result<()> {
        let path = file_upload_path(id, is_group, "upload_part_finish");
        self.post_json_void(&path, request).await
    }

    /// 分片上传·合并分片（`POST /v2/{groups|users}/{id}/files`，响应取 `file_info`）。
    pub async fn merge_file_upload(
        &self,
        id: &str,
        is_group: bool,
        request: &FileUploadMergeRequest,
    ) -> Result<MediaResponse> {
        let path = file_upload_path(id, is_group, "files");
        self.post_json(&path, request).await
    }

    /// 指令面板列表（`GET /v2/panels`）；`cursor` 空串等同首页，`limit` 为 `None` 时不传。
    pub async fn get_panels(
        &self,
        scope: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<PanelList> {
        let path = query_path(
            "/v2/panels",
            &[
                ("scope", Some(scope)),
                ("cursor", cursor),
                ("limit", limit.map(|value| value.to_string()).as_deref()),
            ],
        );
        self.get_json(&path).await
    }

    /// 创建指令面板（`POST /v2/panels`）。
    pub async fn create_panel(&self, request: &CreatePanelRequest) -> Result<CreatePanelResponse> {
        self.post_json("/v2/panels", request).await
    }

    /// 单个指令面板详情（`GET /v2/panels/{panel_id}`）。
    pub async fn get_panel(&self, panel_id: &str) -> Result<PanelRecordDetail> {
        let path = format!("/v2/panels/{}", encode_path_segment(panel_id));
        self.get_json(&path).await
    }

    /// 更新指令面板元素与备注（`PUT /v2/panels/{panel_id}`，覆盖式）。
    pub async fn update_panel(
        &self,
        panel_id: &str,
        request: &UpdatePanelRequest,
    ) -> Result<VersionResponse> {
        let path = format!("/v2/panels/{}", encode_path_segment(panel_id));
        self.send_json(HttpMethod::Put, &path, request).await
    }

    /// 删除指令面板（`DELETE /v2/panels/{panel_id}`）。
    pub async fn delete_panel(&self, panel_id: &str) -> Result<()> {
        let path = format!("/v2/panels/{}", encode_path_segment(panel_id));
        self.send_empty(HttpMethod::Delete, &path).await
    }

    /// 增删指令面板关联对象（`PUT /v2/panels/{panel_id}/target`）。
    pub async fn update_panel_target(
        &self,
        panel_id: &str,
        request: &PanelTargetRequest,
    ) -> Result<()> {
        let path = format!("/v2/panels/{}/target", encode_path_segment(panel_id));
        self.send_void(HttpMethod::Put, &path, request).await
    }

    /// 获取自定义菜单（`GET /v2/menu`）。
    pub async fn get_menu(&self) -> Result<MenuResponse> {
        self.get_json("/v2/menu").await
    }

    /// 设置自定义菜单（`PUT /v2/menu`，覆盖式）。
    pub async fn put_menu(&self, request: &PutMenuRequest) -> Result<VersionResponse> {
        self.send_json(HttpMethod::Put, "/v2/menu", request).await
    }

    /// 创建入群自动审批策略（`POST /v2/groups/join_approval_strategy`）。
    pub async fn create_join_approval_strategy(
        &self,
        request: &CreateJoinApprovalStrategyRequest,
    ) -> Result<CreateJoinApprovalStrategyResponse> {
        self.post_json("/v2/groups/join_approval_strategy", request)
            .await
    }

    /// 审批策略列表（`GET /v2/groups/join_approval_strategy`）。
    pub async fn list_join_approval_strategy(
        &self,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<JoinApprovalStrategyList> {
        let path = query_path(
            "/v2/groups/join_approval_strategy",
            &[
                ("cursor", cursor),
                ("limit", limit.map(|value| value.to_string()).as_deref()),
            ],
        );
        self.get_json(&path).await
    }

    /// 更新审批策略（`PATCH /v2/groups/join_approval_strategy/{strategy_id}`）。
    pub async fn update_join_approval_strategy(
        &self,
        strategy_id: &str,
        request: &UpdateJoinApprovalStrategyRequest,
    ) -> Result<UpdateJoinApprovalStrategyResponse> {
        let path = format!(
            "/v2/groups/join_approval_strategy/{}",
            encode_path_segment(strategy_id)
        );
        self.send_json(HttpMethod::Patch, &path, request).await
    }

    /// 生效审批策略（`POST /v2/groups/join_approval_strategy/{strategy_id}/execute`）。
    pub async fn execute_join_approval_strategy(&self, strategy_id: &str) -> Result<()> {
        let path = format!(
            "/v2/groups/join_approval_strategy/{}/execute",
            encode_path_segment(strategy_id)
        );
        self.send_empty(HttpMethod::Post, &path).await
    }

    /// 增删审批策略白名单（`POST .../{strategy_id}/whitelist_users`）。
    pub async fn update_join_approval_strategy_whitelist(
        &self,
        strategy_id: &str,
        request: &WhitelistUsersRequest,
    ) -> Result<WhitelistUsersResponse> {
        let path = format!(
            "/v2/groups/join_approval_strategy/{}/whitelist_users",
            encode_path_segment(strategy_id)
        );
        self.post_json(&path, request).await
    }

    /// 删除审批策略（`DELETE /v2/groups/join_approval_strategy/{strategy_id}`）。
    pub async fn delete_join_approval_strategy(&self, strategy_id: &str) -> Result<()> {
        let path = format!(
            "/v2/groups/join_approval_strategy/{}",
            encode_path_segment(strategy_id)
        );
        self.send_empty(HttpMethod::Delete, &path).await
    }

    /// 批量移出群成员（`POST /v2/groups/{group_openid}/batch_remove_members`，单批 ≤ 20）。
    pub async fn batch_remove_members(
        &self,
        group_openid: &str,
        request: &BatchRemoveMembersRequest,
    ) -> Result<BatchRemoveMembersResponse> {
        let path = format!(
            "/v2/groups/{}/batch_remove_members",
            encode_path_segment(group_openid)
        );
        self.post_json(&path, request).await
    }

    /// 群黑名单列表（`GET /v2/groups/{group_openid}/member_blacklist`）。
    pub async fn member_blacklist_list(
        &self,
        group_openid: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<MemberBlacklistList> {
        let path = format!(
            "/v2/groups/{}/member_blacklist",
            encode_path_segment(group_openid)
        );
        let path = query_path(
            &path,
            &[
                ("cursor", cursor),
                ("limit", limit.map(|value| value.to_string()).as_deref()),
            ],
        );
        self.get_json(&path).await
    }

    /// 增删群黑名单（`POST /v2/groups/{group_openid}/member_blacklist`，单批 ≤ 20）。
    pub async fn update_member_blacklist(
        &self,
        group_openid: &str,
        request: &MemberBlacklistRequest,
    ) -> Result<MemberBlacklistResponse> {
        let path = format!(
            "/v2/groups/{}/member_blacklist",
            encode_path_segment(group_openid)
        );
        self.post_json(&path, request).await
    }

    /// 审批入群申请（`POST /v2/groups/{group_openid}/approval_join_request/{member_openid}`）。
    pub async fn approve_join_request(
        &self,
        group_openid: &str,
        member_openid: &str,
        request: &ApprovalJoinRequest,
    ) -> Result<()> {
        let path = format!(
            "/v2/groups/{}/approval_join_request/{}",
            encode_path_segment(group_openid),
            encode_path_segment(member_openid)
        );
        self.post_json_void(&path, request).await
    }

    /// 撤回群消息（`DELETE /v2/groups/{group_openid}/messages/{message_id}`）。
    pub async fn retract_group_message(&self, group_openid: &str, message_id: &str) -> Result<()> {
        let path = format!(
            "/v2/groups/{}/messages/{}",
            encode_path_segment(group_openid),
            encode_path_segment(message_id)
        );
        self.send_empty(HttpMethod::Delete, &path).await
    }

    /// 撤回 C2C 消息（`DELETE /v2/users/{user_openid}/messages/{message_id}`）。
    pub async fn retract_c2c_message(&self, user_openid: &str, message_id: &str) -> Result<()> {
        let path = format!(
            "/v2/users/{}/messages/{}",
            encode_path_segment(user_openid),
            encode_path_segment(message_id)
        );
        self.send_empty(HttpMethod::Delete, &path).await
    }

    /// 回应互动事件（`PUT /interactions/{interaction_id}`，注意没有 `/v2` 前缀）。
    ///
    /// `code` 为 0~5：0 成功 / 1 操作失败 / 2 操作频繁 / 3 重复操作 / 4 没有权限 / 5 仅管理员。
    pub async fn put_interaction(&self, interaction_id: &str, code: u32) -> Result<()> {
        let path = format!("/interactions/{}", encode_path_segment(interaction_id));
        self.send_void(HttpMethod::Put, &path, &serde_json::json!({ "code": code }))
            .await
    }

    /// 生成机器人资料页分享链接（`POST /v2/generate_url_link`）。
    ///
    /// 链接在信封的 `data.url`；`retcode=0` 却拿不到链接时按 Go
    /// `openapi/v2/message.go:394-396` 返回错误（handler 因此回 `1500`）。
    pub async fn generate_url_link(
        &self,
        request: &GenerateUrlLinkRequest,
    ) -> Result<GenerateUrlLink> {
        let response = self
            .body_authorized(HttpMethod::Post, "/v2/generate_url_link", request)
            .await?;
        let link: GenerateUrlLink = decode_json(&response)?;
        if link.url.is_empty() {
            return Err(Error::InvalidResponse(format!(
                "上游未返回 url（API success but url is empty）. Response: {}",
                body_for_log(&response.body)
            )));
        }
        Ok(link)
    }

    /// 发起 GET 并解析 JSON 响应（群管理子集复用）。
    pub(crate) async fn get_json<R: DeserializeOwned>(&self, path: &str) -> Result<R> {
        let response = self.authorized(HttpMethod::Get, path, None).await?;
        decode_json(&response)
    }

    /// 发起带 JSON 体的 POST 并解析 JSON 响应。
    pub(crate) async fn post_json<B: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<R> {
        self.send_json(HttpMethod::Post, path, body).await
    }

    /// 发起带 JSON 体的 POST 并忽略响应体（`204` / 空 body 的写接口）。
    pub(crate) async fn post_json_void<B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<()> {
        self.send_void(HttpMethod::Post, path, body).await
    }

    async fn send_json<B: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        method: HttpMethod,
        path: &str,
        body: &B,
    ) -> Result<R> {
        let response = self.body_authorized(method, path, body).await?;
        decode_json(&response)
    }

    async fn send_void<B: Serialize + ?Sized>(
        &self,
        method: HttpMethod,
        path: &str,
        body: &B,
    ) -> Result<()> {
        self.body_authorized(method, path, body).await.map(|_| ())
    }

    /// 发起无 body 的写请求（`DELETE` / 无参 `POST`）并忽略响应体。
    async fn send_empty(&self, method: HttpMethod, path: &str) -> Result<()> {
        self.authorized(method, path, None).await.map(|_| ())
    }

    async fn body_authorized<B: Serialize + ?Sized>(
        &self,
        method: HttpMethod,
        path: &str,
        body: &B,
    ) -> Result<HttpResponse> {
        let body = serde_json::to_vec(body)?;
        self.authorized(method, path, Some(body)).await
    }

    /// 注入 token 与通用请求头，并把非 2xx 响应映射为 [`ApiError`]。
    ///
    /// 所有 OpenAPI 调用都经过这里，因此请求日志也在这里打一条（botgo 的等价物是
    /// `openapi/v2/openapi.go:124-137` 的响应中间件）。
    async fn authorized(
        &self,
        method: HttpMethod,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<HttpResponse> {
        let token = self.tokens.token().await?;
        let url = format!("{}{path}", self.base_url);
        // `req:` 段在发送前算好（发送会把 body 移进请求）：没有体就是 Go 的 `null`。
        let logging = tracing::enabled!(target: OPENAPI_LOG_TARGET, tracing::Level::INFO);
        let req_log = match (&body, logging) {
            (Some(body), true) => body_for_log(body),
            _ => "null".to_owned(),
        };
        let mut request = HttpRequest::new(method, url.clone())
            .with_header("Authorization", format!("{AUTH_SCHEME} {token}"))
            .with_header("X-Union-Appid", self.tokens.config().app_id())
            .with_header("Accept", "application/json")
            .with_header(
                "User-Agent",
                concat!("gensokyo-qq/", env!("CARGO_PKG_VERSION")),
            );
        if body.is_some() {
            request = request.with_header("Content-Type", "application/json");
        }
        if let Some(body) = body {
            request = request.with_body(body);
        }

        let started = Instant::now();
        let response = self.transport.request(request).await;
        let elapsed = started.elapsed();
        let response = response?;
        if logging {
            tracing::info!(
                target: OPENAPI_LOG_TARGET,
                "{}",
                openapi_log_line(
                    method.as_str(),
                    &url,
                    response.header(TRACE_ID_HEADER),
                    response.status,
                    elapsed,
                    &req_log,
                    &response.body,
                )
            );
        }
        if !(200..300).contains(&response.status) {
            // token 失效时丢弃缓存，下一次调用会重新获取；本次仍把上游错误原样上抛。
            if response.status == 401 {
                self.tokens.invalidate().await;
            }
            return Err(ApiError::from_response(
                response.status,
                &response.body,
                response.header(TRACE_ID_HEADER),
            )
            .into());
        }
        Ok(response)
    }
}

/// 解析 JSON 响应体，失败时给出状态码与解析错误（不回显 body）。
fn decode_json<R: DeserializeOwned>(response: &HttpResponse) -> Result<R> {
    serde_json::from_slice(&response.body).map_err(|err| {
        Error::InvalidResponse(format!("解析响应失败(status={}): {err}", response.status))
    })
}

/// 拼一条 OpenAPI 请求日志，字段与顺序照 botgo `openapi/v2/openapi.go:124-137`：
/// `GET https://…, traceID:xx, status:200 OK, elapsed:569.8ms req: null, resp: {…}`。
///
/// `req` 是已渲染好的请求体段（无体为 `null`）；`resp_body` 在这里脱敏并按
/// [`LOG_BODY_LIMIT`] 截断。
fn openapi_log_line(
    method: &str,
    url: &str,
    trace_id: Option<&str>,
    status: u16,
    elapsed: Duration,
    req: &str,
    resp_body: &[u8],
) -> String {
    let reason = reqwest::StatusCode::from_u16(status)
        .ok()
        .and_then(|code| code.canonical_reason())
        .unwrap_or_default();
    let status_text = match reason {
        "" => status.to_string(),
        reason => format!("{status} {reason}"),
    };
    format!(
        "{method} {url}, traceID:{}, status:{status_text}, elapsed:{:.1}ms req: {req}, resp: {}",
        trace_id.unwrap_or_default(),
        elapsed.as_secs_f64() * 1000.0,
        body_for_log(resp_body),
    )
}

/// 请求 / 响应体 → 日志里的一段：JSON 先按 key 脱敏再压成一行，其余按 UTF-8 lossy 文本，
/// 最后统一截断。脱敏只发生在日志里，真实请求不受影响。
fn body_for_log(body: &[u8]) -> String {
    let rendered = match serde_json::from_slice::<Value>(body) {
        Ok(mut value) => {
            redact(&mut value);
            serde_json::to_string(&value)
                .unwrap_or_else(|_| String::from_utf8_lossy(body).into_owned())
        }
        Err(_) => String::from_utf8_lossy(body).into_owned(),
    };
    truncate(rendered)
}

/// 递归把敏感 key 的值换成 `前4位****`（Go 日志里的 `eHNQ****` 形态）。
fn redact(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, value) in map.iter_mut() {
                if REDACT_KEYS
                    .iter()
                    .any(|name| key.eq_ignore_ascii_case(name))
                {
                    *value = Value::String(mask(value));
                } else {
                    redact(value);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact),
        _ => {}
    }
}

/// 字符串取前 4 个字符；非字符串按 JSON 字面量取前 4 个字符（数字型 `app_id` 走这里）。
fn mask(value: &Value) -> String {
    let raw = match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    let prefix: String = raw.chars().take(4).collect();
    format!("{prefix}****")
}

/// 按字节上限截断到字符边界，并标记 `…(truncated)`。
fn truncate(text: String) -> String {
    if text.len() <= LOG_BODY_LIMIT {
        return text;
    }
    let mut end = LOG_BODY_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…(truncated)", &text[..end])
}

/// 分片上传端点：群 / C2C 二选一，`suffix` 取 `upload_prepare` / `upload_part_finish` / `files`。
fn file_upload_path(id: &str, is_group: bool, suffix: &str) -> String {
    let scope = if is_group { "groups" } else { "users" };
    format!("/v2/{scope}/{}/{}", encode_path_segment(id), suffix)
}

/// 拼查询串：`None` / 空串的参数整段跳过，其余按 [`encode_path_segment`] 同规则转义。
fn query_path(path: &str, params: &[(&str, Option<&str>)]) -> String {
    let mut url = path.to_owned();
    let mut separator = '?';
    for (key, value) in params {
        let Some(value) = value.filter(|value| !value.is_empty()) else {
            continue;
        };
        url.push(separator);
        separator = '&';
        url.push_str(key);
        url.push('=');
        url.push_str(&encode_path_segment(value));
    }
    url
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{LOG_BODY_LIMIT, body_for_log, openapi_log_line};

    #[test]
    fn log_line_matches_botgo_fields() {
        let line = openapi_log_line(
            "GET",
            "https://api.bot.qq.com/users/@me",
            Some("db5ba4cb"),
            200,
            Duration::from_micros(569_819),
            "null",
            br#"{"id":"1428"}"#,
        );
        assert_eq!(
            line,
            "GET https://api.bot.qq.com/users/@me, traceID:db5ba4cb, status:200 OK, \
             elapsed:569.8ms req: null, resp: {\"id\":\"1428\"}"
        );
    }

    #[test]
    fn log_bodies_redact_tokens_and_truncate() {
        let body = body_for_log(br#"{"access_token":"eHNQabcdef","app_id":10001,"content":"hi"}"#);
        assert!(body.contains(r#""access_token":"eHNQ****""#), "{body}");
        assert!(!body.contains("abcdef"), "{body}");
        assert!(body.contains(r#""app_id":"1000****""#), "{body}");

        let big = format!(r#"{{"data":"{}"}}"#, "x".repeat(5000));
        let out = body_for_log(big.as_bytes());
        assert!(out.ends_with("…(truncated)"), "{}", out.len());
        assert!(
            out.len() <= LOG_BODY_LIMIT + "…(truncated)".len(),
            "{}",
            out.len()
        );
    }
}
