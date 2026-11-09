//! 事件总线（`tokio::sync::broadcast`）。
//!
//! 入站侧（[`crate::webhook`] 的 QQ 回调、[`crate::inbound`] 的转换结果）只往这里
//! `publish`，出站侧（[`crate::ws_client`] 的反向 WS）各自 `subscribe` 一份。
//! 一条事件因此可以同时送达多个 OneBot 客户端，与 Go `BroadcastMessageToAll` 的语义一致。
//!
//! 语义取舍：
//! - 事件以 `Arc<Event>` 传递，多订阅者共享同一份反序列化结果，省掉逐连接克隆。
//! - broadcast 是**环形缓冲**：订阅者落后 [`DEFAULT_HUB_CAPACITY`] 条以上会收到
//!   `Lagged`，落后的部分直接丢弃（不阻塞发布方）。Go 的 `writeCh` 满了也是丢消息，
//!   这里同样是「宁可丢，不背压」。
//! - 没有订阅者时 `publish` 照样成功（返回 0）：Go 在无连接时同样只是发不出去。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gensokyo_protocol::Event;
use tokio::sync::broadcast;

/// 默认环形缓冲容量（条）。
pub const DEFAULT_HUB_CAPACITY: usize = 1024;

/// 进程内事件总线；克隆只复制发送端句柄。
#[derive(Debug, Clone)]
pub struct EventHub {
    sender: broadcast::Sender<Arc<Event>>,
    /// 后台订阅者数量（[`EventHub::subscribe_background`]），不计入
    /// [`EventHub::downstream_count`]。
    background: Arc<AtomicUsize>,
}

/// 后台订阅（`post_url` 上报这类非 OneBot 下游消费者）。
///
/// **持有** receiver 与计数：两者同生同灭，[`EventHub::downstream_count`] 的扣减项
/// 因此不可能与实际订阅者数量漂移。
#[derive(Debug)]
pub struct BackgroundSubscription {
    receiver: broadcast::Receiver<Arc<Event>>,
    background: Arc<AtomicUsize>,
}

impl BackgroundSubscription {
    /// 收下一条事件；错误语义与 [`broadcast::Receiver::recv`] 一致。
    ///
    /// # Errors
    /// 订阅者落后超过容量时 `Lagged`，总线关闭时 `Closed`。
    pub async fn recv(&mut self) -> Result<Arc<Event>, broadcast::error::RecvError> {
        self.receiver.recv().await
    }
}

impl Drop for BackgroundSubscription {
    fn drop(&mut self) {
        self.background.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new(DEFAULT_HUB_CAPACITY)
    }
}

impl EventHub {
    /// 用指定容量构造；`capacity = 0` 按 1 处理（broadcast 不接受 0）。
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(1));
        Self {
            sender,
            background: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// 新增一个订阅者；订阅之后发布的事件才会送达它。
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Event>> {
        self.sender.subscribe()
    }

    /// 后台订阅：非 OneBot 下游消费者（`post_url` 上报）。
    ///
    /// 返回的 [`BackgroundSubscription`] **同时持有** receiver 与计数：它 drop 时两者一起
    /// 消失，因此这类订阅者不影响 [`EventHub::downstream_count`]。
    #[must_use]
    pub fn subscribe_background(&self) -> BackgroundSubscription {
        self.background.fetch_add(1, Ordering::Relaxed);
        BackgroundSubscription {
            receiver: self.sender.subscribe(),
            background: Arc::clone(&self.background),
        }
    }

    /// 当前 OneBot 下游（正向 / 反向 WS）订阅者数量。
    ///
    /// downtime 兜底用它判定「没有下游可送」（Go `len(Wsclient)+len(WsServerClients) == 0`，
    /// 即 `BroadcastMessageToAll` 里 `failed == 总数` 的退化情形）；后台订阅者（`post_url`
    /// 上报）不计入。
    #[must_use]
    pub fn downstream_count(&self) -> usize {
        self.sender
            .receiver_count()
            .saturating_sub(self.background.load(Ordering::Relaxed))
    }

    /// 发布一条事件，返回送达的订阅者数量（0 表示当前无人订阅）。
    pub fn publish(&self, event: Event) -> usize {
        self.publish_shared(Arc::new(event))
    }

    /// 同 [`EventHub::publish`]，用于调用方已持有 `Arc<Event>` 的场景。
    pub fn publish_shared(&self, event: Arc<Event>) -> usize {
        self.sender.send(event).unwrap_or(0)
    }

    /// 当前订阅者数量（测试与诊断用）。
    #[must_use]
    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

#[cfg(test)]
mod tests {
    use super::EventHub;
    use gensokyo_core::Ob11Id;
    use gensokyo_protocol::Event;
    use gensokyo_protocol::event::{MetaEvent, post_type};

    fn connect() -> Event {
        Event::MetaEvent(MetaEvent {
            time: 1,
            self_id: Ob11Id::num(1),
            meta_event_type: "lifecycle".to_owned(),
            sub_type: Some("connect".to_owned()),
            interval: None,
            status: None,
            extra: Default::default(),
        })
    }

    #[test]
    fn publish_without_subscribers_is_not_an_error() {
        let hub = EventHub::default();
        assert_eq!(hub.publish(connect()), 0);
    }

    #[test]
    fn background_subscription_is_not_a_downstream() {
        let hub = EventHub::default();
        let background = hub.subscribe_background();
        // broadcast 里有 1 个 receiver，但它是 post_url 上报这类后台消费者。
        assert_eq!(hub.receiver_count(), 1);
        assert_eq!(hub.downstream_count(), 0);

        let _downstream = hub.subscribe();
        assert_eq!(hub.downstream_count(), 1);

        // 后台订阅整体 drop：receiver 与计数一起消失，只剩一个真正的下游。
        drop(background);
        assert_eq!(hub.receiver_count(), 1);
        assert_eq!(hub.downstream_count(), 1);

        drop(_downstream);
        assert_eq!(hub.downstream_count(), 0);
    }

    #[tokio::test]
    async fn every_subscriber_receives_the_event() {
        let hub = EventHub::default();
        let mut first = hub.subscribe();
        let mut second = hub.subscribe();
        assert_eq!(hub.publish(connect()), 2);

        for receiver in [&mut first, &mut second] {
            let event = receiver.recv().await.expect("应收到事件");
            assert_eq!(event.post_type(), post_type::META_EVENT);
        }
    }

    #[tokio::test]
    async fn slow_subscriber_gets_lagged_instead_of_blocking_the_publisher() {
        let hub = EventHub::new(1);
        let mut slow = hub.subscribe();
        for _ in 0..3 {
            hub.publish(connect());
        }
        assert!(matches!(
            slow.recv().await,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_))
        ));
    }
}
