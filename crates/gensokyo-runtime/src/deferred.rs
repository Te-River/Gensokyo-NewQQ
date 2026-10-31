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
//! | `[CQ:at,qq=all]` / `[CQ:at,qq=everyone]` / 字面 `<qqbot-at-everyone />` | `<qqbot-at-everyone />` | 仅群聊，且受「每群每自然日 1 次」限制，见下节「@全员」 |
//! | `markdown` | `markdown` + `msg_type=2` | 只取 `content`：模板字段（`template_id` / `custom_template_id`）已废弃，出站不构造；文本被丢弃、`card` 让位 |
//! | `card` | `card` + `msg_type=8` | 群聊与单聊同构；`{type:"tuwen", content:{title,description,url,pic_url}}`，四字段校验见 [`compose_card`] |
//! | 图片 / 语音 / 视频 / 文件 | `media` + `msg_type=7` | 见下节「媒体」 |
//! | `[CQ:reply]` | `msg_id` | 虚拟号先经 idmap / echo 还原成真实消息 ID；还原失败 `1400`，不静默丢 |
//! | `keyboard` / `music` / `stream` / `input_notify` / `unknown` / `unknown_image` / `unknown_record` | 不支持 | `1400`（请求侧段类型），不静默丢 |
//! | `active` / `wakeup` | 忽略 | 控制型标记，不影响本条发送；主动推送 / 唤醒语义仍未接线 |
//!
//! ## 消息体三选一
//!
//! `msg_type` 只能取一个值，因此 markdown / card / rich media 互为排他：同时出现时按
//! markdown > card > media 取更高优先级的那个，media 与 markdown / card 同现直接 `1400`
//! （否则必须静默丢段）。markdown 与 card 同现时 card 不参与校验（避免一张坏卡片拦住
//! 合法的 markdown），与 Go 的「markdown 胜 card」一致。
//!
//! ## @全员 与 @用户
//!
//! 归一在 [`render_text`]（文本）与 [`compose_markdown`]（markdown 内容）各做一次，都走
//! [`at_everyone::normalize`]：`[CQ:at,qq=all]` 与 `[CQ:at,qq=everyone]` 产出官方标签
//! `<qqbot-at-everyone />`，字面标签原样保留；归一后重复的标签由
//! [`at_everyone::dedupe_tag`] 收敛成一个（字面标签与 CQ 写法可能同时出现）。文本与 markdown
//! 的 @用户 走两套标签：
//! 文本仍是旧式 `<@openid>`，markdown 是官方 `<qqbot-at-user id="openid" />`（数字虚拟号
//! 必须经 idmap 反查，失败 `1400`）。发送前扫最终载荷：
//!
//! - 单聊出现标签 → `1400`（官方只允许群聊，静默丢弃不可接受）；
//! - 群聊出现标签 → 先占当天的名额（[`at_everyone::reserve`]，落 idmap config 的
//!   `at_everyone` section），**发送失败回滚**，超限 `1400`；
//! - 文本 + markdown 同现时标签迁移进 markdown 正文（否则随 `content` 清空丢失）；
//! - 卡片与 @全员 同现 → `1400`（`msg_type=8` 会清空正文，标签无处安放）；
//! - `enable_at_everyone_limit=false` 时既不拦也不计数；未装配 idmap 时只 `warn` 放行
//!   （没有计数载体，不能把消息卡死在这里）。
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
use gensokyo_qq::dto::message::{MessageCard, MessageCardContent};
use gensokyo_qq::dto::{FileUpload, Markdown, Media, MessageToCreate, file_type, msg_type};
use gensokyo_storage::EchoStore;
use serde_json::Value;

