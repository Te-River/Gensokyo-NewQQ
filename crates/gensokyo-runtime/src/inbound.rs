//! QQ 事件 → OneBot V11 事件（Go `Processor/`）。
//!
//! 入站转换只做「协议翻译」：把 QQ 机器人平台推送的 `d` 负载（以及 `t` 事件名）变成
//! [`gensokyo_protocol::Event`]，不碰框架指令、黑白名单、被动转主动等
//! [`Processor`](https://github.com/hoshinonyaruko/gensokyo) 里的业务逻辑。
//!
//! 除把群注册进 idmap 外，副作用有三个：喂 echo（[`Inbound::with_echo`]）——消息事件写懒
//! message_id 池与 msg_id / msg_type 映射，入群类事件写 event_id，供出站解析被动回复凭据；
//! 收发统计（`Inbound::record_message_received`）——三个消息事件各记一次当日接收数
//! （Go `botstats.RecordMessageReceived`）；C2C 私信把好友写进 idmap 的 `user_info` 表
//! （`Inbound::record_user_info`，`get_friend_list` 的数据源）。三者缺依赖时静默跳过，绝不
//! 阻断事件上报。
//!
//! # 已覆盖事件
//!
//! botgo `dto` 的 12 类自定义事件（`botgo/dto/websocket_event.go:42-57`）加两个消息事件，
//! 逐条对齐 `Processor/` 的上报字段：
//!
//! | QQ `t` | OneBot `post_type` | 说明 |
//! |--------|--------------------|------|
//! | `GROUP_AT_MESSAGE_CREATE` | `message`（group） | `sub_type=normal`、`to_me=true` |
//! | `GROUP_MESSAGE_CREATE` | `message`（group） | 全量群消息，`to_me=false`、`is_full_group_message=true` |
//! | `C2C_MESSAGE_CREATE` | `message`（private） | `sub_type=friend` |
//! | `GROUP_ADD_ROBOT` | `request` + `notice` | 对齐 Go：`request.group/invite` + `notice.group_increase/invite` |
//! | `FRIEND_ADD` | `request` + `notice` | 对齐 Go：`request.friend` + `notice.friend_add` |
//! | `GROUP_DEL_ROBOT` | `notice` | `group_decrease` + `sub_type=kick_me` |
//! | `GROUP_MSG_REJECT` | `notice` | `group_reject` + `sub_type=create` |
//! | `GROUP_MSG_RECEIVE` | `notice` | `group_receive` + `sub_type=create` |
//! | `FRIEND_DEL` | `notice` | `friend_decrease`；OneBot V11 无此标准类型，Go 自定义 |
//! | `C2C_MSG_REJECT` | `notice` | `c2c_msg_reject`（Go 自定义） |
//! | `C2C_MSG_RECEIVE` | `notice` | `c2c_msg_receive`（Go 自定义） |
//! | `GROUP_MEMBER_ADD` | `notice` | `group_increase` + `sub_type=approve`、`operator_id=0` |
//! | `GROUP_MEMBER_REMOVE` | `notice` | `group_decrease` + `sub_type=leave`、`operator_id=0` |
//! | `GROUP_JOIN_REQUEST` | `request` | `request_type=group` + `sub_type=add`、`flag=join_request_id` |
//! | `INTERACTION_CREATE` | `notice` | `interaction` + `sub_type=create`，互动对象整体落在 `data` |
//!
//! 其余 `t`（`GUILD_*` / `MESSAGE_AUDIT_*` / `FORUM_*` / `AUDIO_*`）
//! 返回空列表：未知事件不能打断链路，这也是 spec「未知事件不 panic」的落点。
//!
//! # 字段口径
//!
//! - `self_id` = `use_uin ? uin : app_id`，恒为数字（Go 两种上报形态里 `self_id` 都是 `int64`）。
//! - **消息事件**（`message`）：`string_ob11 = true` 时 `group_id` / `user_id` / `message_id`
//!   是原始 OpenID 字符串；数字形态时经 [`IdMapStore::retrieve_virtual_value`] 取虚拟 ID，
//!   **idmap 未注入或分配失败则回退原始字符串**（不伪造数字）。群消息在 string 形态下
//!   额外带 `platform = "qq"`、`sender.user_id` 恒为 `0`（Go `OnebotGroupMessageS` 的
//!   M1-B 口径：string 形态不做 idmap 转换），`message_seq` / `font` 恒为 `0`。
//! - **notice / request 事件**：Go 对这些 handler 一律 `idmap.StoreIDv2` 取**数字 vUIN**，
//!   与 `string_ob11` 无关（string 形态只影响 message 事件），这里用
//!   [`Inbound::vuid`]；idmap 未注入时回退原始字符串。
//! - **`union_id` 切换（P42）**：群消息在写完 `id → union_openid` 映射之后，`union_id = true`
//!   且 union 非空时，上报的 `user_id` / `real_user_id` / idmap 身份一律换成 union
//!   （Go `ProcessGroupMessage.go:58-64` 的 `data.Author.ID = data.Author.UnionOpenID`），
//!   string 形态的 `platform` 随之从 `"qq"` 变 `"unionqq"`。union 为空时保留原 id
//!   （Go 会无条件置空，登记为差异）。`INTERACTION_CREATE` 只把回显的
//!   `data.group_member_openid` 换成 union（Go `ProcessInlineSearch.go:112-116`），
//!   `user_id` / `real_user_id` 仍是原始 OpenID——Go 的 `fromuid` 在 `:39` 取值，先于替换。
//! - `!native_ob11` 时补 `real_user_id` / `real_group_id` / `real_message_type` 附加字段
//!   （与 Go 的「增强配置」一致）。例外：`GROUP_MEMBER_ADD/REMOVE` 在 Go 里**不区分**
//!   `native_ob11`（`RealUserID`/`RealGroupID` 只被 `omitempty` 省略空值），这里照搬。
//! - `time`：QQ 负载带 `timestamp` 时用它，否则取当前秒级时间戳。Go 群消息用 `time.Now()`，
//!   这里偏向可复现（同一事件重放得到同一 `time`）。`GROUP_JOIN_REQUEST` 的 `apply_at`
//!   优先按 RFC3339 解析（Go `time.Parse`），失败再按秒级数字解析。
//! - **入站媒体段**（`attachments` / `ark_data` / `msg_elements` 通路，P50）：图片附件、
//!   语音附件、结构化卡片、引用与嵌套正文按 Go 的两条路径各自成型——字符串路径把
//!   `[CQ:image…]` / `[CQ:record…]` / `[CQ:ark…]` / `[CQ:reply…]` 与嵌套正文接在正文之后
//!   （[`media_cq_text`]，Go `RevertTransformedText` `handlers/message_parser.go:2136-2157`），
//!   段数组路径产出 image / record / ark / reply / text 段并排在正文段之后
//!   （[`image_segments`] / [`message_event_segments`]，Go `ConvertToSegmentedMessage`
//!   `:2211-2235, 2331-2334`）。两条路径的图片 `file` 口径本就不同：字符串路径去最后一个扩展名，
//!   段路径删 `{` `}` `.png` `.jpg` `.gif` `-`；字符串路径的 `url` 缺协议头时补 `http://`。
//!   P52 收口：`ark` 段的 `fields` 出真 JSON 对象（[`Segment::param_json`]，对齐 Go
//!   `arkSegmentData` 的 `map[string]interface{}`，空 fields 出 `{}`）。P51 收口：CQ 值转义补齐 `[` → `&#91;`（[`encode_value`]，对齐 Go
//!   `cqparse.EncodeValue`）；`attachments` / `msg_elements` 整数组 `null` 按空数组收
//!   （[`null_as_default`]，对齐 Go `[]*T`）。
//! - **正文明文处理**（`<@OpenID>` mention 反解 → 自身 @ 剥离 → trim → 去首个 `/`）见
//!   [`Inbound::revert_mentions`]，字符串形态与段数组形态各按 Go 自己的路径算一遍。
//!   @bot 判定用「事件 `mentions[]` 的 `is_you`/`bot` 标记 + BotID / AppID」，消息作者
//!   本人永不当作 @bot（`is_you` 在多实例场景会误标发送者）。与 Go 的差异：Go 另有进程级
//!   `selfAtIDs` 跨事件记忆（`RememberSelfAtID`），本仓不引入跨请求的全局可变表。
//!
//! 金样（`Processor/testdata/inbound/`）是这些字段口径的回归保险：改动事件形态时先跑
//! `tests/golden_inbound.rs`。
//!
//! # 未做项
//!
//! - `global_group_msg_rre_to_message` / `global_c2c_msg_switch_to_message`（`true` 时 Go 把
//!   这些开关事件「伪装成 message」的业务转换，`ConvertGroupRejectToMessage` 等）未做：
//!   两个开关打开时这里仍只上报 notice。
//! - `echo` 的 `app_id/s` 映射、双路 echo、`is_binded_*`、`avatar`、master 角色提升、
//!   idmap-pro（`StoreIDv2Pro`）；`GROUP_MEMBER_ADD` / `GROUP_JOIN_REQUEST` 的 `event_id`
//!   只落在 `extra`，Go 侧 `echo.AddEvnetIDv2` 的被动回复缓存未做。
//! - `message_sent`、`request` 的 `flag` 审批链路（Go 侧 `GROUP_ADD_ROBOT` 的 `flag` 也是空串）。
//! - `global_interaction_to_message`（`true` 时 Go `ConvertInteractionToMessage` 把按钮回调
//!   「伪装成 message」）：`INTERACTION_CREATE` 只走上报 `notice.interaction` 的主路径。
//! - `auto_put_interaction` / `put_interaction_except`（自动回 `PutInteraction`）与
//!   idmap-pro：回调的 `id` 原样留在 `data.id`，由客户端自己回应。
//! - `MESSAGE_AUDIT_*`、`FORUM_*`、`AUDIO_*` 与频道 `GUILD_*`。

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use gensokyo_config::{ConfigManager, Settings};
use gensokyo_core::telemetry::metrics;
use gensokyo_core::{Ob11Id, acnode};
use gensokyo_protocol::cqcode::{self, code_spec};
use gensokyo_protocol::event::{
    HeartbeatStatus, MessageEvent, MessageType, MetaEvent, NoticeEvent, RequestEvent, Sender,
};
use gensokyo_protocol::{Event, Extra, MessageContent, Segment};
use gensokyo_qq::dto::interaction::Interaction;
use gensokyo_storage::{
    BotStatsStore, EchoStore, IdMapStore, StorageError, UnionCache, UserInfoRow,
};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::state::AppState;

/// QQ 回调外层的 `{ "id", "op", "s", "t", "d" }` 信封；这里只关心 `id` / `t` / `d`。
#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    t: Option<String>,
    #[serde(default)]
    d: Option<Value>,
}

/// `null` 数组按空数组收：Go 的 `[]*T` 反序列化容忍整数组 `null`，serde 的
/// `Vec<Option<T>>` + `default` 只容忍缺字段与元素 `null`，整数组 `null` 会 Err 丢整条事件。
fn null_as_default<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(d)?.unwrap_or_default())
}

/// QQ 消息负载（`GROUP_AT_MESSAGE_CREATE` / `GROUP_MESSAGE_CREATE` / `C2C_MESSAGE_CREATE`）。
#[derive(Debug, Deserialize)]
struct QqMessage {
    #[serde(default)]
    id: String,
    #[serde(default)]
    group_id: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    timestamp: Option<Value>,
    #[serde(default)]
    author: Option<QqAuthor>,
    #[serde(default)]
    mentions: Vec<QqMention>,
    #[serde(default, deserialize_with = "null_as_default")]
    attachments: Vec<Option<QqAttachment>>,
    #[serde(default)]
    ark_data: Option<QqArkData>,
    /// `0`=普通文本 `3`=结构化卡片 `101`=并行消息 `102`=聊天记录 `103`=引用消息。
    #[serde(default)]
    message_type: i64,
    #[serde(default, deserialize_with = "null_as_default")]
    msg_elements: Vec<Option<QqMsgElement>>,
}

/// 消息附件（`attachments[]`，`botgo/dto/message.go:169-180`）。
///
/// 元素按 `Option` 收：Go 是 `[]*MessageAttachment`，nil 元素在两处循环里都被跳过，
/// `null` 元素不该让整条事件解析失败。
#[derive(Debug, Deserialize)]
struct QqAttachment {
    #[serde(default)]
    content_type: String,
    /// JSON 键是 `filename`（不是 `file_name`）。
    #[serde(default, rename = "filename")]
    file_name: String,
    #[serde(default)]
    url: String,
    /// 语音消息 SILK 转 WAV 后的 URL。
    #[serde(default)]
    voice_wav_url: String,
    /// 语音消息 ASR 参考结果。
    #[serde(default)]
    asr_refer_text: String,
}

/// 结构化卡片数据（`ark_data`，`botgo/dto/message.go:183-192`）。
#[derive(Debug, Deserialize)]
struct QqArkData {
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    ark_type: String,
    #[serde(default)]
    ark_name: String,
    /// 卡片字段（`tag` / `title` / `desc` / `jump_url` …），原样透传。
    #[serde(default)]
    fields: Option<Value>,
}

/// 嵌套消息元素（`msg_elements[]`，`botgo/dto/message.go:195-210`）：递归结构，
/// 引用消息的 `msg_idx` 与正文都在这里。
#[derive(Debug, Deserialize)]
struct QqMsgElement {
    #[serde(default)]
    msg_idx: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    message_type: i64,
    #[serde(default)]
    ark_data: Option<QqArkData>,
    #[serde(default, deserialize_with = "null_as_default")]
    attachments: Vec<Option<QqAttachment>>,
    #[serde(default, deserialize_with = "null_as_default")]
    msg_elements: Vec<Option<QqMsgElement>>,
}

/// 消息 `mentions[]` 的一项：群场景用 OpenID 标识被 @ 的用户，
/// `is_you` / `bot` 是唯一可靠的 @bot 信号（与 Ready 事件的 `me.id` 形态不同）。
#[derive(Debug, Deserialize)]
struct QqMention {
    #[serde(default)]
    id: String,
    #[serde(default)]
    is_you: bool,
    #[serde(default)]
    bot: bool,
}

/// QQ 消息作者。
#[derive(Debug, Deserialize)]
struct QqAuthor {
    #[serde(default)]
    id: String,
    /// 联合 ID（`union_openid`）：`union_id` 模式下列给客户端的是它，markdown @ 只认 `id`，
    /// 两者的对应关系交给 `unioncache`（`GROUP_MEMBER_*` 的负载没有这个字段）。
    #[serde(default)]
    union_openid: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    member_role: Option<String>,
}

/// 群操作类事件（`GROUP_ADD_ROBOT` / `GROUP_DEL_ROBOT` / `GROUP_MSG_REJECT` / `GROUP_MSG_RECEIVE`）。
#[derive(Debug, Deserialize)]
struct QqGroupOp {
    #[serde(default)]
    event_id: String,
    #[serde(default)]
    group_openid: String,
    #[serde(default)]
    op_member_openid: String,
    #[serde(default)]
    timestamp: Option<Value>,
}

