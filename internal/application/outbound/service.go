package outbound

import (
	"context"
	"time"
)

// SendResult 发送结果。
type SendResult struct {
	MessageID string
}

// OutboundService 统一出站发送服务。
// 群聊/私聊/raw/wakeup 的差异由 DeliveryPolicy + QQ Adapter 承担，此处只有一条主链。
type OutboundService struct {
	sender QQSender
	retry  RetryPolicy
	// fallbackToPassive 全局默认回退（可由 DeliveryPolicy 覆盖）。
	fallbackToPassive bool
}

// NewService 创建出站服务。
func NewService(sender QQSender, retry RetryPolicy) *OutboundService {
	return &OutboundService{sender: sender, retry: retry}
}

// Send 发送一条出站指令。
// 流程：Build → Send → Classify error → RetryPolicy → retry / fallback / fail
func (s *OutboundService) Send(ctx context.Context, cmd OutboundCommand) (SendResult, error) {
	msg := cmd.Message
	// Reply 已包含在 cmd.Message.Reply（发送行为独立于消息内容）

	for attempt := 0; ; attempt++ {
		res, err := s.sender.Send(ctx, cmd.Target, msg)
		if err == nil {
			return SendResult{MessageID: res.MessageID}, nil
		}
		if !s.retry.ShouldRetry(err, attempt) {
			return SendResult{}, err
		}
		backoff := s.retry.Backoff(attempt)
		if backoff <= 0 {
			continue
		}
		select {
		case <-time.After(backoff):
		case <-ctx.Done():
			return SendResult{}, ctx.Err()
		}
	}
}