use crate::at_everyone::{self, Reservation};
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
/// 段、markdown 缺 `content` 或其中的 `[CQ:at,qq=数字]` 反查失败、卡片字段非法、卡片与
/// @全员 同现、单聊出现 @全员、同群同日第二次 @全员、reply 无法还原成真实消息 ID
/// （以上 [`Error::Protocol`] → `1400`），或下载 / 上传 / 上游发送失败
/// （[`Error::Network`] / [`Error::Upstream`] → `1500`）。
pub async fn execute(
    context: &ActionContext,
    plan: &SendRequest,
    deps: &DeferredDeps<'_>,
) -> Result<String> {
    let openid = context.resolve_send_target(&plan.target)?;
    let message = compose(context, &plan.target, &openid, &plan.message, deps).await?;
    // 先占当天的 @全员 名额再发送：并发两条 @全员 只有一条能占住
    let reservation = reserve_at_everyone(context, &plan.target, &openid, &message)?;
    let sent = match &plan.target {
        SendTarget::Group(_) => context.api().send_group_message(openid, message).await,
        SendTarget::Private(_) => context.api().send_c2c_message(openid, message).await,
    };
    let response = match sent {
        Ok(response) => response,
        Err(err) => {
            // 发送失败要回滚，不能让一次失败吃掉当天的名额
            if let (Some(reservation), Some(idmap)) = (reservation, context.idmap()) {
                reservation.rollback(idmap);
            }
            return Err(err);
        }
    };
    record_sent_message(deps.echo, &response.id);
    Ok(response.id)
}

/// 消息里是否带 @全员 标签（扫 `content` 与 `markdown.content`）。
fn has_at_everyone(message: &MessageToCreate) -> bool {
    message
        .content
        .as_deref()
        .is_some_and(at_everyone::contains_tag)
        || message
            .markdown
            .as_ref()
            .and_then(|markdown| markdown.content.as_deref())
            .is_some_and(at_everyone::contains_tag)
}

/// 群聊带 @全员 时占当天名额；没有标签直接返回 `None`。
///
/// # Errors
/// 单聊出现标签、同群同日第二次都返回 [`Error::Protocol`]（`1400`）；写 idmap 失败
/// 返回 [`Error::Storage`]（读失败按「未占用」放行，见 [`at_everyone::reserve`]）。
fn reserve_at_everyone(
    context: &ActionContext,
    target: &SendTarget,
    openid: &str,
    message: &MessageToCreate,
) -> Result<Option<Reservation>> {
    if !has_at_everyone(message) {
        return Ok(None);
    }
    if !matches!(target, SendTarget::Group(_)) {
        return Err(Error::Protocol(
            "@全员 只支持群聊：单聊消息不能包含 <qqbot-at-everyone />".to_owned(),
        ));
    }
    if !context.config().snapshot().get_enable_at_everyone_limit() {
        return Ok(None);
    }
    let Some(idmap) = context.idmap() else {
        tracing::warn!("@全员 限流未生效：未装配 idmap，无法写入每日计数");
        return Ok(None);
    };
    Ok(Some(at_everyone::reserve(idmap, openid)?))
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
    // msg_type 只能取一个值：rich media 与 markdown / card 混用必须在这里拒绝，否则只能静默丢段
    if (!message.plan.markdown.is_empty() || !message.plan.card.is_empty())
        && has_media(&message.plan)
    {
        return Err(Error::Protocol(
            "markdown / card 与图片 / 语音 / 视频 / 文件不能出现在同一条消息里（msg_type 只能取一个）"
                .to_owned(),
        ));
    }

    let mut out = MessageToCreate::default();
    let rendered = render_text(context, message)?;
    // Go：`[CQ:reply]` → `msg_id`（被动回复）。事件 ID 不在计划里，不必与 `event_id` 互斥。
    if let Some(reply) = &message.plan.reply_msg_id {
        out.msg_id = Some(resolve_reply(context, deps.echo, reply)?);
    }

    if let Some(markdown) = compose_markdown(context, &message.plan, &rendered)? {
        // markdown 胜 card，文本让位：msg_type=2 只认 markdown.content
        out.content = None;
        out.msg_type = Some(msg_type::MARKDOWN);
        out.markdown = Some(markdown);
        return Ok(out);
    }
    if let Some(card) = compose_card(&message.plan, &rendered)? {
        // 卡片独占正文：官方要求 msg_type=8 时 content 为空
        out.content = None;
        out.msg_type = Some(msg_type::CARD);
        out.card = Some(card);
        return Ok(out);
    }
    if !rendered.is_empty() {
        out.content = Some(rendered);
    }
    if let Some(media) = compose_media(context, target, openid, &message.plan, deps).await? {
        out.media = Some(media);
        out.msg_type = Some(msg_type::RICH_MEDIA);
    }
    Ok(out)
}

