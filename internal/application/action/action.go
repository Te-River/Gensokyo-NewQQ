// Package action 提供 typed action 注册与分发（替代 map[string]interface{} 与 init() 注册）。
//
// 目标：HTTP 与 WS 共用同一 Dispatcher；Action 参数 typed；transport 层无业务逻辑。
package action

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"sync"
)

// 分发错误。
var (
	ErrUnknownAction = errors.New("action: unknown action")
	ErrInvalidParams = errors.New("action: invalid params")
)

// Envelope OneBot 动作信封（transport 层通用）。
type Envelope struct {
	Action string          `json:"action"`
	Params json.RawMessage `json:"params"`
	Echo   interface{}     `json:"echo,omitempty"`
}

// HandlerFunc 处理解码后的 typed action，返回响应（将被编码回 transport）。
type HandlerFunc func(ctx context.Context, req interface{}) (interface{}, error)

// DecoderFunc 把 params JSON 解码为 typed action DTO 并校验。
type DecoderFunc func(data []byte) (interface{}, error)

// Handler 一个 action 的解码 + 处理。
type Handler struct {
	Decode DecoderFunc
	Handle HandlerFunc
}

// Registry 显式 action 注册表（替代 init() 注册）。
type Registry struct {
	mu       sync.RWMutex
	handlers map[string]Handler
}

// NewRegistry 创建注册表。
func NewRegistry() *Registry {
	return &Registry{handlers: map[string]Handler{}}
}

// Register 注册 action。
func (r *Registry) Register(name string, h Handler) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.handlers[name] = h
}

// Find 查找 action。
func (r *Registry) Find(name string) (Handler, bool) {
	r.mu.RLock()
	defer r.mu.RUnlock()
	h, ok := r.handlers[name]
	return h, ok
}

// Len 已注册 action 数。
func (r *Registry) Len() int {
	r.mu.RLock()
	defer r.mu.RUnlock()
	return len(r.handlers)
}

// Dispatcher 统一分发器：HTTP 与 WS 共用。
type Dispatcher struct {
	reg *Registry
}

// NewDispatcher 创建分发器。
func NewDispatcher(reg *Registry) *Dispatcher {
	return &Dispatcher{reg: reg}
}

// Dispatch 处理原始 OneBot 动作字节：JSON → Envelope → typed DTO → Handler。
// transport 层（HTTP/WS）只负责调用本方法并编码返回。
func (d *Dispatcher) Dispatch(ctx context.Context, raw []byte) (interface{}, error) {
	var env Envelope
	if err := json.Unmarshal(raw, &env); err != nil {
		return nil, fmt.Errorf("%w: %v", ErrInvalidParams, err)
	}
	if env.Action == "" {
		return nil, fmt.Errorf("%w: empty action", ErrUnknownAction)
	}
	h, ok := d.reg.Find(env.Action)
	if !ok {
		return nil, fmt.Errorf("%w: %s", ErrUnknownAction, env.Action)
	}
	req, err := h.Decode(env.Params)
	if err != nil {
		return nil, fmt.Errorf("%w: %v", ErrInvalidParams, err)
	}
	return h.Handle(ctx, req)
}
