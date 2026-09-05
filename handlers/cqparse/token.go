package cqparse

// Token/Span/Kind 模型（架构设计 §3/§4.1）。

// Kind 标记 Token 的分发类别。
type Kind uint8

const (
	// KindText 码间原文区间，Splice 时原样输出。
	KindText Kind = iota
	// KindMedia 媒体类：image/record/voice/video/file。
	KindMedia
	// KindContent 内容类：markdown/keyboard/card/input_notify/stream/music/avatar/group_info。
	KindContent
	// KindControl 控制类：reply/active/wakeup。
	KindControl
	// KindAction 动作类：member/remove/set_group（仅群聊 scope，产 PendingAction 不在 Parse 内执行）。
	KindAction
	// KindPassthrough at/face/未知码/畸形码：原样保留在正文。
	KindPassthrough
)

// Span 为原始输入文本的字节偏移 [Start,End)。
// 仅字符串输入有真实 Span；段数组/TRSS 直产 Token 的 Span 按
// 拼接进 Doc.Source 后的位置填充（零宽或片段区间），保证 Splice 统一。
type Span struct{ Start, End int }

// Token 是归一化后的最小解析单元。
type Token struct {
	Kind   Kind
	Action string            // 小写规范形（image/markdown/set_group…）
	Params map[string]string // 已解码值；key 存在但值为空用 ", ok" 区分
	Raw    string            // 原文（[CQ:...] 整段）；文本段为原文切片
	Span   Span
	// Segment 标记来自段数组/TRSS 直产（无真实字符串码原文，
	// Raw 为规范渲染形，Splice 按 Outcome.Replacement 输出）。
	Segment bool
}

// Doc 是一次解析的产物：Token 按 Span.Start 升序、互不重叠。
type Doc struct {
	Tokens []Token
	// Source 为字符串输入的原文；段输入为各段规范文本的拼接结果。
	Source string
}

// InputKind 区分三种入口消息形态。
type InputKind uint8

const (
	// InputString 字符串 CQ 码消息。
	InputString InputKind = iota
	// InputSegments OneBot 消息段数组。
	InputSegments
	// InputMap TRSS 单 map 消息段。
	InputMap
)

// Input 是三输入归一化的统一入口参数（架构设计 §3/§5）。
type Input struct {
	Kind     InputKind
	String   string
	Segments []map[string]interface{} // 段数组与 TRSS map 统一包装成段列表
	// GroupID 为当前会话的虚拟群 ID；HasGroup=false 时为私聊（M7：nil 与 "" 统一）。
	GroupID  string
	HasGroup bool
	UserID   string
}
