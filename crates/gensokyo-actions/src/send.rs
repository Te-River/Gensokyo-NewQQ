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
//! **不会把计划本身直接回给客户端**。反序列化只读 `target` / `message` / `echo_context`，
//! `deferred` 等未知字段被忽略；[`MessageOut::plan`] 的段数组按 [`OutboundPlan::from_segments`]
//! 还原，段里的文本 / `at` 折回 [`MessageOut::text`]（见 [`MessageOut::from_parts`]）。echo 不回传
//! 进计划本体：请求回执的字符串形态由分发器补进 [`EchoContext::echo_key`]，供 runtime 查
//! `msg_id` 表（Go `GetMsgIDByKey(echoStr)`）。
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
//! - **`send_group_msg_raw` 的路由**：Go 先用 echo 消息类型缓存猜 `group` / `group_private`，
//!   猜不出再递归重试；这里没有那份进程内缓存，直接按参数落点路由（`group_id` → 群，仅
//!   `user_id` → 私聊），与 `plan_send_group_msg` 同口径。Go 的 `params.message_id` 在这里映射成
//!   被动回复的 `reply_msg_id`（`"0"` / 空串视为主动发送，Go 会把 `"0"` 原样写进 `msg_id`）。
//! - **`send_private_msg_sse`**：Go 把 `stream` 的 `index` 与续写 `id` 存在进程内 `sync.Map`
//!   （按 message_id 自增、缓存首片返回的消息 ID）；这里没有那份缓存，`index` / `id` 由调用方
//!   显式传入（缺省 `index = 0`、首片不带 `id`），`msg_seq = index + 10` 的防去重写法保留。
//! - **`send_private_msg_wakeup`**：Go 空 `message` 时会补一条 `" "` 的纯唤醒消息；这里要求
//!   `message` 非空（`1400`），唤醒语义全部落在 `SendRequest.wakeup` 标记上，由 runtime 执行链
//!   置 `is_wakeup` 并清空 `msg_id` / `event_id`（互斥约束）。
//! - **合并转发（`send_group_forward_msg` / `send_private_forward_msg`）**：Go 逐条重发 node 正文，
//!   node 内的图片 / 语音在「先解析再拼回文本」的往返中丢失；这里保留 node 的完整出站计划
//!   （媒体走同一条 deferred 发送链），某条 node 组合非法（如 markdown + 图片同条）时该条进
//!   失败清单、其余照发，不回滚已发送的条。`Go` 未注册 `send_private_forward_msg`，本 crate
//!   按群侧同款「逐条重发」口径实现，端点是真实的 C2C 消息端点。
//!
//! # 未做项（M1i 明确不做）
//!
//! - 媒体上传：本地文件 → 图床 / 群 / C2C 上传、语音转 silk、`file_info` 回填。
//! - SSM（`code:22009` 失败入队）、idmap 虚拟 ID ↔ OpenID 映射、echo 懒消息表与
//!   `messageID == "2000"` 的主动推送分支：都需要存储 / 网络，落在 `gensokyo-runtime` 的
//!   deferred 执行链（见 `deferred::resolve_passive`）；本层只把查询所需的键
//!   （[`EchoContext`]）随计划带过去。
//! - `send_private_msg` 的 `group_id`（群临时会话的群上下文）：协议类型能解析，但计划只按
//!   `user_id` 规划、不落 [`SendTarget`]，群上下文由 runtime 决定。

use serde::de::DeserializeOwned;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use gensokyo_core::{Error, Result};
use gensokyo_protocol::action::{SendGroupMsgParams, SendPrivateMsgParams};
use gensokyo_protocol::{MessageContent, OutboundPlan, Segment};
use gensokyo_qq::dto::{Markdown, MessageToCreate, StreamSse, msg_type};

use crate::params::{
    id_field, normalized_number, object as params_object, optional_i64, optional_text, required_id,
};

/// Go `send_private_msg_sse.go:193`：`msg_seq = index + 10`，给上下文消息预留 10 个序号，
/// 避免流式分片与普通回复撞 seq 被去重。
const SSE_MSG_SEQ_RESERVE: i64 = 10;

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

/// 被动回复上下文：客户端原样传入、runtime 查懒消息池 / echo 映射表时另外需要的键。
///
/// 收件人本身已在 [`SendTarget`] 里（群号 / 用户号原样文本），这里只补两样推导不出来的：
/// 群聊可选的 `user_id`（懒消息池 v2 与「群 × 用户」`msg_id` 键的第二维）和请求回执 `echo`
/// 的字符串形态（Go 直接拿 `message.Echo` 当 `msg_id` 键查表）。两者缺省即「没有上下文」，
/// 旧载荷照常解析。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EchoContext {
    /// 群聊可选 `user_id`（客户端原样文本）；`null` / 空串 / `"0"` 视为没有用户上下文。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// 请求 `echo` 的字符串形态（非字符串形态不参与查表）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echo_key: Option<String>,
}