/// 好友添加事件（`FRIEND_ADD`）。
#[derive(Debug, Deserialize)]
struct QqFriendAdd {
    #[serde(default)]
    event_id: String,
    #[serde(default)]
    openid: String,
    #[serde(default)]
    timestamp: Option<Value>,
    #[serde(default)]
    scene: i64,
    #[serde(default)]
    scene_param: String,
}

/// 单向用户事件（`FRIEND_DEL` / `C2C_MSG_REJECT` / `C2C_MSG_RECEIVE`）。
#[derive(Debug, Deserialize)]
struct QqUserOp {
    #[serde(default)]
    openid: String,
    #[serde(default)]
    timestamp: Option<Value>,
}

/// 群成员变动事件（`GROUP_MEMBER_ADD` / `GROUP_MEMBER_REMOVE`）。
#[derive(Debug, Deserialize)]
struct QqGroupMember {
    #[serde(default)]
    event_id: String,
    #[serde(default)]
    group_openid: String,
    #[serde(default)]
    member_openid: String,
    #[serde(default)]
    timestamp: Option<Value>,
}

/// 入群申请事件（`GROUP_JOIN_REQUEST`）。
#[derive(Debug, Deserialize)]
struct QqJoinRequest {
    #[serde(default)]
    join_request_id: String,
    #[serde(default)]
    group_openid: String,
    #[serde(default)]
    member_openid: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    apply_at: Option<Value>,
    #[serde(default)]
    verify_info: Option<Value>,
    #[serde(default)]
    auto_approved: Option<Value>,
}

/// 群消息的两种来源；只影响 `to_me` 与 `is_full_group_message`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupFlavor {
    /// `GROUP_AT_MESSAGE_CREATE`：必然 @ 了机器人。
    At,
    /// `GROUP_MESSAGE_CREATE`：全量群消息（通常没有 @ 机器人）。
    Full,
}

/// 群成员变动的两个方向（`GROUP_MEMBER_ADD` / `GROUP_MEMBER_REMOVE`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupMemberFlavor {
    Add,
    Remove,
}

/// `<@OpenID>` 反解的判定素材：事件 `mentions[]` + 消息作者 + 事件形态。
struct MentionContext<'a> {
    /// 事件 `mentions[]`：`is_you` / `bot` 标记是 OpenID 形态下 @bot 的唯一可靠信号。
    mentions: &'a [QqMention],
    /// 消息作者 OpenID：作者本人永不当作 @bot。
    author_id: &'a str,
    /// `GROUP_MESSAGE_CREATE`：@bot 恒剥离，与 `remove_at` 无关。
    full_group: bool,
}

/// 一条消息事件里由 idmap 派生的四个 ID（P35：四个写入合并成一个写事务）。
struct DerivedIds {
    /// `sender.user_id`：群消息的 string 形态恒 `0`（Go M1-B 口径），其余取 vUIN。
    sender_user_id: Ob11Id,
    /// 事件 `user_id`。
    user_id: Ob11Id,
    /// 事件 `group_id`；私聊恒空串（调用方不读）。
    group_id: Ob11Id,
    /// 事件 `message_id`（虚拟消息 ID）。
    message_id: Ob11Id,
}

impl DerivedIds {
    /// 全部回退原始字符串：idmap 未注入或批量写入失败时用。
    fn raw(
        string_ob11: bool,
        user_raw: &str,
        group_raw: Option<&str>,
        message_raw: &str,
        group_sender: bool,
    ) -> Self {
        Self {
            sender_user_id: if group_sender && string_ob11 {
                Ob11Id::num(0)
            } else {
                Ob11Id::Str(user_raw.to_owned())
            },
            user_id: Ob11Id::Str(user_raw.to_owned()),
            group_id: Ob11Id::Str(group_raw.unwrap_or_default().to_owned()),
            message_id: Ob11Id::Str(message_raw.to_owned()),
        }
    }
}

/// 入站转换器：配置快照 + 可选 idmap / echo / botstats / BotID。
#[derive(Debug, Clone)]
pub struct Inbound {
    config: Arc<ConfigManager>,
    idmap: Option<Arc<IdMapStore>>,
    echo: Option<Arc<EchoStore>>,
    botstats: Option<Arc<BotStatsStore>>,
    /// 机器人真实 ID（`GET /users/@me`）；`None` 时正文不做 BotID→AppID 还原。
    bot_id: Option<Arc<str>>,
}

impl Inbound {
    /// 用配置与可选 idmap 构造。
    #[must_use]
    pub fn new(config: Arc<ConfigManager>, idmap: Option<Arc<IdMapStore>>) -> Self {
        Self {
            config,
            idmap,
            echo: None,
            botstats: None,
            bot_id: None,
        }
    }

    /// 从 [`AppState`] 取配置、idmap、echo、botstats 与 BotID（webhook 每个请求现构造，Arc 克隆开销可忽略）。
    #[must_use]
    pub fn from_state(state: &AppState) -> Self {
        Self {
            config: state.config_arc(),
            idmap: state.idmap_arc(),
            echo: state.echo_arc(),
            botstats: state.botstats_arc(),
            bot_id: state.bot_id_arc(),
        }
    }

    /// 挂上 echo 存储：消息事件入站时喂懒 message_id 池与 msg_id / event_id 映射。
    ///
    /// 未挂载时入站照常上报（池是「被动回复凭据」的加速层，不是上报的前置条件）。
    #[must_use]
    pub fn with_echo(mut self, echo: Arc<EchoStore>) -> Self {
        self.echo = Some(echo);
        self
    }

    /// 挂上机器人真实 ID（`GET /users/@me`，Go `main.go:304` 的 `handlers.BotID`）。
    ///
    /// 未挂载时正文原样上报（对齐 Go 取不到 `Me` 时的降级）。
    #[must_use]
    pub fn with_bot_id(mut self, bot_id: impl Into<Arc<str>>) -> Self {
        self.bot_id = Some(bot_id.into());
        self
    }

    /// 机器人自身 ID（`use_uin ? uin : app_id`）。
    #[must_use]
    pub fn self_id(&self) -> i64 {
        self.self_id_of(&self.config.snapshot())
    }

    /// 记一条收到的消息（Go `botstats.RecordMessageReceived`：内存指标 + 当日计数落盘）。
    ///
    /// Go 在三个 WS 事件处理器里各调一次（`main.go:763` 群 AT、`:785` C2C、`:1135` 群全量），
    /// 且都包在 `if !config.GetDisableErrorChan()` 里——那个开关在 Go 被当成统计开关用，这里
    /// 照搬。与 Go 的差异：Go 的 webhook 回调（`server/webhook.go`）不记账，这里在转换的唯一
    /// 入口记账，四种传输到达的消息事件都记，口径是「每条到达的消息事件一次」。
    fn record_message_received(&self, settings: &Settings) {
        if settings.get_disable_error_chan() {
            return;
        }
        metrics::MESSAGES_RECEIVED.inc();
        let Some(stats) = self.botstats.as_deref() else {
            return;
        };
        if let Err(err) = stats.record_received() {
            tracing::warn!(error = %err, "botstats 写入失败，本次接收只计内存指标");
        }
    }

    /// 把 QQ 回调原文转成零条或多条 OneBot 事件。
    ///
    /// 顶层 JSON 非法（不是对象 / 语法错）返回 `Err`；结构合法但 `t` 缺失或未知返回
    /// `Ok(vec![])`——「看不懂」不等于「报文坏了」，调用方分别映射 400 与 200。
    ///
    /// # Errors
    ///
    /// 仅在整段 JSON 无法解析成 [`Envelope`] 时返回 [`serde_json::Error`]。
    pub fn convert_json(&self, raw: &[u8]) -> Result<Vec<Event>, serde_json::Error> {
        let envelope: Envelope = serde_json::from_slice(raw)?;
        let Some(kind) = envelope.t.as_deref() else {
            tracing::debug!("QQ 事件缺少 t 字段，忽略");
            return Ok(Vec::new());
        };
        self.convert(kind, envelope.d.as_ref(), envelope.id.as_deref())
    }

    /// 同 [`Inbound::convert_json`]，输入是已解析的信封字段（测试与嵌入场景）。
    ///
    /// # Errors
    ///
    /// 负载字段类型与已知事件不符时返回 [`serde_json::Error`]。
    pub fn convert(
        &self,
        kind: &str,
        data: Option<&Value>,
        envelope_id: Option<&str>,
    ) -> Result<Vec<Event>, serde_json::Error> {
        let Some(data) = data else {
            tracing::debug!(kind, "QQ 事件缺少 d 字段，忽略");
            return Ok(Vec::new());
        };
        let settings = self.config.snapshot();
        // Go `main.go:765-770` / `:787-792` / `:1124-1129`：三个消息事件在 `Process*` 之前用
        // `acnode.CheckWordIN` 过滤正文与昵称。载荷先改写再解析，所以 `add_at_group` 补的
        // `[CQ:at,qq=…]` 前缀不会进入过滤（与 Go 的先后顺序一致）；开关逐事件读，热重载生效。
        let filtered = if settings.get_enable_change_word() {
            filtered_message_payload(kind, data)
        } else {
            None
        };
        let data = filtered.as_ref().unwrap_or(data);
        match kind {
            "GROUP_AT_MESSAGE_CREATE" => {
                let event = self.group_message(&settings, data, GroupFlavor::At)?;
                self.record_message_received(&settings);
                Ok(vec![event])
            }
            "GROUP_MESSAGE_CREATE" => {
                let event = self.group_message(&settings, data, GroupFlavor::Full)?;
                self.record_message_received(&settings);
                Ok(vec![event])
            }
            "C2C_MESSAGE_CREATE" => {
                let event = self.private_message(&settings, data)?;
                self.record_message_received(&settings);
                Ok(vec![event])
            }
            "GROUP_ADD_ROBOT" => self.group_add_robot(&settings, data),
            "GROUP_DEL_ROBOT" => {
                self.group_op_notice(&settings, data, "group_decrease", Some("kick_me"))
            }
            "GROUP_MSG_REJECT" => self.group_payload_notice(&settings, data, "group_reject"),
            "GROUP_MSG_RECEIVE" => self.group_payload_notice(&settings, data, "group_receive"),
            "FRIEND_ADD" => self.friend_add(&settings, data, envelope_id),
            "FRIEND_DEL" => self.user_op_notice(&settings, data, "friend_decrease"),
            "C2C_MSG_REJECT" => self.user_op_notice(&settings, data, "c2c_msg_reject"),
            "C2C_MSG_RECEIVE" => self.user_op_notice(&settings, data, "c2c_msg_receive"),
            "GROUP_MEMBER_ADD" => {
                self.group_member(&settings, data, envelope_id, GroupMemberFlavor::Add)
            }
            "GROUP_MEMBER_REMOVE" => {
                self.group_member(&settings, data, envelope_id, GroupMemberFlavor::Remove)
            }
            "GROUP_JOIN_REQUEST" => self.group_join_request(&settings, data),
            "INTERACTION_CREATE" => self.interaction(&settings, data),
            other => {
                tracing::debug!(kind = other, "未支持的 QQ 事件，已忽略");
                Ok(Vec::new())
            }
        }
    }

    /// `meta_event/lifecycle/connect`：连接建立后的首帧（Go `NewWebSocketClient`）。
    #[must_use]
    pub fn lifecycle_connect(&self) -> Event {
        let settings = self.config.snapshot();
        Event::MetaEvent(MetaEvent {
            time: now_unix(),
            self_id: Ob11Id::num(self.self_id_of(&settings)),
            meta_event_type: "lifecycle".to_owned(),
            sub_type: Some("connect".to_owned()),
            interval: None,
            status: None,
            extra: Extra::new(),
        })
    }

    /// `meta_event/heartbeat`；`interval_ms` 与 Go 一致按毫秒上报。
    #[must_use]
    pub fn heartbeat(&self, interval_ms: i64) -> Event {
        let settings = self.config.snapshot();
        let mut extra = Extra::new();
        extra.insert("app_enabled".to_owned(), Value::Bool(true));
        extra.insert("app_good".to_owned(), Value::Bool(true));
        extra.insert("app_initialized".to_owned(), Value::Bool(true));
        Event::MetaEvent(MetaEvent {
            time: now_unix(),
            self_id: Ob11Id::num(self.self_id_of(&settings)),
            meta_event_type: "heartbeat".to_owned(),
            sub_type: None,
            interval: Some(interval_ms),
            status: Some(HeartbeatStatus {
                online: true,
                good: true,
                extra,
            }),
            extra: Extra::new(),
        })
    }

    fn self_id_of(&self, settings: &Settings) -> i64 {
        if settings.get_use_uin() {
            settings.get_uin()
        } else {
            i64::try_from(settings.get_app_id()).unwrap_or(i64::MAX)
        }
    }

    /// 入站正文的 mention 处理：`<@OpenID>` 反解 → 自身 @ 剥离 → trim → 去首个 `/`。
    ///
    /// 顺序照 Go `RevertTransformedText`（字符串路径）与 `ConvertToSegmentedMessage`
    /// （段数组路径）：两条路径都从 `data.Content` 重算，先逐处反解/剥离 mention，再按
    /// 「全量群消息或 `remove_at`」trim 一次，最后按 `remove_prefix` 去掉第一个 `/`。
    /// mention 一律**原位**替换或剥离，不前移、不重排；反查失败的 `<@OpenID>` 原样留在原位。
    ///
    /// `string_form` 选形态，两形态的差异只在开头与 trim 条件：
    ///
    /// - `true`（字符串路径）：`" "` / `"/ "` / `" / "` 归一成 `" "`，并先 trim 前后空白。
    /// - `false`（段数组路径）：只有 `" "` 被保护，没有初始 trim（Go 只在 at 处理之后
    ///   按 `is_full_group_message || remove_at` trim）。
    ///
    /// 自身 @ 的剥离范围：全量群消息（`GROUP_MESSAGE_CREATE`）**恒剥离**，与 `remove_at`
    /// 无关；`GROUP_AT_MESSAGE_CREATE` 只在 `remove_at = true` 时剥离，否则还原成
    /// `[CQ:at,qq=<self_id>]`（Go `message_parser.go:1876-1883` / `:2262-2276`）。
    fn revert_mentions(
        &self,
        settings: &Settings,
        content: &str,
        ctx: &MentionContext<'_>,
        string_form: bool,
    ) -> String {
        // 「保护态」正文：用户就想发一个空格/斜杠，前后两次 trim 都不碰。
        let (text, protected) = match (string_form, content) {
            (_, " ") | (true, "/ " | " / ") => (" ".to_owned(), true),
            (true, _) => (content.trim().to_owned(), false),
            (false, _) => (content.to_owned(), false),
        };
        let strip_self = ctx.full_group || settings.get_remove_at();
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for (range, user) in scan_mentions(&text) {
            out.push_str(&text[last..range.start]);
            let resolved = self.resolve_mention(settings, user);
            if self.is_self_mention(settings, ctx, user, resolved.as_deref()) {
                if !strip_self {
                    // mentions 判定的 @bot 可能不在 idmap 里（反查失败），退回 self_id 形态
                    let qq = resolved.unwrap_or_else(|| self.self_id_of(settings).to_string());
                    out.push_str(&format!("[CQ:at,qq={qq}]"));
                }
            } else if let Some(qq) = resolved {
                out.push_str(&format!("[CQ:at,qq={qq}]"));
            } else {
                // 未开启转换 / idmap 查不到：原样保留在原位
                out.push_str(&text[range.clone()]);
            }
            last = range.end;
        }
        out.push_str(&text[last..]);

        let mut text = out;
        if strip_self && !protected {
            // 剥掉开头的 @ 会留下它后面的空格（Go `message_parser.go:1893-1900`）。
            text = text.trim().to_owned();
        }
        if settings.get_remove_prefix()
            && let Some(idx) = text.find('/')
        {
            // Go 去掉的是正文里**第一个** `/`，不限位置（`message_parser.go:1906-1911`）。
            text.remove(idx);
        }
        text
    }

