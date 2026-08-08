# S0 — 凭据安全处理 验证报告

```
PHASE: S0（凭据安全处理）
STATUS: PARTIAL_PASS
```

> S0.1 / S0.3 已完成并通过验证；S0.2（云端轮换）与 S0.4（Git 历史清理）
> 为需要人工在云控制台/仓库治理层面执行的动作，当前 BLOCKED on user action。

---

## S0.1 检查 Nature 凭据与全仓 Secret 扫描

**结论：** 全仓唯一真实内置云凭据位于 `imagehosting/nature.go`：

| 项目 | 内容 |
|------|------|
| 位置 | `imagehosting/nature.go`（已删除） |
| 凭据 | 2 个 base64 编码的腾讯 COS SecretId / SecretKey |
| 用途 | "Nature" 免费图床（`oss_type=10`）向腾讯 COS 存储桶直传图片 |
| 存储桶 | `sgame-data-service-1252931805`（`ap-nanjing`） |
| 涉及文件 | 仅 `imagehosting/nature.go`（`mustB64` / `_natureSecretID` / `_natureSecretKey`） |

**全仓扫描覆盖：**

- `SecretID / SecretKey / AccessKey / AccessSecret`：其余出现均为**配置注入**（`config.GetTencentCosSecretid()`、
  `config.GetAliyunAccessKeyId()` 等），无硬编码。
- `token / credential / base64.StdEncoding.DecodeString("...")`：其余均为运行时用户提供的数据（图片/文件 base64、
  GitHub Actions `secrets.GITHUB_TOKEN` 引用、`.mcp.json` 空 token），无内置真实凭据。
- 未发现 `sk-*`、`AKIA*`、`ghp_*`、`-----BEGIN` 等其它凭据格式。

**待用户确认：** `readme.md` 中 WebUI 默认口令 `server_user_password: "admin"` 为弱默认口令，
不属于云凭据（超出 S0 范围），建议后续加固。

---

## S0.2 云端轮换（用户操作，BLOCKED）

无法从代码侧完成，需在腾讯云控制台人工执行：

1. 创建替代凭据（建议子账号、最小权限、仅目标存储桶 PutObject）。
2. 更新部署环境中的 `config.yml`（`nature:` 段或 `cos:` 段）。
3. 验证新凭据可正常上传。
4. **revoke 旧凭据**（即被内置在历史版本源码中的那组）。
5. 确认旧凭据已无法继续使用。

> ⚠️ 仅删除 Git 中的 Secret 不算修复。真正的安全修复点是**旧凭据已失效**。
> 请勿在 commit / issue / report / log 中写入真实 Secret 值。

---

## S0.3 删除源码内置 Secret（已完成）

- `imagehosting/nature.go`：删除 `mustB64` 与 base64 硬编码凭据；`tryNature` 改为读取
  `config.GetImageHostingNature()`，凭据缺失时 **fail closed**。
- `structs/structs.go`：`Settings.Nature` 类型 `ImageHostingSimple` → `ImageHostingNature`
  （`secret_id` / `secret_key` / `region` / `bucket` / `domain`）。
- `config/config.go`：`GetImageHostingNature()` 返回类型同步。
- `template/config_template.go`、`readme.md`、`imagehosting/README.md`：配置示例同步更新。
- 默认域名不再指向旧存储桶 CDN，与 `cos.go` 一致（留空用 COS 默认域名）。

**兼容性影响：** `oss_type=10` 由"开箱即用"改为"需配置凭据"。此为安全要求的破坏性变更。

---

## S0.4 Git 历史处理（待用户决策，BLOCKED）

- 建议先完成 S0.2 凭据轮换，再评估 `git filter-repo`。
- 历史清理是仓库治理问题，**不是 Secret 撤销的替代品**。
- 若执行历史重写：必须单独任务，禁止与 P2~P13 混用，需用户明确授权。

---

## S0 验收清单

| 验收项 | 状态 |
|--------|------|
| 当前 HEAD 不包含真实云凭据 | ✅ PASS |
| 旧凭据已 revoke | ⏳ BLOCKED（用户云控制台操作） |
| Provider 可从配置获取凭据 | ✅ PASS |
| 缺少凭据时 fail closed | ✅ PASS（含测试） |
| 日志不会打印凭据 | ✅ PASS（代码中无凭据打印路径） |

---

## 变更文件

- `imagehosting/nature.go`（重写，删除凭据）
- `imagehosting/hosting_test.go`（+fail-closed 测试）
- `structs/structs.go`（Nature 配置结构）
- `config/config.go`（Getter 返回类型）
- `template/config_template.go`（配置示例）
- `readme.md` / `imagehosting/README.md`（文档同步）
- `release_log/CHANGELOG_v013.md`（新增）

---

## 验证结果

| 命令 | 结果 |
|------|------|
| `go build ./...` | ✅ PASS |
| `go vet ./...` | ✅ PASS |
| `go test ./...` | ✅ PASS |
| `go test -race ./...` | NOT_RUN（计划 V2/V3 阶段执行） |
| `govulncheck ./...` | NOT_RUN（计划 V2/V3 阶段执行） |
| frontend | NOT_RUN（本阶段不涉及前端） |

---

## NEXT_PHASE

- 用户完成 S0.2 凭据轮换后，S0 整体可标记 PASS。
- P2 配置系统重构（双轨迁移，建立 `internal/infrastructure/config/`）。
