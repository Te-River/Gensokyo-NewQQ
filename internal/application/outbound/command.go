// Package outbound 提供统一的出站消息模型与发送服务。
//
// 目标：收敛 SendGroup/SendPrivate/RawSend/WakeupSend 多套实现为
// 单一 `OutboundService.Send(ctx, OutboundCommand)`，差异放进 QQ Adapter。
package outbound

import (
	"context"

	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/message"
)

// ReplyRef 回复引用（单独于 Parts，避免被当作消息内容发送）。
type ReplyRef struct {
	MessageID string
}

// OutboundMessage 待发送的消息（类型化 Parts + 回复引用）。
type OutboundMessage struct {
	Parts []message.MessagePart
	Reply *ReplyRef
}

// DeliveryPolicy 投递策略（active/passive、wakeup、fallback 等）。
// QQ 特有的投递参数（wakeup/msgseq/event_id）由此携带，由 QQ Adapter 消费。
type DeliveryPolicy struct {
	// Mode 主动/被动。
	Mode message.DeliveryMode
	// Wakeup 是否使用 send_private_msg_wakeup 语义（仅单聊）。
	Wakeup bool
	// FallbackToPassive 主动失败时是否回退被动补发。
	FallbackToPassive bool
}

// OutboundCommand 一条完整的出站指令。
type OutboundCommand struct {
	Target   identity.ResolvedTarget
	Message  OutboundMessage
	Delivery DeliveryPolicy
}

// QQMessage 交给 QQ Adapter 的消息（与 OutboundMessage 同构；
// adapter 负责转换为 botgo DTO，Application 不 import botgo）。
type QQMessage = OutboundMessage

// QQSendResult QQ Adapter 的发送结果。
type QQSendResult struct {
	MessageID string
}

// QQSender QQ 发送适配器接口。
// Application 层只依赖本接口，不依赖具体 SDK 类型。
type QQSender interface {
	Send(ctx context.Context, target identity.ResolvedTarget, msg QQMessage) (QQSendResult, error)
}
