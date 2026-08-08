# P2 — 配置系统重构 验证报告

```
PHASE: P2（配置系统重构）
STATUS: PASS
```

---

## 目标

```
配置文件 → Parse → Migration → Validation → Immutable Snapshot → Application
```

建立独立配置基础设施，与旧 `config/` 双轨并存；业务层切换（Snapshot 读取、watcher 接入）
由后续 P11（Config DI）完成。本阶段不改动生产行为，旧行为默认保持兼容。

---

## 实现

### 新增 `internal/infrastructure/config/`

| 文件 | 职责 |
|------|------|
| `loader.go` | 读取/解析 `config.yml` → `ConfigDTO` |
| `migrator.go` | `Migrator` 接口（yaml.Node 操作）；v0(legacy)→v1 补 `version` |
| `validator.go` | `ValidateSchema`（类型/必填/枚举/范围/URL/超时）+ `ValidateSemantic`（TLS/图床/Lotus 依赖） |
| `runtime.go` | `ConfigDTO → RuntimeConfig`（QQ/OneBot/Transport/IDMap/Media），slice 深拷贝 |
| `snapshot.go` | `Snapshot`（不可变，防御性拷贝）+ `Manager`（reload 失败保留旧快照） |
| `writer.go` | `AtomicWrite`（前置校验 + tmp + fsync + .bak 备份 + rename） |
| `watcher.go` | fsnotify + debounce 重载 |
| `errors.go` | 分类错误（Parse/Migration/Validation/IO）+ 具体字段路径 |

### 关键设计点

- **ConfigDTO vs RuntimeConfig**：DTO 只对磁盘格式负责（`version + settings` 镜像），
  RuntimeConfig 只对程序行为负责（分组 + 类型化）。业务最终禁止读 DTO。
- **Schema Version**：`version: 1`；`Migrator interface { CanMigrate(from,to); Migrate(*yaml.Node, from, to) }`，
  全部基于 yaml.Node 结构化操作，无字符串 contains / 行号 / 缩进 / 手工 append。
- **Schema 校验错误带路径**：如 `config.idmap.grpc_port: must be between 1 and 65535`、
  `config.transport.post_url[0]: invalid http(s) URL`、`config.qq.app_id: must not be empty`。
- **Semantic 校验**：`use_self_crt` 开启但 crt/key 缺失或文件不存在、`oss_type=4`（cos）但凭据缺失、
  `lotus` 开启但 `server_dir`/`port` 为空。（nature=10 使用公开共享凭据，开箱即用，无需校验）
- **Immutable Snapshot**：`Config()` 返回深拷贝（slice 独立底层数组），外部篡改不影响快照。
- **Atomic Write**：先校验内容可解析 → 写 tmp + fsync → 拷贝 `.bak` 备份 → rename 原子替换；
  拒绝用坏配置覆盖有效配置，失败时原配置可用。
- **Reload 错误策略**：`Manager.Reload` 失败时保留旧快照（不置零、不崩溃）。
- **Debounce**：fsnotify 事件风暴合并为一次重载（300ms 窗口）。

### 测试与 fixtures

`testdata/config/`：`legacy-basic` / `legacy-full` / `v1-basic` / `invalid-port` /
`invalid-url` / `missing-secret` / `unknown-fields` / `malformed`

覆盖：legacy→migration、parse error、未知字段容忍（向后兼容）、schema validation、
semantic validation、atomic write、backup、拒绝坏配置、reload 保留旧快照、
并发 snapshot 读 + reload、debounce。

---

## 验收清单

| 验收项 | 状态 |
|--------|------|
| 业务可以通过 Snapshot 获取配置 | ✅ PASS（`Manager.Snapshot().Config()`） |
| reload 失败不会破坏当前运行配置 | ✅ PASS（`TestManagerReloadKeepsOldOnError`） |
| 配置写入具有 tmp + backup + atomic rename | ✅ PASS（`TestAtomicWriteBackup` 等） |
| 不再通过文本字符串修改 YAML | ✅ PASS（migrator 基于 yaml.Node） |
| migration 有测试 | ✅ PASS |
| validation 有测试 | ✅ PASS |
| go test ./... PASS | ✅ PASS |

---

## 变更文件

- 新增：`internal/infrastructure/config/{errors,dto,runtime,loader,migrator,validator,snapshot,writer,watcher}.go`
- 新增：`internal/infrastructure/config/{loader,migrator,validator,snapshot,writer,watcher}_test.go`
- 新增：`internal/infrastructure/config/testdata/config/*.yml`（8 个 fixtures）
- 更新：`release_log/CHANGELOG_v013.md`

---

## 验证结果

| 命令 | 结果 |
|------|------|
| `go build ./...` | ✅ PASS |
| `go vet ./...` | ✅ PASS |
| `go test ./internal/infrastructure/config/` | ✅ PASS（24 用例） |
| `go test ./...` | ✅ PASS |
| `go test -race ./...` | NOT_RUN（计划 V2/V3 阶段执行） |
| `govulncheck ./...` | NOT_RUN（计划 V2/V3 阶段执行） |
| frontend | NOT_RUN（本阶段不涉及前端） |

---

## KNOWN_LIMITATIONS

- 新基础设施尚未接入生产（`main.go` 仍用旧 `config.LoadConfig` 与旧 watcher）；接入属 P11。
- `ConfigDTO.Settings` 直接复用 `structs.Settings`（即磁盘格式本体），v2 目标结构（QQ/OneBot/Transport 嵌套）
  待配置格式正式重构时引入。
- `unknown-fields` 策略为容忍（向后兼容旧配置），未启用 `KnownFields(true)`。
- 语义校验中"HTTP 开启但 address 为空"在当前 schema 下无意义（HTTP 开启 == address 非空），
  已由 schema 校验 `http_address` 格式覆盖，未单独实现。

---

## LEGACY_REMAINING

- 旧 `config/` 全局 singleton 及其 getter 全部保留（P11 收口）。
- 旧文本式补全/去重（`ensureConfigComplete`/`cleanupDuplicateSettings`）保留（后续阶段替换）。

---

## NEXT_PHASE

- P3 Identity 类型化（`internal/domain/identity/`）。
