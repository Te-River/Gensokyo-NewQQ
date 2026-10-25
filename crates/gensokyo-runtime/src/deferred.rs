//! deferred 计划的执行：`SendRequest` → `MessageToCreate` → QQ 上游。
//!
//! `gensokyo-actions` 的 `send_*` handler 只产出计划（`data.deferred == true`），本模块负责把
//! 计划变成真实发送：[`Dispatcher`](crate::dispatcher::Dispatcher) 反序列化出
//! [`SendRequest`] 后调用 [`execute`]——收件人还原成上游 OpenID、消息合成
//! [`MessageToCreate`]、再按 target 分派到
//! [`QqApi::send_group_message`](gensokyo_actions::QqApi::send_group_message) /
//! [`QqApi::send_c2c_message`](gensokyo_actions::QqApi::send_c2c_message)。
//!
//! # 支持范围
//!
//! | 段 | 落到 | 说明 |
//! |----|------|------|
//! | 文本 | `content` | 按 CQ 重新分词，`[CQ:at,qq=…]` 转 v2 的 `<@openid>`（虚拟号经 idmap 反查）；`auto_escape = true` 时原样发送、不解析 |
//! | 图片 / 语音 / 视频 / 文件 | `media` + `msg_type=7` | 见下节「媒体」 |
//! | `[CQ:reply]` | `msg_id` | 虚拟号先经 idmap / echo 还原成真实消息 ID；还原失败 `1400`，不静默丢 |
//! | `markdown` / `keyboard` / `card` / `music` / `stream` / `input_notify` / `unknown` / `unknown_image` / `unknown_record` | 不支持 | `1400`（请求侧段类型），不静默丢 |
//! | `active` / `wakeup` | 忽略 | 控制型标记，不影响本条发送；主动推送 / 唤醒语义仍未接线 |
//!
//! ## 媒体
//!
//! `MediaRef` 的三种来源（`local_*` / `url_*` / `base64_*`）都会先经
//! [`gensokyo_media::Fetcher`] 取字节（见 [`MediaRuntime`](crate::media::MediaRuntime)），再调
//! [`QqApi::upload_group_file`](gensokyo_actions::QqApi::upload_group_file) /
//! [`QqApi::upload_c2c_file`](gensokyo_actions::QqApi::upload_c2c_file)
//! 换成 `file_info`，最后填进 [`MessageToCreate::media`]，并按 Go
//! `handlers/send_group_msg.go` 的口径把 `msg_type` 置为 [`msg_type::RICH_MEDIA`]（7）。
//!
//! 限制（细节见 [`crate::media`]）：
//! - **单条消息只有一个 `media` 槽**：两个以上媒体段（含多张图片）返回 `1400`，而不是丢掉
//!   后面的段；
//! - **单条媒体上限 5MB**（[`MAX_MEDIA_BYTES`](crate::media::MAX_MEDIA_BYTES)，对齐 Go
//!   `file_data` 直传上限），超限 `1400`；Go 的超限分片上传尚未落地；
//! - [`MediaRef::Unknown`] 视为「已经拿到上游 `file_info`」，直接透传、不再上传（Go 的
//!   `unknown_*` 回退口径）；
//! - 不压缩 / 不转码：原字节直传。
//!
//! ## echo 回写
//!
//! 发送成功后把上游 `message_id` 写进 [`EchoStore::store_cache_in_memory`]（Go
//! `StoreCacheInMemory` 的行号双向映射），后续 `[CQ:reply,id=<虚拟号>]` 才能反查到它；
//! 回写失败只 `warn`，不改变本次发送结果。
//!
//! # 错误口径
//!
//! 请求侧问题（媒体来源 / 大小 / 类型、reply 映射缺失、不支持的段）返回 [`Error::Protocol`]
//! → `retcode 1400`；下载、上传与发送失败（[`Error::Network`] / [`Error::Upstream`]）→ `1500`。
//! 映射在 [`crate::dispatcher`] 里完成。

