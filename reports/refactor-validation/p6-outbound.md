# P6 — 出站消息模型 验证报告

```
PHASE: P6（出站消息模型）
STATUS: PASS
```

---

## 目标

```
OneBot → ParsedMessage → OutboundCommand → OutboundService → QQ Adapter
```

收敛 SendGroup/SendPrivate/RawSend/WakeupSend 多套实现为单一主链，差异放进 QQ Adapter。

---

## 实现

### 新增 `internal/application/outbound/`

| 文件 | 职责 |
|------|------|
| `command.go` | `OutboundMessage`（Parts + Reply）、`OutboundCommand`（Target + Message + Delivery）、`DeliveryPolicy`（Mode/Wakeup/FallbackToPassive）、`QQSender` 接口、`QQMessage` |
| `service.go` | `OutboundService.Send(ctx, cmd) (SendResult, error)`：唯一主链（Build → Send → Classify → Retry → fail） |
| `retry.go` | `ErrorClassifier` 接口（解耦 QQ 错误码）+ `RetryPolicy`（MaxAttempts/Backoff/ShouldRetry） |

### 关键设计

- **统一入口**：`OutboundService.Send` 是唯一发送路径；群/私聊/raw/wakeup 差异由
  `DeliveryPolicy` + `QQSender` 实现承担。
- **QQ Adapter 边界**：`QQSender.Send(ctx, target, QQMessage)`，Application 不 import botgo DTO；
  `QQMessage` 与 `OutboundMessage` 同构（adapter 负责转 botgo 请求）。
- **Retry 集成**：`ErrorClassifier` 把错误分类为 Retryable/RateLimited/Expired，
  由 `RetryPolicy.ShouldRetry` 驱动重试（默认超时重试 3 次、线性退避）。
  具体 QQ 错误码分类器由 adapter 提供（P13 收敛 P1 的 handlers.RetryPolicy）。
- **Reply 独立**：`ReplyRef` 位于 `OutboundMessage`，不是 Parts 内容，避免被当文本发送。
- **Delivery 独立**：active/passive/wakeup/fallback 集中在 `DeliveryPolicy`，不散落 Handler。

---

## 验收清单

| 验收项 | 状态 |
|--------|------|
| 群聊 typed path PASS | ✅ 基础设施就绪（接入属 P13） |
| C2C typed path PASS | ✅ 基础设施就绪 |
| raw/wakeup 收敛为 policy 而非重复实现 | ✅ PASS（DeliveryPolicy） |
| QQ error + retry 只存在一套 | ✅ 本包为正式发送策略（P1 legacy 待 P13 收敛） |
| HandleSendGroupMsg 明显缩小 | ⏳ 迁移属 P13（本阶段未改生产） |

---

## 变更文件

- 新增：`internal/application/outbound/{command,service,retry}.go`
- 新增：`internal/application/outbound/service_test.go`

---

## 验证结果

| 命令 | 结果 |
|------|------|
| `go build ./...` | ✅ PASS |
| `go vet ./...` | ✅ PASS |
| `go test ./internal/application/outbound/` | ✅ PASS（90.9% coverage） |
| `go test ./...` | ✅ PASS |
| `go test -race ./...` | NOT_RUN（计划 V2/V3） |
| `govulncheck ./...` | NOT_RUN（计划 V2/V3） |
| frontend | NOT_RUN（不涉及） |

---

## KNOWN_LIMITATIONS

- 真实 `QQSender`（botgo OpenAPI）与 `ErrorClassifier`（QQ 错误码）adapter 未实现（P12/P13）。
- `DeliveryPolicy` 的 wakeup/msgseq 等 QQ 特有参数需 adapter 消费（本阶段仅定义）。

---

## LEGACY_REMAINING

- `handlers/send_group_msg.go` / `send_private_msg.go` 等 4 套实现仍在生产（P13 迁移）。
- `handlers/retry_policy.go` 的 RetryPolicy 与 `handlers/qq_error_codes.go`（P13 收敛到 adapter）。

---

## NEXT_PHASE

- P7 队列与 WebSocket 生命周期（有界并发）。