    /// `<@user>` 是否为 @bot 自身（Go `isIncomingSelfAt`）。
    ///
    /// 作者本人永不当作 @bot：`is_you` 在多实例等场景会误标发送者，误判会把发送者的 @
    /// 当 @bot 剥掉。其余依次看事件 `mentions[]` 的 `is_you`/`bot` 标记、BotID / AppID
    /// 直等，最后兼容「解析结果等于 `uin` 或 `app_id`」。
    fn is_self_mention(
        &self,
        settings: &Settings,
        ctx: &MentionContext<'_>,
        user: &str,
        resolved: Option<&str>,
    ) -> bool {
        if user.is_empty() || user == ctx.author_id {
            return false;
        }
        if ctx
            .mentions
            .iter()
            .any(|mention| mention.id == user && (mention.is_you || mention.bot))
        {
            return true;
        }
        if self.is_known_self(settings, user) {
            return true;
        }
        resolved.is_some_and(|qq| is_own_id(settings, qq))
    }

    /// `<@user>` → OneBot `qq` 值（Go `resolveIncomingAtID`）。
    ///
    /// 自身 → `self_id` 形态（`use_uin ? uin : app_id`）；他人 → idmap **只查不分配**的
    /// vUIN（`convert_other_at = false` 或查不到映射都返回 `None`，正文原样保留）。
    fn resolve_mention(&self, settings: &Settings, user: &str) -> Option<String> {
        if self.is_known_self(settings, user) {
            return Some(self.self_id_of(settings).to_string());
        }
        if !settings.get_convert_other_at() {
            return None;
        }
        match self.idmap.as_deref()?.lookup_virtual_value(user) {
            Ok(vuin) => Some(vuin.to_string()),
            Err(err) => {
                tracing::debug!(identity = user, error = %err, "mention 反查失败，正文保留原样");
                None
            }
        }
    }

    /// BotID / AppID 直等的自身判定（Go `IsSelfAtID` 里不依赖 mentions 的那部分）。
    fn is_known_self(&self, settings: &Settings, user: &str) -> bool {
        self.bot_id.as_deref() == Some(user) || is_own_id(settings, user)
    }

    /// notice / request 事件的 group_id / user_id：Go 这些 handler 一律
    /// `idmap.StoreIDv2` 取**数字 vUIN**，与 `string_ob11` 无关（string 形态只影响
    /// message 事件）。idmap 未注入时回退原始字符串，不伪造数字。
    fn vuid(&self, raw: &str) -> Ob11Id {
        if raw.is_empty() {
            return Ob11Id::Str(String::new());
        }
        let Some(idmap) = self.idmap.as_deref() else {
            return Ob11Id::Str(raw.to_owned());
        };
        match idmap.retrieve_virtual_value(raw) {
            Ok((_, vuin)) => Ob11Id::Num(vuin),
            Err(err) => {
                tracing::warn!(identity = raw, error = %err, "idmap 分配虚拟 ID 失败，回退原始字符串");
                Ob11Id::Str(raw.to_owned())
            }
        }
    }

    /// 群事件上报形态 → idmap 注册表（`<section>:type = group`）。
    ///
    /// 对齐 Go `WriteConfigv2(GroupID 字符串, "type", "group")` 的注册点：section 直接取
    /// 上报形态本身——数字形态是十进制 vuin，字符串形态是原始 OpenID。
    /// 写失败只 warn，绝不阻断事件上报：漏注册只影响 `get_group_list` 的可见性。
    fn register_group(&self, group_id: &Ob11Id) {
        let Some(store) = self.idmap.as_deref() else {
            return;
        };
        // Display 只输出内部值：Num → 十进制串，Str → 原串（与上报形态一致）
        let section = group_id.to_string();
        if section.is_empty() {
            return;
        }
        if let Err(err) = store.write_config(&section, "type", "group") {
            tracing::warn!(section = %section, error = %err, "群事件注册进 idmap 失败，不影响事件上报");
        }
    }

    /// C2C 私信写一行 `user_info`（Go `ProcessC2CMessage.go:231-241` → `idmap.StoreUserInfo`）。
    ///
    /// `raw_user` 是事件里的 OpenID（Go `data.Author.ID`，C2C 不做 union 切换）；`user_id` 取
    /// **上报形态**——数字模式是虚拟号十进制串，`string_ob11` 下是原始 OpenID，与 Go `:231-234`
    /// 的 `struserid` 一致。昵称 / 备注 Go 恒写空串（`:235-239`），这里照抄零值。
    ///
    /// Go 不检查返回值（`:241` 直接丢弃 err），因此写失败绝不阻断事件上报。重复键
    /// [`StorageError::DuplicateKey`] 是稳态（同一好友的第二条私信起必然命中，见
    /// `idmap/service.go:2271-2272`），压到 debug；其余失败——含真·行编码损坏的
    /// [`StorageError::Corrupt`]——告警后继续。未装配 idmap 时静默跳过——没有本地库可写。
    ///
    /// 空 `raw_user` 直接返回，不写库也不记日志：Go `ProcessC2CMessage.go:30-33` 对
    /// `data.Author == nil || data.Author.ID == ""` 是 `return nil`（整条事件根本不入库），
    /// 这里取其"不写"的一半——空 OpenID 写进 `user_info` 只会产出键 `":"` 的垃圾行，Go 库里
    /// 绝不会有这一行（`derive_message_ids` 已为同类空身份行设守卫）。Go 的判定是 `== ""`、
    /// 不含 trim，本守卫同样只判空串。
    fn record_user_info(&self, raw_user: &str, user_id: &Ob11Id) {
        // Go `ProcessC2CMessage.go:30-33`：Author 为空 ⇒ 早退，不入库
        if raw_user.is_empty() {
            return;
        }
        let Some(store) = self.idmap.as_deref() else {
            return;
        };
        let row = UserInfoRow {
            user_id: user_id.to_string(),
            ..UserInfoRow::default()
        };
        if let Err(err) = store.store_user_info(raw_user, &row) {
            match err {
                StorageError::DuplicateKey(_) => {
                    tracing::debug!(user = raw_user, error = %err, "user_info 行已存在，跳过重复写入");
                }
                err => {
                    tracing::warn!(
                        user = raw_user,
                        error = %err,
                        "写 user_info 失败，get_friend_list 看不到该用户"
                    );
                }
            }
        }
    }

    /// 一条消息事件要落的 idmap 写入（群/用户身份、message_id、群注册）合并成一次批量提交。
    ///
    /// 与逐条提交（每个字段一次写事务）相比只有提交次数不同：成功路径逐项结果一致（同一批内
    /// 重复解析同一身份命中本批已写入的行，不会重号）。身份库与消息库是两个 redb 文件，
    /// **跨库非原子**：身份库先提交，消息库提交失败时身份写入保留（与 Go 两个 bbolt 库各自
    /// `Update` 同口径）；失败时本事件上报仍整组回退原始字符串，但已提交的身份行不回滚。
    ///
    /// `group_sender`：群消息的 `sender.user_id` 在 string 形态下恒 `0`（Go M1-B），私聊照常
    /// 取 idmap。
    fn derive_message_ids(
        &self,
        settings: &Settings,
        user_raw: &str,
        group_raw: Option<&str>,
        message_raw: &str,
        group_sender: bool,
    ) -> DerivedIds {
        let string_ob11 = settings.get_string_ob11();
        let Some(idmap) = self.idmap.as_deref() else {
            return DerivedIds::raw(string_ob11, user_raw, group_raw, message_raw, group_sender);
        };
        let derived = idmap.with_write_batch(|batch| {
            // 派生顺序对齐 Go `ProcessGroupMessage.go:74-75`：先群、后用户。
            let group_id = match (group_raw, string_ob11) {
                (None | Some(""), _) => Ob11Id::Str(String::new()),
                (Some(raw), true) => Ob11Id::Str(raw.to_owned()),
                (Some(raw), false) => Ob11Id::Num(batch.retrieve_virtual_value(raw)?.1),
            };
            // 空身份串不过 idmap：Go `StoreIDv2("")` 与旧的逐条路径都会原样上报空串，
            // 让 store 去分配只会写出一行 `raw:` 的垃圾身份（旧 `ob11_id` 的 `raw.is_empty()` 分支）
            let user_id = if string_ob11 || user_raw.is_empty() {
                Ob11Id::Str(user_raw.to_owned())
            } else {
                Ob11Id::Num(batch.retrieve_virtual_value(user_raw)?.1)
            };
            let sender_user_id = if group_sender && string_ob11 {
                Ob11Id::num(0)
            } else {
                user_id.clone()
            };
            // 空 message_id 同样不过 idmap：`store_cache` 对空串报错，会把整批身份写入一起回滚
            let message_id = if string_ob11 || message_raw.is_empty() {
                Ob11Id::Str(message_raw.to_owned())
            } else {
                Ob11Id::Num(batch.store_cache(message_raw)?)
            };
            // 群注册（Go `WriteConfigv2(group, "type", "group")`）：section 取上报形态本身
            let section = group_id.to_string();
            if !section.is_empty() {
                batch.write_config(&section, "type", "group")?;
            }
            Ok(DerivedIds {
                sender_user_id,
                user_id,
                group_id,
                message_id,
            })
        });
        match derived {
            Ok(ids) => ids,
            Err(err) => {
                tracing::warn!(
                    user = user_raw,
                    error = %err,
                    "idmap 批量写入失败，本事件的虚拟 ID 回退原始字符串"
                );
                DerivedIds::raw(string_ob11, user_raw, group_raw, message_raw, group_sender)
            }
        }
    }

    /// 群消息喂 echo（Go `ProcessGroupMessage.go:283-302`、`ProcessGroupNormalMessage.go:262-273`）。
    ///
    /// 数字形态用虚拟号做键；`string_action` 时再补一份原始 OpenID 键（Go string 分支只写后者），
    /// 该键在 `union_id` 打开时是**切换后**的值（Go `:302` 读的是 `data.Author.ID`）。
    /// echo 是内存表、没有失败路径；未装配时静默跳过，绝不影响事件上报。
    fn feed_group_echo(
        &self,
        settings: &Settings,
        payload: &QqMessage,
        user_raw: &str,
        group_id: &Ob11Id,
        user_id: &Ob11Id,
        message_id: &Ob11Id,
    ) {
        let Some(echo) = self.echo.as_deref() else {
            return;
        };
        let app_id = settings.get_app_id().to_string();
        let now = SystemTime::now();

        // 懒 message_id 池：群维度 + 群 × 用户维度（被动回复复用同一 msg_id）。
        let group_key = group_id.to_string();
        echo.add_lazy_message_id(&group_key, &payload.id, now);
        echo.add_lazy_message_id_v2(&group_key, &user_id.to_string(), &payload.id, now);
        if settings.get_string_action() && group_key != payload.group_id {
            echo.add_lazy_message_id(&payload.group_id, &payload.id, now);
            echo.add_lazy_message_id_v2(&payload.group_id, user_raw, &payload.id, now);
        }

        // msg_id / msg_type 映射：数字形态用虚拟群号 / 群 × 用户做键。Go 的
        // `AddMsgID(app_id, s, …)` 那一路用的是 botgo 全局序列号（`client.GetGlobalS()`），
        // Rust 侧没有该计数器，这里用上报的数字 message_id 顶替——客户端回传的正是它。
        if let Some(s) = message_id.as_i64() {
            echo.add_msg_id(&app_id, s, &payload.id);
            echo.add_msg_type(&app_id, s, "group");
        }
        if let (Some(group), Some(user)) = (group_id.as_i64(), user_id.as_i64()) {
            echo.add_msg_id(&app_id, group, &payload.id);
            echo.add_msg_id_v2(&app_id, group, user, &payload.id);
            echo.add_msg_type(&app_id, group, "group");
        }
    }

    /// 私聊消息喂 echo（Go `ProcessC2CMessage.go:164-174,213-218`）。
    ///
    /// Go 私聊只有 v1 懒池槽，键放虚拟用户号（string 形态放原始 OpenID），没有 v2 维度。
    fn feed_private_echo(
        &self,
        settings: &Settings,
        payload: &QqMessage,
        user_id: &Ob11Id,
        message_id: &Ob11Id,
    ) {
        let Some(echo) = self.echo.as_deref() else {
            return;
        };
        let app_id = settings.get_app_id().to_string();
        let raw_user = payload
            .author
            .as_ref()
            .map_or("", |author| author.id.as_str());
        let numeric = user_id.as_i64();

        echo.add_lazy_message_id(&user_id.to_string(), &payload.id, SystemTime::now());
        if let Some(s) = message_id.as_i64() {
            echo.add_msg_id(&app_id, s, &payload.id);
            echo.add_msg_type(&app_id, s, "group_private");
        }
        if let Some(user) = numeric {
            echo.add_msg_id(&app_id, user, &payload.id);
            echo.add_msg_type(&app_id, user, "group_private");
        } else if !raw_user.is_empty() {
            // Go string 形态：`AddMsgIDv3(AppIDString, data.Author.ID, data.ID)`。
            echo.add_msg_id_v3(&app_id, raw_user, &payload.id);
        }
    }

