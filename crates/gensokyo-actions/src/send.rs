//! 文本类出站 action 的纯规划层：params → [`SendRequest`]，不做任何网络调用。
//!
//! # 与 Go 的分工
//!
//! Go 侧的 `handlers/send_group_msg.go` / `send_private_msg.go` / `send_msg.go` 把
//! 「解析参数 → 上传媒体 → 调 OpenAPI 发送 → 回写 echo / 懒消息表」全挤在一个函数里，
//! 离开网络就无法单测。本模块只做第一步：
//!
//! | 阶段 | Go | 本 crate |
//! |------|----|----------|
//! | 参数解析（`group_id` / `user_id` / `message` / `auto_escape`） | handler 内联 | [`plan_send_group_msg`] / [`plan_send_private_msg`] / [`plan_send_msg`] |
//! | CQ / 消息段 → `messageText` + `foundItems` | `parseMessageContent` | [`MessageOut`]（文本 + [`OutboundPlan`]） |
//! | 媒体上传（图床 / 群文件 / C2C）+ `MessageToCreate` + 发送 | handler 内联 | **不做**，由 `gensokyo-runtime` 接线 |
//!
//! 文本与结构化意图的分工与 Go 的 `parseMessageContent` 返回值一致：文本单独返回，
//! `foundItems` 只承载 reply / 媒体等；因此 [`MessageOut::plan`] 的 `text` 恒为空。
//!
//! # 计划的生命周期
//!
//! [`crate::handler`] 成功时把 [`SendRequest`] 序列化进响应 `data` 并附 `"deferred": true`。
//! 该 `data` 只是 handler → runtime 的内部握手载荷：runtime 注册这些 action 后，用
//! `serde_json::from_value::<SendRequest>(data)` 还原计划并接管真正的发送（媒体上传 →
//! `MessageToCreate` → echo / 懒消息回写），最终回给 OneBot 客户端的是发送结果，
//! **不会把计划本身直接回给客户端**。反序列化只读 `target` / `message`，`deferred` 等未知字段
//! 被忽略；[`MessageOut::plan`] 的段数组按 [`OutboundPlan::from_segments`] 还原，段里的文本 /
//! `at` 折回 [`MessageOut::text`]（见 [`MessageOut::from_parts`]）。echo 不进计划：由 handler
//! 在响应层原样回传。
//!
//! # 与协议类型（`gensokyo_protocol::action`）的关系
//!
//! 参数最终由 `SendGroupMsgParams` / `SendPrivateMsgParams` 反序列化，`message` 的两种形态、
//! `auto_escape` 的类型、未知字段落 `extra` 等语义与协议层同源，避免与 M1j 漂移。刻意的差异：
//!
//! - **浮点 ID**：协议层 `Ob11Id` 不接受浮点；这里先把整值浮点（`1e3` / `1000.0`）规范成整数
//!   （`1000`），非整值（`1.5`）拒绝——兼容把 ID 发成浮点数的客户端。
//! - **单个消息段对象**：`message` 除 CQ 字符串 / 段数组外，还接受单个段对象
//!   （`{"type":"text","data":{"text":"x"}}`），包成单元素数组后交给协议类型。
//! - **`send_msg` 的分派**：`SendMsgParams.message_type` 用 `Other` 兜底，会吞掉 `guild` 等非法值；
//!   因此 `message_type` 仍手写分派以保持 Go 的拒绝语义，其余字段照旧走协议类型。
//! - **guild**：`guild_id` / `channel_id` 不在协议 send 参数里，本模块不支持（返回参数错误）。
//!
//! # 与 Go 的口径差异
//!
//! - **`auto_escape`**：Go 的 `handlers/` 全仓不读该字段（`send_*_msg.go` 始终按 CQ 解析）；
//!   这里在字符串形态下尊重它（`true` 时原样保留 CQ 字面量），数组形态无语义、字段记为 `false`。
//! - **`send_group_msg` 的 `user_id`（群临时会话）**：仅 `user_id` 时规划成 [`SendTarget::Private`]
//!   （Go 的 `group_private` 分支最终也调 `HandleSendPrivateMsg`）；`group_id` 与 `user_id` 同时
//!   出现时以 `group_id` 为准。Go 会按 32 位 OpenID 形态判定优先级，那需要 idmap，留给 runtime。
//!
//! # 未做项（M1i 明确不做）
//!
//! - 媒体上传：本地文件 → 图床 / 群 / C2C 上传、语音转 silk、`file_info` 回填。
//! - SSM（`code:22009` 失败入队）与主动消息（`messageID == "2000"`、
//!   `send_private_msg_wakeup`）。
//! - idmap 虚拟 ID ↔ OpenID 映射、echo 懒消息表：都需要存储，留给 runtime。
//! - `send_private_msg` 的 `group_id`（群临时会话的群上下文）：协议类型能解析，但计划只按
//!   `user_id` 规划、不落 [`SendTarget`]，群上下文由 runtime 决定。