impl EchoContext {
    /// 两项都缺省时整个键不落盘（配合 `skip_serializing_if`）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.user_id.is_none() && self.echo_key.is_none()
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
    /// 互动召回标记（`send_private_msg_wakeup`）：runtime 执行链置 `is_wakeup`，
    /// 并强制不带 `msg_id` / `event_id`（Go 的互斥语义，主动召回永远不是被动回复）。
    #[serde(default)]
    pub wakeup: bool,
    /// 被动回复上下文（懒消息池 / echo 键查表用）；缺省即「按主动推送处理」。
    #[serde(default, skip_serializing_if = "EchoContext::is_empty")]
    pub echo_context: EchoContext,
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
            wakeup: false,
            // 群聊的 user_id 不参与收件人判定，只作为懒消息池 / 群 × 用户 msg_id 键的上下文
            echo_context: EchoContext {
                user_id: user_context(object),
                echo_key: None,
            },
        });
    }

    // 群临时会话：Go 的 `group_private` 分支最终也走 HandleSendPrivateMsg
    if let Some(user_id) = id_field(object, "user_id")? {
        let parsed: SendPrivateMsgParams =
            deserialize_params(normalized_params(object, &[("user_id", user_id)])?)?;
        return Ok(SendRequest {
            target: SendTarget::Private(parsed.user_id.to_string()),
            message: message_out(&parsed.message, auto_escape)?,
            wakeup: false,
            echo_context: EchoContext::default(),
        });
    }

    Err(Error::Protocol(
        "缺少 group_id（群聊）或 user_id（群临时会话）".to_owned(),
    ))
}

/// 群聊可选的 `user_id` 上下文：`null` / 空串 / `"0"` / 非 ID 形态都算「没有用户上下文」
/// （对齐 Go 的 `UserID != nil && != "" && != "0"` 判定；取不到就当主动推送）。
fn user_context(object: &Map<String, Value>) -> Option<String> {
    match object.get("user_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) if text.is_empty() || text == "0" => None,
        Some(Value::String(text)) => Some(text.clone()),
        // `42.0` 这类整值浮点按 Go 的 `FormatFloat('f', 0)` 归一；非整值当没有上下文
        Some(Value::Number(number)) => normalized_number("user_id", number)
            .ok()
            .and_then(|normalized| normalized.as_i64())
            .filter(|id| *id != 0)
            .map(|id| id.to_string()),
        Some(_) => None,
    }
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
        wakeup: false,
        // 收件人本身就是用户号，没有额外的群上下文
        echo_context: EchoContext::default(),
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

/// `send_private_msg_wakeup`（`send_private_msg` 的召回形态）的参数规划。
///
/// 与 [`plan_send_private_msg`] 同一条链，只差 [`SendRequest::wakeup`] 标记；
/// `is_wakeup` 的落体与 `msg_id` 清空由 runtime 的 deferred 执行链完成（Go 在
/// `send_private_msg_wakeup.go` 里逐条强制 `IsWakeup=true`、`MsgID=""`、`EventID=""`）。
///
/// # Errors
/// 同 [`plan_send_private_msg`]。
pub fn plan_send_private_msg_wakeup(params: &Value) -> Result<SendRequest> {
    let mut planned = plan_send_private_msg(params)?;
    planned.wakeup = true;
    Ok(planned)
}

/// `send_group_msg_raw` 的参数规划：与 [`plan_send_group_msg`] 同一条发送链。
///
/// Go 的 `send_group_msg_raw.go` 并不是「原生 JSON 直传」：它先用 echo 的消息类型缓存猜
/// `group` / `group_private`，命中后仍走 `parseMessageContent` → 普通发送链（`group_private`
/// 直接转 `HandleSendPrivateMsg`）。本 crate 没有那份缓存，按参数落点路由（`group_id` →
/// 群、仅 `user_id` → 私聊），与 [`plan_send_group_msg`] 完全同口径；差异只有一处——
/// Go 读的 `params.message_id` 在这里映射为被动回复的 `reply_msg_id`。
///
/// # Errors
/// 同 [`plan_send_group_msg`]；`message_id` 存在但不是数字 / 字符串时也是参数错误。
pub fn plan_send_group_msg_raw(params: &Value) -> Result<SendRequest> {
    let mut planned = plan_send_group_msg(params)?;
    let object = params_object(params)?;
    if let Some(value) = raw_message_id(object)? {
        let reply = serde_json::from_value(value)
            .map_err(|err| Error::Protocol(format!("message_id 形态非法: {err}")))?;
        planned.message.plan.reply_msg_id = Some(reply);
    }
    Ok(planned)
}

/// `send_group_msg_raw` 的 `message_id`：缺失 / `null` / 空串 / `0` 都按主动发送
/// （Go 把 `"0"` 原样写进 `msg_id`，上游同样发不出去，这里归一为不回复）；其余规范化成
/// `Ob11Id` 的线上形态，数字虚拟号 ↔ 真实 ID 的还原留给 runtime 的 reply 反查。
fn raw_message_id(object: &Map<String, Value>) -> Result<Option<Value>> {
    match object.get("message_id") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.is_empty() || text == "0" => Ok(None),
        Some(Value::String(text)) => Ok(Some(Value::String(text.clone()))),
        Some(Value::Number(number)) => {
            let normalized = normalized_number("message_id", number)?;
            if normalized.as_i64() == Some(0) {
                return Ok(None);
            }
            Ok(Some(Value::Number(normalized)))
        }
        Some(other) => Err(Error::Protocol(format!(
            "message_id 必须是数字或字符串: {other}"
        ))),
    }
}

