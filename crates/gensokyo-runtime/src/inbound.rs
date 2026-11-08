//! QQ 事件 → OneBot V11 事件（Go `Processor/`）。
//!
//! 入站转换只做「协议翻译」：把 QQ 机器人平台推送的 `d` 负载（以及 `t` 事件名）变成
//! [`gensokyo_protocol::Event`]，不碰框架指令、黑白名单、被动转主动等
//! [`Processor`](https://github.com/hoshinonyaruko/gensokyo) 里的业务逻辑。
//!
//! 除把群注册进 idmap 外，副作用有两个：喂 echo（[`Inbound::with_echo`]）——消息事件写懒
//! message_id 池与 msg_id / msg_type 映射，入群类事件写 event_id，供出站解析被动回复凭据；
//! 以及收发统计（`Inbound::record_message_received`）——三个消息事件各记一次当日接收数
//! （Go `botstats.RecordMessageReceived`）。两者未装配时静默跳过，绝不阻断事件上报。
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
//! - `!native_ob11` 时补 `real_user_id` / `real_group_id` / `real_message_type` 附加字段
//!   （与 Go 的「增强配置」一致）。例外：`GROUP_MEMBER_ADD/REMOVE` 在 Go 里**不区分**
//!   `native_ob11`（`RealUserID`/`RealGroupID` 只被 `omitempty` 省略空值），这里照搬。
//! - `time`：QQ 负载带 `timestamp` 时用它，否则取当前秒级时间戳。Go 群消息用 `time.Now()`，
//!   这里偏向可复现（同一事件重放得到同一 `time`）。`GROUP_JOIN_REQUEST` 的 `apply_at`
//!   优先按 RFC3339 解析（Go `time.Parse`），失败再按秒级数字解析。
//!
//! 金样（`Processor/testdata/inbound/`）是这些字段口径的回归保险：改动事件形态时先跑
//! `tests/golden_inbound.rs`。
//!
//! # 未做项
//!
//! - 消息段：图片 / 语音 / 视频 / ark 卡片 / markdown / card 一律停留在文本形态，
//!   `array_value` 不生效（`message` 恒为字符串形态，且没有 `foundItems` 解析：
//!   出站方向的 markdown/card/媒体分片属于 `handlers/` 的职责）；`[CQ:at]` 反解
//!   （Go `RevertTransformedText`）未做。
//! - `global_group_msg_rre_to_message` / `global_c2c_msg_switch_to_message`（`true` 时 Go 把
//!   这些开关事件「伪装成 message」的业务转换，`ConvertGroupRejectToMessage` 等）未做：
//!   两个开关打开时这里仍只上报 notice。
//! - `echo` 的 `app_id/s` 映射、双路 echo、`is_binded_*`、`avatar`、master 角色提升、
//!   idmap-pro（`StoreIDv2Pro`）与 `union_id` 切换；`GROUP_MEMBER_ADD` / `GROUP_JOIN_REQUEST`
//!   的 `event_id` 只落在 `extra`，Go 侧 `echo.AddEvnetIDv2` 的被动回复缓存未做。
//! - `message_sent`、`request` 的 `flag` 审批链路（Go 侧 `GROUP_ADD_ROBOT` 的 `flag` 也是空串）。
//! - `global_interaction_to_message`（`true` 时 Go `ConvertInteractionToMessage` 把按钮回调
//!   「伪装成 message」）：`INTERACTION_CREATE` 只走上报 `notice.interaction` 的主路径。
//! - `auto_put_interaction` / `put_interaction_except`（自动回 `PutInteraction`）、
//!   `union_id` 切换与 idmap-pro：回调的 `id` 原样留在 `data.id`，由客户端自己回应。
//! - `MESSAGE_AUDIT_*`、`FORUM_*`、`AUDIO_*` 与频道 `GUILD_*`。
//! - `union_webhook` 扇出（见 [`crate`] 文档）。

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use gensokyo_config::{ConfigManager, Settings};
use gensokyo_core::telemetry::metrics;
use gensokyo_core::{Ob11Id, acnode};
use gensokyo_protocol::event::{
    HeartbeatStatus, MessageEvent, MessageType, MetaEvent, NoticeEvent, RequestEvent, Sender,
};
use gensokyo_protocol::{Event, Extra, MessageContent};
use gensokyo_qq::dto::interaction::Interaction;
use gensokyo_storage::{BotStatsStore, EchoStore, IdMapStore};
use serde::Deserialize;
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
}

