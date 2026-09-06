package cqparse

// handler 注册表与替换协议（架构设计 §3/§6）。
// 与 callapi 相同的 init() 自注册模式；每个码一个 Handler。

// FoundItem 是 handler 对 foundItems 的单项贡献。
type FoundItem struct{ Key, Value string }

// Outcome 是单个 Token 的解析产物。
type Outcome struct {
	// Replacement 为该 Span 的替换文本；""=删除；等于 tok.Raw=保留原文。
	Replacement string
	Found       []FoundItem    // foundItems 贡献（按文档顺序收集）
	Pending     *PendingAction // 动作码产物（Parse 不执行）
	Warn        string         // 非空时经 mylog 输出
}

// Scope 位掩码，决定码在何种会话生效。
type Scope uint8

const (
	ScopeGroup   Scope = 1 << iota // 群聊
	ScopePrivate                   // 私聊（含转发节点解析：Input 层无转发标志）
	ScopeForward                   // 预留：显式转发上下文
)

// ResolveCtx 贯穿一次 Parse 的解析上下文。
type ResolveCtx struct {
	Input *Input
	Deps  *Deps
}

// Handler 是单 Token 处理器。
type Handler interface {
	Kind() Kind
	Scope() Scope
	Resolve(ctx *ResolveCtx, tok Token) Outcome
}

// BatchHandler 是同消息多 Token 合并处理器（group_info 同群一次取数）。
type BatchHandler interface {
	ResolveBatch(ctx *ResolveCtx, toks []Token) []Outcome
}

var (
	handlers      = map[string]Handler{}
	batchHandlers = map[string]BatchHandler{}
)

// Register 注册单 Token handler（init() 自注册）。
func Register(action string, h Handler) {
	handlers[action] = h
}

// RegisterBatch 注册批量 handler；同 action 不得再注册单 Token handler。
func RegisterBatch(action string, h BatchHandler) {
	batchHandlers[action] = h
}

// knownSetGroupActions 与执行器支持的子动作集一致；
// 未知 action 与今日一致保留原文（不产 pending 不执行）。
var knownSetGroupActions = map[string]bool{
	"ban":              true,
	"whole_ban":        true,
	"add_request":      true,
	"strategy_execute": true,
	"strategy_delete":  true,
	"kick":             true,
	"blacklist_add":    true,
	"blacklist_del":    true,
}