use gensokyo_actions::{ActionContext, MessageOut, SendRequest, SendTarget};
use gensokyo_core::{Error, Ob11Id, Result};
use gensokyo_protocol::cqcode;
use gensokyo_protocol::{MediaRef, OutboundPlan};
use gensokyo_qq::dto::{FileUpload, Media, MessageToCreate, file_type, msg_type};
use gensokyo_storage::EchoStore;
use serde_json::Value;

use crate::media::MediaRuntime;

/// handler 响应 `data` 里的 deferred 标记（与 `gensokyo_actions::DEFERRED_FIELD` 同值）。
pub const DEFERRED_FIELD: &str = gensokyo_actions::DEFERRED_FIELD;

/// `data` 是否是一份 deferred 计划（`{"deferred": true, …}`）。
#[must_use]
pub fn is_deferred(data: &Value) -> bool {
    data.get(DEFERRED_FIELD).and_then(Value::as_bool) == Some(true)
}

/// deferred 执行链的额外依赖（[`ActionContext`] 之外的运行时句柄）。
#[derive(Debug, Clone, Copy)]
pub struct DeferredDeps<'a> {
    /// 媒体下载器；生产用 [`MediaRuntime::default`]，单测注入假传输。
    pub media: &'a MediaRuntime,
    /// echo 存储：发送成功后回写 `message_id`，`[CQ:reply]` 反查虚拟号也走它。
    pub echo: Option<&'a EchoStore>,
}

/// 执行一条 deferred 计划，返回上游消息 ID。
///
/// 成功后把 `message_id` 回写 echo（见模块文档「echo 回写」）。
///
/// # Errors
/// 收件人无法反查成 OpenID（[`ActionContext::resolve_send_target`]）、消息含不支持的媒体 /
/// 段、reply 无法还原成真实消息 ID（以上 [`Error::Protocol`] → `1400`），或下载 / 上传 /
/// 上游发送失败（[`Error::Network`] / [`Error::Upstream`] → `1500`）。
pub async fn execute(
    context: &ActionContext,
    plan: &SendRequest,
    deps: &DeferredDeps<'_>,
) -> Result<String> {
    let openid = context.resolve_send_target(&plan.target)?;
    let message = compose(context, &plan.target, &openid, &plan.message, deps).await?;
    let response = match &plan.target {
        SendTarget::Group(_) => context.api().send_group_message(openid, message).await?,
        SendTarget::Private(_) => context.api().send_c2c_message(openid, message).await?,
    };
    record_sent_message(deps.echo, &response.id);
    Ok(response.id)
}

/// 发送成功后的 echo 回写（Go `StoreCacheInMemory`）：失败只告警，不改变发送结果。
fn record_sent_message(echo: Option<&EchoStore>, message_id: &str) {
    let Some(echo) = echo else {
        return;
    };
    if message_id.is_empty() {
        tracing::warn!("deferred 发送成功但上游未返回 message_id，跳过 echo 回写");
        return;
    }
    if let Err(err) = echo.store_cache_in_memory(message_id) {
        tracing::warn!(
            message_id = %message_id,
            error = %err,
            "deferred 发送成功后写 echo 缓存失败"
        );
    }
}

/// 把一条待发送消息合成 [`MessageToCreate`]（文本 + reply + 单个 media）。
///
/// `openid` 由调用方从 `target` 还原：媒体上传要按群 / C2C 选端点，两者都要它。
///
/// # Errors
/// 文本里的 `[CQ:at]` 缺 `qq`、`at` 虚拟号无法反查、reply 无法还原，或消息含本阶段不支持的段。
pub async fn compose(
    context: &ActionContext,
    target: &SendTarget,
    openid: &str,
    message: &MessageOut,
    deps: &DeferredDeps<'_>,
) -> Result<MessageToCreate> {
    if let Some(kind) = unsupported_segment(&message.plan) {
        return Err(Error::Protocol(format!(
            "deferred 发送暂不支持 {kind} 段：{message:?}"
        )));
    }

    let mut out = MessageToCreate::default();
    let content = render_text(context, message)?;
    if !content.is_empty() {
        out.content = Some(content);
    }
    // Go：`[CQ:reply]` → `msg_id`（被动回复）。事件 ID 不在计划里，不必与 `event_id` 互斥。
    if let Some(reply) = &message.plan.reply_msg_id {
        out.msg_id = Some(resolve_reply(context, deps.echo, reply)?);
    }
    if let Some(media) = compose_media(context, target, openid, &message.plan, deps).await? {
        out.media = Some(media);
        out.msg_type = Some(msg_type::RICH_MEDIA);
    }
    Ok(out)
}

