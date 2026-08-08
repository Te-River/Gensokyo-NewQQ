# P3 — Identity 类型化 验证报告

```
PHASE: P3（Identity 类型化）
STATUS: PASS
```

---

## 目标

彻底区分 OpenID / VirtualUserID / VirtualGroupID / UIN / AppID，
把散落的 `len(id)==32` 长度启发式收敛到转换边界（legacy adapter）。

---

## 实现

### 新增 `internal/domain/identity/`

| 文件 | 职责 |
|------|------|
| `types.go` | `OpenID` / `OpenGroupID` / `VirtualUserID` / `VirtualGroupID` / `UIN` / `AppID` 类型 + 转换方法 |
| `classify.go` | legacy adapter：`IsOpenID` / `IsVirtualID`（长度启发式的唯一收敛点） |
| `resolver.go` | `UserRef` / `GroupRef` / `ResolvedUser` / `ResolvedGroup` / `IdentityResolver` 接口 + `ErrNotFound`/`ErrAmbiguous` |
| `target.go` | `TargetKind`（Group/Private）+ `ResolvedTarget` |

### 新增 `adapter/identity/`

- `idmap_resolver.go`：基于现有 idmap 存储的真实 `IdentityResolver` 实现
  （OpenID → 虚拟 ID 经 `StoreUserID/StoreGroupID`，反向经 `RetrieveUserID/RetrieveGroupID`），
  是 string ↔ typed identity 的转换边界；`idmap.ErrKeyNotFound` 映射为 `identity.ErrNotFound`。

### 长度启发式收敛（P3.2 / 验收核心）

全仓 `len(id)==32 / !=32` 身份判断已**全部归零**，只剩 `classify.go` 的 `IsOpenID`（legacy adapter）：

| 位置 | 收敛前 | 收敛后 |
|------|--------|--------|
| `handlers/delete_msg.go` | `len(GroupID.(string)) != 32` | `!identity.IsOpenID(...)` |
| `handlers/send_msg.go` | ×2 | `identity.IsOpenID(...)` |
| `handlers/send_group_msg_raw.go` | ×4 | `identity.IsOpenID(...)` |
| `handlers/send_group_msg.go` | ×5 | `identity.IsOpenID(...)` |
| `handlers/send_private_msg.go` | ×4 | `identity.IsOpenID(...)` |
| `handlers/send_private_msg_sse.go` | ×1 | `identity.IsOpenID(...)` |
| `handlers/send_private_msg_wakeup.go` | ×2 | `identity.IsOpenID(...)` |
| `idmap/service.go` | ×3 | `ididentity.IsOpenID(...)`（别名避免局部变量冲突） |
| `idmap/new_service.go` | ×1 | `ididentity.IsOpenID(...)` |
| `idmap/map_service.go` | ×2 | `ididentity.IsOpenID(...)` |

所有替换均为纯等价替换（`IsOpenID(s) == (len(s)==32)`），零行为差异。

### P3.5 原字段覆盖

`delete_msg.go` / `send_group_msg.go` 中仍存在 `params.GroupID = originalOpenID` 式的字段覆盖
（撤回/还原路径）。本轮未改（涉及行为），已在 inventory 记录，P13 重写时统一改为
`resolver.Resolve(...)`，保证原始 Action 不变。

---

## 验收清单

| 验收项 | 状态 |
|--------|------|
| 全仓 `len(id)==32` 身份判断归零 / 只剩 legacy adapter | ✅ PASS（仅 `classify.go` 内） |
| Resolver 单测 PASS | ✅ PASS |
| ID 字段不会运行中改变含义 | ✅ PASS（typed 类型，`Config()` 深拷贝；字段覆盖遗留已在 inventory） |
| Application 主链使用 typed identity | ✅ PARTIAL（Resolver 已就绪，主链切换属 P6/P13） |

---

## 变更文件

- 新增：`internal/domain/identity/{types,classify,resolver,target}.go`
- 新增：`internal/domain/identity/{types,classify,resolver}_test.go`
- 新增：`adapter/identity/{idmap_resolver.go,idmap_resolver_test.go}`
- 收敛：`handlers/{delete_msg,send_msg,send_group_msg_raw,send_group_msg,send_private_msg,send_private_msg_sse,send_private_msg_wakeup}.go`
- 收敛：`idmap/{service,new_service,map_service}.go`

---

## 验证结果

| 命令 | 结果 |
|------|------|
| `go build ./...` | ✅ PASS |
| `go vet ./...` | ✅ PASS |
| `go test ./internal/domain/identity/ ./adapter/identity/` | ✅ PASS（12 用例） |
| `go test ./...` | ✅ PASS |
| `go build -tags=map_idmap ./...` | ⚠️ 既有失败（见 KNOWN_LIMITATIONS） |
| `go test -race ./...` | NOT_RUN（计划 V2/V3） |
| `govulncheck ./...` | NOT_RUN（计划 V2/V3） |
| frontend | NOT_RUN（不涉及） |

---

## KNOWN_LIMITATIONS

- `IsOpenID` / `IsVirtualID` 是 legacy 启发式：32 位全数字字符串会同时命中两者（重叠），
  已用注释标注"非永久方案"，P13 以显式解析器替代。
- `-tags=map_idmap` 编译失败为**既有问题**：`map_service.go`（tag）与 `new_service.go`（无条件）
  重复定义符号，与本次改动无关，未修复（超 P3 范围）。
- `idmap.ErrKeyNotFound` 以外的存储错误（如 DB 损坏）原样透传，未包装为 domain 错误。
- `params.GroupID = realOpenID` 式字段覆盖遗留于撤回/还原路径（inventory，P13 处理）。

---

## LEGACY_REMAINING

- handlers 仍直接用 idmap 全局 API（`idmap.StoreUserID` 等）而非 Resolver——主链切换属 P6/P13。
- `callapi.ActionMessage.GroupID/UserID` 仍为 `interface{}`（P9 处理）。

---

## NEXT_PHASE

- P4 消息解析类型化（`ParsedMessage` / `MessagePart`，冻结 foundItems）。
