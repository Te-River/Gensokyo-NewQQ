//! QQ 事件 → OneBot V11 事件（Go `Processor/`）。
//!
//! 入站转换只做「协议翻译」：把 QQ 机器人平台推送的 `d` 负载（以及 `t` 事件名）变成
//! [`gensokyo_protocol::Event`]，不碰 echo 映射、框架指令、黑白名单、被动转主动等
//! [`Processor`](https://github.com/hoshinonyaruko/gensokyo) 里的业务逻辑。
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
//! - `group_id` / `user_id`：`string_ob11 = true` 时是原始 OpenID 字符串；数字形态时经
//!   [`IdMapStore::retrieve_virtual_value`] 取虚拟 ID，**idmap 未注入或分配失败则回退原始
//!   字符串**（不伪造数字）。
//! - `message_id`：字符串形态原样上报；数字形态经 [`IdMapStore::store_cache`] 拿虚拟 ID，
//!   失败同样回退字符串。
//! - `!native_ob11` 时补 `real_user_id` / `real_group_id` / `real_message_type` 附加字段
//!   （与 Go 的「增强配置」一致）。例外：`GROUP_MEMBER_ADD/REMOVE` 在 Go 里**不区分**
//!   `native_ob11`（`RealUserID`/`RealGroupID` 只被 `omitempty` 省略空值），这里照搬。
//! - `time`：QQ 负载带 `timestamp` 时用它，否则取当前秒级时间戳。Go 群消息用 `time.Now()`，
//!   这里偏向可复现（同一事件重放得到同一 `time`）。`GROUP_JOIN_REQUEST` 的 `apply_at`
//!   优先按 RFC3339 解析（Go `time.Parse`），失败再按秒级数字解析。
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
use gensokyo_core::Ob11Id;
use gensokyo_protocol::event::{
    HeartbeatStatus, MessageEvent, MessageType, MetaEvent, NoticeEvent, RequestEvent, Sender,
};
use gensokyo_protocol::{Event, Extra, MessageContent};
use gensokyo_qq::dto::interaction::Interaction;
use gensokyo_storage::IdMapStore;
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

/// 入站转换器：配置快照 + 可选 idmap。
#[derive(Debug, Clone)]
pub struct Inbound {
    config: Arc<ConfigManager>,
    idmap: Option<Arc<IdMapStore>>,
}

impl Inbound {
    /// 用配置与可选 idmap 构造。
    #[must_use]
    pub fn new(config: Arc<ConfigManager>, idmap: Option<Arc<IdMapStore>>) -> Self {
        Self { config, idmap }
    }

    /// 从 [`AppState`] 取配置与 idmap（webhook 每个请求现构造，Arc 克隆开销可忽略）。
    #[must_use]
    pub fn from_state(state: &AppState) -> Self {
        Self {
            config: state.config_arc(),
            idmap: state.idmap_arc(),
        }
    }

