//! WS 客户端全部不可达时的兜底回复（Go `Processor/Processor.go:374-399`）。
//!
//! Go 在 `BroadcastMessageToAll` 末尾统计 WS 发送结果：`!http_only_bot` 且
//! `failed == len(Wsclient)+len(WsServerClients)`（两个列表都为空时 `0 == 0` 同样命中，
//! 该开关正是为屏蔽这种无客户端场景）时，用 `downtime_message` 向事件来源回一条**被动**
//! 消息（`MsgID` = 原始消息 ID、`MsgSeq = 1`、`MsgType = 0`）：
//!
//! - `*dto.WSGroupATMessageData` → `PostGroupMessage(v.GroupID, …)`（@ 机器人的群消息）
//! - `*dto.WSC2CMessageData` → `PostC2CMessage(v.Author.ID, …)`
//!
//! 其余事件类型不在 switch 里，Go 不会回——包括 notice / request，以及全量群消息
//! （`GROUP_MESSAGE_CREATE` 传的是 `*dto.WSGroupMessageData`，类型对不上）。
//!
//! # 与 Go 的差异
//!
//! - Go 的触发条件是「所有 WS 发送都失败」；broadcast 没有逐订阅者的写失败语义，这里用
//!   「没有任何 OneBot 下游订阅者」等价替代（[`EventHub::downstream_count`]，已扣除
//!   `post_url` 上报这类后台订阅者）。因此「WS 连接在但写失败」这一支不会触发兜底。
//! - `downtime_message` 为空时不发：Go 会照发一条空正文（必然被 API 拒绝），这里直接跳过。
//! - 事件里的 ID 是 OneBot 侧 ID：`string_ob11 = true` 时本身就是原始 OpenID / 消息 ID，
//!   数字形态经 idmap 反查（[`IdMapStore::retrieve_row_by_id`] /
//!   [`IdMapStore::retrieve_row_by_cache`]）。反查失败时 Go 不存在这种情况（它拿着原始
//!   DTO），这里 `warn` 后跳过，**不**退回主动消息（主动群消息 / C2C 唤醒是另一套语义）。
//! - 没有节流：Go 也没有，同一来源的高频事件每次发布都会回一条（每次都真发一次 API 请求）。
//!   `http_only_bot: true` 是关掉这条兜底的总开关。

use std::sync::Arc;

use gensokyo_actions::ActionContext;
use gensokyo_config::ConfigManager;
use gensokyo_core::Ob11Id;
use gensokyo_protocol::Event;
use gensokyo_protocol::event::MessageType;
use gensokyo_qq::dto::{MessageToCreate, msg_type};
use gensokyo_storage::IdMapStore;

use crate::event_hub::EventHub;
use crate::state::AppState;

/// 兜底回复的投递目标（上游 OpenID）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// 群聊（`POST /v2/groups/{group_openid}/messages`）。
    Group(String),
    /// 单聊（`POST /v2/users/{user_openid}/messages`）。
    Private(String),
}

/// 一条待发的兜底回复。
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    /// 投递目标。
    pub target: Target,
    /// 请求体：正文 + 被动回复的 `msg_id` + `msg_seq = 1` + `msg_type = 0`。
    pub message: MessageToCreate,
}

/// 兜底回复器：从 [`AppState`] 取总线、配置、idmap 与 action 依赖。
#[derive(Debug, Clone)]
pub struct Downtime {
    hub: EventHub,
    config: Arc<ConfigManager>,
    idmap: Option<Arc<IdMapStore>>,
    actions: Option<Arc<ActionContext>>,
}

impl Downtime {
    /// 取状态里的句柄；每个字段都是 `Arc` 克隆，构造开销可忽略。
    #[must_use]
    pub fn from_state(state: &AppState) -> Self {
        Self {
            hub: state.hub().clone(),
            config: state.config_arc(),
            idmap: state.idmap_arc(),
            actions: state.actions_arc(),
        }
    }

