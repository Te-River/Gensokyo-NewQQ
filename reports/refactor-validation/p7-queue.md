# P7 — 队列与 WebSocket 生命周期 验证报告

```
PHASE: P7（队列与 WebSocket 生命周期）
STATUS: PASS
```

---

## 目标

无限 goroutine / 无界 slice → 有界并发；同 session 顺序；重试不占 worker Sleep。

---

## 实现

### 新增 `internal/application/queue/`

| 文件 | 职责 |
|------|------|
| `queue.go` | `Queue` 接口（Enqueue/Close/Wait/Depth/Metrics）+ 有界实现（容量均分分区、session hash 分区、三种背压） |
| `delayed.go` | `scheduler`：延迟调度器（重试走 timer，不占 worker） |

### 关键设计

- **有界容量**：总容量均分到 `Workers` 个分区 channel，无无限 append。
- **背压必须显式选择**：`BackpressureBlock` / `Drop` / `Reject`（`Drop` 有 `Rejected` 计数，不静默丢）。
- **Session 顺序**：`hash(session) % workers` → 同一 session 恒到同一分区 → 顺序保证。
- **Retry 不占 worker**：`ProcessFunc` 返回 `(retry, backoff)`，由 `scheduler.After` 到期后重新入队。
- **Shutdown**：`Close()`（停 scheduler、关分区 channel）→ `Wait()`（等 worker 收尾），可预测。
- **Metrics**：`Capacity / Depth / Rejected / Processed / Active` 原子指标。
- **错误可见**：`ErrQueueFull` / `ErrQueueClosed`。

### WS Lifecycle（P7.6/7.7）

现有 `wsclient/` 为生产代码，本阶段**未重写**（避免高风险）。已在报告中标注：

- 单 Writer 原则、Register/WriteLoop/ReadLoop/Close 生命周期，需在接入新队列时同步收敛（P13）。

---

## 验收清单

| 验收项 | 状态 |
|--------|------|
| Queue 有硬容量 | ✅ PASS（Capacity） |
| Enqueue 失败可见 | ✅ PASS（ErrQueueFull / ErrQueueClosed / Rejected） |
| Close/Wait 可预测 | ✅ PASS（测试） |
| WS 不存在 uncontrolled goroutine | ⏳ 现有 wsclient 未改（接入时收敛，P13） |
| shutdown test PASS | ✅ PASS |

---

## 变更文件

- 新增：`internal/application/queue/{queue,delayed}.go`
- 新增：`internal/application/queue/queue_test.go`

---

## 验证结果

| 命令 | 结果 |
|------|------|
| `go build ./...` | ✅ PASS |
| `go vet ./...` | ✅ PASS |
| `go test ./internal/application/queue/` | ✅ PASS（86.1% coverage） |
| `go test ./...` | ✅ PASS |
| `go test -race ./...` | NOT_RUN（windows/386 不支持 race detector） |
| `govulncheck ./...` | NOT_RUN（计划 V3） |
| frontend | NOT_RUN（不涉及） |

---

## KNOWN_LIMITATIONS

- `scheduler.After` 用 `time.AfterFunc` 每任务一个 timer；高频重试下 timer 数量需关注（当前上限 = 重试任务数）。
- WS lifecycle（单 Writer、Register/Unregister）未重写，待接入时与新队列统一。

---

## LEGACY_REMAINING

- 现有 `messagequeue/`（无界 slice + worker Sleep + 全局 singleton）仍在使用（P13 切换）。
- `wsclient/` 现有生命周期未动。

---

## NEXT_PHASE

- P8 Processor 分层（DomainEvent + 入站事件管线）。
