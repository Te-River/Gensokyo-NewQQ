package callapi

import (
	"context"
	"encoding/json"
	"sync"

	"github.com/hoshinonyaruko/gensokyo/internal/application/action"
	"github.com/tencent-connect/botgo/openapi"
)

// ctxKey 用于在 Dispatch 的 ctx 中携带 Envelope 之外的顶层字段（Echo/PostType/MessageType）。
// action.Dispatcher 只把 params 交给 Decode、把 req 交给 Handle，Echo 等顶层字段不在其契约内；
// 旧 handler 依赖 message.Echo 回填响应，故经 ctx 透传，保证 OneBot 响应逐字节兼容。
type ctxKey int

const (
	ctxKeyEcho ctxKey = iota
	ctxKeyPostType
	ctxKeyMessageType
	ctxKeyShadowDryRun
)

// handlerFromLegacy 把现有 callapi.HandlerFunc 适配为 action.Handler。
// Decode 将 params JSON 解码为 ParamsContent（沿用其 int/string 兼容 UnmarshalJSON，callapi.go:157）；
// Handle 还原 ActionMessage 并调用旧 HandlerFunc，返回其 JSON 字符串（逐字节一致）。
func handlerFromLegacy(h HandlerFunc, client Client, api, apiv2 openapi.OpenAPI) action.Handler {
	return action.Handler{
		Decode: func(data []byte) (interface{}, error) {
			var p ParamsContent
			if len(data) > 0 {
				if err := json.Unmarshal(data, &p); err != nil {
					return nil, err
				}
			}
			return p, nil
		},
		Handle: func(ctx context.Context, req interface{}) (interface{}, error) {
			p, _ := req.(ParamsContent)
			msg := ActionMessage{Params: p}
			if v := ctx.Value(ctxKeyEcho); v != nil {
				msg.Echo = v
			}
			if v, ok := ctx.Value(ctxKeyPostType).(string); ok {
				msg.PostType = v
			}
			if v, ok := ctx.Value(ctxKeyMessageType).(string); ok {
				msg.MessageType = v
			}
			if v, ok := ctx.Value(ctxKeyShadowDryRun).(bool); ok && v {
				msg.ShadowDryRun = true
			}
			return h(client, api, apiv2, msg)
		},
	}
}

// BuildRegistry 从全局 handlers map（callapi.go:252）构建 Registry。
// bootstrap 可调用一次预热；未预热时 CallAPIFromDict 按 client 惰性构建并缓存。
func BuildRegistry(client Client, api, apiv2 openapi.OpenAPI) *action.Registry {
	reg := action.NewRegistry()
	for name, h := range handlers {
		reg.Register(name, handlerFromLegacy(h, client, api, apiv2))
	}
	return reg
}

// registryCache 按 client 缓存 Registry：handler 闭包捕获了 client/api/apiv2，
// 多连接场景下每个 client 需各自的 Registry（响应经 client.SendMessage 回发）。
var registryCache sync.Map // Client -> *action.Registry

// registryFor 返回该 client 的 Registry：首次调用构建并缓存，后续复用。
func registryFor(client Client, api, apiv2 openapi.OpenAPI) *action.Registry {
	if r, ok := registryCache.Load(client); ok {
		return r.(*action.Registry)
	}
	r := BuildRegistry(client, api, apiv2)
	actual, _ := registryCache.LoadOrStore(client, r)
	return actual.(*action.Registry)
}

// PrewarmRegistry 由 bootstrap 为已知 client 预热 Registry（可选）。
// 未预热时 CallAPIFromDict 在首次 new 模式调用时惰性构建，行为一致。
func PrewarmRegistry(client Client, api, apiv2 openapi.OpenAPI) *action.Registry {
	return registryFor(client, api, apiv2)
}
