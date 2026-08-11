# P13 — 删除兼容层 验证报告

```
PHASE: P13（删除兼容层）
STATUS: PARTIAL_PASS（破坏性删除 BLOCKED，待生产切换 + 真实联调）
```

---

## P13.1 删除条件检查（来自计划，全部必须满足）

| 条件 | 状态 | 说明 |
|------|------|------|
| Typed parser 已覆盖生产路径 | ❌ | 生产仍走 `handlers/message_parser.go` |
| Typed Identity 已覆盖生产路径 | ❌ | handlers 仍用 idmap 全局 API |
| OutboundService 已覆盖 group/C2C | ❌ | 生产仍走 4 套旧 handler |
| Inbound DomainEvent 已覆盖 QQ events | ❌ | Processor 仍为生产主路径 |
| HTTP/WS typed actions 已稳定 | ❌ | callapi + init() 注册仍在使用 |
| repository 已替换 echo/idmap globals | ❌ | 全局单例仍为生产存储 |
| 至少完整运行一个稳定测试周期 | ❌ | 尚无真实 QQ/OneBot 联调 |

> **结论：删除条件未满足。** 按计划 Stop Conditions 与"禁止为了让重构继续跑而删除失败测试/破坏生产"，
> 本阶段**不执行破坏性删除**。生产路径切换需在真实联调（V5）验证后，以独立任务执行。

---

## 已完成的非破坏性收尾（P13 范围内可安全执行部分）

| 项目 | 状态 |
|------|------|
| 新架构无 foundItems 生产引用（仅 `message/compat.go` 迁移桥 + 测试） | ✅ PASS |
| ID 长度启发式只剩 legacy adapter（`identity.IsOpenID`） | ✅ PASS（P3） |
| 新架构无 `config.GetXxx()` / 无 botgo / 无 go-silk / 无 `callapi.RegisterHandler` / 无 package init goroutine | ✅ PASS |
| go vet 全仓无新问题 | ✅ PASS |

## P13 主删除项（BLOCKED，需前置条件）

- P13.2 删除 foundItems（生产引用归零）
- P13.3 删除 ID heuristic（legacy adapter 移除）
- P13.4 删除旧 Handler（HandleSendGroupMsg legacy 分支、legacy CQ processor、legacy retry/media path）
- P13.5 删除全局状态（Settings singleton、echo maps、package init ticker、全局 handler registry）
- P13.6 废弃配置（deprecated → warning → migration → remove）
- P13.7 dead code 清理（staticcheck/golangci-lint）

---

## 验收（计划目标值）

| 目标 | 当前 |
|------|------|
| foundItems production refs: 0 | 生产仍 >0（handlers/message_parser.go）→ BLOCKED |
| ID length heuristic: 0 | 生产已归零（只剩 legacy adapter）→ ✅ 前置已满足 |
| business-level config global getter: 0 | 生产仍 >0（P11 inventory）→ BLOCKED |
| business-level err.Error parsing: 0 | 生产仍存在 → BLOCKED |
| handler init registry: 0 | 生产仍 >0 → BLOCKED |
| package-level lifecycle goroutine: 0 | 生产仍存在 → BLOCKED |

---

## NEXT_PHASE

1. **接入生产**：把新架构（typed parser / outbound / inbound / action / repository）逐个接入
   handlers/Processor/callapi/idmap/echo 主链（需独立任务，逐段 shadow-compare）。
2. **真实联调**（V5）：QQ ↔ Gensokyo ↔ OneBot 双向矩阵 + Failure Matrix。
3. 稳定周期通过后，再执行 P13 破坏性删除。