use serde::de::DeserializeOwned;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

use gensokyo_core::{Error, Result};
use gensokyo_protocol::action::{SendGroupMsgParams, SendPrivateMsgParams};
use gensokyo_protocol::{MessageContent, OutboundPlan, Segment};

/// 出站消息的收件人。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendTarget {
    /// 群聊：虚拟群 ID 的文本形态（数字 ID 或 32 位 OpenID）。
    Group(String),
    /// 私聊 / C2C（含 `send_group_msg` 的群临时会话）：虚拟用户 ID 的文本形态。
    Private(String),
}

impl SendTarget {
    /// 目标类型（`"group"` / `"private"`），用于日志与指标标签。
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Group(_) => "group",
            Self::Private(_) => "private",
        }
    }

    /// 收件人 ID 文本。
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Group(id) | Self::Private(id) => id,
        }
    }
}

/// 一条待发送消息：文本主体 + 结构化出站意图。
///
/// [`Self::text`] 与 [`Self::plan`] 的分工见模块文档：文本单独持有，`plan.text` 恒为空，
/// 所以 [`Self::to_segments`] 只输出非文本段。
#[derive(Debug, Clone, PartialEq)]
pub struct MessageOut {
    /// 文本主体（保留文本内 CQ 码字面量，如 `[CQ:at,qq=…]`，与 Go 的 `messageText` 一致）。
    pub text: String,
    /// 文本以外的出站意图（reply / media / markdown / …）；`plan.text` 恒为空。
    pub plan: OutboundPlan,
    /// 是否按纯文本发送（`auto_escape = true`，未解析 CQ 码）。
    pub auto_escape: bool,
}

impl MessageOut {
    /// 由协议层消息内容构造。
    ///
    /// - 字符串形态：`auto_escape = false` 时按 CQ 解析（`[CQ:at]` 字面量留在文本里，
    ///   `[CQ:image]` / `[CQ:reply]` 等落进 `plan`）；`auto_escape = true` 时原样保留、不解析。
    /// - 数组形态：段已是结构化数据，`auto_escape` 无语义，忽略（字段记为 `false`）。
    #[must_use]
    pub fn from_content(content: &MessageContent, auto_escape: bool) -> Self {
        if auto_escape && !content.is_array() {
            return Self {
                text: content.as_str().unwrap_or_default().to_owned(),
                plan: OutboundPlan::default(),
                auto_escape: true,
            };
        }
        let mut plan = OutboundPlan::from_segments(&content.segments());
        let text = std::mem::take(&mut plan.text);
        Self {
            text,
            plan,
            auto_escape: false,
        }
    }

