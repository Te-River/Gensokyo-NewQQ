package onebot

// ClientBroadcaster 抽象 Processor 的入站广播能力（覆盖反向 WS / 正向 WS / HTTP POST），
// 避免 adapter → Processor 的循环依赖。
type ClientBroadcaster interface {
	BroadcastInbound(message map[string]interface{}) error
	// BroadcastInboundWithRaw 携带原始 DTO（downtime 兜底需要）。
	BroadcastInboundWithRaw(message map[string]interface{}, data interface{}) error
}

// BroadcastSender 把 onebot.Sender 的 SendMessage 桥接到 Processor 广播。
type BroadcastSender struct {
	b ClientBroadcaster
}

// NewBroadcastSender 创建广播发送器。
func NewBroadcastSender(b ClientBroadcaster) *BroadcastSender {
	return &BroadcastSender{b: b}
}

// SendMessage 实现 onebot.Sender。
func (s *BroadcastSender) SendMessage(payload map[string]interface{}) error {
	if s == nil || s.b == nil {
		return nil
	}
	return s.b.BroadcastInbound(payload)
}

// SendMessageWithRaw 实现 onebot.RawSender：把原始 DTO 一并交给广播链。
func (s *BroadcastSender) SendMessageWithRaw(payload map[string]interface{}, raw interface{}) error {
	if s == nil || s.b == nil {
		return nil
	}
	return s.b.BroadcastInboundWithRaw(payload, raw)
}
