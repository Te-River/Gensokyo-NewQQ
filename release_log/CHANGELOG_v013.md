# Changelog — Release013

> 自 Release012 以来的所有变更。

---

## 🔒 安全：移除源码内置云凭据（S0）

**背景：** `imagehosting/nature.go` 中曾硬编码一组腾讯 COS 的 SecretId/SecretKey（base64 编码），
用于 "Nature" 免费图床（oss_type=10）直传。内置真实云凭据属于高危安全缺陷，必须移除。

**变更：**

- `imagehosting/nature.go` 删除 base64 硬编码凭据及 `mustB64` 函数，改为从配置读取。
- `structs.Settings.Nature` 类型由空结构 `ImageHostingSimple` 改为 `ImageHostingNature`
  （含 `secret_id` / `secret_key` / `region` / `bucket` / `domain` 字段）。
- `config.GetImageHostingNature()` 返回类型同步更新。
- 凭据缺失时 **fail closed**（返回错误），不再回退到任何内置凭据。
- 默认域名不再指向旧存储桶的 CDN，改为与 `cos.go` 一致：留空时使用 COS 默认域名。
- `template/config_template.go`、`readme.md`、`imagehosting/README.md` 同步更新配置示例。

**⚠️ 破坏性变更：** `oss_type=10`（Nature）不再开箱即用，必须自行填写 COS 凭据。

**安全注意：** 请到腾讯云控制台 revoke 旧凭据（被内置在历史版本源码中的那组），
并确认旧凭据已无法继续使用。删除源码中的 Secret 不等于修复，**旧凭据失效**才是真正的修复点。

---

## 🏗 配置基础设施：Immutable Snapshot 管线（P2）

**背景：** 旧 `config/config.go` 全局 singleton 同时负责读取、补全、写入、重载，
缺少独立的 parse → migrate → validate → snapshot 管线，直接覆盖写存在截断风险。

**新增 `internal/infrastructure/config/`（与旧 config/ 双轨并存，未接入生产）：**

- `loader.go`：读取/解析 `config.yml` → `ConfigDTO`（只对磁盘格式负责）
- `migrator.go`：`Migrator` 接口（基于 `yaml.Node`，禁止字符串/行号/缩进 hack）；v0(legacy) → v1 自动补 `version`
- `validator.go`：`ValidateSchema` + `ValidateSemantic`
  - Schema：app_id 必填、端口范围、URL/地址格式、oss_type 枚举、超时/媒体上限非负，错误带具体路径（如 `config.idmap.grpc_port`）
  - Semantic：TLS 开启但证书缺失、图床开启但凭据缺失、Lotus 开启但 endpoint 缺失
- `runtime.go`：`ConfigDTO → RuntimeConfig`（QQ/OneBot/Transport/IDMap/Media 分组，slice 深拷贝）
- `snapshot.go`：`Snapshot`（不可变）+ `Manager`（重载失败保留旧快照，不置零不崩溃）
- `writer.go`：`AtomicWrite`（前置校验 + 临时文件 + fsync + `.bak` 备份 + rename，失败时原配置可用）
- `watcher.go`：fsnotify + debounce（事件风暴合并为一次重载）
- `errors.go`：分类错误（Parse/Migration/Validation/IO）+ 字段路径

**Golden fixtures（`testdata/config/`）：** `legacy-basic` / `legacy-full` / `v1-basic` /
`invalid-port` / `invalid-url` / `missing-secret` / `unknown-fields` / `malformed`

**说明：** 本阶段只建基础设施与测试，业务层切换（Snapshot 读取、watcher 接入）由后续 P11 完成，
保证旧行为完全兼容。

---

## 🧪 验证

| 命令 | 结果 |
|------|------|
| `go build ./...` | ✅ 通过 |
| `go vet ./...` | ✅ 通过 |
| `go test ./imagehosting/ ./config/ ./structs/ ./template/` | ✅ 通过 |
| `go test ./internal/infrastructure/config/` | ✅ 通过（24 用例） |
| `go test ./...` | ✅ 通过 |

---

## ✅ 提交记录

```
<commit hash>（S0）
<commit hash>（P2）
```