    /// 由 `text` + 非文本段 + `auto_escape` 还原（[`Deserialize`] 与 runtime 手工构造共用）。
    ///
    /// 与 [`Self::from_content`] 同口径：段里的 `text` / `at` 折回 [`Self::text`]（追加在正文
    /// 末尾，段序内保持原样），[`Self::plan`] 的 `text` 保持为空。
    #[must_use]
    pub fn from_parts(text: impl Into<String>, segments: &[Segment], auto_escape: bool) -> Self {
        let mut plan = OutboundPlan::from_segments(segments);
        let mut text = text.into();
        text.push_str(&std::mem::take(&mut plan.text));
        Self {
            text,
            plan,
            auto_escape,
        }
    }

    /// 展开为非文本消息段（序列化 / 交给 runtime 转发时使用）。
    #[must_use]
    pub fn to_segments(&self) -> Vec<Segment> {
        self.plan.to_segments()
    }
}

impl Serialize for MessageOut {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("MessageOut", 3)?;
        state.serialize_field("text", &self.text)?;
        state.serialize_field("auto_escape", &self.auto_escape)?;
        state.serialize_field("plan", &self.plan.to_segments())?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for MessageOut {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        /// 线上形态：`text` 单独持有，`plan` 是非文本段数组（与 [`Serialize`] 对称）。
        #[derive(Deserialize)]
        struct Wire {
            #[serde(default)]
            text: String,
            #[serde(default)]
            auto_escape: bool,
            #[serde(default)]
            plan: Vec<Segment>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Ok(Self::from_parts(wire.text, &wire.plan, wire.auto_escape))
    }
}

/// 规划结果：收件人 + 消息，供 runtime 转 `MessageToCreate`。
///
/// 同时实现 `Serialize` / `Deserialize`：handler 把计划序列化进响应 `data`（附
/// `"deferred": true`），runtime 注册后用 `serde_json::from_value::<SendRequest>(data)` 还原
/// （`deferred` 等未知字段被忽略）。echo 不进计划，由 [`crate::handler`] 在响应层原样回传。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SendRequest {
    /// 收件人。
    pub target: SendTarget,
    /// 消息内容。
    pub message: MessageOut,
}

/// `send_group_msg`（含别名 `send_to_group`）的参数规划。
///
/// # Errors
/// `params` 不是对象、缺 `group_id` 与 `user_id`、ID 为 `null` / 空串 / `0` / 非数字非字符串、
/// `message` 缺失或为空、`auto_escape` 非布尔、`message` 既不是 CQ 字符串 / 消息段数组 / 单个
/// 消息段对象时返回 [`Error::Protocol`]。
pub fn plan_send_group_msg(params: &Value) -> Result<SendRequest> {
    let object = params_object(params)?;
    require_message(object)?;
    let auto_escape = auto_escape_flag(object)?;

    if let Some(group_id) = id_field(object, "group_id")? {
        let parsed: SendGroupMsgParams =
            deserialize_params(normalized_params(object, &[("group_id", group_id)])?)?;
        return Ok(SendRequest {
            target: SendTarget::Group(parsed.group_id.to_string()),
            message: message_out(&parsed.message, auto_escape)?,
        });
    }

    // 群临时会话：Go 的 `group_private` 分支最终也走 HandleSendPrivateMsg
    if let Some(user_id) = id_field(object, "user_id")? {
        let parsed: SendPrivateMsgParams =
            deserialize_params(normalized_params(object, &[("user_id", user_id)])?)?;
        return Ok(SendRequest {
            target: SendTarget::Private(parsed.user_id.to_string()),
            message: message_out(&parsed.message, auto_escape)?,
        });
    }

    Err(Error::Protocol(
        "缺少 group_id（群聊）或 user_id（群临时会话）".to_owned(),
    ))
}