    fn group_message(
        &self,
        settings: &Settings,
        data: &Value,
        flavor: GroupFlavor,
    ) -> Result<Event, serde_json::Error> {
        let payload = QqMessage::deserialize(data)?;
        let author = payload.author.as_ref();
        let raw_user = author.map_or("", |author| author.id.as_str());
        let union_openid = author.map_or("", |author| author.union_openid.as_str());

        // Go `ProcessGroupMessage.go:53-56` / `ProcessGroupNormalMessage.go:44-47`：在 ID 切换与
        // idmap 转换**之前**用原始 id 建映射（顺序即语义：反查出来必须是原始 OpenID）。
        // 空键由 `store` 自己忽略，与 Go 的 `if union != "" && id != ""` 等价。
        UnionCache::shared().store(raw_user, union_openid);

        // 切换（Go `ProcessGroupMessage.go:58-64` / `ProcessGroupNormalMessage.go:49-54`）：
        // `union_id` 打开时，上报与 idmap 一律用 union——下面所有 `user_raw` 都在切换之后取值，
        // 与 Go 把它写回 `data.Author.ID` 后再算一致。
        // 差异：Go 在 union 为空时会无条件置空 `author.id`（`:60`），这里保留原 id——空 ID
        // 事件对客户端只是噪声，这条差异登记在 `reports/refactor/17-go-rust-parity.md` §E-3。
        let user_raw = if settings.get_union_id() && !union_openid.is_empty() {
            union_openid
        } else {
            raw_user
        };

        let mut extra = Extra::new();
        if !settings.get_native_ob11() {
            extra.insert(
                "real_user_id".to_owned(),
                Value::String(user_raw.to_owned()),
            );
            extra.insert(
                "real_group_id".to_owned(),
                Value::String(payload.group_id.clone()),
            );
            extra.insert(
                "real_message_type".to_owned(),
                Value::String("group".to_owned()),
            );
        }
        if flavor == GroupFlavor::Full {
            extra.insert("is_full_group_message".to_owned(), Value::Bool(true));
        }
        // Go `OnebotGroupMessageS` 的 `platform` 非 omitempty：string 形态恒上报；
        // 数字形态的 `OnebotGroupMessage` 根本没有该字段。`union_id` 打开时 Go 报 `unionqq`
        // （`ProcessGroupMessage.go:58-64`）。
        if settings.get_string_ob11() {
            let platform = if settings.get_union_id() {
                "unionqq"
            } else {
                "qq"
            };
            extra.insert("platform".to_owned(), Value::String(platform.to_owned()));
        }

        // P35：用户/群身份、message_id 与群注册合并成一个写事务（成功路径逐项与逐条提交一致）
        let DerivedIds {
            sender_user_id,
            user_id,
            group_id,
            message_id,
        } = self.derive_message_ids(
            settings,
            user_raw,
            Some(&payload.group_id),
            &payload.id,
            true,
        );

        let sender = Sender {
            // Go M1-B：string 形态不做 idmap 转换，`sender.user_id` 恒为 0（见 `derive_message_ids`）。
            user_id: Some(sender_user_id),
            nickname: Some(
                author
                    .map_or("", |author| author.username.as_str())
                    .to_owned(),
            ),
            role: Some(
                author
                    .and_then(|author| author.member_role.clone())
                    .filter(|role| !role.is_empty())
                    .unwrap_or_else(|| "member".to_owned()),
            ),
            sex: Some("0".to_owned()),
            level: Some("0".to_owned()),
            // Go `Sender.TinyID` 非 omitempty：未取到时上报空串。
            tiny_id: Some(String::new()),
            ..Sender::default()
        };

        self.feed_group_echo(
            settings,
            &payload,
            user_raw,
            &group_id,
            &user_id,
            &message_id,
        );
        let ctx = MentionContext {
            mentions: &payload.mentions,
            author_id: user_raw,
            full_group: flavor == GroupFlavor::Full,
        };
        // 媒体码在 `RevertTransformedText` 末尾追加（图片在前、事件码在后），`add_at_group`
        // 前缀在其后才前置：字符串形态的 `message` 与 `raw_message` 都是这一串
        let media = media_cq_text(&payload);
        let content = bot_id_to_app_id(settings, &payload.content, self.bot_id.as_deref());
        let text = self.revert_mentions(settings, &content, &ctx, true);
        let string_form = format!("{text}{media}");
        let raw_message = reported_group_content(settings, &string_form, flavor);
        // 段数组形态与字符串形态不同源：Go `ConvertToSegmentedMessage` 从 `data.Content`
        // 重新处理，不做字符串路径的初始 trim、也不吃 `add_at_group` 前缀
        // （见 [`Inbound::revert_mentions`] 的 `string_form`）。关闭 `array_value` 时不读。
        let segment_source = if settings.get_array_value() {
            self.revert_mentions(settings, &content, &ctx, false)
        } else {
            String::new()
        };
        let message = reported_message(settings, &segment_source, &raw_message, &payload);
        Ok(Event::Message(MessageEvent {
            time: timestamp_of(payload.timestamp.as_ref()),
            self_id: Ob11Id::num(self.self_id_of(settings)),
            message_type: MessageType::Group,
            sub_type: Some("normal".to_owned()),
            message_id,
            user_id,
            group_id: Some(group_id),
            message,
            raw_message,
            font: Some(0),
            sender: Some(sender),
            anonymous: None,
            message_seq: Some(0),
            // Go `ToMe` 是 omitempty：只有 @ 机器人的 AT 事件才出现。
            to_me: (flavor == GroupFlavor::At).then_some(true),
            extra,
        }))
    }

    fn private_message(
        &self,
        settings: &Settings,
        data: &Value,
    ) -> Result<Event, serde_json::Error> {
        let payload = QqMessage::deserialize(data)?;
        let author = payload.author.as_ref();
        let user_raw = author.map_or("", |author| author.id.as_str());

        let mut extra = Extra::new();
        if !settings.get_native_ob11() {
            extra.insert(
                "real_user_id".to_owned(),
                Value::String(user_raw.to_owned()),
            );
            extra.insert(
                "real_message_type".to_owned(),
                Value::String("group_private".to_owned()),
            );
        }

        // P35：身份与 message_id 合并成一个写事务（私聊没有群注册）
        let DerivedIds {
            sender_user_id,
            user_id,
            message_id,
            ..
        } = self.derive_message_ids(settings, user_raw, None, &payload.id, false);

        // Go `ProcessC2CMessage.go:231-241`：私信顺手把好友写进 `user_info`（`get_friend_list`
        // 的数据源，纯 Rust 部署此前没有任何写入口）。Go 在 `dispatchInbound` 之后写，这里放在
        // 转换末尾：只动本地库，**事件上报 JSON 字节不变**；本包唯一的外部变化是
        // `get_friend_list` 开始出数据。
        self.record_user_info(user_raw, &user_id);

        let sender = Sender {
            user_id: Some(sender_user_id),
            // Go `PrivateSenderS`：nickname / tiny_id 非 omitempty，恒上报（空串）。
            nickname: Some(String::new()),
            tiny_id: Some(String::new()),
            ..Sender::default()
        };

        self.feed_private_echo(settings, &payload, &user_id, &message_id);
        let ctx = MentionContext {
            mentions: &[],
            author_id: user_raw,
            full_group: false,
        };
        let media = media_cq_text(&payload);
        let content = bot_id_to_app_id(settings, &payload.content, self.bot_id.as_deref());
        // 单聊在 Go 里同样走 `RevertTransformedText`（`ProcessC2CMessage.go:104`），
        // 只是没有 `mentions`、也不算全量群消息；媒体码同样接在正文之后。
        let text = self.revert_mentions(settings, &content, &ctx, true);
        let string_form = format!("{text}{media}");
        let segment_source = if settings.get_array_value() {
            self.revert_mentions(settings, &content, &ctx, false)
        } else {
            String::new()
        };
        let message = reported_message(settings, &segment_source, &string_form, &payload);
        Ok(Event::Message(MessageEvent {
            time: timestamp_of(payload.timestamp.as_ref()),
            self_id: Ob11Id::num(self.self_id_of(settings)),
            message_type: MessageType::Private,
            sub_type: Some("friend".to_owned()),
            message_id,
            user_id,
            group_id: None,
            message,
            raw_message: string_form,
            font: Some(0),
            sender: Some(sender),
            anonymous: None,
            message_seq: Some(0),
            to_me: None,
            extra,
        }))
    }

    fn group_add_robot(
        &self,
        settings: &Settings,
        data: &Value,
    ) -> Result<Vec<Event>, serde_json::Error> {
        let payload = QqGroupOp::deserialize(data)?;
        let time = timestamp_of(payload.timestamp.as_ref());
        let group_id = self.vuid(&payload.group_openid);
        self.register_group(&group_id);
        // Go `echo.AddEvnetID(app_id, 虚拟群号, event_id)`（ProcessGroupAddBot.go:157）
        if !payload.event_id.is_empty()
            && let Some(echo) = self.echo.as_deref()
        {
            let app_id = settings.get_app_id().to_string();
            if let Some(group) = group_id.as_i64() {
                echo.add_event_id(&app_id, group, &payload.event_id);
            } else {
                echo.add_event_id_v2(&app_id, &payload.group_openid, &payload.event_id);
            }
        }
        let user_id = self.vuid(&payload.op_member_openid);
        let extra = self.enhanced_extra(settings, &payload.op_member_openid, &payload.group_openid);

        // 对齐 Go `ProcessGroupAddBot`：先 request（方便客户端审批），再 notice。
        let request = Event::Request(RequestEvent {
            time,
            self_id: Ob11Id::num(self.self_id_of(settings)),
            request_type: "group".to_owned(),
            sub_type: Some("invite".to_owned()),
            user_id: Some(user_id.clone()),
            group_id: Some(group_id.clone()),
            comment: String::new(),
            flag: String::new(),
            extra: extra.clone(),
        });
        let notice = Event::Notice(NoticeEvent {
            time,
            self_id: Ob11Id::num(self.self_id_of(settings)),
            notice_type: "group_increase".to_owned(),
            sub_type: Some("invite".to_owned()),
            group_id: Some(group_id),
            user_id: Some(user_id),
            operator_id: Some(Ob11Id::num(0)),
            target_id: None,
            message_id: None,
            duration: None,
            file: None,
            card_new: None,
            card_old: None,
            honor_type: None,
            online: None,
            client: None,
            extra,
        });
        Ok(vec![request, notice])
    }

    fn group_op_notice(
        &self,
        settings: &Settings,
        data: &Value,
        notice_type: &str,
        sub_type: Option<&str>,
    ) -> Result<Vec<Event>, serde_json::Error> {
        let payload = QqGroupOp::deserialize(data)?;
        let extra = self.enhanced_extra(settings, &payload.op_member_openid, &payload.group_openid);
        let group_id = self.vuid(&payload.group_openid);
        self.register_group(&group_id);
        Ok(vec![Event::Notice(NoticeEvent {
            time: timestamp_of(payload.timestamp.as_ref()),
            self_id: Ob11Id::num(self.self_id_of(settings)),
            notice_type: notice_type.to_owned(),
            sub_type: sub_type.map(str::to_owned),
            group_id: Some(group_id),
            user_id: Some(self.vuid(&payload.op_member_openid)),
            operator_id: Some(Ob11Id::num(0)),
            target_id: None,
            message_id: None,
            duration: None,
            file: None,
            card_new: None,
            card_old: None,
            honor_type: None,
            online: None,
            client: None,
            extra,
        })])
    }

    /// `GROUP_MSG_REJECT` / `GROUP_MSG_RECEIVE`：Go 用 `OnebotGroupRejectNotice` /
    /// `OnebotGroupReceiveNotice` 上报——**没有** `operator_id`，整块 dto 落在 `data`。
    fn group_payload_notice(
        &self,
        settings: &Settings,
        data: &Value,
        notice_type: &str,
    ) -> Result<Vec<Event>, serde_json::Error> {
        let payload = QqGroupOp::deserialize(data)?;
        let mut extra =
            self.enhanced_extra(settings, &payload.op_member_openid, &payload.group_openid);
        // Go 的 `Data` 就是该事件的 dto：四个字段非 omitempty，缺失时上报 null。
        let mut dto = serde_json::Map::new();
        dto.insert("event_id".to_owned(), Value::String(payload.event_id));
        dto.insert(
            "group_openid".to_owned(),
            Value::String(payload.group_openid.clone()),
        );
        dto.insert(
            "op_member_openid".to_owned(),
            Value::String(payload.op_member_openid.clone()),
        );
        dto.insert(
            "timestamp".to_owned(),
            payload.timestamp.clone().unwrap_or(Value::Null),
        );
        extra.insert("data".to_owned(), Value::Object(dto));

        let group_id = self.vuid(&payload.group_openid);
        self.register_group(&group_id);
        Ok(vec![Event::Notice(NoticeEvent {
            time: timestamp_of(payload.timestamp.as_ref()),
            self_id: Ob11Id::num(self.self_id_of(settings)),
            notice_type: notice_type.to_owned(),
            sub_type: Some("create".to_owned()),
            group_id: Some(group_id),
            user_id: Some(self.vuid(&payload.op_member_openid)),
            operator_id: None,
            target_id: None,
            message_id: None,
            duration: None,
            file: None,
            card_new: None,
            card_old: None,
            honor_type: None,
            online: None,
            client: None,
            extra,
        })])
    }

    fn friend_add(
        &self,
        settings: &Settings,
        data: &Value,
        envelope_id: Option<&str>,
    ) -> Result<Vec<Event>, serde_json::Error> {
        let payload = QqFriendAdd::deserialize(data)?;
        let time = timestamp_of(payload.timestamp.as_ref());
        let user_id = self.vuid(&payload.openid);
        let event_id = if payload.event_id.is_empty() {
            envelope_id.unwrap_or_default()
        } else {
            payload.event_id.as_str()
        };

        let mut extra = Extra::new();
        if !settings.get_native_ob11() {
            extra.insert(
                "real_user_id".to_owned(),
                Value::String(payload.openid.clone()),
            );
        }

        // Go 把 scene_param 塞进 comment 供上层归因；为空时退化成 `Scene: <n>`。
        let comment = if payload.scene_param.is_empty() {
            format!("Scene: {}", payload.scene)
        } else {
            payload.scene_param.clone()
        };
        let request = Event::Request(RequestEvent {
            time,
            self_id: Ob11Id::num(self.self_id_of(settings)),
            request_type: "friend".to_owned(),
            sub_type: None,
            user_id: Some(user_id.clone()),
            group_id: None,
            comment,
            flag: event_id.to_owned(),
            extra: extra.clone(),
        });

        let mut notice_extra = extra;
        // Go `FriendNoticeEvent`：`scene` / `scene_param` 都带 omitempty。
        if payload.scene != 0 {
            notice_extra.insert("scene".to_owned(), Value::Number(payload.scene.into()));
        }
        if !payload.scene_param.is_empty() {
            notice_extra.insert("scene_param".to_owned(), Value::String(payload.scene_param));
        }
        let notice = Event::Notice(NoticeEvent {
            time,
            self_id: Ob11Id::num(self.self_id_of(settings)),
            notice_type: "friend_add".to_owned(),
            sub_type: None,
            group_id: None,
            user_id: Some(user_id),
            operator_id: None,
            target_id: None,
            message_id: None,
            duration: None,
            file: None,
            card_new: None,
            card_old: None,
            honor_type: None,
            online: None,
            client: None,
            extra: notice_extra,
        });
        Ok(vec![request, notice])
    }

    /// `FRIEND_DEL` / `C2C_MSG_REJECT` / `C2C_MSG_RECEIVE`：单向通知，Go 三处都只上报
    /// notice（`ProcessFriendDel`、`ProcessC2CMsgReject`/`Receive` 的 Branch A）。
    fn user_op_notice(
        &self,
        settings: &Settings,
        data: &Value,
        notice_type: &str,
    ) -> Result<Vec<Event>, serde_json::Error> {
        let payload = QqUserOp::deserialize(data)?;
        let mut extra = Extra::new();
        if !settings.get_native_ob11() {
            extra.insert(
                "real_user_id".to_owned(),
                Value::String(payload.openid.clone()),
            );
        }
        Ok(vec![Event::Notice(NoticeEvent {
            time: timestamp_of(payload.timestamp.as_ref()),
            self_id: Ob11Id::num(self.self_id_of(settings)),
            notice_type: notice_type.to_owned(),
            sub_type: None,
            group_id: None,
            user_id: Some(self.vuid(&payload.openid)),
            operator_id: None,
            target_id: None,
            message_id: None,
            duration: None,
            file: None,
            card_new: None,
            card_old: None,
            honor_type: None,
            online: None,
            client: None,
            extra,
        })])
    }

