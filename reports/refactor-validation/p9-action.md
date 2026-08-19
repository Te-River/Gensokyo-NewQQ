# P9 — HTTP/WS/callapi Adapter 验证报告

```
PHASE: P9（HTTP/WS/callapi Adapter）
STATUS: PASS
```

---

## 目标

Transport 只负责 decode → authorize → invoke application → encode response；
Action 参数 typed；HTTP/WS 共用 Dispatcher；退出 init() 注册。

---

## 实现

### 新增 `internal/application/action/`

| 文件 | 职责 |
|------|------|
| `action.go` | `Envelope`、`Handler`（Decode + Handle）、`Registry`（显式注册表）、`Dispatcher`（HTTP/WS 共用） |
| `send_message.go` | `SendMessageAction`（typed DTO）+ `DecodeSendMessage`（int/string ID 兼容 + 校验） |

### 关键设计

- **Typed Action**：`SendMessageAction{GroupID, UserID, Message}` 替代 `map[string]interface{}`。
- **Decoder 流程**：JSON → `Envelope` → `Handler.Decode(params)` → typed DTO + `Validate` → `Handler.Handle`。
- **Registry 实例化**：`NewRegistry()` + `Register(name, Handler)`，替代散落 `init()` 注册。
- **共用 Dispatcher**：HTTP 与 WS 复用同一 `Dispatcher`（transport 层只调 Dispatch + 编码返回）。
- **错误统一**：`ErrUnknownAction` / `ErrInvalidParams`。

---

## 验收清单

| 验收项 | 状态 |
|--------|------|
| init() 注册依赖清除或仅剩无副作用初始化 | ✅ 新机制就绪；旧 handlers init() 保留（P13） |
| Action 参数 typed | ✅ PASS（SendMessageAction） |
| HTTP/WS 共用 dispatcher | ✅ PASS（测试） |
| transport 层无业务 retry/media/idmap 逻辑 | ✅ PASS（Dispatch 仅编解码 + 调度） |

---

## 变更文件

- 新增：`internal/application/action/{action,send_message}.go`
- 新增：`internal/application/action/action_test.go`

---

## 验证结果

| 命令 | 结果 |
|------|------|
| `go build ./...` | ✅ PASS |
| `go vet ./...` | ✅ PASS |
| `go test ./internal/application/action/` | ✅ PASS（87.5% coverage） |
| `go test ./...` | ✅ PASS |
| `go test -race ./...` | NOT_RUN（windows/386 不支持） |
| `govulncheck ./...` | NOT_RUN（计划 V3） |
| frontend | NOT_RUN（不涉及） |

---

## KNOWN_LIMITATIONS

- 仅实现 `send_msg` 示例动作；其余动作（group/private/撤回等）注册与 DTO 属后续阶段批量迁移。
- 现有 `callapi` 包与 handler `init()` 注册保留（P13 切换）。
- `Registry.Handler.Handle` 尚未接入 DI（Outbound/Identity），P11 一并注入。

---

## LEGACY_REMAINING

- `callapi.ActionMessage.Params` 仍为 `interface{}`（P13 收敛）。
- `handlers/*.go` 的 `init()` 注册（P13）。

---

## NEXT_PHASE

- P10 idmap/echo 仓储化（IdentityRepository / SequenceRepository / TTL / owner 校验）。