/// `send_private_msg`（含 `send_private_msg_wakeup` 的规划部分）的参数规划。
///
/// # Errors
/// 同 [`plan_send_group_msg`]，ID 字段换成 `user_id`（`group_id` 不参与规划）。
pub fn plan_send_private_msg(params: &Value) -> Result<SendRequest> {
    let object = params_object(params)?;
    require_message(object)?;
    let auto_escape = auto_escape_flag(object)?;

    let Some(user_id) = id_field(object, "user_id")? else {
        return Err(Error::Protocol("缺少 user_id".to_owned()));
    };
    let parsed: SendPrivateMsgParams =
        deserialize_params(normalized_params(object, &[("user_id", user_id)])?)?;
    Ok(SendRequest {
        target: SendTarget::Private(parsed.user_id.to_string()),
        message: message_out(&parsed.message, auto_escape)?,
    })
}

/// `send_msg` 的参数规划：按 `message_type` 分派到群聊 / 私聊。
///
/// # Errors
/// `message_type` 缺失、非字符串、或不是 `group` / `private` 时返回 [`Error::Protocol`]；
/// 其余校验同 [`plan_send_group_msg`] / [`plan_send_private_msg`]。
pub fn plan_send_msg(params: &Value) -> Result<SendRequest> {
    let object = params_object(params)?;
    match object.get("message_type") {
        None | Some(Value::Null) => Err(Error::Protocol("缺少 message_type".to_owned())),
        Some(Value::String(kind)) => match kind.as_str() {
            "group" => plan_send_group_msg(params),
            "private" => plan_send_private_msg(params),
            other => Err(Error::Protocol(format!("message_type 非法: {other}"))),
        },
        Some(_) => Err(Error::Protocol("message_type 必须是字符串".to_owned())),
    }
}

fn params_object(params: &Value) -> Result<&Map<String, Value>> {
    params
        .as_object()
        .ok_or_else(|| Error::Protocol("params 缺失或不是 JSON 对象".to_owned()))
}

fn require_message(object: &Map<String, Value>) -> Result<()> {
    match object.get("message") {
        None | Some(Value::Null) => Err(Error::Protocol("缺少 message".to_owned())),
        Some(_) => Ok(()),
    }
}

/// `auto_escape` 必须是布尔值。
///
/// 协议类型的 serde 报错不带字段名，这里先校验一次，保证错误信息能定位到字段；
/// 取值与 `SendGroupMsgParams.auto_escape` / `SendPrivateMsgParams.auto_escape` 同义。
fn auto_escape_flag(object: &Map<String, Value>) -> Result<bool> {
    match object.get("auto_escape") {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(flag)) => Ok(*flag),
        Some(_) => Err(Error::Protocol("auto_escape 必须是布尔值".to_owned())),
    }
}

/// 读取 ID 字段：`None` 表示缺失 / `null` / 空串；`0` 非法（对齐 Go 的 `checkZero*`）。
///
/// 返回值是规范化后的 JSON 值（数字 → 整数，字符串原样），供协议类型反序列化。
fn id_field(object: &Map<String, Value>, key: &str) -> Result<Option<Value>> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.is_empty() => Ok(None),
        Some(Value::String(text)) => {
            if text == "0" {
                return Err(Error::Protocol(format!("{key} 不能为 0")));
            }
            Ok(Some(Value::String(text.clone())))
        }
        Some(Value::Number(number)) => {
            let normalized = normalized_number(key, number)?;
            if normalized.as_i64() == Some(0) {
                return Err(Error::Protocol(format!("{key} 不能为 0")));
            }
            Ok(Some(Value::Number(normalized)))
        }
        Some(_) => Err(Error::Protocol(format!("{key} 必须是数字或字符串"))),
    }
}

