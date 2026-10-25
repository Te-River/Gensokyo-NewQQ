//! deferred 计划的执行：`SendRequest` → `MessageToCreate` → QQ 上游。
//!
//! `gensokyo-actions` 的 `send_*` handler 只产出计划（`data.deferred == true`），本模块负责把
//! 计划变成真实发送：[`Dispatcher`](crate::dispatcher::Dispatcher) 反序列化出
//! [`SendRequest`] 后调用 [`execute`]——收件人还原成上游 OpenID、消息合成
//! [`MessageToCreate`]、再按 target 分派到 [`QqApi::send_group_message`] /
//! [`QqApi::send_c2c_message`](gensokyo_actions::QqApi::send_c2c_message)。
//!
//! # 支持范围
//! - **文本**：`MessageOut.text` 按 CQ 重新分词，`[CQ:at,qq=…]` 转成 v2 的 `<@openid>`（虚拟号经
//!   idmap 反查）；`auto_escape = true` 时原样发送、不解析。
//! - **图片**：只接受已经是上游 `file_info` 的形态（[`MediaRef::Unknown`]），单张图片落到
//!   `MessageToCreate.media`。URL / 本地路径 / base64 需要先走媒体上传，本阶段直接报错，
//!   不做静默丢弃。
//! - **其余段**（record / video / file / markdown / keyboard / card / music / stream /
//!   input_notify / 未知段）：[`Error::Unsupported`]；`reply` 段需要真实消息 ID（echo 回写链路
//!   未接线），本阶段只记 `debug` 日志。
//!
//! 回执：成功返回上游 `message_id`；失败由 dispatcher 统一映射成 `retcode 1500`。

use gensokyo_actions::{ActionContext, MessageOut, SendRequest, SendTarget};
use gensokyo_core::{Error, Result};
use gensokyo_protocol::cqcode;
use gensokyo_protocol::{MediaRef, OutboundPlan};
use gensokyo_qq::dto::{Media, MessageToCreate};
use serde_json::Value;

/// handler 响应 `data` 里的 deferred 标记（与 `gensokyo_actions::DEFERRED_FIELD` 同值）。
pub const DEFERRED_FIELD: &str = gensokyo_actions::DEFERRED_FIELD;

/// `data` 是否是一份 deferred 计划（`{"deferred": true, …}`）。
#[must_use]
pub fn is_deferred(data: &Value) -> bool {
    data.get(DEFERRED_FIELD).and_then(Value::as_bool) == Some(true)
}

/// 执行一条 deferred 计划，返回上游消息 ID。
///
/// # Errors
/// 收件人无法反查成 OpenID（[`ActionContext::resolve_send_target`]）、消息含本阶段不支持的
/// 消息段，或上游发送失败（[`Error::Upstream`] / [`Error::Network`]）。
pub async fn execute(context: &ActionContext, plan: &SendRequest) -> Result<String> {
    let message = compose(context, &plan.message)?;
    let openid = context.resolve_send_target(&plan.target)?;
    let response = match &plan.target {
        SendTarget::Group(_) => context.api().send_group_message(openid, message).await?,
        SendTarget::Private(_) => context.api().send_c2c_message(openid, message).await?,
    };
    Ok(response.id)
}

/// 把一条待发送消息合成 [`MessageToCreate`]（文本 + `at` + 单个 media）。
///
/// # Errors
/// 文本里的 `[CQ:at]` 缺 `qq`、`at` 虚拟号无法反查，或消息含本阶段不支持的消息段。
pub fn compose(context: &ActionContext, message: &MessageOut) -> Result<MessageToCreate> {
    if let Some(reply) = &message.plan.reply_msg_id {
        // 引用需要真实消息 ID；echo 回写未接线，先不阻断发送，只在日志里留痕。
        tracing::debug!(reply = %reply, "deferred 发送忽略 reply 段（echo 回写未接线）");
    }
    if let Some(kind) = unsupported_segment(&message.plan) {
        return Err(Error::Unsupported(format!(
            "deferred 发送暂不支持 {kind} 段：{message:?}"
        )));
    }

    let mut out = MessageToCreate::default();
    let content = render_text(context, message)?;
    if !content.is_empty() {
        out.content = Some(content);
    }
    if let Some(media) = compose_media(&message.plan)? {
        out.media = Some(media);
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

/// 图片 → `MessageToCreate.media`。
///
/// v2 文本消息只承载一个 `media`，因此多图同样报错而不是丢掉后面的图。
fn compose_media(plan: &OutboundPlan) -> Result<Option<Media>> {
    let Some(image) = plan.images.first() else {
        return Ok(None);
    };
    if plan.images.len() > 1 {
        return Err(Error::Unsupported(format!(
            "QQ v2 单条消息只支持一个 media（收到 {} 张图片）",
            plan.images.len()
        )));
    }
    match image {
        MediaRef::Unknown(file_info) => Ok(Some(Media {
            file_info: file_info.clone(),
        })),
        other => Err(Error::Unsupported(format!(
            "图片 {} 需先上传（媒体上传链路未接线）",
            other.to_wire()
        ))),
    }
}

/// 本阶段不支持的消息段；返回第一个命中的段名。
fn unsupported_segment(plan: &OutboundPlan) -> Option<&'static str> {
    let candidates: [(&'static str, bool); 12] = [
        ("record", !plan.records.is_empty()),
        ("video", !plan.videos.is_empty()),
        ("file", !plan.files.is_empty()),
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