/// 文本主体 → v2 `content`：`[CQ:at,qq=…]` 转 `<@openid>`，其余 CQ 码保持原样。
fn render_text(context: &ActionContext, message: &MessageOut) -> Result<String> {
    if message.auto_escape {
        return Ok(message.text.clone());
    }

    let mut out = String::new();
    for segment in cqcode::parse(&message.text) {
        match segment.kind.as_str() {
            "text" => out.push_str(segment.text_content().unwrap_or_default()),
            "at" => {
                // CQ `at` 段的参数名是 `qq`（不是 `id`），见 `cqcode::CODES`。
                let id = segment
                    .get("qq")
                    .ok_or_else(|| Error::Protocol("[CQ:at] 缺少 qq".to_owned()))?;
                let openid = context.resolve_openid("qq", id, "user_openid")?;
                out.push_str("<@");
                out.push_str(&openid);
                out.push('>');
            }
            _ => out.push_str(&segment.to_cq()),
        }
    }
    Ok(out)
}

/// `[CQ:reply]` 目标 → 上游真实消息 ID。
///
/// 字符串形态（32 位 OpenID / flag 型 ID）本身就是真实 ID，直接使用；数字形态是 OneBot
/// 虚拟消息编号，按两条来源依次反查：
/// 1. idmap（入站消息经 [`gensokyo_storage::IdMapStore::store_cache`] 登记，Go
///    `RetrieveRowByCachev2`），值为 `"<群号> <消息ID>"` 时取最后一段；
/// 2. echo 行号反表（本进程发送成功的消息经 [`EchoStore::store_cache_in_memory`] 登记）。
///
/// 都查不到说明引用的是别的进程 / 已过期的 ID——报 `1400` 说明原因，而不是静默丢掉 reply。
fn resolve_reply(
    context: &ActionContext,
    echo: Option<&EchoStore>,
    reply: &Ob11Id,
) -> Result<String> {
    if let Some(raw) = reply.as_str() {
        if raw.trim().is_empty() {
            return Err(Error::Protocol("[CQ:reply] 的 id 为空".to_owned()));
        }
        return Ok(raw.to_owned());
    }
    let row = reply
        .as_i64()
        .ok_or_else(|| Error::Protocol(format!("[CQ:reply] 的 id 非法: {reply}")))?;

    if let Some(idmap) = context.idmap() {
        match idmap.retrieve_row_by_cache(row) {
            Ok(real) => {
                let real = last_token(&real);
                if !real.is_empty() {
                    return Ok(real);
                }
                tracing::debug!(row, "idmap 反查到空消息 ID，继续查 echo");
            }
            // 未登记 / 已过期都按「查不到」处理，继续走 echo
            Err(err) if err.is_not_found() => {}
            Err(err) => return Err(Error::Storage(format!("reply 反查 idmap 失败: {err}"))),
        }
    }

    if let Some(real) =
        echo.and_then(|echo| echo.get_cache_id_from_memory_by_row_id(&row.to_string()))
        && !real.trim().is_empty()
    {
        return Ok(real);
    }

    Err(Error::Protocol(format!(
        "[CQ:reply,id={row}] 无法还原成真实消息 ID（idmap / echo 均未命中）：\
         请改用真实消息 ID，或确认被引用的消息未被清理"
    )))
}