/// `send_private_msg_sse` 的出站体：SSE 形态的消息体直发 C2C 消息端点。
///
/// Go 的 `PostC2CMessageSSE` 就是 `POST /v2/users/{user_id}/messages`（与普通 C2C 同端点），
/// 所以执行侧复用 [`crate::qqapi::QqApi::send_c2c_message`]；`message_sse` 是 Go
/// `dto.MessageSSE` 的形状（`msg_type=2` + `markdown.content` + `stream`，`prompt_keyboard` /
/// `action_button` 走 [`MessageToCreate::with_extra`] 透传）。deferred 握手载荷里
/// `message_sse` 键同时是 runtime 识别这条形状的判别键。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SseRequest {
    /// 收件人（规划侧保证恒为私聊）。
    pub target: SendTarget,
    /// SSE 消息体。
    pub message_sse: MessageToCreate,
}

/// `send_private_msg_sse` 的参数规划：`message` 是 Go `structs.InterfaceBody` 形态的对象。
///
/// # Errors
/// 缺 `user_id`、`message` 不是对象、`content` 不是字符串、缺 `state`（Go 缺省会发一条
/// `state=0` 的无效分片，这里明确拒绝）、`prompt_keyboard` 不是字符串数组、
/// `index` 越界时返回 [`Error::Protocol`]。
pub fn plan_send_private_msg_sse(params: &Value) -> Result<SseRequest> {
    let object = params_object(params)?;
    let user_id = required_id(object, "user_id")?;
    let body = match object.get("message") {
        Some(Value::Object(body)) => body,
        None | Some(Value::Null) => {
            return Err(Error::Protocol("缺少 message（SSE 消息体对象）".to_owned()));
        }
        Some(_) => {
            return Err(Error::Protocol(
                "message 必须是 JSON 对象（Go InterfaceBody 形态）".to_owned(),
            ));
        }
    };
    let content = match body.get("content") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => {
            return Err(Error::Protocol(format!("content 必须是字符串: {other}")));
        }
    };
    let state = optional_i64(body, "state")?
        .ok_or_else(|| Error::Protocol("state 缺失或不是整数".to_owned()))?;
    // Go 的 index 是进程内按 message_id 自增的 sync.Map、续写 id 是首片返回值的缓存；
    // 这里没有那份状态，两者由调用方显式传入（缺省 index = 0、首片不带 id）。
    let index = optional_i64(body, "index")?.unwrap_or(0);
    // 别名只在 `id` 缺位时求值：`id` 合法时 `stream_msg_id` 写了错误类型也不该 1400（评审 ⑦）
    let id = match optional_text(body, "id")? {
        Some(id) => Some(id),
        None => optional_text(body, "stream_msg_id")?,
    };
    let msg_seq = u32::try_from(index.saturating_add(SSE_MSG_SEQ_RESERVE).max(0))
        .map_err(|_| Error::Protocol(format!("index 超出范围: {index}")))?;
    let msg_id = optional_text(body, "msg_id")?;

    let mut message = MessageToCreate {
        msg_type: Some(msg_type::MARKDOWN),
        markdown: (!content.is_empty()).then(|| Markdown::content(content)),
        msg_id,
        msg_seq: Some(msg_seq),
        stream: Some(StreamSse { state, index, id }),
        ..MessageToCreate::default()
    };

    // Go `generateMessageSSE`：每个 label 独立成一行，`render_data.style=2`、`action.type=2`
    let labels = match body.get("prompt_keyboard") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => {
            let mut labels = Vec::with_capacity(items.len());
            for item in items {
                let text = item.as_str().ok_or_else(|| {
                    Error::Protocol("prompt_keyboard 的元素必须是字符串".to_owned())
                })?;
                labels.push(text.to_owned());
            }
            labels
        }
        Some(_) => {
            return Err(Error::Protocol(
                "prompt_keyboard 必须是字符串数组".to_owned(),
            ));
        }
    };
    if !labels.is_empty() {
        let rows: Vec<Value> = labels
            .iter()
            .map(|label| {
                json!({
                    "buttons": [{
                        "render_data": { "label": label, "style": 2 },
                        "action": { "type": 2 }
                    }]
                })
            })
            .collect();
        message = message.with_extra(
            "prompt_keyboard",
            json!({ "keyboard": { "content": { "rows": rows } } }),
        );
    }

    // Go：`CallbackData` 非空才挂 action_button（`template_id` 缺省 0，两者都序列化）
    if let Some(callback_data) = optional_text(body, "callback_data")? {
        let template_id = optional_i64(body, "action_button")?.unwrap_or(0);
        message = message.with_extra(
            "action_button",
            json!({ "template_id": template_id, "callback_data": callback_data }),
        );
    }

    Ok(SseRequest {
        target: SendTarget::Private(user_id),
        message_sse: message,
    })
}