    /// `GROUP_MEMBER_ADD` / `GROUP_MEMBER_REMOVE`（Go `Processor/ProcessGroupMember.go`）。
    fn group_member(
        &self,
        settings: &Settings,
        data: &Value,
        envelope_id: Option<&str>,
        flavor: GroupMemberFlavor,
    ) -> Result<Vec<Event>, serde_json::Error> {
        let payload = QqGroupMember::deserialize(data)?;
        let group_id = self.vuid(&payload.group_openid);
        self.register_group(&group_id);
        let user_id = self.vuid(&payload.member_openid);

        // Go 对这两个事件不区分 native_ob11：RealUserID/RealGroupID 只被 omitempty 省略空值。
        let mut extra = Extra::new();
        if !payload.member_openid.is_empty() {
            extra.insert(
                "real_user_id".to_owned(),
                Value::String(payload.member_openid.clone()),
            );
        }
        if !payload.group_openid.is_empty() {
            extra.insert(
                "real_group_id".to_owned(),
                Value::String(payload.group_openid.clone()),
            );
        }

        // CQ 码描述与 Go 一致（数字形态 = 虚拟 ID；字符串形态退化为原始 OpenID）。
        let (notice_type, sub_type, cq_type) = match flavor {
            GroupMemberFlavor::Add => ("group_increase", "approve", "add"),
            GroupMemberFlavor::Remove => ("group_decrease", "leave", "remove"),
        };
        extra.insert(
            "message".to_owned(),
            Value::String(format!(
                "[CQ:member,type={cq_type},group_id={group_id},user_id={user_id}]"
            )),
        );
        // Go 只给入群方向保留 event_id（`echo.AddEvnetIDv2` 的被动回复凭据）。
        if flavor == GroupMemberFlavor::Add {
            let event_id = if payload.event_id.is_empty() {
                envelope_id.unwrap_or_default()
            } else {
                payload.event_id.as_str()
            };
            if !event_id.is_empty() {
                // Go `echo.AddEvnetIDv2(app_id, 原始 group_openid, event_id)`（ProcessGroupMember.go:67-71）
                if let Some(echo) = self.echo.as_deref() {
                    echo.add_event_id_v2(
                        &settings.get_app_id().to_string(),
                        &payload.group_openid,
                        event_id,
                    );
                }
                extra.insert("event_id".to_owned(), Value::String(event_id.to_owned()));
            }
        }

        Ok(vec![Event::Notice(NoticeEvent {
            time: timestamp_of(payload.timestamp.as_ref()),
            self_id: Ob11Id::num(self.self_id_of(settings)),
            notice_type: notice_type.to_owned(),
            sub_type: Some(sub_type.to_owned()),
            group_id: Some(group_id),
            user_id: Some(user_id),
            operator_id: Some(Ob11Id::num(0)),
            target_id: None,
            message_id: None,
            duration: None,
            file: None,
            card_new: None,
            card_old: None,
            honor_type: None,
            online: None,
            client: None,
            extra,
        })])
    }

    /// `GROUP_JOIN_REQUEST`：上报 request（Go `Processor/ProcessGroupJoinRequest.go`），
    /// `flag` 为 `join_request_id`，可直接回传 `set_group_add_request` 审批。
    fn group_join_request(
        &self,
        settings: &Settings,
        data: &Value,
    ) -> Result<Vec<Event>, serde_json::Error> {
        let payload = QqJoinRequest::deserialize(data)?;
        let mut extra = Extra::new();
        if !settings.get_native_ob11() {
            extra.insert(
                "real_user_id".to_owned(),
                Value::String(payload.member_openid.clone()),
            );
            extra.insert(
                "real_group_id".to_owned(),
                Value::String(payload.group_openid.clone()),
            );
        }
        if !payload.username.is_empty() {
            extra.insert("username".to_owned(), Value::String(payload.username));
        }
        let (auto_approved, approved) = auto_approved_pair(payload.auto_approved.as_ref());
        extra.insert("auto_approved".to_owned(), auto_approved);
        extra.insert("auto_approved_bool".to_owned(), Value::Bool(approved));

        Ok(vec![Event::Request(RequestEvent {
            time: apply_at_unix(payload.apply_at.as_ref()),
            self_id: Ob11Id::num(self.self_id_of(settings)),
            request_type: "group".to_owned(),
            sub_type: Some("add".to_owned()),
            user_id: Some(self.vuid(&payload.member_openid)),
            group_id: Some(self.vuid(&payload.group_openid)),
            comment: verify_comment(payload.verify_info.as_ref()),
            flag: payload.join_request_id,
            extra,
        })])
    }

    /// `INTERACTION_CREATE`：按钮 / 快捷菜单回调
    /// （Go `Processor/ProcessInlineSearch.go:122-137` 的 `OnebotInteractionNotice`）。
    ///
    /// Go 的 `Data` 是整块 `dto.WSInteractionData`，`id`、`event_id`、`data.resolved.button_data`
    /// 都在里面，因此这里不拆平到 notice 顶层，而是原样塞进 `extra["data"]`。
    /// `time` 取当前秒（Go 同样用 `time.Now()`，不用负载里的 RFC3339 `timestamp`）。
    fn interaction(
        &self,
        settings: &Settings,
        data: &Value,
    ) -> Result<Vec<Event>, serde_json::Error> {
        let payload = Interaction::deserialize(data)?;
        // Go 的 `fromuid` 只取 `GroupMemberOpenID`；单聊回调没有群成员，回退 `user_openid`
        // 以免 user_id 恒空（群聊场景与 Go 完全一致）。
        let user_raw = if payload.group_member_openid.is_empty() {
            payload.user_openid.as_str()
        } else {
            payload.group_member_openid.as_str()
        };
        let mut extra = self.enhanced_extra(settings, user_raw, &payload.group_openid);
        // Go `ProcessInlineSearch.go:106-119`：`union_id` 打开且缓存命中时，只把回显的
        // `data.group_member_openid` 换成 union。`fromuid`（`:39`）在替换之前取值，因此
        // `user_id` / `real_user_id` 仍是原始 OpenID——这是 Go 的实际行为，不是漏做。
        let mut echoed = serde_json::to_value(&payload)?;
        if settings.get_union_id()
            && !payload.group_member_openid.is_empty()
            && let Some(union) = UnionCache::shared().union(&payload.group_member_openid)
            && !union.is_empty()
            && let Value::Object(map) = &mut echoed
        {
            map.insert("group_member_openid".to_owned(), Value::String(union));
        }
        extra.insert("data".to_owned(), echoed);

        Ok(vec![Event::Notice(NoticeEvent {
            time: now_unix(),
            self_id: Ob11Id::num(self.self_id_of(settings)),
            notice_type: "interaction".to_owned(),
            sub_type: Some("create".to_owned()),
            group_id: non_empty(&payload.group_openid).map(|raw| self.vuid(&raw)),
            user_id: non_empty(user_raw).map(|raw| self.vuid(&raw)),
            operator_id: None,
            target_id: None,
            message_id: None,
            duration: None,
            file: None,
            card_new: None,
            card_old: None,
            honor_type: None,
            online: None,
            client: None,
            extra,
        })])
    }

    /// `!native_ob11` 时补齐 Go 的「增强配置」字段。
    fn enhanced_extra(&self, settings: &Settings, user_raw: &str, group_raw: &str) -> Extra {
        let mut extra = Extra::new();
        if !settings.get_native_ob11() {
            extra.insert(
                "real_user_id".to_owned(),
                Value::String(user_raw.to_owned()),
            );
            extra.insert(
                "real_group_id".to_owned(),
                Value::String(group_raw.to_owned()),
            );
        }
        extra
    }
}

/// 上报正文：`add_at_group` 打开时给全量群消息补回 `[CQ:at,qq=<appid>]`。
///
/// Go `ProcessGroupNormalMessage.go:148-150` 在剥离 @bot / 去前缀之后无条件前置
/// （`GetAppIDStr()`，与 `use_uin` 无关），`message` 与 `raw_message` 同源
/// （`:205` / `:281` 都赋 `messageText`），这里两条一起改。
///
/// 只在 `GROUP_MESSAGE_CREATE`（[`GroupFlavor::Full`]）生效：AT 事件的正文本身就带 @，
/// 再补一次就是重复 @（Go `ProcessGroupMessage.go:154-157`）。单聊不适用。
///
/// 不去重：Go 也不去重，且全量消息里的 @bot 更早的入站阶段已被剥离，重复分支走不到。
fn reported_group_content(settings: &Settings, content: &str, flavor: GroupFlavor) -> String {
    if flavor != GroupFlavor::Full || !settings.get_add_at_group() {
        return content.to_owned();
    }
    format!("[CQ:at,qq={}] {content}", settings.get_app_id())
}

/// 上报 `message` 字段：`array_value` 打开时按 CQ 解析成消息段数组，否则用字符串形态。
///
/// 两种形态读的正文不同：Go 字符串路径上报 `messageText`（含 `add_at_group` 前缀，
/// `ProcessGroupNormalMessage.go:148-150` / `:176`），数组路径的
/// `ConvertToSegmentedMessage` 则从 `data.Content` 重算（`handlers/message_parser.go:2182`），
/// 前缀不进段数组。`content` 传段数组形态的正文（[`Inbound::revert_mentions`] 的
/// `string_form = false` 结果，`array_value` 关闭时不读），`string_form` 传字符串形态。
/// 段名不在入站码表（[`code_spec`] 查无此项或 `inbound` 为假，例如用户手打的 `[CQ:remove,…]`）
/// 时折回 text 原文，不向客户端交出它不认识的段；相邻 text 段合并成一段，
/// 与 Go 文本段「连续切片」的粒度一致。
///
/// 段数组末尾还要接图片段与事件段（[`image_segments`] / [`message_event_segments`]）；
/// 字符串形态的媒体码由调用方并进 `string_form`（[`media_cq_text`]），这里只管形态选择。
fn reported_message(
    settings: &Settings,
    content: &str,
    string_form: &str,
    msg: &QqMessage,
) -> MessageContent {
    if !settings.get_array_value() {
        return MessageContent::Text(string_form.to_owned());
    }
    let mut segments: Vec<Segment> = Vec::new();
    for segment in cqcode::parse(content) {
        match code_spec(&segment.kind) {
            Some(spec) if spec.inbound && !segment.is_text() => segments.push(segment),
            // `raw_or_cq` 保留码段原始字节：解析失败的码本就在 text 段里，未知段名也按原文折回
            _ => push_text_segment(&mut segments, segment.raw_or_cq().as_ref()),
        }
    }
    // 图片段排在正文段之后，事件段再排图片段之后（Go `handlers/message_parser.go:2331-2334`）
    segments.extend(image_segments(msg));
    segments.extend(message_event_segments(msg));
    MessageContent::Array(segments)
}

/// 追加文本段并合并相邻文本（空串不落段）。
fn push_text_segment(segments: &mut Vec<Segment>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = segments.last_mut().filter(|segment| segment.is_text()) {
        let merged = format!("{}{text}", last.get("text").unwrap_or_default());
        *last = Segment::text(merged);
        return;
    }
    segments.push(Segment::text(text));
}

/// 消息事件新增字段（语音附件 / 结构化卡片 / 引用 / 嵌套正文）的降级单元。
///
/// 字符串路径与段数组路径共用同一份递归展开（Go `collectMessageEventExtras`
/// `handlers/message_parser.go:2412-2429`）：这里只决定「有哪些单元、按什么顺序」，
/// 两条路径各自决定落成什么形态。
enum InboundExtra<'a> {
    /// 语音附件：`file` 取 `voice_wav_url`，缺失时回退附件 `url`；两者都空时整条不产出。
    Record {
        file: &'a str,
        url: &'a str,
        asr: &'a str,
    },
    /// 结构化卡片：`ark_data` 缺失（只凭 `message_type == 3` 判定）时为 `None`。
    Ark(Option<&'a QqArkData>),
    Reply {
        msg_idx: &'a str,
        content: String,
    },
    Text(&'a str),
}

/// 语音附件降级为 record 单元（Go `voiceAttachmentExtra` `handlers/message_parser.go:2350-2365`）。
fn voice_attachment_extra(attachment: &QqAttachment) -> Option<InboundExtra<'_>> {
    if attachment.content_type != "voice" {
        return None;
    }
    let file = if attachment.voice_wav_url.is_empty() {
        attachment.url.as_str()
    } else {
        attachment.voice_wav_url.as_str()
    };
    (!file.is_empty()).then(|| InboundExtra::Record {
        file,
        url: &attachment.url,
        asr: &attachment.asr_refer_text,
    })
}

/// 引用单元：`msg_idx` 取元素列表里第一个非空索引，`content` 是各元素正文的拼接
/// （Go `replyExtra` `handlers/message_parser.go:2368-2382`）。
fn reply_extra(elements: &[Option<QqMsgElement>]) -> InboundExtra<'_> {
    let mut msg_idx = "";
    let mut content = String::new();
    for element in elements.iter().flatten() {
        if msg_idx.is_empty() {
            msg_idx = element.msg_idx.as_str();
        }
        content.push_str(&element.content);
    }
    InboundExtra::Reply { msg_idx, content }
}

/// 递归降级 `msg_elements`（Go `collectElementExtras` `handlers/message_parser.go:2386-2410`）：
/// 语音附件→record、卡片→ark、正文→text、嵌套引用→reply + 递归。
fn collect_element_extras<'a>(
    elements: &'a [Option<QqMsgElement>],
    extras: &mut Vec<InboundExtra<'a>>,
) {
    for element in elements.iter().flatten() {
        for attachment in element.attachments.iter().flatten() {
            if let Some(extra) = voice_attachment_extra(attachment) {
                extras.push(extra);
            }
        }
        if element.ark_data.is_some() || element.message_type == 3 {
            extras.push(InboundExtra::Ark(element.ark_data.as_ref()));
        }
        if !element.content.is_empty() {
            extras.push(InboundExtra::Text(&element.content));
        }
        // `msg_elements` 为空时不产出空 reply 段（msg_idx 空，与 CQ 路径的跳过语义对齐）
        if !element.msg_elements.is_empty() {
            extras.push(reply_extra(&element.msg_elements));
            collect_element_extras(&element.msg_elements, extras);
        }
    }
}

