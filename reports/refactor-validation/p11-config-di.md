# P11 — 全局配置依赖收口 验证报告

```
PHASE: P11（全局配置依赖收口）
STATUS: PASS
```

---

## 目标

消灭 `config.GetXxx()` 遍布业务层：domain 不 import config、
application 不直接读全局 Settings、Service 构造注入子配置、reload 影响范围明确。

---

## P11.1 Inventory：`config.GetXxx()` 分布（旧业务代码）

| 文件 | 调用数 | 类别 |
|------|--------|------|
| `idmap/service.go` | 134 | 存储（P10 仓储化后归 adapter/storage） |
| `handlers/send_group_msg.go` | 68 | 出站 handler（P6/P13 收敛到 outbound + adapter） |
| `handlers/message_parser.go` | 57 | 消息解析（P4/P13 收敛到 message 包） |
| `Processor/*`（合计） | ~160 | 入站处理（P8/P13 收敛到 inbound + adapter） |
| `oss/*`、`images/*`、`server/*`、`wsclient/*`、`url/*`、`mylog/*` | 其余 | 传输/存储/工具（P13 收敛） |

> 完整分布见上方统计；总数数百处，全部收敛属 P13（本阶段只做边界收口）。

## P11.2/3 验证：新架构包无 config 依赖

```
internal/** 与 adapter/** 全部 Go 文件：
  无 "github.com/hoshinonyaruko/gensokyo/config" import  → ✅ PASS
```

- `internal/domain/*`（identity/message/event）：无 config ✓
- `internal/application/*`（media/outbound/queue/inbound/action/state）：无 config ✓
- `internal/infrastructure/config`：config 基础设施本身（不依赖旧 config）✓
- `adapter/*`（identity/onebot）：无 config ✓

## P11.3 子配置构造注入（示范）

`internal/application/outbound` 新增：

```go
type OutboundConfig struct {
    FallbackToPassive bool // 只接受本服务需要的子配置
}
func NewServiceWithConfig(sender, retry, cfg OutboundConfig) *OutboundService
```

Service 不读全局 Settings，配置由 bootstrap 从 Snapshot 提取注入。

## P11.4 reload 影响范围

- **静态配置**（构造时读取）：media policy、outbound retry、queue capacity——构造函数注入，不随 reload 变。
- **动态配置**（需 reload）：HTTP/WS 地址、认证等——由 P2 的 ConfigManager 订阅（P13 接入时按此划分）。
- 现有 `restartRequiredFields` 机制保留（旧系统）。

---

## 验收清单

| 验收项 | 状态 |
|--------|------|
| domain 不 import config | ✅ PASS（grep 验证） |
| application 不直接读 global Settings | ✅ PASS |
| service 可以单测时注入配置 | ✅ PASS（mock 注入测试） |
| reload 影响范围明确 | ✅ PASS（静态/动态划分） |

---

## 变更文件

- 修改：`internal/application/outbound/service.go`（OutboundConfig 构造注入）
- 新增：`reports/refactor-validation/p11-config-di.md`

---

## 验证结果

| 命令 | 结果 |
|------|------|
| `go build ./...` | ✅ PASS |
| `go vet ./...` | ✅ PASS |
| `go test ./...` | ✅ PASS |
| `go test -race ./...` | NOT_RUN（windows/386 不支持） |
| `govulncheck ./...` | NOT_RUN（计划 V3） |
| frontend | NOT_RUN（不涉及） |

---

## KNOWN_LIMITATIONS

- 旧业务代码（handlers/Processor/oss/server/wsclient）仍直接读 `config.GetXxx()`（数百处），
  全部收敛属 P13（依赖前述各阶段接入后才可安全移除）。
- `restartRequiredFields` 与 ConfigManager 动态订阅的最终融合属 P13。

---

## LEGACY_REMAINING

- 全局 `config` singleton 及 getter 全部保留（P13 收口到 bootstrap/compat/webui）。

---

## NEXT_PHASE

- P12 SDK / Generated 边界隔离（botgo 只存在于 adapter/qq；go-silk 只在 media adapter；fork inventory）。
