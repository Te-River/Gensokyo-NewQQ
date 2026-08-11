# P8 — Processor 分层（入站事件管线）验证报告

```
PHASE: P8（Processor 分层）
STATUS: PASS
```

---

## 目标

```
QQ Adapter → Inbound Application Service → DomainEvent → OneBot Publisher
```

拆解 Processor God Module；@Bot 逻辑只有一个 canonical 实现；String/Array serializer 无业务判断。

---

## 实现

### 新增 `internal/domain/event/`

- `event.go`：`DomainEvent`（ID/Time/Source/Actor/Target/Message + Raw）+ `EventSource` 枚举

### 新增 `internal/application/inbound/`

- `inbound.go`：
  - `EventNormalizer` 接口（QQ SDK DTO → DomainEvent；只有 QQ Adapter 可 import botgo）
  - `EventPublisher` 接口（DomainEvent → 下游）
  - `IsSelfMention` / `NormalizeMentions`：@Bot 的唯一 canonical 实现（P8.3）

### 新增 `adapter/onebot/`

- `serializer.go`：`SerializeString` / `SerializeArray`（DomainEvent.Message → OneBot 格式），
  复用 `message.Canonicalize`，**不含任何身份判断业务**（P8.5）
- `publisher.go`：`Publisher` 实现 `EventPublisher`，通过 `Sender` 抽象发送（P9 收敛为 typed action）

---

## 验收清单

| 验收项 | 状态 |
|--------|------|
| Processor 不再直接发送 WS | ✅ 基础设施就绪（Publisher 接口），接入属 P13 |
| Processor 不再直接操作 idmap storage | ✅ DomainEvent 携带 ResolvedTarget，接入属 P13 |
| String/Array serializer 无身份判断业务 | ✅ PASS（仅表示层） |
| @Bot 逻辑只有一个 canonical 实现 | ✅ PASS（IsSelfMention/NormalizeMentions） |

---

## 变更文件

- 新增：`internal/domain/event/event.go`
- 新增：`internal/application/inbound/{inbound.go,inbound_test.go}`
- 新增：`adapter/onebot/{serializer.go,publisher.go,serializer_test.go}`

---

## 验证结果

| 命令 | 结果 |
|------|------|
| `go build ./...` | ✅ PASS |
| `go vet ./...` | ✅ PASS |
| `go test ./internal/application/inbound/` | ✅ PASS（92.9% coverage） |
| `go test ./adapter/onebot/` | ✅ PASS（63.6% coverage） |
| `go test ./...` | ✅ PASS |
| `go test -race ./...` | NOT_RUN（windows/386 不支持） |
| `govulncheck ./...` | NOT_RUN（计划 V3） |
| frontend | NOT_RUN（不涉及） |

---

## KNOWN_LIMITATIONS

- 真实 `EventNormalizer`（QQ SDK DTO → DomainEvent）未实现（P12 隔离 botgo 时建 adapter/qq）。
- `Publisher` 依赖轻量 `Sender` 接口；OneBot 完整事件字段（notice/self_id 等）待 P13 对齐。

---

## LEGACY_REMAINING

- `Processor/` 目录的 God Module 仍为生产主路径（P13 重写为 QQ Adapter + 本管线）。
- 现有 `handlers/message_parser.go` 的两套 serializer 逻辑（P13 删除）。

---

## NEXT_PHASE

- P9 HTTP/WS/callapi Adapter（typed action + 显式 registry）。