/// 一条消息的全部事件级降级单元（Go `collectMessageEventExtras`
/// `handlers/message_parser.go:2412-2429`）：语音附件 → `ark_data` / `message_type == 3`
/// → 顶层 `msg_elements` 的 reply + 递归。
fn collect_message_event_extras(msg: &QqMessage) -> Vec<InboundExtra<'_>> {
    let mut extras = Vec::new();
    for attachment in msg.attachments.iter().flatten() {
        if let Some(extra) = voice_attachment_extra(attachment) {
            extras.push(extra);
        }
    }
    if msg.ark_data.is_some() || msg.message_type == 3 {
        extras.push(InboundExtra::Ark(msg.ark_data.as_ref()));
    }
    if !msg.msg_elements.is_empty() {
        extras.push(reply_extra(&msg.msg_elements));
        collect_element_extras(&msg.msg_elements, &mut extras);
    }
    extras
}

/// `ark` 段：`ark_type` / `ark_name` / `prompt` / `fields` 四个键恒定存在
/// （Go `arkSegmentData` `handlers/message_parser.go:2431-2449`）。
///
/// `fields` 在 Go 里是真 JSON 对象：经 [`Segment::param_json`] 标记，序列化出对象（P52），
/// 缺省 `{}`。
fn ark_segment(ark: Option<&QqArkData>) -> Segment {
    let (ark_type, ark_name, prompt) = ark.map_or(("", "", ""), |ark| {
        (
            ark.ark_type.as_str(),
            ark.ark_name.as_str(),
            ark.prompt.as_str(),
        )
    });
    let fields = ark
        .and_then(|ark| ark.fields.as_ref())
        // 非对象在 Go 侧解码为 nil/空 map，折 "{}" 零影响
        .filter(|fields| fields.is_object())
        .map_or_else(|| "{}".to_owned(), Value::to_string);
    Segment::new("ark")
        .param("ark_type", ark_type)
        .param("ark_name", ark_name)
        .param("prompt", prompt)
        .param_json("fields", fields)
}

/// 段数组路径的事件段（Go `messageEventSegments` `handlers/message_parser.go:2452-2477`）：
/// record 的 `url` / `asr_refer_text` 只在非空时出现；reply 恒带 `msg_idx` + `content`。
fn message_event_segments(msg: &QqMessage) -> Vec<Segment> {
    collect_message_event_extras(msg)
        .into_iter()
        .map(|extra| match extra {
            InboundExtra::Record { file, url, asr } => Segment::new("record")
                .param("file", file)
                .param_opt("url", (!url.is_empty()).then_some(url))
                .param_opt("asr_refer_text", (!asr.is_empty()).then_some(asr)),
            InboundExtra::Ark(ark) => ark_segment(ark),
            InboundExtra::Reply { msg_idx, content } => Segment::new("reply")
                .param("msg_idx", msg_idx)
                .param("content", content),
            InboundExtra::Text(text) => Segment::text(text),
        })
        .collect()
}

/// 入站 CQ 值转义（Go `cqparse.EncodeValue` `handlers/cqparse/escape.go:13-19`）：
/// 在 [`cqcode::escape`]（`&` `,` `]`）之上补 `[` → `&#91;`。asr / ark_type / ark_name /
/// prompt / msg_idx 来自远端卡片，含 `[CQ:…]` 形状时不转 `[` 会被下游重解析成段。
/// `cqcode::escape` 本身刻意不转 `[`（服务 parse 往返），故这里单独补，不改它。
fn encode_value(s: &str) -> String {
    cqcode::escape(s).replace('[', "&#91;")
}

/// 字符串路径的事件码（Go `messageEventCQText` `handlers/message_parser.go:2480-2509`）：
/// record 只带 `file` 与 `asr`（`url` 不进码），`message_type=3` 但无 `ark_data` 时保留
/// `[CQ:ark]`（不静默丢弃卡片存在的事实），空 `msg_idx` 的 reply 整段跳过。
fn message_event_cq_text(msg: &QqMessage) -> String {
    let mut text = String::new();
    for extra in collect_message_event_extras(msg) {
        match extra {
            InboundExtra::Record { file, asr, .. } => {
                text.push_str("[CQ:record,file=");
                text.push_str(file);
                if !asr.is_empty() {
                    text.push_str(",asr=");
                    text.push_str(&encode_value(asr));
                }
                text.push(']');
            }
            InboundExtra::Ark(None) => text.push_str("[CQ:ark]"),
            InboundExtra::Ark(Some(ark)) => {
                text.push_str("[CQ:ark,ark_type=");
                text.push_str(&encode_value(&ark.ark_type));
                text.push_str(",ark_name=");
                text.push_str(&encode_value(&ark.ark_name));
                text.push_str(",prompt=");
                text.push_str(&encode_value(&ark.prompt));
                text.push(']');
            }
            InboundExtra::Reply { msg_idx, .. } => {
                if !msg_idx.is_empty() {
                    text.push_str("[CQ:reply,msg_idx=");
                    text.push_str(&encode_value(msg_idx));
                    text.push(']');
                }
            }
            InboundExtra::Text(content) => text.push_str(content),
        }
    }
    text
}

/// 图片附件判定：`content_type` 前缀 `image/`（Go 两条路径同一个过滤器）。
fn is_image(attachment: &QqAttachment) -> bool {
    attachment.content_type.starts_with("image/")
}

/// 段数组路径的图片段（Go `handlers/message_parser.go:2214-2235`）：`file` 是把 `{` `}`
/// `.png` `.jpg` `.gif` `-` 全部删掉后的 `FileName` 加 `.image`，`url` 原样（不补协议头）。
fn image_segments(msg: &QqMessage) -> Vec<Segment> {
    msg.attachments
        .iter()
        .flatten()
        .filter(|attachment| is_image(attachment))
        .map(|attachment| {
            let mut name = attachment.file_name.clone();
            for piece in ["{", "}", ".png", ".jpg", ".gif", "-"] {
                name = name.replace(piece, "");
            }
            Segment::new("image")
                .param("file", format!("{name}.image"))
                .param("subType", "0")
                .param("url", attachment.url.as_str())
        })
        .collect()
}

/// 字符串路径的图片码（Go `handlers/message_parser.go:2137-2154`）：`file` 去掉最后一个
/// 扩展名后加 `.image`，`url` 缺协议头时补 `http://`——与段数组路径口径不同，两条路径各自照搬。
fn image_cq_text(msg: &QqMessage) -> String {
    let mut text = String::new();
    for attachment in msg.attachments.iter().flatten().filter(|a| is_image(a)) {
        text.push_str("[CQ:image,file=");
        text.push_str(strip_extension(&attachment.file_name));
        text.push_str(".image,subType=0,url=");
        if attachment.url.starts_with("http://") || attachment.url.starts_with("https://") {
            text.push_str(&attachment.url);
        } else {
            text.push_str("http://");
            text.push_str(&attachment.url);
        }
        text.push(']');
    }
    text
}

/// Go `filepath.Ext` + `strings.TrimSuffix`：去掉最后一个路径元素里最后一个 `.` 起的后缀
/// （没有点时返回原文；`.` 是首字符时返回空串，与 Go 对 `.bashrc` 的处理一致）。
/// `\` 也当路径分隔符是 Go `filepath.Ext` 在 Windows 的语义（`os.IsPathSeparator` 认双斜杠）；
/// 对齐基准是 Go 的 Windows 行为，故不按宿主平台 `cfg!(windows)` 收敛。
fn strip_extension(file_name: &str) -> &str {
    let base_start = file_name
        .rfind(['/', '\\'])
        .map_or(0, |separator| separator + 1);
    match file_name[base_start..].rfind('.') {
        Some(dot) => &file_name[..base_start + dot],
        None => file_name,
    }
}

/// 字符串路径的媒体尾巴：图片码在前、事件码在后，接在正文末尾。
///
/// Go 这两步都在 `RevertTransformedText` 里（`handlers/message_parser.go:2136-2157`），
/// 结果先成为 `messageText`，handler 之后才加 `add_at_group` 前缀（`:148-150`）——
/// 所以前缀落在媒体码之前；`message` 与 `raw_message` 同源（`:205` / `:281`）。
fn media_cq_text(msg: &QqMessage) -> String {
    let mut text = image_cq_text(msg);
    text.push_str(&message_event_cq_text(msg));
    text
}

/// 入站正文 `BotID → AppID`（Go `RevertTransformedText` 的第一步，
/// `message_parser.go:1850-1854`；消息段路径同口径 `:2236-2240`）。
///
/// QQ 事件正文的 `<@!BotID>` 带的是机器人真实 ID，而 OneBot 侧的自身标识是 `app_id`
/// （`self_id` 与 `add_at_group` 的前缀都用它），不还原客户端就认不出 @bot。
/// BotID 未取到或与 AppID 相同时原样返回——Go 用空串守卫（`BotID != ""`）防的是空串
/// 替换把正文逐字符穿插打散，这里连「相等」也一起跳过（相等替换本就是空操作）。
///
/// 与 Go 的差异：Go 只在 `disable_error_chan=false` 时走 `RevertTransformedText`
/// （群聊两条路径 `ProcessGroupNormalMessage.go:110-113` / `ProcessGroupMessage.go:119`，
/// C2C 无条件），这里不跟这个开关——本仓的 `disable_error_chan` 只表示统计/错误通道开关。
fn bot_id_to_app_id(settings: &Settings, content: &str, bot_id: Option<&str>) -> String {
    let app_id = settings.get_app_id().to_string();
    let Some(bot_id) = bot_id.filter(|id| !id.is_empty() && *id != app_id.as_str()) else {
        return content.to_owned();
    };
    content.replace(bot_id, &app_id)
}

/// 正文里的裸 mention 扫描（Go `incomingAtRe` = `<@!?([0-9A-Fa-f]+)>`）。
///
/// 返回 (字节区间, 用户 ID)，按出现顺序、互不重叠。字符类与 Go 一致只认十六进制——QQ 的
/// OpenID 与数字 UIN 都落在其中，`<@用户名>` 这类形态不匹配、原样留在正文。
fn scan_mentions(text: &str) -> Vec<(std::ops::Range<usize>, &str)> {
    let bytes = text.as_bytes();
    let mut mentions = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] == b'<' && bytes.get(cursor + 1) == Some(&b'@') {
            let mut end = cursor + 2;
            if bytes.get(end) == Some(&b'!') {
                end += 1;
            }
            let id_start = end;
            while bytes.get(end).is_some_and(|byte| byte.is_ascii_hexdigit()) {
                end += 1;
            }
            if end > id_start && bytes.get(end) == Some(&b'>') {
                // ID 段全是 ASCII，切片边界必然落在字符边界上
                mentions.push((cursor..end + 1, &text[id_start..end]));
                cursor = end + 1;
                continue;
            }
        }
        cursor += 1;
    }
    mentions
}

/// `id` 是否等于配置里的 `uin` 或 `app_id`（Go `GetUinStr()` / `GetAppIDStr()` 比对）。
fn is_own_id(settings: &Settings, id: &str) -> bool {
    id == settings.get_uin().to_string() || id == settings.get_app_id().to_string()
}

/// 入站敏感词过滤：只改写三个消息事件的 `content` 与 `author.username`
/// （Go `main.go:765-770` / `:787-792` / `:1124-1129`），其余事件返回 `None` 走原载荷。
///
/// 空正文/空昵称直接跳过：Go 对空串会返回「错误：缺少 'word' 参数」，那是它已删除的
/// 词表 HTTP 端点遗留的分支，照搬会把纯媒体消息的正文改成错误提示。
fn filtered_message_payload(kind: &str, data: &Value) -> Option<Value> {
    if !matches!(
        kind,
        "GROUP_AT_MESSAGE_CREATE" | "GROUP_MESSAGE_CREATE" | "C2C_MESSAGE_CREATE"
    ) {
        return None;
    }
    let mut data = data.clone();
    if let Some(object) = data.as_object_mut() {
        if let Some(Value::String(content)) = object.get_mut("content")
            && !content.is_empty()
        {
            *content = acnode::filter_in(content);
        }
        if let Some(Value::Object(author)) = object.get_mut("author")
            && let Some(Value::String(username)) = author.get_mut("username")
            && !username.is_empty()
        {
            *username = acnode::filter_in(username);
        }
    }
    Some(data)
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn timestamp_of(value: Option<&Value>) -> i64 {
    match value {
        Some(Value::String(raw)) => raw.trim().parse().unwrap_or_else(|_| now_unix()),
        Some(Value::Number(number)) => number.as_i64().unwrap_or_else(now_unix),
        _ => now_unix(),
    }
}

pub(crate) fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

/// 入群申请的 `verify_info`：对象取 `verify_message`，旧格式纯字符串原样返回
/// （Go `dto.VerifyInfo.String`）。
fn verify_comment(raw: Option<&Value>) -> String {
    match raw {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Object(map)) => map
            .get("verify_message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        _ => String::new(),
    }
}

/// `auto_approved` 透传原始 JSON + 与 Go `AutoApprovedInfo.Approved` 一致的布尔语义：
/// 对象 / `true` 为真，`false` / `null` / 缺失为假。
fn auto_approved_pair(raw: Option<&Value>) -> (Value, bool) {
    match raw {
        Some(Value::Object(_)) => (raw.cloned().unwrap_or(Value::Null), true),
        Some(Value::Bool(flag)) => (Value::Bool(*flag), *flag),
        Some(Value::Null) | None => (Value::Null, false),
        Some(other) => (other.clone(), false),
    }
}

/// 入群申请的 `apply_at`：RFC3339 → 秒级数字 → 当前时间（Go `ProcessGroupJoinRequest`）。
fn apply_at_unix(value: Option<&Value>) -> i64 {
    let parsed = match value {
        Some(Value::String(text)) => parse_rfc3339(text)
            .or_else(|| text.trim().parse().ok())
            .unwrap_or(0),
        Some(Value::Number(_)) => timestamp_of(value),
        _ => 0,
    };
    if parsed == 0 { now_unix() } else { parsed }
}

/// 最小 RFC3339 → Unix 秒（`YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)`），非法返回 `None`。
///
/// 与 `gensokyo-actions::rfc3339` 同算法（Howard Hinnant 的 days-from-civil）：那边是私有
/// 模块，本 crate 又不能为它改 `Cargo.lock`，因此只在这里保留解析方向。
fn parse_rfc3339(text: &str) -> Option<i64> {
    let (date, clock) = text.trim().split_once(['T', 't'])?;

    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let (clock, offset_secs) = split_rfc3339_offset(clock)?;
    let mut clock_parts = clock.split(':');
    let hour: i64 = clock_parts.next()?.parse().ok()?;
    let minute: i64 = clock_parts.next()?.parse().ok()?;
    // 秒可带小数（QQ 偶有 `.000`），小数部分不参与秒级精度。
    let second: i64 = clock_parts.next()?.split('.').next()?.parse().ok()?;
    if clock_parts.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset_secs)
}

/// 拆出时钟与偏移秒数：`Z`/`z` 为 0，否则 `±HH:MM`。
fn split_rfc3339_offset(clock: &str) -> Option<(&str, i64)> {
    if let Some(clock) = clock.strip_suffix(['Z', 'z']) {
        return Some((clock, 0));
    }
    // 偏移符号出现在秒之后：`HH:MM:SS` 已占 8 个字符，从第 9 个字符起找。
    let (index, _) = clock
        .char_indices()
        .skip(8)
        .find(|(_, character)| *character == '+' || *character == '-')?;
    let (clock, offset) = clock.split_at(index);
    let sign = if offset.starts_with('-') { -1 } else { 1 };
    let (hours, minutes) = offset.get(1..)?.split_once(':')?;
    let hours: i64 = hours.parse().ok()?;
    let minutes: i64 = minutes.parse().ok()?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some((clock, sign * (hours * 3600 + minutes * 60)))
}