/// idmap 里有的记录形如 `"<群号> <消息ID>"`（Go `StoreCachev2`），取最后一段。
fn last_token(raw: &str) -> String {
    raw.split_whitespace().last().unwrap_or_default().to_owned()
}

/// `MediaToCreate.media` 的段类型。
#[derive(Clone, Copy)]
enum MediaKind {
    /// 图片（`file_type=1`）。
    Image,
    /// 语音（`file_type=3`）。
    Record,
    /// 视频（`file_type=2`）。
    Video,
    /// 文件（`file_type=4`）。
    File,
}

impl MediaKind {
    /// 段名（错误文案用）。
    const fn label(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Record => "record",
            Self::Video => "video",
            Self::File => "file",
        }
    }

    /// 上游 `file_type`。
    const fn file_type(self) -> u32 {
        match self {
            Self::Image => file_type::IMAGE,
            Self::Record => file_type::VOICE,
            Self::Video => file_type::VIDEO,
            Self::File => file_type::FILE,
        }
    }
}

/// 单个媒体段 → `MessageToCreate.media`（取字节 → 上传 → `file_info`）。
///
/// v2 文本消息只承载一个 `media`，因此多段媒体报错而不是丢掉后面的段。
async fn compose_media(
    context: &ActionContext,
    target: &SendTarget,
    openid: &str,
    plan: &OutboundPlan,
    deps: &DeferredDeps<'_>,
) -> Result<Option<Media>> {
    let mut candidates: Vec<(MediaKind, &MediaRef, Option<&str>)> = Vec::new();
    candidates.extend(
        plan.images
            .iter()
            .map(|media| (MediaKind::Image, media, None)),
    );
    candidates.extend(
        plan.records
            .iter()
            .map(|media| (MediaKind::Record, media, None)),
    );
    candidates.extend(
        plan.videos
            .iter()
            .map(|media| (MediaKind::Video, media, None)),
    );
    candidates.extend(
        plan.files
            .iter()
            .map(|file| (MediaKind::File, &file.media, file.file_name.as_deref())),
    );

    let Some(&(kind, reference, file_name)) = candidates.first() else {
        return Ok(None);
    };
    if candidates.len() > 1 {
        let labels = candidates
            .iter()
            .map(|(kind, _, _)| kind.label())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(Error::Protocol(format!(
            "QQ v2 单条消息只支持一个 media（收到 {} 个媒体段：{labels}），请拆分后重发",
            candidates.len()
        )));
    }

    // 已是上游 `file_info` 的形态（Go 的 `unknown_*` 回退）：直接透传，不再上传
    if let MediaRef::Unknown(file_info) = reference {
        return Ok(Some(Media {
            file_info: file_info.clone(),
        }));
    }

    let fetched = deps.media.fetch(reference).await?;
    let upload = FileUpload {
        file_type: kind.file_type(),
        file_data: Some(gensokyo_media::encode_base64(&fetched.bytes)),
        file_name: Some(file_name.map_or_else(|| fetched.file_name.clone(), str::to_owned)),
        ..FileUpload::default()
    };
    let uploaded = match target {
        SendTarget::Group(_) => context.api().upload_group_file(openid, upload).await?,
        SendTarget::Private(_) => context.api().upload_c2c_file(openid, upload).await?,
    };
    if uploaded.file_info.is_empty() {
        return Err(Error::Upstream {
            code: 0,
            message: "媒体上传成功但未返回 file_info".to_owned(),
        });
    }
    Ok(Some(Media {
        file_info: uploaded.file_info,
    }))
}

