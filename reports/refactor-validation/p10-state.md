# P10 — idmap/echo 仓储化 验证报告

```
PHASE: P10（idmap/echo 仓储化）
STATUS: PASS
```

---

## 目标

把 idmap/echo 全局状态变成有明确 owner 与生命周期的 Repository；
msgseq 只允许原子 Next；TTL 统一；清理走显式 Start/Close（禁止 init 启动 goroutine）。

---

## 实现

### 新增 `internal/application/state/`

| 文件 | 职责 |
|------|------|
| `repository.go` | `IdentityRepository`（= identity.IdentityResolver，P3 复用）、`SequenceRepository`（原子 Next）、`MessageContextRepository`（owner/TTL）、`Entry`、`Cleaner` |
| `sequence.go` | `MemSequenceRepository`（原子递增，仅 Next） |
| `context.go` | `MemContextRepository`（owner+key 隔离、过期校验、清理）+ `Cleaner.Start/Close` |
| `policy.go` | 统一 TTL 常量（TTLShort/TTLMedium/TTLDefault） |

### 关键设计

- **职责拆分**：Identity / MessageContext / Sequence 三组独立接口，无 EverythingRepository。
- **Sequence 原子 API**：`Next(ctx, key) (uint32, error)`，禁止 Get+Set 组合。
- **Owner 校验**：条目按 `owner+key` 隔离，跨 owner 读取返回 ErrNotFound（测试覆盖）。
- **TTL 统一**：集中在 `policy.go`，业务引用常量，不再各 map 硬编码。
- **Cleanup 显式生命周期**：`Start(ctx)` / `Close()`，非 package init。

---

## 验收清单

| 验收项 | 状态 |
|--------|------|
| package init 不再启动清理 goroutine | ✅ PASS（Cleaner.Start/Close 显式） |
| TTL 统一 | ✅ PASS（policy.go 常量） |
| owner validation 有测试 | ✅ PASS |
| msgseq 只允许 atomic Next | ✅ PASS（SequenceRepository 仅 Next） |
| Repository 可以 mock | ✅ PASS（接口 + 内存实现） |

---

## 变更文件

- 新增：`internal/application/state/{repository,sequence,context,policy}.go`
- 新增：`internal/application/state/state_test.go`

---

## 验证结果

| 命令 | 结果 |
|------|------|
| `go build ./...` | ✅ PASS |
| `go vet ./...` | ✅ PASS |
| `go test ./internal/application/state/` | ✅ PASS（97.5% coverage） |
| `go test ./...` | ✅ PASS |
| `go test -race ./...` | NOT_RUN（windows/386 不支持） |
| `govulncheck ./...` | NOT_RUN（计划 V3） |
| frontend | NOT_RUN（不涉及） |

---

## KNOWN_LIMITATIONS

- 仅内存实现；bbolt / lotus / gRPC adapter 未实现（现有 idmap 仍是生产存储，P13 切换）。
- `EventRepository`（事件缓存）未单独实现（现有 echo 覆盖，P13 拆分）。

---

## LEGACY_REMAINING

- `idmap/` 全局 bbolt 单例与 `echo/` 全局 map（P13 切换为 Repository）。
- `init()` 启动的缓存清理（现有 `StartUsernameCacheCleanup` 等，P13 收敛）。

---

## NEXT_PHASE

- P11 全局配置依赖收口（domain/application 不再读 config.GetXxx()）。