/// 合并转发（`send_group_forward_msg` / `send_private_forward_msg`）的出站计划。
///
/// 官方没有折叠转发端点，Go 的口径就是「逐条重发 node 正文」；deferred 握手载荷里
/// `forward` 键同时是 runtime 识别这条形状的判别键。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForwardRequest {
    /// 逐 node 的发送计划（已按 `forward_msg_limit` 截断），顺序与可发送 node 一致。
    pub forward: Vec<ForwardPlan>,
    /// 规划层就解析失败的 node（`content` 是数组但段反序列化报错）：计入失败回执、
    /// 带原 node 位置，不静默吞（评审 ②：与「形状本就不是 node」的静默跳过区分开）。
    #[serde(default)]
    pub parse_failed: Vec<ForwardParseFailure>,
}

/// 一条转发的计划项：携带来源 node 在原 `messages` 数组里的位置。
///
/// `node_index` 让失败回执能回查「是哪一条 node 失败」——坏形状 / 空正文 node 被跳过后，
/// 计划下标与 node 序号并不一致（评审 ①）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForwardPlan {
    /// 来源 node 的 0 基下标（`messages` 数组原位）。
    #[serde(default)]
    pub node_index: usize,
    /// 发送请求。
    pub request: SendRequest,
}

/// 规划层解析失败的 node（进 [`ForwardRequest::parse_failed`]）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardParseFailure {
    /// 来源 node 的 0 基下标。
    pub node_index: usize,
    /// 段反序列化的报错文本。
    pub error: String,
}

/// `send_group_forward_msg`：`group_id` + `messages` 的每个 node 逐条转群消息。
///
/// `limit` 来自配置 `forward_msg_limit`（Go `config.GetForwardMsgLimit()`，执行层没有
/// 那份进程状态，规划层就按它截断）。
///
/// # Errors
/// `params` 不是对象、缺 `group_id`、`messages` 缺失 / 空数组 / 非数组时 [`Error::Protocol`]；
/// node 形状不对（非对象、缺 `data`、`content` 既非字符串也非段数组）**跳过该条**，
/// 与 Go 的 `continue` 一致，不把整次转发拒掉；`content` 是数组但段反序列化失败时**不吞**
/// ——`warn` 日志并记入 [`ForwardRequest::parse_failed`]，随失败回执上报。
pub fn plan_send_group_forward_msg(params: &Value, limit: i64) -> Result<ForwardRequest> {
    plan_forward(params, limit, true)
}

/// `send_private_forward_msg`：群侧同款逐条重发口径，目标换成私聊。
///
/// Go 未注册该 action（go-cqhttp 扩展名）；官方同样没有 C2C 折叠转发端点，因此按 Go
/// 群侧的「逐条重发」口径实现，端点就是 [`crate::qqapi::QqApi::send_c2c_message`]。
///
/// # Errors
/// 同 [`plan_send_group_forward_msg`]，ID 字段换成 `user_id`。
pub fn plan_send_private_forward_msg(params: &Value, limit: i64) -> Result<ForwardRequest> {
    plan_forward(params, limit, false)
}