/// 本阶段不支持的消息段；返回第一个命中的段名。
fn unsupported_segment(plan: &OutboundPlan) -> Option<&'static str> {
    let candidates: [(&'static str, bool); 9] = [
        ("markdown", !plan.markdown.is_empty()),
        ("keyboard", !plan.keyboard.is_empty()),
        ("card", !plan.card.is_empty()),
        ("music", !plan.qqmusic.is_empty()),
        ("stream", plan.stream.is_some()),
        ("input_notify", plan.input_notify.is_some()),
        ("unknown", !plan.extra.is_empty()),
        ("unknown_image", !plan.unknown_images.is_empty()),
        ("unknown_record", !plan.unknown_records.is_empty()),
    ];
    candidates
        .iter()
        .find(|(_, present)| *present)
        .map(|(kind, _)| *kind)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use gensokyo_actions::{ActionContext, MessageOut, QqApi, QqFuture, SendRequest, SendTarget};
    use gensokyo_config::ConfigManager;
    use gensokyo_core::{Error, Ob11Id, Result};
    use gensokyo_media::{BoxFuture, FetchRequest, Fetcher, MediaTransport, TransportResponse};
    use gensokyo_protocol::{MediaRef, OutboundPlan};
    use gensokyo_qq::dto::{
        BotInGroupState, FileUpload, GroupInfo, GroupMember, GroupMemberList, JoinRequestList,
        MediaResponse, MessageResponse, MessageToCreate, file_type, msg_type,
    };
    use gensokyo_storage::EchoStore;
    use tempfile::TempDir;

    use super::{DeferredDeps, execute};
    use crate::media::{MAX_MEDIA_BYTES, MediaRuntime};

    /// 测试统一的 32 位 OpenID（`resolve_openid` 对 32 位直通，不需要 idmap）。
    const OPENID: &str = "0123456789ABCDEF0123456789ABCDEF";

    // ---- 装置 ----

    fn manager() -> (TempDir, Arc<ConfigManager>) {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings: {}\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        (dir, manager)
    }

    fn context(api: Arc<FakeQqApi>) -> (TempDir, ActionContext) {
        let (dir, manager) = manager();
        (dir, ActionContext::new(api, manager))
    }

    /// 记录入参、按槽位注入结果的 `QqApi` 替身（只覆盖 deferred 用到的四个方法）。
    #[derive(Debug, Default)]
    struct FakeQqApi {
        uploads: Mutex<Vec<FileUpload>>,
        sent: Mutex<Vec<MessageToCreate>>,
        upload_result: Mutex<Option<Result<MediaResponse>>>,
        send_result: Mutex<Option<Result<MessageResponse>>>,
    }

    impl FakeQqApi {
        fn with_upload(file_info: &str) -> Self {
            Self {
                upload_result: Mutex::new(Some(Ok(MediaResponse {
                    file_info: file_info.to_owned(),
                    ..MediaResponse::default()
                }))),
                ..Self::default()
            }
        }

        fn with_upload_error(err: Error) -> Self {
            Self {
                upload_result: Mutex::new(Some(Err(err))),
                ..Self::default()
            }
        }

        fn with_send(mut self, message_id: &str) -> Self {
            self.send_result = Mutex::new(Some(Ok(MessageResponse {
                id: message_id.to_owned(),
                ..MessageResponse::default()
            })));
            self
        }

        fn uploads(&self) -> Vec<FileUpload> {
            self.uploads.lock().expect("上传记录锁未中毒").clone()
        }

        fn sent(&self) -> Vec<MessageToCreate> {
            self.sent.lock().expect("发送记录锁未中毒").clone()
        }

        fn upload(&self, file: FileUpload) -> Result<MediaResponse> {
            self.uploads.lock().expect("上传记录锁未中毒").push(file);
            self.upload_result
                .lock()
                .expect("上传槽锁未中毒")
                .take()
                .expect("测试未配置 upload 响应")
        }

        fn send(&self, msg: MessageToCreate) -> Result<MessageResponse> {
            self.sent.lock().expect("发送记录锁未中毒").push(msg);
            self.send_result
                .lock()
                .expect("发送槽锁未中毒")
                .take()
                .expect("测试未配置 send 响应")
        }
    }

    impl QqApi for FakeQqApi {
        fn get_group_info(&self, _group_openid: &str) -> QqFuture<'_, GroupInfo> {
            Box::pin(async { Err(Error::Unsupported("未提供".to_owned())) })
        }

        fn get_group_member_info(
            &self,
            _group_openid: &str,
            _member_openid: &str,
        ) -> QqFuture<'_, GroupMember> {
            Box::pin(async { Err(Error::Unsupported("未提供".to_owned())) })
        }

        fn get_group_member_list(
            &self,
            _group_openid: &str,
            _limit: u32,
        ) -> QqFuture<'_, GroupMemberList> {
            Box::pin(async { Err(Error::Unsupported("未提供".to_owned())) })
        }

        fn set_group_ban(
            &self,
            _group_openid: &str,
            _member_openid: &str,
            _duration_secs: i64,
        ) -> QqFuture<'_, ()> {
            Box::pin(async { Err(Error::Unsupported("未提供".to_owned())) })
        }

        fn get_group_bot_state(&self, _group_openid: &str) -> QqFuture<'_, BotInGroupState> {
            Box::pin(async { Err(Error::Unsupported("未提供".to_owned())) })
        }

        fn get_group_join_request_list(
            &self,
            _group_openid: &str,
            _cursor: Option<&str>,
            _limit: Option<u32>,
        ) -> QqFuture<'_, JoinRequestList> {
            Box::pin(async { Err(Error::Unsupported("未提供".to_owned())) })
        }

        fn send_group_message(
            &self,
            _group_openid: String,
            msg: MessageToCreate,
        ) -> QqFuture<'_, MessageResponse> {
            Box::pin(async move { self.send(msg) })
        }

        fn send_c2c_message(
            &self,
            _user_openid: String,
            msg: MessageToCreate,
        ) -> QqFuture<'_, MessageResponse> {
            Box::pin(async move { self.send(msg) })
        }

        fn upload_group_file(
            &self,
            _group_openid: &str,
            file: FileUpload,
        ) -> QqFuture<'_, MediaResponse> {
            Box::pin(async move { self.upload(file) })
        }

        fn upload_c2c_file(
            &self,
            _user_openid: &str,
            file: FileUpload,
        ) -> QqFuture<'_, MediaResponse> {
            Box::pin(async move { self.upload(file) })
        }
    }

    /// 假下载出口：固定响应体，记录每次请求。
    #[derive(Debug, Clone)]
    struct FakeTransport {
        body: Vec<u8>,
        requests: Arc<Mutex<Vec<FetchRequest>>>,
    }

    impl FakeTransport {
        fn new(body: &[u8]) -> Self {
            Self {
                body: body.to_vec(),
                requests: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn requests(&self) -> Vec<FetchRequest> {
            self.requests.lock().expect("下载记录锁未中毒").clone()
        }
    }

    impl MediaTransport for FakeTransport {
        fn get(
            &self,
            request: FetchRequest,
        ) -> BoxFuture<'_, gensokyo_media::Result<TransportResponse>> {
            self.requests
                .lock()
                .expect("下载记录锁未中毒")
                .push(request);
            let body = self.body.clone();
            Box::pin(async move {
                Ok(TransportResponse {
                    status: 200,
                    content_type: Some("image/png".to_owned()),
                    body,
                })
            })
        }
    }

    fn request(target: SendTarget, plan: OutboundPlan) -> SendRequest {
        SendRequest {
            target,
            message: MessageOut {
                text: String::new(),
                plan,
                auto_escape: false,
            },
        }
    }

    fn group_request(plan: OutboundPlan) -> SendRequest {
        request(SendTarget::Group(OPENID.to_owned()), plan)
    }

    fn deps<'a>(media: &'a MediaRuntime, echo: Option<&'a EchoStore>) -> DeferredDeps<'a> {
        DeferredDeps { media, echo }
    }

    fn image_plan(image: MediaRef) -> OutboundPlan {
        OutboundPlan {
            images: vec![image],
            ..OutboundPlan::default()
        }
    }

    // ---- 媒体 ----

    #[tokio::test]
    async fn image_url_is_fetched_then_uploaded() {
        let api = Arc::new(FakeQqApi::with_upload("FILE-INFO").with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let transport = FakeTransport::new(b"PNG-BYTES");
        let media = MediaRuntime::with_transport(Arc::new(transport.clone()));
        let plan = group_request(image_plan(MediaRef::Url(
            "https://example.com/a.png".to_owned(),
        )));

        let message_id = execute(&context, &plan, &deps(&media, None))
            .await
            .expect("URL 图片应发送成功");

        assert_eq!(message_id, "MSG-1");
        let requests = transport.requests();
        assert_eq!(requests.len(), 1, "URL 必须先经下载器");
        assert_eq!(requests[0].url, "https://example.com/a.png");
        assert_eq!(requests[0].max_bytes, MAX_MEDIA_BYTES);

        let uploads = api.uploads();
        assert_eq!(uploads.len(), 1);
        assert_eq!(uploads[0].file_type, file_type::IMAGE);
        assert_eq!(
            uploads[0].file_data.as_deref(),
            Some(gensokyo_media::encode_base64(b"PNG-BYTES").as_str())
        );
        assert_eq!(uploads[0].file_name.as_deref(), Some("media.png"));

        let sent = api.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].media.as_ref().map(|media| media.file_info.as_str()),
            Some("FILE-INFO")
        );
        assert_eq!(sent[0].msg_type, Some(msg_type::RICH_MEDIA));
    }

    #[tokio::test]
    async fn local_image_is_read_without_copy_and_keeps_name() {
        let api = Arc::new(FakeQqApi::with_upload("FILE-INFO").with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("cat.png");
        std::fs::write(&path, b"LOCAL-BYTES").expect("写本地图片失败");
        let media = MediaRuntime::default();
        let plan = group_request(image_plan(MediaRef::Local(path.clone())));

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("本地图片应发送成功");

        let uploads = api.uploads();
        assert_eq!(
            uploads[0].file_data.as_deref(),
            Some(gensokyo_media::encode_base64(b"LOCAL-BYTES").as_str())
        );
        assert_eq!(uploads[0].file_name.as_deref(), Some("cat.png"));
        assert!(path.exists(), "本地原文件不能被删除");
    }

    #[tokio::test]
    async fn local_image_over_limit_is_a_request_error() {
        let api = Arc::new(FakeQqApi::with_upload("FILE-INFO").with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("big.png");
        std::fs::write(&path, b"0123456789").expect("写本地图片失败");
        // 上限收到 4 字节：10 字节的本地文件必然超限
        let media =
            MediaRuntime::with_fetcher(Fetcher::new(FakeTransport::new(b"")).with_max_bytes(4));
        let plan = group_request(image_plan(MediaRef::Local(path)));

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("超限必须报错");

        assert!(matches!(err, Error::Protocol(_)), "超限应映射 1400: {err}");
        assert!(api.uploads().is_empty(), "超限不应触发上传");
        assert!(api.sent().is_empty(), "超限不应发送");
    }

    #[tokio::test]
    async fn two_media_segments_are_a_request_error() {
        let api = Arc::new(FakeQqApi::with_upload("FILE-INFO").with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = MediaRuntime::with_transport(Arc::new(FakeTransport::new(b"x")));
        let plan = group_request(OutboundPlan {
            images: vec![
                MediaRef::Url("https://example.com/a.png".to_owned()),
                MediaRef::Url("https://example.com/b.png".to_owned()),
            ],
            ..OutboundPlan::default()
        });

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("多图必须报错");

        assert!(matches!(err, Error::Protocol(_)), "多图应映射 1400: {err}");
        assert!(api.sent().is_empty(), "多图不应发送（不静默丢段）");
    }

    #[tokio::test]
    async fn upload_failure_is_an_upstream_error() {
        let api = Arc::new(FakeQqApi::with_upload_error(Error::Upstream {
            code: 40034001,
            message: "file too large".to_owned(),
        }));
        let (_dir, context) = context(Arc::clone(&api));
        let media = MediaRuntime::with_transport(Arc::new(FakeTransport::new(b"PNG-BYTES")));
        let plan = group_request(image_plan(MediaRef::Url(
            "https://example.com/a.png".to_owned(),
        )));

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("上传失败必须报错");

        assert!(
            matches!(err, Error::Upstream { .. }),
            "应保留上游错误: {err}"
        );
        assert!(api.sent().is_empty(), "上传失败不应继续发送");
    }

    #[tokio::test]
    async fn unknown_media_is_passed_through_as_file_info() {
        let api = Arc::new(FakeQqApi::with_upload("FILE-INFO").with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = MediaRuntime::with_transport(Arc::new(FakeTransport::new(b"x")));
        let plan = group_request(image_plan(MediaRef::Unknown("FILE-INFO".to_owned())));

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("未知媒体按 file_info 透传");

        assert!(api.uploads().is_empty(), "透传不应再上传");
        assert_eq!(
            api.sent()[0]
                .media
                .as_ref()
                .map(|media| media.file_info.as_str()),
            Some("FILE-INFO")
        );
    }

    // ---- reply / echo ----

    #[tokio::test]
    async fn reply_without_mapping_is_a_request_error() {
        let api = Arc::new(FakeQqApi::with_upload("FILE-INFO").with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = MediaRuntime::with_transport(Arc::new(FakeTransport::new(b"x")));
        let plan = group_request(OutboundPlan {
            reply_msg_id: Some(Ob11Id::Num(424_242)),
            ..OutboundPlan::default()
        });

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("无映射的虚拟 reply 必须报错");

        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
        assert!(
            err.to_string().contains("无法还原"),
            "错误文案要说清原因: {err}"
        );
        assert!(api.sent().is_empty(), "reply 失败不应发送");
    }

    #[tokio::test]
    async fn reply_via_echo_sets_msg_id() {
        let api = Arc::new(FakeQqApi::with_upload("FILE-INFO").with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = MediaRuntime::with_transport(Arc::new(FakeTransport::new(b"x")));
        let echo = EchoStore::new();
        let row = echo
            .store_cache_in_memory("REAL-MSG-ID")
            .expect("登记虚拟号");
        let plan = group_request(OutboundPlan {
            reply_msg_id: Some(Ob11Id::Num(row)),
            ..OutboundPlan::default()
        });

        execute(&context, &plan, &deps(&media, Some(&echo)))
            .await
            .expect("echo 能反查时应发送成功");

        assert_eq!(api.sent()[0].msg_id.as_deref(), Some("REAL-MSG-ID"));
    }

    #[tokio::test]
    async fn reply_with_real_string_id_passes_through() {
        let api = Arc::new(FakeQqApi::with_upload("FILE-INFO").with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = MediaRuntime::with_transport(Arc::new(FakeTransport::new(b"x")));
        let plan = group_request(OutboundPlan {
            reply_msg_id: Some(Ob11Id::str("REAL-MSG-ID")),
            ..OutboundPlan::default()
        });

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("字符串形态直接当真实 ID");

        assert_eq!(api.sent()[0].msg_id.as_deref(), Some("REAL-MSG-ID"));
    }

    #[tokio::test]
    async fn send_success_writes_message_id_into_echo() {
        let api = Arc::new(FakeQqApi::with_upload("FILE-INFO").with_send("MSG-9"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = MediaRuntime::with_transport(Arc::new(FakeTransport::new(b"x")));
        let echo = EchoStore::new();
        let plan = request(
            SendTarget::Private(OPENID.to_owned()),
            OutboundPlan::default(),
        );

        execute(&context, &plan, &deps(&media, Some(&echo)))
            .await
            .expect("纯文本应发送成功");

        // 行号是 MD5 派生的稳定值：用独立实例算出期望值，再查被测实例，才能证明回写真的发生了
        let expected_row = EchoStore::new()
            .store_cache_in_memory("MSG-9")
            .expect("换算行号");
        assert_eq!(
            echo.get_cache_id_from_memory_by_row_id(&expected_row.to_string()),
            Some("MSG-9".to_owned())
        );
    }
}
