package onebot

import (
	"context"

	"github.com/hoshinonyaruko/gensokyo/internal/domain/event"
)

// Sender 出站发送抽象（P9 将收敛为 typed action dispatcher）。
type Sender interface {
	SendMessage(payload map[string]interface{}) error
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
func (p *Publisher) Publish(_ context.Context, ev event.DomainEvent) error {
	var msg interface{}
	if p.useArray {
		msg = SerializeArray(ev.Message)
	} else {
		msg = SerializeString(ev.Message)
	}
	payload := map[string]interface{}{
		"post_type":   "message",
		"message_type": eventMessageType(ev),
		"message":     msg,
		"time":        ev.Time.Unix(),
	}
	if ev.Actor.VirtualUserID != "" {
		payload["user_id"] = ev.Actor.VirtualUserID.String()
	}
	return p.sender.SendMessage(payload)
}

func eventMessageType(ev event.DomainEvent) string {
	switch ev.Source {
	case event.SourceC2CMessage:
		return "private"
	default:
		return "group"
	}
}