/// 公历日期 → 距 1970-01-01 的天数（`month` 为 `1..=12`）。
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_from_march = i64::from((month + 9) % 12);
    let day_of_year = (153 * month_from_march + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::{
        auto_approved_pair, parse_rfc3339, scan_mentions, strip_extension, timestamp_of,
        verify_comment,
    };
    use serde_json::json;

    /// `strip_extension` 对齐 Go `filepath.Ext` + `TrimSuffix`（go1.26.10 实测）：
    /// 多重点只去最后一个；无扩展名原样；`.bashrc` 整串即扩展名 → 空串；`\` 也当分隔符。
    #[test]
    fn strip_extension_matches_go_filepath_ext() {
        for (input, expected) in [
            ("a.tar.gz", "a.tar"),
            ("abc.png", "abc"),
            ("noext", "noext"),
            (".bashrc", ""),
            ("dir/x.png", "dir/x"),
            ("dir\\x.png", "dir\\x"),
            ("", ""),
        ] {
            assert_eq!(strip_extension(input), expected, "input={input:?}");
        }
    }

    #[test]
    fn scan_mentions_matches_go_hex_character_class() {
        let text = "<@ABC123> a <@!def456> b <@zzz> c <@> d <@ABC";
        let found: Vec<&str> = scan_mentions(text).into_iter().map(|(_, id)| id).collect();
        // 与 Go `incomingAtRe = <@!?([0-9A-Fa-f]+)>` 一致：`!` 可选、只认十六进制、
        // 空 ID 与缺 `>` 都不匹配（原样留在正文）。
        assert_eq!(found, vec!["ABC123", "def456"]);
    }

    #[test]
    fn scan_mentions_reports_in_place_byte_ranges() {
        let text = "前<@AB>后";
        let found = scan_mentions(text);
        assert_eq!(found.len(), 1);
        let (range, id) = &found[0];
        assert_eq!(*id, "AB");
        assert_eq!(&text[range.clone()], "<@AB>");
    }

    #[test]
    fn timestamp_accepts_string_number_and_missing() {
        assert_eq!(timestamp_of(Some(&json!("1700000000"))), 1_700_000_000);
        assert_eq!(timestamp_of(Some(&json!(1_700_000_000))), 1_700_000_000);
        assert!(timestamp_of(Some(&json!("not-a-number"))) > 0);
        assert!(timestamp_of(None) > 0);
    }

    #[test]
    fn rfc3339_parses_utc_and_offset_forms() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2023-11-14T22:13:20Z"), Some(1_700_000_000));
        assert_eq!(
            parse_rfc3339("2026-08-05T14:19:09+08:00"),
            Some(1_785_910_749)
        );
        assert_eq!(
            parse_rfc3339("2026-08-05T06:19:09.512Z"),
            Some(1_785_910_749)
        );
        assert_eq!(parse_rfc3339("2024-02-29T12:00:00Z"), Some(1_709_208_000));
        for bad in [
            "",
            "not-a-time",
            "2026-08-05",
            "2026-13-01T00:00:00Z",
            "2026-08-05T24:00:00Z",
            "2026-08-05T00:00:00+24:00",
        ] {
            assert_eq!(parse_rfc3339(bad), None, "{bad} 应被拒绝");
        }
    }

    #[test]
    fn verify_info_accepts_string_and_object() {
        assert_eq!(verify_comment(Some(&json!("111"))), "111");
        assert_eq!(
            verify_comment(Some(
                &json!({"method": "verify_message", "verify_message": "222"})
            )),
            "222"
        );
        assert_eq!(
            verify_comment(Some(&json!({"method": "admin_review_qa"}))),
            ""
        );
        assert_eq!(verify_comment(None), "");
    }

    #[test]
    fn interaction_create_becomes_notice_interaction() {
        use std::sync::Arc;

        use gensokyo_config::ConfigManager;
        use gensokyo_core::Ob11Id;
        use gensokyo_protocol::Event;
        use gensokyo_protocol::event::NoticeKind;

        use super::Inbound;

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings: {}\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        let inbound = Inbound::new(manager, None);

        let data = json!({
            "id": "i-1",
            "event_id": "e-1",
            "type": 11,
            "chat_type": 1,
            "group_openid": "G",
            "group_member_openid": "M",
            "data": {
                "name": "按钮",
                "type": 9,
                "resolved": {"button_data": "/菜单", "button_id": "b-1"}
            },
            "version": 1
        });
        let events = inbound
            .convert("INTERACTION_CREATE", Some(&data), None)
            .expect("互动事件应可转换");

        assert_eq!(events.len(), 1, "一条回调对应一条 notice");
        let Event::Notice(notice) = &events[0] else {
            panic!("INTERACTION_CREATE 应转成 notice");
        };
        assert_eq!(notice.notice_type, "interaction");
        assert_eq!(notice.sub_type.as_deref(), Some("create"));
        assert_eq!(notice.kind(), NoticeKind::Interaction);
        // 没有 idmap 时回退原始 OpenID 字符串（不伪造数字）
        assert_eq!(notice.group_id, Some(Ob11Id::Str("G".to_owned())));
        assert_eq!(notice.user_id, Some(Ob11Id::Str("M".to_owned())));
        // Go 把整块互动对象放在 `data`：id / event_id / button_data 都在里面
        let payload = notice.extra.get("data").expect("应带 data 字段");
        assert_eq!(payload["event_id"], json!("e-1"));
        assert_eq!(payload["data"]["resolved"]["button_data"], json!("/菜单"));
    }

    #[test]
    fn auto_approved_mirrors_go_bool_semantics() {
        assert!(
            auto_approved_pair(Some(&json!({"strategy_id": "s"}))).1,
            "对象形态视为已自动审批"
        );
        assert!(auto_approved_pair(Some(&json!(true))).1);
        assert!(!auto_approved_pair(Some(&json!(false))).1);
        assert!(!auto_approved_pair(Some(&json!(null))).1);
        assert_eq!(auto_approved_pair(None), (json!(null), false));
    }

    use std::sync::Arc;

    use gensokyo_config::ConfigManager;
    use gensokyo_storage::{EchoStore, IdMapOptions, IdMapStore, UserInfoRow};

    use super::Inbound;

    /// 构造指向临时目录的 idmap 存储（同 `at_everyone` 测试的打开方式）。
    fn test_store(dir: &std::path::Path) -> std::sync::Arc<IdMapStore> {
        std::sync::Arc::new(
            IdMapStore::open(
                IdMapOptions::new(dir.join("openid-map.redb"), dir.join("msgid-map.redb"))
                    .with_app_id("10001"),
            )
            .expect("打开 idmap 存储失败"),
        )
    }

    fn group_at_payload() -> serde_json::Value {
        json!({
            "id": "m-1",
            "group_id": "g0123456789abcdef0123456789abcdef",
            "content": "hello",
            "author": {"id": "u0123456789abcdef0123456789abcdef"}
        })
    }

    #[test]
    fn group_message_registers_virtual_id_section() {
        use gensokyo_protocol::Event;

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings: {}\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        let store = test_store(dir.path());
        let inbound = Inbound::new(manager, Some(Arc::clone(&store)));

        let events = inbound
            .convert("GROUP_AT_MESSAGE_CREATE", Some(&group_at_payload()), None)
            .expect("群消息应可转换");
        let Event::Message(message) = &events[0] else {
            panic!("GROUP_AT_MESSAGE_CREATE 应转成群消息");
        };
        // 数字模式：上报与注册都用虚拟号形态
        let group_id = message.group_id.clone().expect("群消息应带 group_id");
        assert!(group_id.as_i64().is_some(), "有 idmap 时应上报数字形态");
        assert_eq!(
            store
                .read_config(&group_id.to_string(), "type")
                .expect("群应已注册"),
            "group"
        );
    }

    /// 群消息作者带 `union_openid` 时建 `id ↔ union` 映射（Go `ProcessGroupMessage.go:55` /
    /// `ProcessGroupNormalMessage.go:46`），缺这个字段时不写。
    #[test]
    fn group_message_records_union_openid_pair() {
        use gensokyo_storage::UnionCache;

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings: {}\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        let inbound = Inbound::new(manager, None);

        // 键用本用例独有的字面量：共享实例在同进程的并行用例间是同一张表
        let raw = "u-union-cache-inbound-1";
        let union = "union-cache-inbound-1";
        let payload = json!({
            "id": "m-union-1",
            "group_id": "g-union-1",
            "content": "hi",
            "author": {"id": raw, "union_openid": union}
        });
        inbound
            .convert("GROUP_AT_MESSAGE_CREATE", Some(&payload), None)
            .expect("群消息应可转换");
        assert_eq!(UnionCache::shared().union(raw).as_deref(), Some(union));
        assert_eq!(UnionCache::shared().id(union).as_deref(), Some(raw));

        // 没有 `union_openid` 的负载不建映射（空键由 `store` 忽略，行为零变化）
        let bare = "u-union-cache-inbound-2";
        let payload = json!({
            "id": "m-union-2",
            "group_id": "g-union-1",
            "content": "hi",
            "author": {"id": bare}
        });
        inbound
            .convert("GROUP_MESSAGE_CREATE", Some(&payload), None)
            .expect("全量群消息应可转换");
        assert_eq!(UnionCache::shared().union(bare), None, "缺 union 不建映射");
    }

    /// 缺 `author` / 空 `id` 的群消息：空身份不写进 idmap，ID 原样回退空串。
    #[test]
    fn group_message_without_author_skips_empty_identity() {
        use gensokyo_protocol::Event;

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings: {}\n").expect("写配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        let store = test_store(dir.path());
        let inbound = Inbound::new(manager, Some(Arc::clone(&store)));

        let payload = json!({
            "id": "",
            "group_id": "g0123456789abcdef0123456789abcdef",
            "content": "hello"
        });
        let events = inbound
            .convert("GROUP_AT_MESSAGE_CREATE", Some(&payload), None)
            .expect("缺 author 的群消息仍应转换");
        let Event::Message(message) = &events[0] else {
            panic!("GROUP_AT_MESSAGE_CREATE 应转群消息");
        };
        assert!(
            message.user_id.as_str() == Some(""),
            "无 author 时 user_id 回退空串"
        );
        assert!(
            message.message_id.as_str() == Some(""),
            "空 message_id 原样上报"
        );
        // 空的身份串不能进 idmap（旧逐条路径同样不写，这里保证批量路径一致）
        assert!(
            store
                .lookup_virtual_value("")
                .expect_err("空身份不应登记")
                .is_not_found()
        );
    }

    #[test]
    fn group_message_string_mode_registers_raw_openid() {
        use gensokyo_protocol::Event;

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "string_ob11: true\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        let store = test_store(dir.path());
        let inbound = Inbound::new(manager, Some(Arc::clone(&store)));

        let events = inbound
            .convert("GROUP_AT_MESSAGE_CREATE", Some(&group_at_payload()), None)
            .expect("群消息应可转换");
        let Event::Message(message) = &events[0] else {
            panic!("GROUP_AT_MESSAGE_CREATE 应转成群消息");
        };
        // string 模式：注册 section 就是上报的原始 OpenID
        assert!(
            message
                .group_id
                .as_ref()
                .is_some_and(|id| id.as_str() == Some("g0123456789abcdef0123456789abcdef"))
        );
        assert_eq!(
            store
                .read_config("g0123456789abcdef0123456789abcdef", "type")
                .expect("群应已注册"),
            "group"
        );
    }

    /// 入站正文 BotID → AppID（Go `RevertTransformedText` 的 `message_parser.go:1850-1854`）：
    /// 群聊与私聊同一口径，否则客户端认不出 @bot（`self_id` 用的是 AppID）。
    /// 还原后的 `<@!AppID>` 是 @bot 自身：AT 事件在 `remove_at` 关闭时归一成
    /// `[CQ:at,qq=<self_id>]`（`message_parser.go:1883`），全量群消息则直接剥离。
    #[test]
    fn bot_id_in_content_is_reported_as_app_id() {
        use gensokyo_protocol::{Event, MessageContent};

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "app_id: 102001234\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        let store = test_store(dir.path());
        let inbound = Inbound::new(manager, Some(store)).with_bot_id("2633000001");

        let mut group = group_at_payload();
        group["content"] = json!("<@!2633000001> 你好");
        let events = inbound
            .convert("GROUP_AT_MESSAGE_CREATE", Some(&group), None)
            .expect("群消息应可转换");
        let Event::Message(message) = &events[0] else {
            panic!("GROUP_AT_MESSAGE_CREATE 应转成群消息");
        };
        let MessageContent::Text(text) = &message.message else {
            panic!("群消息正文应为文本");
        };
        assert_eq!(text, "[CQ:at,qq=102001234] 你好");
        assert_eq!(message.raw_message, "[CQ:at,qq=102001234] 你好", "两条同源");

        let private = json!({
            "id": "m-2",
            "content": "<@!2633000001> 你好",
            "author": {"id": "u0123456789abcdef0123456789abcdef"}
        });
        let events = inbound
            .convert("C2C_MESSAGE_CREATE", Some(&private), None)
            .expect("C2C 消息应可转换");
        let Event::Message(message) = &events[0] else {
            panic!("C2C_MESSAGE_CREATE 应转成私聊消息");
        };
        let MessageContent::Text(text) = &message.message else {
            panic!("私聊正文应为文本");
        };
        assert_eq!(text, "[CQ:at,qq=102001234] 你好");
    }

    /// BotID 缺失时正文逐字透传（Go 的 `BotID != ""` 守卫：空串替换会打散正文）。
    #[test]
    fn missing_bot_id_keeps_content_verbatim() {
        use gensokyo_protocol::{Event, MessageContent};

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "app_id: 102001234\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        let store = test_store(dir.path());
        let inbound = Inbound::new(manager, Some(store));

        let mut group = group_at_payload();
        group["content"] = json!("<@!2633000001> 你好");
        let events = inbound
            .convert("GROUP_AT_MESSAGE_CREATE", Some(&group), None)
            .expect("群消息应可转换");
        let Event::Message(message) = &events[0] else {
            panic!("GROUP_AT_MESSAGE_CREATE 应转成群消息");
        };
        let MessageContent::Text(text) = &message.message else {
            panic!("群消息正文应为文本");
        };
        assert_eq!(text, "<@!2633000001> 你好");
    }

    /// `add_at_group` 只给全量群消息补 `[CQ:at,qq=<appid>]`（Go `ProcessGroupNormalMessage.go:148-150`）。
    #[test]
    fn add_at_group_prefixes_full_group_only() {
        use gensokyo_protocol::{Event, MessageContent};

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "app_id: 102001234\nadd_at_group: true\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        let store = test_store(dir.path());
        let inbound = Inbound::new(manager, Some(store));

        let events = inbound
            .convert("GROUP_MESSAGE_CREATE", Some(&group_at_payload()), None)
            .expect("全量群消息应可转换");
        let Event::Message(full) = &events[0] else {
            panic!("GROUP_MESSAGE_CREATE 应转成群消息");
        };
        let MessageContent::Text(text) = &full.message else {
            panic!("群消息正文应为文本");
        };
        assert_eq!(text, "[CQ:at,qq=102001234] hello");
        // Go 的 `message` 与 `raw_message` 同源（`ProcessGroupNormalMessage.go:205` / `:281`）
        assert_eq!(full.raw_message, "[CQ:at,qq=102001234] hello");

        // AT 事件的正文自带 @，再补一次就是重复 @（Go `ProcessGroupMessage.go:154-157`）
        let events = inbound
            .convert("GROUP_AT_MESSAGE_CREATE", Some(&group_at_payload()), None)
            .expect("群 @ 消息应可转换");
        let Event::Message(at) = &events[0] else {
            panic!("GROUP_AT_MESSAGE_CREATE 应转成群消息");
        };
        let MessageContent::Text(text) = &at.message else {
            panic!("群消息正文应为文本");
        };
        assert_eq!(text, "hello");
    }

    /// 开关关闭零变化：正文逐字透传。
    #[test]
    fn add_at_group_off_keeps_content_verbatim() {
        use gensokyo_protocol::{Event, MessageContent};

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "app_id: 102001234\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        let store = test_store(dir.path());
        let inbound = Inbound::new(manager, Some(store));

        let events = inbound
            .convert("GROUP_MESSAGE_CREATE", Some(&group_at_payload()), None)
            .expect("全量群消息应可转换");
        let Event::Message(full) = &events[0] else {
            panic!("GROUP_MESSAGE_CREATE 应转成群消息");
        };
        let MessageContent::Text(text) = &full.message else {
            panic!("群消息正文应为文本");
        };
        assert_eq!(text, "hello");
        assert_eq!(full.raw_message, "hello");
    }

    /// 单聊不适用：Go 的私聊路径没有 `add_at_group`。
    #[test]
    fn add_at_group_ignores_private_msg() {
        use gensokyo_protocol::{Event, MessageContent};

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "app_id: 102001234\nadd_at_group: true\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        let store = test_store(dir.path());
        let inbound = Inbound::new(manager, Some(store));

        let events = inbound
            .convert("C2C_MESSAGE_CREATE", Some(&c2c_payload()), None)
            .expect("单聊消息应可转换");
        let Event::Message(c2c) = &events[0] else {
            panic!("C2C_MESSAGE_CREATE 应转成单聊消息");
        };
        let MessageContent::Text(text) = &c2c.message else {
            panic!("单聊正文应为文本");
        };
        assert_eq!(text, "hi");
    }

    /// Go `main.go:765-770` / `:787-792` / `:1124-1129`：`enableChangeWord` 打开时正文与昵称
    /// 都过 `CheckWordIN`；关掉则逐字透传。
    #[test]
    fn change_word_filters_content_and_nickname_when_enabled() {
        use gensokyo_protocol::{Event, MessageContent};

        if !gensokyo_core::acnode::is_installed() {
            let dir = std::env::temp_dir()
                .join(format!("gensokyo-inbound-acnode-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("建临时目录失败");
            std::fs::write(dir.join("sensitive_words_in.txt"), "违禁字样####□\n").unwrap();
            std::fs::write(dir.join("sensitive_words_out.txt"), "").unwrap();
            std::fs::write(dir.join("white.txt"), "").unwrap();
            assert!(gensokyo_core::acnode::load_into_process(&dir, "*"));
            let _ = std::fs::remove_dir_all(&dir);
        }

        let payload = || {
            json!({
                "id": "m-1",
                "group_id": "g0123456789abcdef0123456789abcdef",
                "content": "违禁字样混进来了",
                "author": {
                    "id": "u0123456789abcdef0123456789abcdef",
                    "username": "违禁字样"
                }
            })
        };

        let on_dir = tempfile::tempdir().expect("临时目录");
        let on = on_dir.path().join("config.yml");
        std::fs::write(&on, "app_id: 102001234\nenableChangeWord: true\n").expect("写测试配置失败");
        let inbound = Inbound::new(
            Arc::new(ConfigManager::new(&on).expect("加载配置失败")),
            Some(test_store(on_dir.path())),
        );
        let events = inbound
            .convert("GROUP_MESSAGE_CREATE", Some(&payload()), None)
            .expect("全量群消息应可转换");
        let Event::Message(message) = &events[0] else {
            panic!("GROUP_MESSAGE_CREATE 应转成群消息");
        };
        let MessageContent::Text(text) = &message.message else {
            panic!("群消息正文应为文本");
        };
        assert_eq!(text, "□混进来了");
        assert_eq!(
            message
                .sender
                .as_ref()
                .and_then(|sender| sender.nickname.as_deref()),
            Some("□")
        );

        let off_dir = tempfile::tempdir().expect("临时目录");
        let off = off_dir.path().join("config.yml");
        std::fs::write(&off, "app_id: 102001234\n").expect("写测试配置失败");
        let inbound = Inbound::new(
            Arc::new(ConfigManager::new(&off).expect("加载配置失败")),
            Some(test_store(off_dir.path())),
        );
        let events = inbound
            .convert("GROUP_MESSAGE_CREATE", Some(&payload()), None)
            .expect("全量群消息应可转换");
        let Event::Message(message) = &events[0] else {
            panic!("GROUP_MESSAGE_CREATE 应转成群消息");
        };
        let MessageContent::Text(text) = &message.message else {
            panic!("群消息正文应为文本");
        };
        assert_eq!(text, "违禁字样混进来了");
        assert_eq!(
            message
                .sender
                .as_ref()
                .and_then(|sender| sender.nickname.as_deref()),
            Some("违禁字样")
        );
    }

    fn c2c_payload() -> serde_json::Value {
        json!({
            "id": "m-c2c",
            "content": "hi",
            "author": {"id": "u0123456789abcdef0123456789abcdef"}
        })
    }

    /// 群消息入站喂 echo：懒池两个维度 + msg_id / msg_type 映射（Go 群消息路径）。
    #[test]
    fn group_message_feeds_echo_pool_and_msg_maps() {
        use gensokyo_protocol::Event;
        use gensokyo_storage::echo::{echo_key, echo_key_v2};

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings: {}\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        let store = test_store(dir.path());
        let echo = Arc::new(EchoStore::new());
        let inbound = Inbound::new(Arc::clone(&manager), Some(Arc::clone(&store)))
            .with_echo(Arc::clone(&echo));

        let events = inbound
            .convert("GROUP_AT_MESSAGE_CREATE", Some(&group_at_payload()), None)
            .expect("群消息应可转换");
        let Event::Message(message) = &events[0] else {
            panic!("GROUP_AT_MESSAGE_CREATE 应转成群消息");
        };
        let app_id = manager.snapshot().get_app_id().to_string();
        let group = message.group_id.clone().expect("群消息应带 group_id");
        let group_num = group.as_i64().expect("有 idmap 时应为数字形态");
        let user_num = message.user_id.as_i64().expect("有 idmap 时应为数字形态");

        assert_eq!(
            echo.get_lazy_messages_id(&group.to_string()),
            "m-1",
            "群维度懒池"
        );
        assert_eq!(
            echo.get_lazy_messages_id_v2(&group.to_string(), &message.user_id.to_string()),
            "m-1",
            "群 × 用户维度懒池"
        );
        assert_eq!(
            echo.get_msg_id_by_key(&echo_key(&app_id, group_num)),
            "m-1",
            "虚拟群号 → 真实 msg_id"
        );
        assert_eq!(
            echo.get_msg_id_by_key(&echo_key_v2(&app_id, group_num, user_num)),
            "m-1",
            "群 × 用户 → 真实 msg_id"
        );
        assert_eq!(
            echo.get_message_type_by_group_id_v2(&app_id, &group.to_string()),
            "group"
        );
        let msg_num = message
            .message_id
            .as_i64()
            .expect("有 idmap 时应为数字形态");
        assert_eq!(echo.get_msg_id_by_key(&echo_key(&app_id, msg_num)), "m-1");
    }

    /// C2C 入站只喂 v1 槽（Go `ProcessC2CMessage.go:171` 无 v2 维度），键为虚拟用户号。
    #[test]
    fn c2c_message_feeds_echo_v1_slot_only() {
        use gensokyo_protocol::Event;
        use gensokyo_storage::echo::{DEFAULT_MESSAGE_ID, echo_key};

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings: {}\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        let store = test_store(dir.path());
        let echo = Arc::new(EchoStore::new());
        let inbound = Inbound::new(Arc::clone(&manager), Some(Arc::clone(&store)))
            .with_echo(Arc::clone(&echo));

        let events = inbound
            .convert("C2C_MESSAGE_CREATE", Some(&c2c_payload()), None)
            .expect("私聊消息应可转换");
        let Event::Message(message) = &events[0] else {
            panic!("C2C_MESSAGE_CREATE 应转成私聊消息");
        };
        let app_id = manager.snapshot().get_app_id().to_string();
        let user = message.user_id.to_string();
        let user_num = message.user_id.as_i64().expect("有 idmap 时应为数字形态");

        assert_eq!(echo.get_lazy_messages_id(&user), "m-c2c", "私聊复用 v1 槽");
        assert_eq!(
            echo.get_msg_id_by_key(&echo_key(&app_id, user_num)),
            "m-c2c"
        );
        assert_eq!(
            echo.get_lazy_messages_id_v2(&user, &user),
            DEFAULT_MESSAGE_ID,
            "私聊不写 v2 维度"
        );
    }

    /// C2C 私信把好友写进 `user_info`（Go `ProcessC2CMessage.go:231-241`）——`get_friend_list`
    /// 的数据源；写侧键/value 与重复键语义由 storage 用例覆盖，这里只看接线与持久化。
    #[test]
    fn c2c_message_stores_user_info() {
        use gensokyo_protocol::Event;

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings: {}\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));

        let user_id;
        {
            let store = test_store(dir.path());
            let inbound = Inbound::new(Arc::clone(&manager), Some(Arc::clone(&store)));

            let events = inbound
                .convert("C2C_MESSAGE_CREATE", Some(&c2c_payload()), None)
                .expect("私聊消息应可转换");
            let Event::Message(message) = &events[0] else {
                panic!("C2C_MESSAGE_CREATE 应转成私聊消息");
            };
            assert!(
                message.user_id.as_i64().is_some(),
                "有 idmap 时应上报数字形态"
            );
            user_id = message.user_id.to_string();
            assert_eq!(
                store.list_all_users().expect("枚举"),
                vec![UserInfoRow {
                    user_id: user_id.clone(),
                    ..UserInfoRow::default()
                }],
                "昵称 / 备注 Go 恒写空串（ProcessC2CMessage.go:235-239）"
            );

            // 同一好友的第二条私信命中重复键：Go 丢弃错误（`:241`），事件照常上报
            inbound
                .convert("C2C_MESSAGE_CREATE", Some(&c2c_payload()), None)
                .expect("重复写 user_info 不得影响事件上报");
            assert_eq!(store.list_all_users().expect("枚举").len(), 1);
        }

        // 关库重开仍在：持久化不变式（`get_friend_list` 跨重启可见）
        let reopened = test_store(dir.path());
        let rows = reopened.list_all_users().expect("枚举");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].user_id, user_id);
        assert!(rows[0].nickname.is_empty() && rows[0].remark.is_empty());
    }

    /// 空 author 的 C2C 事件**不写** `user_info`（Go `ProcessC2CMessage.go:30-33` 对
    /// `data.Author == nil || data.Author.ID == ""` 直接 `return nil`，库里绝不会有键 `":"`
    /// 的垃圾行）。事件本身照常上报——上报形态不由本守卫负责。
    #[test]
    fn c2c_message_without_author_id_writes_no_user_info() {
        use gensokyo_protocol::Event;

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings: {}\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        let store = test_store(dir.path());
        let inbound = Inbound::new(Arc::clone(&manager), Some(Arc::clone(&store)));

        // Go 的两种空法（`Author.ID == ""` 与 `Author == nil`）在 Rust 侧都归一成空串
        let mut blank_id = c2c_payload();
        blank_id["author"]["id"] = json!("");
        let mut no_author = c2c_payload();
        no_author
            .as_object_mut()
            .expect("事件负载是对象")
            .remove("author");

        for payload in [&blank_id, &no_author] {
            let events = inbound
                .convert("C2C_MESSAGE_CREATE", Some(payload), None)
                .expect("空 author 的事件仍照常上报");
            assert!(
                matches!(events[0], Event::Message(_)),
                "C2C_MESSAGE_CREATE 应转成私聊消息"
            );
        }

        // 一条都不写：`list_all_users` 为空即排除了键退化成 `":"` 的那一行
        let rows = store.list_all_users().expect("枚举");
        assert!(rows.is_empty(), "空 OpenID 不得写 user_info: {rows:?}");
    }

    /// 入群 / 邀请入群把 event_id 写进 echo（Go `ProcessGroupMember.go:67`、`ProcessGroupAddBot.go:157`）。
    #[test]
    fn group_events_feed_echo_event_id() {
        use gensokyo_protocol::Event;
        use gensokyo_storage::echo::{echo_key_event_id, echo_key_event_id_v2};

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings: {}\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        let store = test_store(dir.path());
        let echo = Arc::new(EchoStore::new());
        let inbound = Inbound::new(Arc::clone(&manager), Some(Arc::clone(&store)))
            .with_echo(Arc::clone(&echo));
        let app_id = manager.snapshot().get_app_id().to_string();
        let group_openid = "g0123456789abcdef0123456789abcdef";

        let member_add = json!({
            "event_id": "ev-member",
            "group_openid": group_openid,
            "member_openid": "u0123456789abcdef0123456789abcdef"
        });
        let events = inbound
            .convert("GROUP_MEMBER_ADD", Some(&member_add), None)
            .expect("入群事件应可转换");
        assert!(matches!(events[0], Event::Notice(_)));
        assert_eq!(
            echo.get_event_id_by_key(&echo_key_event_id_v2(&app_id, group_openid)),
            "ev-member"
        );

        let add_bot = json!({
            "event_id": "ev-invite",
            "group_openid": group_openid,
            "op_member_openid": "u0123456789abcdef0123456789abcdef"
        });
        let _ = inbound
            .convert("GROUP_ADD_ROBOT", Some(&add_bot), None)
            .expect("邀请入群事件应可转换");
        let (_, group_num) = store
            .retrieve_virtual_value(group_openid)
            .expect("群应有虚拟号");
        assert_eq!(
            echo.get_event_id_by_key(&echo_key_event_id(&app_id, group_num)),
            "ev-invite"
        );
    }

    /// 未装配 echo 时入站照常上报（池写入静默跳过，不 panic）。
    #[test]
    fn convert_without_echo_store_still_reports() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings: {}\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        let inbound = Inbound::new(manager, None);

        let events = inbound
            .convert("GROUP_AT_MESSAGE_CREATE", Some(&group_at_payload()), None)
            .expect("群消息应可转换");
        assert_eq!(events.len(), 1);
    }
}