/// 两个合并转发 action 的公共规划：逐 node 还原正文 → 一条 [`ForwardPlan`]。
///
/// Go 只取 node 解析后的 `messageText`（媒体在「解析→拼回文本」的往返中丢弃）；这里保留
/// node 的完整出站意图（媒体走同一条 deferred 发送链），避免静默丢内容——差异已文档化。
fn plan_forward(params: &Value, limit: i64, to_group: bool) -> Result<ForwardRequest> {
    let object = params_object(params)?;
    let nodes = match object.get("messages") {
        Some(Value::Array(items)) if !items.is_empty() => items,
        Some(Value::Array(_)) => return Err(Error::Protocol("messages 不能是空数组".to_owned())),
        None | Some(Value::Null) => return Err(Error::Protocol("缺少 messages".to_owned())),
        Some(_) => return Err(Error::Protocol("messages 必须是数组".to_owned())),
    };
    let id_key = if to_group { "group_id" } else { "user_id" };
    let target = if to_group {
        SendTarget::Group(required_id(object, id_key)?)
    } else {
        SendTarget::Private(required_id(object, id_key)?)
    };

    let mut plans = Vec::new();
    let mut parse_failed = Vec::new();
    // Go 的循环条件：`count > limit` 才 break、count 只在形状合格的 node 上递增——
    // 上界实际是 limit+1 条。保留 Go 的 this off-by-one，行为对齐优先于修它；
    // 计划项携带 node_index，失败回执按原 node 位置上报（评审 ①）。
    let mut count: i64 = 0;
    for (node_index, node) in nodes.iter().enumerate() {
        if count > limit {
            break;
        }
        let content = match classify_node(node) {
            NodeContent::Skipped => continue,
            NodeContent::ParseFailed(error) => {
                // 数组形态但段不认识：Go 此时也计数并跑一遍空发送；这里计数并上报错误
                count += 1;
                tracing::warn!(
                    node_index,
                    error = %error,
                    "合并转发 node 的消息段解析失败，计入失败回执"
                );
                parse_failed.push(ForwardParseFailure { node_index, error });
                continue;
            }
            NodeContent::Planned(content) => content,
        };
        count += 1;
        let message = MessageOut::from_content(&content, false);
        // 空正文 node（空串 / 空段数组）：Go 会跑完发送链但没有任何上游调用，这里不入队
        if message.text.is_empty() && message.plan.is_empty() {
            continue;
        }
        plans.push(ForwardPlan {
            node_index,
            request: SendRequest {
                target: target.clone(),
                message,
                wakeup: false,
                // 转发没有「群 × 用户」的被动上下文，只按群维度的懒消息池走
                echo_context: EchoContext::default(),
            },
        });
    }
    Ok(ForwardRequest {
        forward: plans,
        parse_failed,
    })
}

/// node 的正文分类：区分「形状不对可静默跳过」与「段解析失败必须上报」（评审 ②）。
enum NodeContent {
    /// 非对象 / 缺 `data` / `content` 缺失且既非字符串也非段数组（Go 的两层 `continue`）。
    Skipped,
    /// `content` 是数组但消息段反序列化失败：错误文本进失败回执。
    ParseFailed(String),
    /// `content` 是字符串（CQ 文本）或可反序列化的段数组。
    Planned(MessageContent),
}

