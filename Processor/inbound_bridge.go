package Processor

import (
	"context"
	"errors"

	"github.com/hoshinonyaruko/gensokyo/adapter/onebot"
	"github.com/hoshinonyaruko/gensokyo/adapter/qq"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/internal/application/inbound"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/event"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
)

// 入站新路径的装配件；由 SetInbound 注入，nil 时 new 路径回退 legacy 广播。
var (
	inboundNormalizer inbound.EventNormalizer
	inboundPublisher  inbound.EventPublisher
)

// errInboundNotConfigured 表示 SetInbound 尚未调用，new 路径回退 legacy。
var errInboundNotConfigured = errors.New("Processor: inbound new path not configured (SetInbound not called)")

// SetInbound 装配入站新路径（Normalizer + Publisher + BroadcastSender）。
// 由 bootstrap（main.go）在构造 Processors 后调用一次；未调用时 new 路径回退 legacy。
func SetInbound(p *Processors) {
	if p == nil {
		return
	}
	inboundNormalizer = qq.NewNormalizer()
	inboundPublisher = onebot.NewPublisher(onebot.NewBroadcastSender(p), config.GetArrayValue())
}

// dispatchInbound 按 arch_mode 选择入站上报路径。
//   legacy: 执行 legacyBroadcast（逐字节不变）。
//   new:    Normalize → Publisher.Publish；payload 为 legacy 构造的上报 map（逐字节复用）。
//   shadow: 执行 legacyBroadcast，并额外 Normalize 记录（结果不生效）。
func (p *Processors) dispatchInbound(data interface{}, payload map[string]interface{}, legacyBroadcast func()) {
	mode := config.GetArchMode()
	if mode == "legacy" {
		legacyBroadcast()
		return
	}
	ev, err := p.normalizeInbound(data)
	if err != nil {
		// 未覆盖的事件类型（inline search / join request / reject / friend 等）回退 legacy 广播。
		legacyBroadcast()
		return
	}
	if mode == "shadow" {
		legacyBroadcast()
		mylog.Printf("[arch] shadow inbound: source=%d id=%s (legacy broadcast kept)", ev.Source, ev.ID)
		return
	}
	ev.Payload = payload
	go p.publishInbound(ev)
}

// dispatchGroupMember 处理群成员变动事件（DTO 无 add/remove 判别字段，需显式 eventType）。
func (p *Processors) dispatchGroupMember(data *dto.GroupMemberEvent, eventType string, payload map[string]interface{}, legacyBroadcast func()) {
	p.dispatchInbound(&qq.MemberEvent{Data: data, EventType: eventType}, payload, legacyBroadcast)
}

// BroadcastInbound 把入站 payload 广播到全部传输通道（反向 WS / 正向 WS / HTTP POST），
// 复用 legacy BroadcastMessageToAll 的通道覆盖，保证 new 路径与 legacy 送达一致。
func (p *Processors) BroadcastInbound(message map[string]interface{}) error {
	return p.BroadcastMessageToAll(message, p.Apiv2, nil)
}

// BroadcastInboundWithRaw 同 BroadcastInbound，但携带原始 QQ DTO：
// 全通道失败时 BroadcastMessageToAll 的 downtime 兜底依赖 data 的类型断言（Processor.go:370-395）。
func (p *Processors) BroadcastInboundWithRaw(message map[string]interface{}, data interface{}) error {
	return p.BroadcastMessageToAll(message, p.Apiv2, data)
}

func (p *Processors) normalizeInbound(data interface{}) (event.DomainEvent, error) {
	if inboundNormalizer == nil {
		return event.DomainEvent{}, errInboundNotConfigured
	}
	return inboundNormalizer.Normalize(data)
}

func (p *Processors) publishInbound(ev event.DomainEvent) {
	if inboundPublisher == nil {
		return
	}
	if err := inboundPublisher.Publish(context.Background(), ev); err != nil {
		mylog.Errorf("[arch] inbound publish failed: %v", err)
	}
}