/// 数字 ID → 整数。
///
/// `Number::to_string()` 会把 `1e3` / `1000.0` 渲染成 `"1000.0"`；这里整数优先，浮点仅在
/// 整值且在 `i64` 范围内时折叠（`1e3` → `1000`），非整值 / 越界拒绝。
fn normalized_number(key: &str, number: &Number) -> Result<Number> {
    if let Some(value) = number.as_i64() {
        return Ok(Number::from(value));
    }
    if let Some(value) = number.as_u64() {
        return Ok(Number::from(value));
    }
    let value = number
        .as_f64()
        .ok_or_else(|| Error::Protocol(format!("{key} 不是有效数字")))?;
    let in_range = (i64::MIN as f64..=i64::MAX as f64).contains(&value);
    if value.fract() != 0.0 || !in_range {
        return Err(Error::Protocol(format!("{key} 必须是整数: {number}")));
    }
    Ok(Number::from(value as i64))
}

/// 把 `params` 规范成协议类型可接受的 JSON：覆盖已规范化的 ID 字段，单个消息段对象包成数组。
fn normalized_params(
    object: &Map<String, Value>,
    ids: &[(&str, Value)],
) -> Result<Map<String, Value>> {
    let mut normalized = object.clone();
    for (key, value) in ids {
        normalized.insert((*key).to_owned(), value.clone());
    }
    // 另一个 ID 字段（如 `send_private_msg` 的 `group_id`）也走同一套浮点折叠，
    // 否则协议类型会抛出看不懂的 serde 报错
    for key in ["group_id", "user_id"] {
        if let Some(Value::Number(number)) = normalized.get(key).cloned() {
            normalized.insert(
                key.to_owned(),
                Value::Number(normalized_number(key, &number)?),
            );
        }
    }

    // `MessageContent` 只认字符串 / 数组；单个段对象按单元素数组处理
    if let Some(raw @ Value::Object(_)) = object.get("message") {
        let segment: Segment = serde_json::from_value(raw.clone()).map_err(|err| {
            Error::Protocol(format!(
                "message 必须是 CQ 字符串、消息段数组或单个消息段: {err}"
            ))
        })?;
        let wrapped = serde_json::to_value([segment])
            .map_err(|err| Error::Protocol(format!("message 消息段序列化失败: {err}")))?;
        normalized.insert("message".to_owned(), wrapped);
    }
    Ok(normalized)
}

/// 交给协议类型反序列化（`extra` 会吞掉未知字段）。
fn deserialize_params<T: DeserializeOwned>(object: Map<String, Value>) -> Result<T> {
    serde_json::from_value(Value::Object(object))
        .map_err(|err| Error::Protocol(format!("参数非法: {err}")))
}