/// 计划里是否有任何媒体段。
fn has_media(plan: &OutboundPlan) -> bool {
    !plan.images.is_empty()
        || !plan.records.is_empty()
        || !plan.videos.is_empty()
        || !plan.files.is_empty()
}

/// `plan.markdown` → markdown 消息体：**只取 `content`**。
///
/// `template_id` / `custom_template_id` 官方已标注废弃（出站不再构造）；载荷里带了也不读，
/// 缺 `content` 直接 `1400`。多份 markdown 取第一份（与 card 口径一致）。
///
/// 组装顺序与 Go 一致：先把文本侧的 @全员 标签迁移进正文（`migrateAtEveryoneToMarkdown`），
/// 再把 `[CQ:at,qq=数字]` 换成 `qqbot-at-user` 标签（`ResolveMarkdownAtMentions`）。
fn compose_markdown(
    context: &ActionContext,
    plan: &OutboundPlan,
    rendered_text: &str,
) -> Result<Option<Markdown>> {
    let Some(payload) = plan.markdown.first() else {
        return Ok(None);
    };
    let value = payload.decode()?;
    let Some(content) = value.get("content").and_then(Value::as_str) else {
        return Err(Error::Protocol(
            "markdown 载荷缺少 content：模板消息已废弃，只支持原生 markdown 内容".to_owned(),
        ));
    };
    let content = migrate_at_everyone(content, rendered_text);
    Ok(Some(Markdown::content(render_markdown(context, &content)?)))
}

/// markdown 抢占正文时，把文本里的 @全员 标签并入 markdown 正文（Go `migrateAtEveryoneToMarkdown`）。
///
/// markdown 胜出时 `content` 会被清空，文本里的标签不迁移就会整条丢失（额度却已经占了）。
/// 正文为空（模板 markdown 没有 `content`）时不塞标签：官方模板消息塞进正文就是非法载荷；
/// 正文已有标签则保持原样，避免重复 @。
fn migrate_at_everyone(content: &str, rendered_text: &str) -> String {
    if content.is_empty()
        || at_everyone::contains_tag(content)
        || !at_everyone::contains_tag(rendered_text)
    {
        return content.to_owned();
    }
    // Go：strings.TrimSpace(content + " " + atEveryoneTag)
    format!("{content} {}", at_everyone::TAG).trim().to_owned()
}

/// markdown 正文里的 `[CQ:at,qq=…]` → 官方 `qqbot-at-user` 标签（Go `ResolveMarkdownAtMentions`）。
///
/// markdown 只认 `<qqbot-at-user id="OpenID" />`，文本那套 `<@openid>` 在 markdown 里不会
/// 渲染成 @。数字虚拟号必须经 idmap 反查到真实 OpenID；反查失败直接 `1400`——Go 这里会原样
/// 留着 CQ 码（等于发一段不可渲染的文本），Rust 侧不静默。
///
/// 只替换 at 段，其余字节原样保留：不重新转义 `[` `]`，否则会破坏 `[文本](链接)` 语法。
fn render_markdown(context: &ActionContext, content: &str) -> Result<String> {
    // 归一后再去重：字面标签与 `[CQ:at,qq=all]` 归一后是同一个标签，留两个会重复 @
    let normalized = at_everyone::dedupe_tag(&at_everyone::normalize(content));
    let Some(first) = at_everyone::find_at_code(&normalized, 0) else {
        return Ok(normalized);
    };
    let mut out = String::with_capacity(normalized.len());
    out.push_str(&normalized[..first]);
    let mut from = first;
    loop {
        let Some(start) = at_everyone::find_at_code(&normalized, from) else {
            out.push_str(&normalized[from..]);
            return Ok(out);
        };
        out.push_str(&normalized[from..start]);
        let after = &normalized[start..];
        let Some(end) = after.find(']') else {
            // 未闭合的 at 段按原文保留（与 normalize 的 malformed 分支一致）
            out.push_str(after);
            return Ok(out);
        };
        let code = &after[..=end];
        let id = at_everyone::at_mention_id(code).ok_or_else(|| {
            Error::Protocol(format!("markdown 里的 {code} 缺少 qq 参数"))
        })?;
        let openid = context.resolve_openid("qq", id, "user_openid")?;
        out.push_str("<qqbot-at-user id=\"");
        out.push_str(&openid);
        out.push_str("\" />");
        from = start + end + 1;
    }
}