    /// 机器人自身 ID（`use_uin ? uin : app_id`）。
    #[must_use]
    pub fn self_id(&self) -> i64 {
        self.self_id_of(&self.config.snapshot())
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
        match kind {
            "GROUP_AT_MESSAGE_CREATE" => Ok(vec![self.group_message(
                &settings,
                data,
                GroupFlavor::At,
            )?]),
            "GROUP_MESSAGE_CREATE" => Ok(vec![self.group_message(
                &settings,
                data,
                GroupFlavor::Full,
            )?]),
            "C2C_MESSAGE_CREATE" => Ok(vec![self.private_message(&settings, data)?]),
            "GROUP_ADD_ROBOT" => self.group_add_robot(&settings, data),
            "GROUP_DEL_ROBOT" => {
                self.group_op_notice(&settings, data, "group_decrease", Some("kick_me"))
            }
            "GROUP_MSG_REJECT" => {
                self.group_op_notice(&settings, data, "group_reject", Some("create"))
            }
            "GROUP_MSG_RECEIVE" => {
                self.group_op_notice(&settings, data, "group_receive", Some("create"))
            }
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

        let sender = Sender {
            user_id: Some(self.ob11_id(settings, user_raw)),
            nickname: author.and_then(|author| non_empty(&author.username)),
            role: author
                .and_then(|author| author.member_role.clone())
                .or_else(|| Some("member".to_owned())),
            sex: Some("0".to_owned()),
            age: Some(0),
            area: Some("0".to_owned()),
            level: Some("0".to_owned()),
            ..Sender::default()
        };

        let message_id = self.message_id(settings, &payload.id);
        Ok(Event::Message(MessageEvent {
            time: timestamp_of(payload.timestamp.as_ref()),
            self_id: Ob11Id::num(self.self_id_of(settings)),
            message_type: MessageType::Group,
            sub_type: Some("normal".to_owned()),
            message_id,
            user_id: self.ob11_id(settings, user_raw),
            group_id: Some(self.ob11_id(settings, &payload.group_id)),
            message: MessageContent::Text(payload.content.clone()),
            raw_message: payload.content,
            font: Some(0),
            sender: Some(sender),
            anonymous: None,
            message_seq: None,
            to_me: Some(flavor == GroupFlavor::At),
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
            nickname: author.and_then(|author| non_empty(&author.username)),
            sex: Some("0".to_owned()),
            age: Some(0),
            area: Some("0".to_owned()),
            level: Some("0".to_owned()),
            ..Sender::default()
        };

        let message_id = self.message_id(settings, &payload.id);
        Ok(Event::Message(MessageEvent {
            time: timestamp_of(payload.timestamp.as_ref()),
            self_id: Ob11Id::num(self.self_id_of(settings)),
            message_type: MessageType::Private,
            sub_type: Some("friend".to_owned()),
            message_id,
            user_id: self.ob11_id(settings, user_raw),
            group_id: None,
            message: MessageContent::Text(payload.content.clone()),
            raw_message: payload.content,
            font: Some(0),
            sender: Some(sender),
            anonymous: None,
            message_seq: None,
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
        let group_id = self.ob11_id(settings, &payload.group_openid);
        let user_id = self.ob11_id(settings, &payload.op_member_openid);
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
        Ok(vec![Event::Notice(NoticeEvent {
            time: timestamp_of(payload.timestamp.as_ref()),
            self_id: Ob11Id::num(self.self_id_of(settings)),
            notice_type: notice_type.to_owned(),
            sub_type: sub_type.map(str::to_owned),
            group_id: Some(self.ob11_id(settings, &payload.group_openid)),
            user_id: Some(self.ob11_id(settings, &payload.op_member_openid)),
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

    fn friend_add(
        &self,
        settings: &Settings,
        data: &Value,
        envelope_id: Option<&str>,
    ) -> Result<Vec<Event>, serde_json::Error> {
        let payload = QqFriendAdd::deserialize(data)?;
        let time = timestamp_of(payload.timestamp.as_ref());
        let user_id = self.ob11_id(settings, &payload.openid);
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
        extra.insert("scene".to_owned(), Value::Number(payload.scene.into()));

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
        notice_extra.insert("scene_param".to_owned(), Value::String(payload.scene_param));
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
            user_id: Some(self.ob11_id(settings, &payload.openid)),
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
        let group_id = self.ob11_id(settings, &payload.group_openid);
        let user_id = self.ob11_id(settings, &payload.member_openid);

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
            user_id: Some(self.ob11_id(settings, &payload.member_openid)),
            group_id: Some(self.ob11_id(settings, &payload.group_openid)),
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
            group_id: non_empty(&payload.group_openid).map(|raw| self.ob11_id(settings, &raw)),
            user_id: non_empty(user_raw).map(|raw| self.ob11_id(settings, &raw)),
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

fn now_unix() -> i64 {
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
}
