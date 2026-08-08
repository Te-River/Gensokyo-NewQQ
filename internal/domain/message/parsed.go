package message

// DeliveryMode 消息投递模式。
//
// 注意：active/passive 不是"消息内容"，因此不放入 Parts，而是作为独立的投递模式。
type DeliveryMode uint8

const (
	// ModeDefault 默认（被动回复，跟随当前会话上下文）。
	ModeDefault DeliveryMode = iota
	// ModeActive 主动推送（不受被动回复限制）。
	ModeActive
)

// ParsedMessage 解析后的类型化消息。
type ParsedMessage struct {
	Parts []MessagePart
	Reply *ReplyPart

	Mode DeliveryMode
}