/// `plan.card` → 图文卡片：`{type:"tuwen", content:{title,description,url,pic_url}}`。
///
/// 四字段官方校验（title 1~50、description 1~200、url / pic_url https 且 ≤1024）在
/// [`MessageCardContent::validate`]；四项缺一或任一字段非法都 `1400`。
/// 多份 card 只发第一份（与 Go 的第一份语义一致）。
fn compose_card(plan: &OutboundPlan, rendered_text: &str) -> Result<Option<MessageCard>> {
    let Some(payload) = plan.card.first() else {
        return Ok(None);
    };
    // 卡片独占正文（msg_type=8 时 content 必须为空）：@全员 标签无处安放，直接 1400，
    // 而不是清空正文后静默丢标签（还白占当天额度）
    if at_everyone::contains_tag(rendered_text) {
        return Err(Error::Protocol(
            "卡片消息不能同时 @全员：msg_type=8 的正文会被清空，标签无法投递".to_owned(),
        ));
    }
    let fields = payload.fields()?;
    let content = MessageCardContent {
        title: fields.title,
        description: fields.desc,
        url: fields.url,
        pic_url: fields.pic,
    };
    content
        .validate()
        .map_err(|err| Error::Protocol(format!("卡片字段非法（retcode 1400）: {err}")))?;
    Ok(Some(MessageCard::tuwen(content)))
}