fn classify_node(node: &Value) -> NodeContent {
    let Some(data) = node
        .as_object()
        .and_then(|object| object.get("data"))
        .and_then(Value::as_object)
    else {
        return NodeContent::Skipped;
    };
    match data.get("content") {
        // 字符串形态永不反序列化失败；数组形态认不出的段显式上报，不能整条消失
        Some(content @ (Value::String(_) | Value::Array(_))) => {
            match serde_json::from_value::<MessageContent>(content.clone()) {
                Ok(parsed) => NodeContent::Planned(parsed),
                Err(err) => NodeContent::ParseFailed(err.to_string()),
            }
        }
        _ => NodeContent::Skipped,
    }
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
        ForwardRequest, MessageOut, SendRequest, SendTarget, SseRequest,
        plan_send_group_forward_msg, plan_send_group_msg, plan_send_group_msg_raw, plan_send_msg,
        plan_send_private_forward_msg, plan_send_private_msg, plan_send_private_msg_sse,
        plan_send_private_msg_wakeup,
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
    fn group_user_id_becomes_echo_context() {
        let planned = plan_send_group_msg(&json!({"group_id": 1, "user_id": 42, "message": "hi"}))
            .expect("应规划成功");
        assert_eq!(
            planned.echo_context.user_id.as_deref(),
            Some("42"),
            "群聊的 user_id 只作为被动回复上下文"
        );

        // Go 的判定：nil / 空串 / "0" 都算「没有用户上下文」
        for empty in [json!(null), json!(""), json!(0), json!(0.0)] {
            let planned =
                plan_send_group_msg(&json!({"group_id": 1, "user_id": empty, "message": "hi"}))
                    .expect("应规划成功");
            assert_eq!(planned.echo_context.user_id, None, "{empty} 应视为无上下文");
        }
    }

    #[test]
    fn old_plan_payload_without_echo_context_still_parses() {
        // echo 上下文是后加的字段：旧握手载荷（没有 echo_context）必须照常还原
        let data = json!({
            "target": {"group": "100"},
            "message": {"text": "hi", "auto_escape": false, "plan": []},
            "wakeup": false
        });
        let planned: SendRequest = serde_json::from_value(data).expect("旧载荷应能还原");
        assert!(planned.echo_context.is_empty());
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

    #[test]
    fn wakeup_flag_marks_planned_private_send() {
        let planned = plan_send_private_msg_wakeup(&json!({"user_id": "abc", "message": "召回"}))
            .expect("唤醒应规划成功");
        assert!(planned.wakeup);
        assert_eq!(planned.target, SendTarget::Private("abc".to_owned()));
        assert_eq!(planned.message.text, "召回");

        // 普通私聊发送不带唤醒标记
        let normal = plan_send_private_msg(&json!({"user_id": "abc", "message": "hi"}))
            .expect("普通私聊应规划成功");
        assert!(!normal.wakeup);

        // user_id / message 与 plan_send_private_msg 同一套校验
        let err =
            plan_send_private_msg_wakeup(&json!({"message": "hi"})).expect_err("缺 user_id 应报错");
        assert!(err.to_string().contains("user_id"));
    }

    #[test]
    fn raw_routes_like_group_send_and_keeps_message_id() {
        let planned = plan_send_group_msg_raw(&json!({
            "group_id": 100, "message": "收到", "message_id": "REAL-7"
        }))
        .expect("raw 应规划成功");
        assert_eq!(planned.target, SendTarget::Group("100".to_owned()));
        assert_eq!(planned.message.text, "收到");
        assert_eq!(
            planned.message.plan.reply_msg_id,
            Some(Ob11Id::str("REAL-7"))
        );
        assert!(!planned.wakeup);

        // 仅 user_id → 群临时会话（Go 的 group_private 分支转 HandleSendPrivateMsg）
        let temp = plan_send_group_msg_raw(&json!({"user_id": 42, "message": "hi"}))
            .expect("临时会话应规划成功");
        assert_eq!(temp.target, SendTarget::Private("42".to_owned()));
        assert_eq!(temp.message.plan.reply_msg_id, None);

        // "0" / 数字 0 按主动发送（Go 把 "0" 原样写进 msg_id 也发不出去）
        let zero = plan_send_group_msg_raw(&json!({
            "group_id": 1, "message": "hi", "message_id": "0"
        }))
        .expect("message_id 0 应放行");
        assert_eq!(zero.message.plan.reply_msg_id, None);

        // 数字形态归一成 Num（虚拟号还原留给 runtime 的 reply 反查）
        let numeric = plan_send_group_msg_raw(&json!({
            "group_id": 1, "message": "hi", "message_id": 9
        }))
        .expect("数字 message_id 应放行");
        assert_eq!(numeric.message.plan.reply_msg_id, Some(Ob11Id::Num(9)));

        let err =
            plan_send_group_msg_raw(&json!({"message": "hi"})).expect_err("两个 ID 都缺应报错");
        assert!(err.to_string().contains("group_id"));

        let err = plan_send_group_msg_raw(&json!({
            "group_id": 1, "message": "hi", "message_id": [1]
        }))
        .expect_err("非法 message_id 应报错");
        assert!(err.to_string().contains("message_id"), "实际错误: {err}");
    }

    #[test]
    fn sse_body_becomes_stream_markdown() {
        let planned = plan_send_private_msg_sse(&json!({
            "user_id": "u1",
            "message": {
                "content": "第一段",
                "state": 1,
                "index": 3,
                "id": "stream-9",
                "msg_id": "M-1",
                "prompt_keyboard": ["继续", "停止"],
                "action_button": 5,
                "callback_data": "cb"
            }
        }))
        .expect("SSE 应规划成功");

        assert_eq!(planned.target, SendTarget::Private("u1".to_owned()));
        let body = serde_json::to_value(&planned.message_sse).expect("SSE 体应可序列化");
        assert_eq!(body["msg_type"], json!(2));
        assert_eq!(body["markdown"]["content"], json!("第一段"));
        assert_eq!(
            body["stream"],
            json!({"state": 1, "index": 3, "id": "stream-9"})
        );
        assert_eq!(body["msg_seq"], json!(13), "Go 口径 msg_seq = index + 10");
        assert_eq!(body["msg_id"], json!("M-1"));
        assert_eq!(
            body["prompt_keyboard"],
            json!({"keyboard": {"content": {"rows": [
                {"buttons": [{"render_data": {"label": "继续", "style": 2}, "action": {"type": 2}}]},
                {"buttons": [{"render_data": {"label": "停止", "style": 2}, "action": {"type": 2}}]}
            ]}}})
        );
        assert_eq!(
            body["action_button"],
            json!({"template_id": 5, "callback_data": "cb"})
        );

        // runtime 侧从握手载荷还原必须无损（flatten 的 extra 也要能回来）
        let restored: SseRequest =
            serde_json::from_value(serde_json::to_value(&planned).expect("SseRequest 应可序列化"))
                .expect("SseRequest 应可反序列化");
        assert_eq!(restored, planned);

        // 缺 callback_data 不挂 action_button；空 content 省略 markdown；首片不带 id
        let bare = plan_send_private_msg_sse(&json!({
            "user_id": "u1", "message": {"state": 10, "id": "stream-9"}
        }))
        .expect("空正文终片应规划成功");
        let bare_body = serde_json::to_value(&bare.message_sse).expect("SSE 体应可序列化");
        assert!(bare_body.get("markdown").is_none());
        assert!(bare_body.get("action_button").is_none());
        assert!(bare_body.get("prompt_keyboard").is_none());
        assert_eq!(bare_body["msg_seq"], json!(10));
        assert_eq!(
            bare_body["stream"],
            json!({"state": 10, "index": 0, "id": "stream-9"})
        );

        // Go 的 unmarshal 拒绝非数字 state；十进制字符串按本仓 ID 家规放行，"abc" 必须拒
        let err = plan_send_private_msg_sse(&json!({"user_id": "u1", "message": {"state": "abc"}}))
            .expect_err("state 非数字应报错");
        assert!(err.to_string().contains("state"), "实际错误: {err}");
        let ok = plan_send_private_msg_sse(&json!({"user_id": "u1", "message": {"state": "1"}}))
            .expect("十进制字符串 state 应放行（本仓数字字段家规）");
        assert_eq!(
            ok.message_sse.stream.as_ref().expect("应带 stream").state,
            1
        );
        let err = plan_send_private_msg_sse(&json!({"user_id": "u1", "message": "hi"}))
            .expect_err("message 非对象应报错");
        assert!(err.to_string().contains("message"), "实际错误: {err}");
        let err = plan_send_private_msg_sse(&json!({"message": {"state": 1}}))
            .expect_err("缺 user_id 应报错");
        assert!(err.to_string().contains("user_id"));
    }

    #[test]
    fn forward_plans_per_node_with_go_limit_semantics() {
        let params = json!({
            "group_id": 100,
            "messages": [
                {"data": {"content": "第一段"}},
                {"data": {"content": [{"type": "text", "data": {"text": "第二段"}}]}},
                {"bogus": true},
                {"data": {"content": ""}},
                {"data": {"content": "第四段"}},
            ]
        });

        // 形状合格的 4 个 node 计数；空正文 node 计数但不产发送（Go 跑完链也没有上游调用）。
        // 计划项携带原 node 下标（评审 ①）：第四段是 node 4，不是计划下标 2
        let all = plan_send_group_forward_msg(&params, 10).expect("转发应规划成功");
        assert_eq!(all.forward.len(), 3);
        assert_eq!(all.forward[0].node_index, 0);
        assert_eq!(all.forward[1].node_index, 1);
        assert_eq!(
            all.forward[2].node_index, 4,
            "坏形状 / 空正文 node 被跳过后仍按原位编号"
        );
        assert_eq!(
            all.forward[0].request.target,
            SendTarget::Group("100".to_owned())
        );
        assert_eq!(all.forward[0].request.message.text, "第一段");
        assert_eq!(all.forward[1].request.message.text, "第二段");
        assert_eq!(all.forward[2].request.message.text, "第四段");
        assert!(all.forward.iter().all(|plan| !plan.request.wakeup));
        assert!(all.parse_failed.is_empty(), "字符串 content 不会解析失败");

        // Go 的 off-by-one：`count > limit` 才 break，limit=2 时第 3 个计数 node（空正文）
        // 把计数推到 3，第四段被截断——保留 Go 的实际行为（含空 node 占额度）
        let capped = plan_send_group_forward_msg(&params, 2).expect("截断应成功");
        assert_eq!(capped.forward.len(), 2);

        // limit=0 也能发首条（Go 同款：计数从 0 起）
        let first = plan_send_group_forward_msg(&params, 0).expect("limit 0 应规划成功");
        assert_eq!(first.forward.len(), 1);

        // 载荷回环（dispatcher 用 ForwardRequest 还原逐条计划与解析失败清单）
        let restored: ForwardRequest =
            serde_json::from_value(serde_json::to_value(&all).expect("ForwardRequest 应可序列化"))
                .expect("ForwardRequest 应可反序列化");
        assert_eq!(restored, all);

        // 段数组解析失败（评审 ②）：计数、进 parse_failed，不静默消失
        let with_bad_segment = json!({
            "group_id": 100,
            "messages": [
                {"data": {"content": "第一段"}},
                {"data": {"content": [123]}},
                {"data": {"content": "第三段"}},
            ]
        });
        let parsed = plan_send_group_forward_msg(&with_bad_segment, 10).expect("应规划成功");
        assert_eq!(parsed.forward.len(), 2);
        assert_eq!(
            parsed.forward[1].node_index, 2,
            "解析失败 node 之后的仍按原位编号"
        );
        assert_eq!(parsed.parse_failed.len(), 1);
        assert_eq!(parsed.parse_failed[0].node_index, 1);
        assert!(
            !parsed.parse_failed[0].error.is_empty(),
            "失败回执要带 serde 的解析错误文本"
        );
        let round_tripped: ForwardRequest =
            serde_json::from_value(serde_json::to_value(&parsed).expect("序列化应成功"))
                .expect("parse_failed 应可回环");
        assert_eq!(round_tripped, parsed);

        // 私聊转发：同口径，target 换成 Private，node 媒体保留（Go 会在文本往返中丢）
        let private = plan_send_private_forward_msg(
            &json!({
                "user_id": 42,
                "messages": [{"data": {"content": "[CQ:image,file=https://example.com/a.png]"}}]
            }),
            3,
        )
        .expect("私聊转发应规划成功");
        assert_eq!(private.forward.len(), 1);
        assert_eq!(
            private.forward[0].request.target,
            SendTarget::Private("42".to_owned())
        );
        assert_eq!(
            private.forward[0].request.message.plan.images,
            vec![MediaRef::Url("https://example.com/a.png".to_owned())]
        );

        // 空 messages / 缺失都是参数错误
        let err = plan_send_group_forward_msg(&json!({"group_id": 1, "messages": []}), 3)
            .expect_err("空数组应报错");
        assert!(err.to_string().contains("messages"), "实际错误: {err}");
        let err = plan_send_group_forward_msg(&json!({"messages": [{"data": {}}]}), 3)
            .expect_err("缺 group_id 应报错");
        assert!(err.to_string().contains("group_id"), "实际错误: {err}");
    }

    #[test]
    fn legacy_send_payload_without_wakeup_key_still_deserializes() {
        // 旧握手载荷没有 `wakeup` 键：serde default 必须能还原（runtime 向后兼容，评审 ⑥）
        let legacy = json!({
            "target": {"group": "1"},
            "message": {"text": "hi", "auto_escape": false, "plan": []}
        });
        let restored: SendRequest =
            serde_json::from_value(legacy).expect("缺 wakeup 键的旧载荷应可还原");
        assert!(!restored.wakeup);
        assert_eq!(restored.target, SendTarget::Group("1".to_owned()));
    }

    #[test]
    fn sse_prefers_id_over_mistyped_stream_msg_id_alias() {
        // 评审 ⑦：`id` 有值时别名 `stream_msg_id` 放错误类型也不该 1400（惰性回退）
        let planned = plan_send_private_msg_sse(&json!({
            "user_id": "u1",
            "message": {"state": 1, "id": "S-1", "stream_msg_id": {"bad": true}}
        }))
        .expect("id 合法时别名错误类型应被忽略");
        assert_eq!(
            planned
                .message_sse
                .stream
                .as_ref()
                .expect("应带 stream")
                .id
                .as_deref(),
            Some("S-1")
        );

        // `id` 缺位时别名照常生效；别名本身类型错才报错
        let via_alias = plan_send_private_msg_sse(&json!({
            "user_id": "u1",
            "message": {"state": 1, "stream_msg_id": "S-2"}
        }))
        .expect("缺 id 时别名应生效");
        assert_eq!(
            via_alias
                .message_sse
                .stream
                .as_ref()
                .expect("应带 stream")
                .id
                .as_deref(),
            Some("S-2")
        );
        let err = plan_send_private_msg_sse(&json!({
            "user_id": "u1",
            "message": {"state": 1, "stream_msg_id": {"bad": true}}
        }))
        .expect_err("id 缺位且别名类型错应报错");
        assert!(err.to_string().contains("stream_msg_id"), "实际错误: {err}");
    }
}