/// 计划里的消息：协议类型解析结果 + `auto_escape`（数组形态无语义）。
fn message_out(content: &MessageContent, auto_escape: bool) -> Result<MessageOut> {
    if content.is_empty() {
        return Err(Error::Protocol("message 不能为空".to_owned()));
    }
    Ok(MessageOut::from_content(content, auto_escape))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::{
        MessageOut, SendRequest, SendTarget, plan_send_group_msg, plan_send_msg,
        plan_send_private_msg,
    };
    use gensokyo_core::Ob11Id;
    use gensokyo_protocol::MediaRef;

    #[test]
    fn group_text_message_plans_without_media() {
        let planned = plan_send_group_msg(&json!({"group_id": 100, "message": "你好"}))
            .expect("纯文本应规划成功");

        assert_eq!(planned.target, SendTarget::Group("100".to_owned()));
        assert_eq!(planned.message.text, "你好");
        assert!(planned.message.plan.is_empty());
        assert!(!planned.message.auto_escape);
    }

    #[test]
    fn ids_keep_their_text_form() {
        let numeric =
            plan_send_group_msg(&json!({"group_id": 42, "message": "x"})).expect("数字 ID 应成功");
        assert_eq!(numeric.target, SendTarget::Group("42".to_owned()));

        let openid = plan_send_group_msg(&json!({"group_id": "BAC3N5VPRKPRQ1GJ", "message": "x"}))
            .expect("OpenID 应成功");
        assert_eq!(
            openid.target,
            SendTarget::Group("BAC3N5VPRKPRQ1GJ".to_owned())
        );
        assert_eq!(openid.target.kind(), "group");
        assert_eq!(openid.target.id(), "BAC3N5VPRKPRQ1GJ");
    }

    #[test]
    fn float_ids_are_normalized_to_integers() {
        for raw in [json!(1e3), json!(1000.0)] {
            let planned = plan_send_group_msg(&json!({"group_id": raw, "message": "x"}))
                .expect("浮点整值 ID 应规划成功");
            assert_eq!(planned.target, SendTarget::Group("1000".to_owned()));
        }

        let err = plan_send_group_msg(&json!({"group_id": 1.5, "message": "x"}))
            .expect_err("非整值 ID 应报错");
        assert!(err.to_string().contains("group_id"), "实际错误: {err}");

        // 不参与规划的 ID 字段（`send_private_msg` 的 `group_id`）同样折叠，避免协议类型
        // 抛出不可读的 serde 报错
        let ok = plan_send_private_msg(&json!({"user_id": 7, "group_id": 100.0, "message": "x"}))
            .expect("整值 group_id 应放行");
        assert_eq!(ok.target, SendTarget::Private("7".to_owned()));

        let err = plan_send_private_msg(&json!({"user_id": 7, "group_id": 2.5, "message": "x"}))
            .expect_err("非整值 group_id 应报错");
        assert!(err.to_string().contains("group_id"), "实际错误: {err}");
    }

    #[test]
    fn cq_at_stays_in_text() {
        let planned =
            plan_send_group_msg(&json!({"group_id": 1, "message": "[CQ:at,qq=42] 早上好"}))
                .expect("at 段应规划成功");

        assert_eq!(planned.message.text, "[CQ:at,qq=42] 早上好");
        // at 折进文本（与 Go 的 messageText 一致），不再留在 plan 里
        assert!(planned.message.plan.is_empty());
    }

    #[test]
    fn cq_reply_becomes_reply_msg_id() {
        let planned =
            plan_send_group_msg(&json!({"group_id": 1, "message": "[CQ:reply,id=12345]收到"}))
                .expect("reply 段应规划成功");

        assert_eq!(planned.message.text, "收到");
        assert_eq!(planned.message.plan.reply_msg_id, Some(Ob11Id::Num(12345)));
        // reply 不算「可发送内容」（`OutboundPlan::is_empty` 与 Go 的跳过语义一致），
        // 出站意图落在上一行的 reply_msg_id 上，由 runtime 读取。
        assert!(planned.message.plan.is_empty());
    }

    #[test]
    fn cq_image_becomes_media_ref() {
        let planned = plan_send_group_msg(
            &json!({"group_id": 1, "message": "[CQ:image,file=https://example.com/a.png]"}),
        )
        .expect("image 段应规划成功");

        assert_eq!(planned.message.text, "");
        assert_eq!(
            planned.message.plan.images,
            vec![MediaRef::Url("https://example.com/a.png".to_owned())]
        );
    }

    #[test]
    fn segment_array_plans_text_and_media() {
        let planned = plan_send_group_msg(&json!({
            "group_id": 1,
            "message": [
                {"type": "text", "data": {"text": "看这个"}},
                {"type": "image", "data": {"file": "file:///tmp/a.png"}}
            ]
        }))
        .expect("消息段数组应规划成功");

        assert_eq!(planned.message.text, "看这个");
        assert_eq!(
            planned.message.plan.images,
            vec![MediaRef::Local(PathBuf::from("/tmp/a.png"))]
        );
        assert!(!planned.message.auto_escape, "数组形态不设 auto_escape");
    }

    #[test]
    fn single_segment_object_is_accepted() {
        let text = plan_send_group_msg(&json!({
            "group_id": 1,
            "message": {"type": "text", "data": {"text": "单个段"}}
        }))
        .expect("单个文本段对象应规划成功");
        assert_eq!(text.message.text, "单个段");

        let media = plan_send_group_msg(&json!({
            "group_id": 1,
            "message": {"type": "image", "data": {"file": "https://example.com/a.png"}}
        }))
        .expect("单个媒体段对象应规划成功");
        assert_eq!(
            media.message.plan.images,
            vec![MediaRef::Url("https://example.com/a.png".to_owned())]
        );
    }

    #[test]
    fn array_message_ignores_auto_escape() {
        let planned = plan_send_group_msg(&json!({
            "group_id": 1,
            "message": [{"type": "text", "data": {"text": "[CQ:at,qq=42]"}}],
            "auto_escape": true
        }))
        .expect("数组 + auto_escape 应规划成功");

        assert_eq!(planned.message.text, "[CQ:at,qq=42]");
        assert!(
            !planned.message.auto_escape,
            "数组形态不设 auto_escape（段已是结构化数据）"
        );
    }

    #[test]
    fn unknown_fields_are_tolerated() {
        let planned = plan_send_group_msg(&json!({
            "group_id": 1,
            "message": "x",
            "msg_type": 0,
            "active": true,
            "unknown": {"nested": [1, 2]}
        }))
        .expect("未知字段应被忽略");
        assert_eq!(planned.message.text, "x");

        let private = plan_send_private_msg(&json!({
            "user_id": 7,
            "message": "p",
            "foo": "bar"
        }))
        .expect("未知字段应被忽略");
        assert_eq!(private.target, SendTarget::Private("7".to_owned()));
    }

    #[test]
    fn send_request_round_trips_through_json() {
        let planned = plan_send_group_msg(&json!({
            "group_id": 1,
            "message": "[CQ:reply,id=9][CQ:image,file=https://example.com/a.png]hi"
        }))
        .expect("规划应成功");

        let mut value = serde_json::to_value(&planned).expect("SendRequest 应可序列化");
        let restored: SendRequest = serde_json::from_value(value.clone()).expect("应可反序列化");
        assert_eq!(restored, planned);

        // handler 在同一个对象里加 `deferred` 标记；反序列化必须忽略它
        value["deferred"] = json!(true);
        let restored: SendRequest =
            serde_json::from_value(value).expect("带 deferred 也应可反序列化");
        assert_eq!(restored, planned);
    }

    #[test]
    fn auto_escape_keeps_cq_text_verbatim() {
        let planned = plan_send_group_msg(&json!({
            "group_id": 1,
            "message": "[CQ:at,qq=42]",
            "auto_escape": true
        }))
        .expect("auto_escape 应规划成功");

        assert_eq!(planned.message.text, "[CQ:at,qq=42]");
        assert!(planned.message.auto_escape);
        assert!(
            planned.message.plan.is_empty(),
            "auto_escape=true 时不得解析 CQ"
        );
        assert!(planned.message.to_segments().is_empty());
    }

    #[test]
    fn auto_escape_must_be_bool() {
        let err = plan_send_group_msg(&json!({
            "group_id": 1,
            "message": "x",
            "auto_escape": "true"
        }))
        .expect_err("auto_escape 非布尔应报错");
        assert!(err.to_string().contains("auto_escape"));
    }

    #[test]
    fn private_message_uses_user_id() {
        let planned = plan_send_private_msg(&json!({"user_id": "abc", "message": "hi"}))
            .expect("私聊应规划成功");

        assert_eq!(planned.target, SendTarget::Private("abc".to_owned()));
        assert_eq!(planned.target.kind(), "private");
        assert_eq!(planned.message.text, "hi");
    }

    #[test]
    fn group_send_falls_back_to_user_id_for_temp_session() {
        // Go `handlers/send_group_msg.go`：只给 user_id 时走群临时会话（group_private）
        let planned = plan_send_group_msg(&json!({"user_id": 42, "message": "hi"}))
            .expect("群临时会话应规划成功");
        assert_eq!(planned.target, SendTarget::Private("42".to_owned()));

        // 两者同时出现时以 group_id 为准
        let both = plan_send_group_msg(&json!({"group_id": 1, "user_id": 42, "message": "hi"}))
            .expect("两者同时出现应规划成功");
        assert_eq!(both.target, SendTarget::Group("1".to_owned()));
    }

    #[test]
    fn send_msg_dispatches_by_message_type() {
        let group = plan_send_msg(&json!({
            "message_type": "group",
            "group_id": 7,
            "message": "g"
        }))
        .expect("group 分派应成功");
        assert_eq!(group.target, SendTarget::Group("7".to_owned()));

        let private = plan_send_msg(&json!({
            "message_type": "private",
            "user_id": 8,
            "message": "p"
        }))
        .expect("private 分派应成功");
        assert_eq!(private.target, SendTarget::Private("8".to_owned()));
    }

    #[test]
    fn send_msg_requires_message_type() {
        let err = plan_send_msg(&json!({"group_id": 1, "message": "x"}))
            .expect_err("缺 message_type 应报错");
        assert!(err.to_string().contains("message_type"));
    }

    #[test]
    fn send_msg_rejects_unknown_message_type() {
        for params in [
            json!({"message_type": "guild", "group_id": 1, "message": "x"}),
            json!({"message_type": 1, "group_id": 1, "message": "x"}),
        ] {
            let err = plan_send_msg(&params).expect_err("非法 message_type 应报错");
            assert!(err.to_string().contains("message_type"), "实际错误: {err}");
        }
    }

    #[test]
    fn missing_or_empty_ids_are_rejected() {
        for params in [
            json!({"message": "x"}),
            json!({"group_id": null, "message": "x"}),
            json!({"group_id": "", "message": "x"}),
            json!({"group_id": "0", "message": "x"}),
            json!({"group_id": 0, "message": "x"}),
            json!({"group_id": [1], "message": "x"}),
        ] {
            let err = plan_send_group_msg(&params).expect_err("非法 group_id 应报错");
            assert!(err.to_string().contains("group_id"), "实际错误: {err}");
        }

        let err = plan_send_private_msg(&json!({"message": "x"})).expect_err("缺 user_id 应报错");
        assert!(err.to_string().contains("user_id"));
    }

    #[test]
    fn missing_or_empty_message_is_rejected() {
        for params in [
            json!({"group_id": 1}),
            json!({"group_id": 1, "message": ""}),
            json!({"group_id": 1, "message": []}),
            json!({"group_id": 1, "message": null}),
        ] {
            let err = plan_send_group_msg(&params).expect_err("非法 message 应报错");
            assert!(err.to_string().contains("message"), "实际错误: {err}");
        }
    }

    #[test]
    fn params_must_be_an_object() {
        let err = plan_send_group_msg(&json!(null)).expect_err("null params 应报错");
        assert!(err.to_string().contains("params"));
    }

    #[test]
    fn message_out_serializes_text_and_non_text_segments() {
        let planned = plan_send_group_msg(&json!({
            "group_id": 1,
            "message": "[CQ:reply,id=9][CQ:image,file=https://example.com/a.png]hi"
        }))
        .expect("规划应成功");

        let value = serde_json::to_value(&planned.message).expect("MessageOut 应可序列化");
        assert_eq!(value["text"], json!("hi"));
        assert_eq!(value["auto_escape"], json!(false));
        assert_eq!(
            value["plan"],
            json!([
                {"type": "reply", "data": {"id": "9"}},
                {"type": "image", "data": {"file": "https://example.com/a.png"}}
            ])
        );

        let restored: MessageOut = serde_json::from_value(value).expect("MessageOut 应可反序列化");
        assert_eq!(restored, planned.message);

        let empty = MessageOut {
            text: "x".to_owned(),
            plan: Default::default(),
            auto_escape: true,
        };
        assert!(empty.to_segments().is_empty());
    }
}