/// 文本主体 → v2 `content`：`[CQ:at,qq=…]` 转 `<@openid>`，`qq=all` / `everyone` 转 @全员标签，
/// 其余 CQ 码保持原样。
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
                // 文本消息官方仍在用旧式 `<@openid>`（标注即将弃用但当前可用）；
                // @全员 是另一套标签，`all` / `everyone` 大小写不敏感（Go 同样认 `ALL`）。
                if at_everyone::is_everyone_id(id) {
                    out.push_str(at_everyone::TAG);
                    continue;
                }
                let openid = context.resolve_openid("qq", id, "user_openid")?;
                out.push_str("<@");
                out.push_str(&openid);
                out.push('>');
            }
            _ => out.push_str(&segment.to_cq()),
        }
    }
    // 字面标签与 CQ 写法可能同时出现，收敛成一个
    Ok(at_everyone::dedupe_tag(&out))
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
    let candidates: [(&'static str, bool); 7] = [
        ("keyboard", !plan.keyboard.is_empty()),
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
    use gensokyo_protocol::outbound::{CardFields, CardPayload, MarkdownPayload};
    use gensokyo_protocol::{MediaRef, OutboundPlan};
    use gensokyo_qq::dto::{
        BotInGroupState, FileUpload, GroupInfo, GroupMember, GroupMemberList, JoinRequestList,
        MediaResponse, MessageResponse, MessageToCreate, file_type, msg_type,
    };
    use gensokyo_storage::{EchoStore, IdMapOptions, IdMapStore};
    use tempfile::TempDir;

    use super::{DeferredDeps, execute};
    use crate::at_everyone::SECTION;
    use crate::media::{MAX_MEDIA_BYTES, MediaRuntime};

    /// 测试统一的 32 位 OpenID（`resolve_openid` 对 32 位直通，不需要 idmap）。
    const OPENID: &str = "0123456789ABCDEF0123456789ABCDEF";

    /// @用户 目标的 32 位 OpenID（与收件人区分，验证反查走的是 idmap 而不是直通）。
    const MEMBER_OPENID: &str = "FEDCBA9876543210FEDCBA9876543210";

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
        /// 按顺序消费的发送结果；同一条用例可以排多个（成功 / 失败）。
        send_results: Mutex<Vec<Result<MessageResponse>>>,
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

        fn with_send(self, message_id: &str) -> Self {
            self.push_send(Ok(MessageResponse {
                id: message_id.to_owned(),
                ..MessageResponse::default()
            }))
        }

        fn with_send_error(self, err: Error) -> Self {
            self.push_send(Err(err))
        }

        fn push_send(self, result: Result<MessageResponse>) -> Self {
            self.send_results
                .lock()
                .expect("发送槽锁未中毒")
                .push(result);
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
            let mut results = self.send_results.lock().expect("发送槽锁未中毒");
            assert!(!results.is_empty(), "测试未配置 send 响应");
            results.remove(0)
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

    /// 临时 idmap（redb + bbolt 同构的身份库），返回值里的 `TempDir` 必须一直持有。
    fn idmap_store() -> (TempDir, Arc<IdMapStore>) {
        let dir = tempfile::tempdir().expect("临时目录");
        let options = IdMapOptions::new(
            dir.path().join("openid-map.redb"),
            dir.path().join("msgid-map.redb"),
        )
        .with_app_id("10001");
        let store = IdMapStore::open(options).expect("打开 idmap 存储失败");
        (dir, Arc::new(store))
    }

    /// 带自定义 `settings` 段的配置（每行已按两格缩进写成 YAML）。
    fn manager_with(settings: &str) -> (TempDir, Arc<ConfigManager>) {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, format!("version: 1\nsettings:\n{settings}"))
            .expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        (dir, manager)
    }

    fn context_with_settings(api: Arc<FakeQqApi>, settings: &str) -> (TempDir, ActionContext) {
        let (dir, manager) = manager_with(settings);
        (dir, ActionContext::new(api, manager))
    }

    /// 四字段齐备的合法卡片计划。
    fn card_plan() -> OutboundPlan {
        OutboundPlan {
            card: vec![CardPayload::from_fields(&CardFields {
                title: Some("标题".to_owned()),
                desc: Some("描述".to_owned()),
                pic: Some("https://example.com/a.png".to_owned()),
                url: Some("https://example.com/a".to_owned()),
            })],
            ..OutboundPlan::default()
        }
    }

    fn text_plan(target: SendTarget, text: &str) -> SendRequest {
        let mut request = request(target, OutboundPlan::default());
        request.message.text = text.to_owned();
        request
    }

    /// deferred 用不到的传输：纯文本 / markdown / card 路径不下载任何东西。
    fn no_media() -> MediaRuntime {
        MediaRuntime::with_transport(Arc::new(FakeTransport::new(b"")))
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

    // ---- markdown ----

    #[tokio::test]
    async fn markdown_plan_sends_msg_type_two_with_content_only() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = request(
            SendTarget::Group(OPENID.to_owned()),
            OutboundPlan {
                markdown: vec![MarkdownPayload::from_json(
                    r##"{"content":"# 标题","custom_template_id":"TPL","params":[{"key":"a","values":["b"]}]}"##,
                )
                .expect("markdown 载荷")],
                ..OutboundPlan::default()
            },
        );

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("markdown 应发送成功");

        let sent = api.sent();
        assert_eq!(sent[0].msg_type, Some(msg_type::MARKDOWN));
        assert_eq!(
            sent[0]
                .markdown
                .as_ref()
                .and_then(|markdown| markdown.content.as_deref()),
            Some("# 标题")
        );
        assert_eq!(sent[0].content, None, "markdown 消息的 content 必须为空");

        // 出站 JSON 只有 content：模板字段已废弃，不构造也不透传
        let value = serde_json::to_value(&sent[0]).expect("序列化失败");
        assert_eq!(value["markdown"], serde_json::json!({"content": "# 标题"}));
    }

    #[tokio::test]
    async fn markdown_without_content_is_a_request_error() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = request(
            SendTarget::Group(OPENID.to_owned()),
            OutboundPlan {
                markdown: vec![
                    MarkdownPayload::from_json(r#"{"custom_template_id":"TPL"}"#)
                        .expect("markdown 载荷"),
                ],
                ..OutboundPlan::default()
            },
        );

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("缺 content 必须拒绝");
        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
        assert!(api.sent().is_empty(), "非法 markdown 不应发送");
    }

    // ---- card ----

    #[tokio::test]
    async fn card_plan_sends_msg_type_eight_for_group_and_private() {
        for target in [
            SendTarget::Group(OPENID.to_owned()),
            SendTarget::Private(OPENID.to_owned()),
        ] {
            let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
            let (_dir, context) = context(Arc::clone(&api));
            let media = no_media();
            let plan = request(target, card_plan());

            execute(&context, &plan, &deps(&media, None))
                .await
                .expect("卡片应发送成功");

            let sent = api.sent();
            assert_eq!(sent[0].msg_type, Some(msg_type::CARD), "卡片必须是 msg_type=8");
            assert_eq!(sent[0].content, None, "卡片消息的 content 必须为空");
            let card = sent[0].card.as_ref().expect("card 必须有值");
            assert_eq!(card.card_type, "tuwen");
            assert_eq!(card.content.title.as_deref(), Some("标题"));
            assert_eq!(
                card.content.pic_url.as_deref(),
                Some("https://example.com/a.png")
            );
        }
    }

    #[tokio::test]
    async fn card_without_title_is_a_request_error() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = request(
            SendTarget::Group(OPENID.to_owned()),
            OutboundPlan {
                card: vec![CardPayload::from_fields(&CardFields {
                    desc: Some("有描述没标题".to_owned()),
                    pic: Some("https://example.com/a.png".to_owned()),
                    url: Some("https://example.com/a".to_owned()),
                    ..CardFields::default()
                })],
                ..OutboundPlan::default()
            },
        );

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("缺 title 必须拒绝");
        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
        assert!(api.sent().is_empty(), "非法卡片不应发送");
    }

    #[tokio::test]
    async fn markdown_wins_over_card() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = request(
            SendTarget::Group(OPENID.to_owned()),
            OutboundPlan {
                markdown: vec![
                    MarkdownPayload::from_json(r##"{"content":"# 标题"}"##).expect("markdown 载荷"),
                ],
                card: vec![CardPayload::from_fields(&CardFields {
                    title: Some("卡片标题".to_owned()),
                    ..CardFields::default()
                })],
                ..OutboundPlan::default()
            },
        );

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("markdown 与 card 同现应发送");

        let sent = api.sent();
        assert_eq!(sent[0].msg_type, Some(msg_type::MARKDOWN), "markdown 胜 card");
        assert!(sent[0].card.is_none(), "card 必须让位");
    }

    #[tokio::test]
    async fn markdown_with_media_is_a_request_error() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = request(
            SendTarget::Group(OPENID.to_owned()),
            OutboundPlan {
                markdown: vec![
                    MarkdownPayload::from_json(r##"{"content":"# 标题"}"##).expect("markdown 载荷"),
                ],
                images: vec![MediaRef::Url("https://example.com/a.png".to_owned())],
                ..OutboundPlan::default()
            },
        );

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("markdown 与媒体同现必须拒绝");
        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
    }

    // ---- @全员 ----

    #[tokio::test]
    async fn at_everyone_cq_is_rendered_as_tag_and_reserves_quota() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), "[CQ:at,qq=all] 开会");

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("@全员 应发送成功");

        let sent = api.sent();
        assert_eq!(
            sent[0].content.as_deref(),
            Some("<qqbot-at-everyone /> 开会"),
            "CQ 段必须归一成官方标签，且不留 CQ 残留"
        );
        assert!(
            idmap.read_config(SECTION, OPENID).is_ok(),
            "发送成功应留下当天计数"
        );
    }

    #[tokio::test]
    async fn at_everyone_literal_tag_is_also_accepted() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = text_plan(
            SendTarget::Group(OPENID.to_owned()),
            "<qqbot-at-everyone /> 开会",
        );

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("字面标签应放行");

        assert_eq!(
            api.sent()[0].content.as_deref(),
            Some("<qqbot-at-everyone /> 开会")
        );
        assert!(idmap.read_config(SECTION, OPENID).is_ok(), "字面标签也要计数");
    }

    #[tokio::test]
    async fn at_everyone_in_private_is_a_request_error() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = text_plan(
            SendTarget::Private(OPENID.to_owned()),
            "<qqbot-at-everyone /> 开会",
        );

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("单聊 @全员 必须拒绝");
        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
        assert!(api.sent().is_empty(), "单聊 @全员 不应发送");
    }

    #[tokio::test]
    async fn second_at_everyone_in_same_group_same_day_is_rejected() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), "[CQ:at,qq=all]开会");

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("首次应放行");
        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("同日第二次必须拒绝");

        assert!(matches!(err, Error::Protocol(_)), "超限应映射 1400: {err}");
        assert_eq!(api.sent().len(), 1, "超限的第二次不应发送");
    }

    #[tokio::test]
    async fn at_everyone_limit_switch_off_allows_repeats_without_counting() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1").with_send("MSG-2"));
        let (_dir, context) =
            context_with_settings(Arc::clone(&api), "  enable_at_everyone_limit: false\n");
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), "[CQ:at,qq=all]开会");

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("开关关闭时首次应放行");
        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("开关关闭时同日第二次也应放行");

        assert_eq!(api.sent().len(), 2, "开关关闭时不应拦截");
        assert!(
            idmap.read_config(SECTION, OPENID).is_err(),
            "开关关闭时不应写计数"
        );
    }

    #[tokio::test]
    async fn send_failure_rolls_back_the_at_everyone_quota() {
        let api = Arc::new(
            FakeQqApi::default().with_send_error(Error::Network("上游挂了".to_owned())),
        );
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), "[CQ:at,qq=all]开会");

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("上游失败应返回错误");
        assert!(matches!(err, Error::Network(_)), "发送失败应映射 1500: {err}");

        assert!(
            idmap.read_config(SECTION, OPENID).is_err(),
            "发送失败必须回滚计数"
        );
        crate::at_everyone::reserve(&idmap, OPENID).expect("回滚后应能再次占位");
    }

    #[tokio::test]
    async fn at_everyone_tag_inside_markdown_content_is_reserved_and_kept() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = request(
            SendTarget::Group(OPENID.to_owned()),
            OutboundPlan {
                markdown: vec![
                    MarkdownPayload::from_json(r#"{"content":"[CQ:at,qq=all] 早安"}"#)
                        .expect("markdown 载荷"),
                ],
                ..OutboundPlan::default()
            },
        );

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("markdown @全员 应发送成功");

        let sent = api.sent();
        assert_eq!(sent[0].msg_type, Some(msg_type::MARKDOWN));
        assert_eq!(sent[0].content, None, "markdown 消息的 content 为空");
        assert_eq!(
            sent[0]
                .markdown
                .as_ref()
                .and_then(|markdown| markdown.content.as_deref()),
            Some("<qqbot-at-everyone /> 早安")
        );
        assert!(idmap.read_config(SECTION, OPENID).is_ok(), "markdown 也要计数");
    }

    #[tokio::test]
    async fn duplicate_at_everyone_tags_collapse_to_one() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        // 正文同时带字面标签与 `[CQ:at,qq=all]`，文本侧还有一个：归一后只能留一个
        let mut plan = text_plan(SendTarget::Group(OPENID.to_owned()), "[CQ:at,qq=all] 开会");
        plan.message.plan = markdown_plan("<qqbot-at-everyone /> 议题 [CQ:at,qq=all]");

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("重复标签应归一后发送");

        let content = api.sent()[0]
            .markdown
            .as_ref()
            .and_then(|markdown| markdown.content.clone())
            .expect("markdown 正文");
        assert_eq!(
            content.matches("<qqbot-at-everyone />").count(),
            1,
            "同一份正文只能有一个 @全员 标签: {content}"
        );
        assert_eq!(content.trim_end(), "<qqbot-at-everyone /> 议题");

        // 额度只按一条消息计一次：同群同日第二条必须被限流拒绝
        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("同日第二条必须拒绝");
        assert!(matches!(err, Error::Protocol(_)), "超限应映射 1400: {err}");
    }

    #[tokio::test]
    async fn uppercase_cq_at_all_is_treated_as_everyone() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), "[CQ:at,qq=ALL] 开会");

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("qq=ALL 应归一为 @全员 标签，而不是走 idmap 反查");

        assert_eq!(
            api.sent()[0].content.as_deref(),
            Some("<qqbot-at-everyone /> 开会")
        );
        assert!(
            idmap.read_config(SECTION, OPENID).is_ok(),
            "qq=ALL 同样要占当天额度"
        );
    }

    // ---- markdown @用户 / 文本+markdown 的 @全员 迁移 / 卡片冲突 ----

    /// 用 `content` 构造一份 markdown 计划（json 编码由 serde 负责，避免手写转义）。
    fn markdown_plan(content: &str) -> OutboundPlan {
        OutboundPlan {
            markdown: vec![
                MarkdownPayload::from_json(&serde_json::json!({ "content": content }).to_string())
                    .expect("markdown 载荷"),
            ],
            ..OutboundPlan::default()
        }
    }

    #[tokio::test]
    async fn markdown_at_mention_becomes_qqbot_at_user() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let (_, vuin) = idmap
            .retrieve_virtual_value(MEMBER_OPENID)
            .expect("登记身份失败");
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = request(
            SendTarget::Group(OPENID.to_owned()),
            markdown_plan(&format!(
                "[CQ:at,qq={vuin}] 你好 [文档](https://example.com/a)"
            )),
        );

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("markdown @用户 应发送成功");

        let sent = api.sent();
        assert_eq!(sent[0].msg_type, Some(msg_type::MARKDOWN));
        let content = sent[0]
            .markdown
            .as_ref()
            .and_then(|markdown| markdown.content.as_deref())
            .expect("markdown 必须有正文");
        assert_eq!(
            content,
            format!("<qqbot-at-user id=\"{MEMBER_OPENID}\" /> 你好 [文档](https://example.com/a)"),
            "数字虚拟号必须反查成官方 @ 标签，且不破坏 markdown 链接语法"
        );
        assert!(
            !content.contains("[CQ:at,"),
            "markdown 正文不得残留 CQ 码: {content}"
        );
    }

    #[tokio::test]
    async fn markdown_at_mention_resolution_failure_is_a_request_error() {
        // 未装配 idmap：数字虚拟号无法反查 → 1400，不静默留 CQ 码
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = request(
            SendTarget::Group(OPENID.to_owned()),
            markdown_plan("[CQ:at,qq=123456] 你好"),
        );

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("反查失败必须拒绝");
        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
        assert!(api.sent().is_empty(), "反查失败不应发送");
    }

    #[tokio::test]
    async fn text_at_everyone_migrates_into_markdown_content() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let mut plan = text_plan(SendTarget::Group(OPENID.to_owned()), "[CQ:at,qq=all] 开会");
        plan.message.plan = markdown_plan("# 今天的议题");

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("文本 + markdown 应发送成功");

        let sent = api.sent();
        assert_eq!(sent[0].msg_type, Some(msg_type::MARKDOWN));
        assert_eq!(sent[0].content, None, "markdown 消息的 content 必须为空");
        assert_eq!(
            sent[0]
                .markdown
                .as_ref()
                .and_then(|markdown| markdown.content.as_deref()),
            Some("# 今天的议题 <qqbot-at-everyone />"),
            "文本里的 @全员 必须迁移进 markdown 正文"
        );
        assert!(
            idmap.read_config(SECTION, OPENID).is_ok(),
            "迁移后的标签同样要占当天额度"
        );
    }

    #[test]
    fn migrate_at_everyone_matches_go_semantics() {
        assert_eq!(
            super::migrate_at_everyone("# 议题", "<qqbot-at-everyone /> 开会"),
            "# 议题 <qqbot-at-everyone />"
        );
        assert_eq!(
            super::migrate_at_everyone("# 议题\n", "<qqbot-at-everyone />"),
            "# 议题\n <qqbot-at-everyone />",
            "Go 只做首尾 TrimSpace，正文内部换行保留"
        );
        assert_eq!(
            super::migrate_at_everyone("# 已有 <qqbot-at-everyone />", "<qqbot-at-everyone />"),
            "# 已有 <qqbot-at-everyone />",
            "正文已有标签不得重复追加"
        );
        assert_eq!(
            super::migrate_at_everyone("", "<qqbot-at-everyone />"),
            "",
            "模板 markdown 没有正文，塞标签会产出非法载荷"
        );
        assert_eq!(
            super::migrate_at_everyone("# 议题", "没有标签"),
            "# 议题"
        );
    }

    #[tokio::test]
    async fn card_with_at_everyone_is_a_request_error() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let mut plan = request(SendTarget::Group(OPENID.to_owned()), card_plan());
        plan.message.text = "[CQ:at,qq=all] 开会".to_owned();

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("卡片 + @全员 必须拒绝");
        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
        assert!(api.sent().is_empty(), "冲突消息不应发送");
    }
}