/// QQ 消息作者。
#[derive(Debug, Deserialize)]
struct QqAuthor {
    #[serde(default)]
    id: String,
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

/// 入站转换器：配置快照 + 可选 idmap / echo / botstats。
#[derive(Debug, Clone)]
pub struct Inbound {
    config: Arc<ConfigManager>,
    idmap: Option<Arc<IdMapStore>>,
    echo: Option<Arc<EchoStore>>,
    botstats: Option<Arc<BotStatsStore>>,
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
        }
    }

    /// 从 [`AppState`] 取配置、idmap、echo 与 botstats（webhook 每个请求现构造，Arc 克隆开销可忽略）。
    #[must_use]
    pub fn from_state(state: &AppState) -> Self {
        Self {
            config: state.config_arc(),
            idmap: state.idmap_arc(),
            echo: state.echo_arc(),
            botstats: state.botstats_arc(),
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

    /// OpenID → OneBot ID；数字形态优先走 idmap，失败不伪造数字。
    fn ob11_id(&self, settings: &Settings, raw: &str) -> Ob11Id {
        if raw.is_empty() || settings.get_string_ob11() {
            return Ob11Id::Str(raw.to_owned());
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

    /// QQ message_id → OneBot message_id（数字形态经 idmap 的 echo 映射）。
    fn message_id(&self, settings: &Settings, raw: &str) -> Ob11Id {
        if raw.is_empty() || settings.get_string_ob11() {
            return Ob11Id::Str(raw.to_owned());
        }
        let Some(idmap) = self.idmap.as_deref() else {
            return Ob11Id::Str(raw.to_owned());
        };
        match idmap.store_cache(raw) {
            Ok(virtual_id) => Ob11Id::Num(virtual_id),
            Err(err) => {
                tracing::warn!(message_id = raw, error = %err, "message_id 虚拟化失败，回退原始字符串");
                Ob11Id::Str(raw.to_owned())
            }
        }
    }

    /// 群消息喂 echo（Go `ProcessGroupMessage.go:283-302`、`ProcessGroupNormalMessage.go:262-273`）。
    ///
    /// 数字形态用虚拟号做键；`string_action` 时再补一份原始 OpenID 键（Go string 分支只写后者）。
    /// echo 是内存表、没有失败路径；未装配时静默跳过，绝不影响事件上报。
    fn feed_group_echo(
        &self,
        settings: &Settings,
        payload: &QqMessage,
        group_id: &Ob11Id,
        user_id: &Ob11Id,
        message_id: &Ob11Id,
    ) {
        let Some(echo) = self.echo.as_deref() else {
            return;
        };
        let app_id = settings.get_app_id().to_string();
        let now = SystemTime::now();
        let raw_user = payload.author.as_ref().map_or("", |author| author.id.as_str());

        // 懒 message_id 池：群维度 + 群 × 用户维度（被动回复复用同一 msg_id）。
        let group_key = group_id.to_string();
        echo.add_lazy_message_id(&group_key, &payload.id, now);
        echo.add_lazy_message_id_v2(&group_key, &user_id.to_string(), &payload.id, now);
        if settings.get_string_action() && group_key != payload.group_id {
            echo.add_lazy_message_id(&payload.group_id, &payload.id, now);
            echo.add_lazy_message_id_v2(&payload.group_id, raw_user, &payload.id, now);
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
        let raw_user = payload.author.as_ref().map_or("", |author| author.id.as_str());
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
        let user_raw = author.map_or("", |author| author.id.as_str());

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
        // 数字形态的 `OnebotGroupMessage` 根本没有该字段。
        if settings.get_string_ob11() {
            extra.insert("platform".to_owned(), Value::String("qq".to_owned()));
        }

        let sender = Sender {
            // Go M1-B：string 形态不做 idmap 转换，`sender.user_id` 恒为 0。
            user_id: Some(if settings.get_string_ob11() {
                Ob11Id::num(0)
            } else {
                self.ob11_id(settings, user_raw)
            }),
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

        let message_id = self.message_id(settings, &payload.id);
        let group_id = self.ob11_id(settings, &payload.group_id);
        let user_id = self.ob11_id(settings, user_raw);
        self.register_group(&group_id);
        self.feed_group_echo(settings, &payload, &group_id, &user_id, &message_id);
        let content = reported_group_content(settings, payload.content, flavor);
        Ok(Event::Message(MessageEvent {
            time: timestamp_of(payload.timestamp.as_ref()),
            self_id: Ob11Id::num(self.self_id_of(settings)),
            message_type: MessageType::Group,
            sub_type: Some("normal".to_owned()),
            message_id,
            user_id,
            group_id: Some(group_id),
            message: MessageContent::Text(content.clone()),
            raw_message: content,
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

        let sender = Sender {
            user_id: Some(self.ob11_id(settings, user_raw)),
            // Go `PrivateSenderS`：nickname / tiny_id 非 omitempty，恒上报（空串）。
            nickname: Some(String::new()),
            tiny_id: Some(String::new()),
            ..Sender::default()
        };

        let message_id = self.message_id(settings, &payload.id);
        let user_id = self.ob11_id(settings, user_raw);
        self.feed_private_echo(settings, &payload, &user_id, &message_id);
        Ok(Event::Message(MessageEvent {
            time: timestamp_of(payload.timestamp.as_ref()),
            self_id: Ob11Id::num(self.self_id_of(settings)),
            message_type: MessageType::Private,
            sub_type: Some("friend".to_owned()),
            message_id,
            user_id,
            group_id: None,
            message: MessageContent::Text(payload.content.clone()),
            raw_message: payload.content,
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
        extra.insert("data".to_owned(), serde_json::to_value(&payload)?);

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
fn reported_group_content(settings: &Settings, content: String, flavor: GroupFlavor) -> String {
    if flavor != GroupFlavor::Full || !settings.get_add_at_group() {
        return content;
    }
    format!("[CQ:at,qq={}] {content}", settings.get_app_id())
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
    use super::{auto_approved_pair, parse_rfc3339, timestamp_of, verify_comment};
    use serde_json::json;

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
    use gensokyo_storage::{EchoStore, IdMapOptions, IdMapStore};

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
        let inbound =
            Inbound::new(Arc::clone(&manager), Some(Arc::clone(&store))).with_echo(Arc::clone(&echo));

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

        assert_eq!(echo.get_lazy_messages_id(&group.to_string()), "m-1", "群维度懒池");
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
        let msg_num = message.message_id.as_i64().expect("有 idmap 时应为数字形态");
        assert_eq!(echo.get_msg_id_by_key(&echo_key(&app_id, msg_num)), "m-1");
    }

    /// C2C 入站只喂 v1 槽（Go `ProcessC2CMessage.go:171` 无 v2 维度），键为虚拟用户号。
    #[test]
    fn c2c_message_feeds_echo_v1_slot_only() {
        use gensokyo_protocol::Event;
        use gensokyo_storage::echo::{echo_key, DEFAULT_MESSAGE_ID};

        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings: {}\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        let store = test_store(dir.path());
        let echo = Arc::new(EchoStore::new());
        let inbound =
            Inbound::new(Arc::clone(&manager), Some(Arc::clone(&store))).with_echo(Arc::clone(&echo));

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
        assert_eq!(echo.get_msg_id_by_key(&echo_key(&app_id, user_num)), "m-c2c");
        assert_eq!(
            echo.get_lazy_messages_id_v2(&user, &user),
            DEFAULT_MESSAGE_ID,
            "私聊不写 v2 维度"
        );
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
        let inbound =
            Inbound::new(Arc::clone(&manager), Some(Arc::clone(&store))).with_echo(Arc::clone(&echo));
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