    /// 发布一条事件；需要兜底时**异步**回一条，不阻塞发布方。
    ///
    /// 返回广播送达的订阅者数量（与 [`EventHub::publish`] 一致）。
    pub fn publish(&self, event: Event) -> usize {
        let event = Arc::new(event);
        let delivered = self.hub.publish_shared(Arc::clone(&event));
        if let Some(notice) = self.notice(&event) {
            let downtime = self.clone();
            tokio::spawn(async move { downtime.send(&notice, &event).await });
        }
        delivered
    }

    /// 判定这条事件要不要兜底、发给谁；`None` 表示不发。
    #[must_use]
    pub fn notice(&self, event: &Event) -> Option<Notice> {
        let settings = self.config.snapshot();
        if settings.get_http_only_bot() || self.hub.downstream_count() > 0 {
            return None;
        }
        let content = settings.get_downtime_message();
        if content.is_empty() {
            return None;
        }

        // Go 的 switch 只匹配入站消息 DTO：`message_sent`（机器人自己发出的消息）不在其中。
        let Event::Message(message) = event else {
            return None;
        };
        // Go 先用 `MsgID: v.ID` 组被动回复；反查不到就发不了被动消息。
        let msg_id = self.raw_message_id(&message.message_id)?;
        let target = match message.message_type {
            // switch 只匹配 `*dto.WSGroupATMessageData`：`to_me` 只有 @ 机器人的群消息才有。
            MessageType::Group if message.to_me == Some(true) => {
                Target::Group(self.raw_identity(message.group_id.as_ref())?)
            }
            MessageType::Private => Target::Private(self.raw_identity(Some(&message.user_id))?),
            _ => return None,
        };

        let message = MessageToCreate::text(content)
            .with_msg_id(msg_id)
            .with_msg_seq(1)
            .with_msg_type(msg_type::TEXT);
        Some(Notice { target, message })
    }

    /// 真发一次；失败只 `warn`（Go 同样忽略 `PostGroupMessage` / `PostC2CMessage` 的返回值）。
    async fn send(&self, notice: &Notice, event: &Event) {
        let Some(actions) = self.actions.as_deref() else {
            // 没有凭据时 Go 连网关都不会启动，这里同样不发。
            tracing::debug!("未装配 action 依赖，downtime 兜底回复跳过");
            return;
        };
        let api = actions.api();
        let result = match &notice.target {
            Target::Group(openid) => {
                api.send_group_message(openid.clone(), notice.message.clone())
                    .await
            }
            Target::Private(openid) => {
                api.send_c2c_message(openid.clone(), notice.message.clone())
                    .await
            }
        };
        match result {
            Ok(response) => tracing::info!(
                post_type = event.post_type(),
                target = ?notice.target,
                msg_id = %response.id,
                "没有 WS 订阅者，已回 downtime 兜底消息"
            ),
            Err(err) => tracing::warn!(
                post_type = event.post_type(),
                target = ?notice.target,
                error = %err,
                "downtime 兜底消息发送失败"
            ),
        }
    }

    /// OneBot 身份 ID → 上游 OpenID（string 形态原样，数字形态经 idmap 反查）。
    fn raw_identity(&self, id: Option<&Ob11Id>) -> Option<String> {
        let id = id?;
        if let Some(raw) = id.as_str() {
            return (!raw.is_empty()).then(|| raw.to_owned());
        }
        let vuin = id.as_i64()?;
        let openid = self
            .idmap
            .as_deref()?
            .retrieve_row_by_id(vuin)
            .ok()
            .filter(|openid| !openid.is_empty());
        if openid.is_none() {
            tracing::warn!(vuin, "downtime 兜底回复跳过：虚拟号未绑定 OpenID");
        }
        openid
    }

    /// 上报的消息 ID → 上游消息 ID（string 形态原样，数字形态经 idmap 缓存反查）。
    fn raw_message_id(&self, id: &Ob11Id) -> Option<String> {
        if let Some(raw) = id.as_str() {
            return (!raw.is_empty()).then(|| raw.to_owned());
        }
        let vuin = id.as_i64()?;
        let msg_id = self
            .idmap
            .as_deref()?
            .retrieve_row_by_cache(vuin)
            .ok()
            .filter(|msg_id| !msg_id.is_empty());
        if msg_id.is_none() {
            tracing::warn!(vuin, "downtime 兜底回复跳过：消息 ID 未绑定原始 ID");
        }
        msg_id
    }
}
