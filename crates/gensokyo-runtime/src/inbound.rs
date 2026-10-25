//! QQ 事件 → OneBot V11 事件（Go `Processor/`）。
//!
//! 入站转换只做「协议翻译」：把 QQ 机器人平台推送的 `d` 负载（以及 `t` 事件名）变成
//! [`gensokyo_protocol::Event`]，不碰 echo 映射、框架指令、黑白名单、被动转主动等
//! [`Processor`](https://github.com/hoshinonyaruko/gensokyo) 里的业务逻辑。
//!
//! # 已覆盖事件
//!
//! | QQ `t` | OneBot `post_type` | 说明 |
//! |--------|--------------------|------|
//! | `GROUP_AT_MESSAGE_CREATE` | `message`（group） | `sub_type=normal`、`to_me=true` |
//! | `GROUP_MESSAGE_CREATE` | `message`（group） | 全量群消息，`to_me=false`、`is_full_group_message=true` |
//! | `C2C_MESSAGE_CREATE` | `message`（private） | `sub_type=friend` |
//! | `GROUP_ADD_ROBOT` | `request` + `notice` | 对齐 Go：`request.group/invite` + `notice.group_increase` |
//! | `FRIEND_ADD` | `request` + `notice` | 对齐 Go：`request.friend` + `notice.friend_add` |
//! | `GROUP_DEL_ROBOT` | `notice` | `group_decrease` + `sub_type=kick_me` |
//! | `GROUP_MSG_REJECT` | `notice` | `group_reject` + `sub_type=create` |
//! | `GROUP_MSG_RECEIVE` | `notice` | `group_receive` + `sub_type=create` |
//!
//! 其余 `t`（`GUILD_*` / `INTERACTION_CREATE` / `C2C_MSG_REJECT` …）返回空列表：未知事件
//! 不能打断链路，这也是 spec「未知事件不 panic」的落点。
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
//!   （与 Go 的「增强配置」一致）。
//! - `time`：QQ 负载带 `timestamp` 时用它，否则取当前秒级时间戳。Go 群消息用 `time.Now()`，
//!   这里偏向可复现（同一事件重放得到同一 `time`）。
//!
//! # 未做项
//!
//! - 消息段：图片 / 语音 / 视频 / ark 卡片一律fold 进 `content` 文本，`array_value`
//!   不生效（`message` 恒为字符串形态）；`[CQ:at]` 反解（Go `RevertTransformedText`）未做。
//! - `echo` 的 `app_id/s` 映射、双路 echo、`is_binded_*`、`avatar`、master 角色提升、
//!   idmap-pro（`StoreIDv2Pro`）与 `union_id` 切换。
//! - `message_sent`、`request` 的 `flag` 审批链路（Go 侧 `GROUP_ADD_ROBOT` 的 `flag` 也是空串）。
//! - 群管理事件全量（`GROUP_MEMBER_ADD/REMOVE`、`GROUP_JOIN_REQUEST`、`INTERACTION_CREATE`、
//!   频道 `GUILD_*`）。

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use gensokyo_config::{ConfigManager, Settings};
use gensokyo_core::Ob11Id;
use gensokyo_protocol::event::{
    HeartbeatStatus, MessageEvent, MessageType, MetaEvent, NoticeEvent, RequestEvent, Sender,
};
use gensokyo_protocol::{Event, Extra, MessageContent};
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

/// 群消息的两种来源；只影响 `to_me` 与 `is_full_group_message`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupFlavor {
    /// `GROUP_AT_MESSAGE_CREATE`：必然 @ 了机器人。
    At,
    /// `GROUP_MESSAGE_CREATE`：全量群消息（通常没有 @ 机器人）。
    Full,
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

#[cfg(test)]
mod tests {
    use super::timestamp_of;
    use serde_json::json;

    #[test]
    fn timestamp_accepts_string_number_and_missing() {
        assert_eq!(timestamp_of(Some(&json!("1700000000"))), 1_700_000_000);
        assert_eq!(timestamp_of(Some(&json!(1_700_000_000))), 1_700_000_000);
        assert!(timestamp_of(Some(&json!("not-a-number"))) > 0);
        assert!(timestamp_of(None) > 0);
    }
}
