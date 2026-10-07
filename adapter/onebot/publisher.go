package onebot

import (
	"context"

	"github.com/hoshinonyaruko/gensokyo/internal/domain/event"
)

// Sender 出站发送抽象（P9 将收敛为 typed action dispatcher）。
type Sender interface {
	SendMessage(payload map[string]interface{}) error
}

// RawSender 可选接口：Publisher 在 ev.Raw 非空时优先调用，
// 让广播链拿到原始 DTO（Processor 全通道失败时的 downtime 兜底需要）。
type RawSender interface {
	SendMessageWithRaw(payload map[string]interface{}, raw interface{}) error
}

// Publisher 实现 inbound.EventPublisher：DomainEvent → OneBot → Sender。
type Publisher struct {
	sender Sender
	// useArray 选择 array/string 上报格式（由配置注入）。
	useArray bool
}

// NewPublisher 创建 OneBot 发布器。
func NewPublisher(sender Sender, useArray bool) *Publisher {
	return &Publisher{sender: sender, useArray: useArray}
}

// Publish 序列化并发送 DomainEvent。
// ev.Payload 非空时逐字节复用 legacy 构造的上报 map（保证与 legacy 入站等价）；
// 为空时回退最小构造（仅覆盖 post_type/message_type/message/time/user_id）。
func (p *Publisher) Publish(_ context.Context, ev event.DomainEvent) error {
	payload := ev.Payload
	if payload == nil {
		payload = p.buildPayload(ev)
	}
	if ev.Raw != nil {
		if rs, ok := p.sender.(RawSender); ok {
			return rs.SendMessageWithRaw(payload, ev.Raw)
		}
	}
	return p.sender.SendMessage(payload)
}

// buildPayload 从 DomainEvent 构造最小 OneBot payload（Payload 缺失时的回退）。
func (p *Publisher) buildPayload(ev event.DomainEvent) map[string]interface{} {
	var msg interface{}
	if p.useArray {
		msg = SerializeArray(ev.Message)
	} else {
		msg = SerializeString(ev.Message)
	}
	payload := map[string]interface{}{
		"post_type":    "message",
		"message_type": eventMessageType(ev),
		"message":      msg,
		"time":         ev.Time.Unix(),
	}
	if ev.Actor.VirtualUserID != "" {
		payload["user_id"] = ev.Actor.VirtualUserID.String()
	}
	return payload
}

func eventMessageType(ev event.DomainEvent) string {
	switch ev.Source {
	case event.SourceC2CMessage:
		return "private"
	default:
		return "group"
	}
}
