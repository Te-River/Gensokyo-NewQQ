# Changelog — Release012

> 自 Release011 (`d5c780b`) 以来的所有变更。本轮引入 Dependabot 依赖自动更新机制，并合并首批 4 个依赖升级 PR。

---

## 🛠 工程配置

### Dependabot 自动依赖更新

**文件：** `.github/dependabot.yml`（新增）

引入 Dependabot 对三大依赖生态的自动更新：

| 生态系统 | 目录 | 更新频率 | 说明 |
|---------|------|---------|------|
| `gomod` | `/` | 每周一 08:00（Asia/Shanghai） | Go 模块依赖 |
| `npm` | `/frontend` | 每周一 08:00（Asia/Shanghai） | 前端 Vue3 依赖 |
| `github-actions` | `/` | 每月周一 08:00（Asia/Shanghai） | Actions 工作流版本 |

- **忽略本地 fork 依赖**：`github.com/tencent-connect/botgo`、`github.com/wdvxdr1123/go-silk` 通过 `go.mod replace` 引用本地目录，Dependabot 不尝试升级
- **分组更新**：`minor`/`patch` 更新合并为单个 PR，减少 PR 噪音；`major` 单独成 PR
- **PR 规范**：统一打 `dependencies` + 生态标签，指定 `Te-River` 为 reviewer
- **`target-branch: main`**：Dependabot PR 直接面向 `main` 分支

### .gitignore 新增忽略项

| 条目 | 说明 |
|------|------|
| `.qoder/` | Qoder IDE 缓存目录 |

---

## 📦 依赖升级（首批 Dependabot PR）

| PR | 依赖 | 版本变化 | 说明 |
|----|------|---------|------|
| #22 | `form-data` | 4.0.0 → 4.0.6 | 前端 `package-lock.json` |
| #21 | `golang.org/x/crypto` | 0.23.0 → 0.52.0 | Go 模块 |
| #23 | `google.golang.org/grpc` | 1.65.0 → 1.82.1 | Go 模块（跨多个大版本） |
| #20 | `golang.org/x/net` | → 0.55.0 | `botgo/examples` 嵌套模块 |

## 📦 依赖升级（第二批 Dependabot PR）

| PR | 依赖 | 版本变化 | 说明 |
|----|------|---------|------|
| #26 | `golang.org/x/image` | 2019-02 版 → 0.41.0 | `botgo` 嵌套模块 |
| #28 | `actions/setup-node` | 6 → 7 | GitHub Actions |
| #29 | `actions/setup-go` | 6 → 7 | GitHub Actions |
| #30 | go_modules minor-and-patch 组（14 项） | 升级 | 根模块依赖 |

> 注：CodeQL `Analyze` 检查失败为 CI 基础设施问题（default setup 与 advanced 配置冲突），与 PR 内容无关；前端 major 版本升级 PR（#24/#31-#39）因 ERESOLVE 依赖冲突或 Quasar 构建失败暂未合并。

## 📦 依赖升级（第三批 Dependabot PR）

| PR | 依赖 | 版本变化 | 说明 |
|----|------|---------|------|
| #43 | go_modules 组（3 项） | 升级 | `botgo`/`botgo/examples`/根模块（替代冲突的 #42） |
| #44 | go_modules 组（2 项） | 升级 | `botgo`/根模块 |
| #40 | `@types/node` | 20.8.10 → 26.1.2 | 前端 devDependencies |

> 前端 major 升级 PR（#24/#31/#32/#33/#35/#38/#39/#41/#45）因 ERESOLVE 依赖冲突或构建失败已全部关闭，等待 Dependabot 基于最新 main 重新生成。

---

## 🧰 前端依赖协调修复

手动升级前端依赖，解决 Dependabot major 升级 PR 的 peer 依赖冲突（本地验证通过后直接合并到 main）：

| 依赖 | 版本变化 | 说明 |
|------|---------|------|
| `@typescript-eslint/parser` | 5.62.0 → 8.66.0 | 与 eslint-plugin 配对升级（对应 PR #34） |
| `@typescript-eslint/eslint-plugin` | 5.62.0 → 8.66.0 | 与 parser 配对升级（对应 PR #37） |
| `eslint` | 8.52.0 → 8.57.1 | 兼容 @typescript-eslint 8.x（不升 10.x，避免 flat config 迁移） |

配套修复：

- `.eslintrc.js`：新增 `@typescript-eslint/no-unused-vars` 下划线前缀忽略配置（`argsIgnorePattern`/`varsIgnorePattern`/`caughtErrorsIgnorePattern: '^_'`）
- 修复升级后新增的 12 处 lint 错误（`no-unused-vars` / `no-explicit-any`）：`ChannelView.vue`、`GroupView.vue`、`ChannelList.vue`、`GroupList.vue`、`LoginView.vue`
- 前端构建验证通过（`npm ci` + `quasar build`）
- 已关闭被覆盖的 Dependabot PR #34/#37（目标版本已落后于 main）

---

## 🧪 验证

| 命令 | 结果 |
|------|------|
| `go build ./...` | ✅ 通过（每个 PR 合并后均验证） |
| `go vet ./...` | ✅ 通过 |
| `go test ./handlers/` | ✅ 通过 |

> 注：`botgo/examples` 为嵌套模块，其示例代码引用已删除的 botgo 旧 API 无法独立编译，属预先存在的问题，与本次依赖升级无关，不影响主项目。

---

## ⚠️ 已知安全告警（Dependabot CVE）

### 修复结果（2026-08-04）

通过升级直接依赖 + `overrides` 强制升级传递依赖，前端漏洞从 **51 个降至 2 个 moderate**（均为 `@quasar/app-webpack` 内部开发期依赖，需升级到 4.x breaking change 才能消除，暂保留）：

| 操作 | 内容 |
|------|------|
| 升级 `axios` | 1.5.0 → 1.19.0（消除 29 个 high/critical 告警，含 SSRF/拒绝服务/原型污染） |
| 升级 `quasar` | 2.12.7 → 2.23.5 |
| `npm audit fix` | 自动升级 `@quasar/app-webpack` 3.11.3 → 3.15.1 等，消除 critical（websocket-driver/shell-quote）与大部分 high |
| `overrides` | `serialize-javascript` ^7.0.7、`tmp` ^0.2.7、`uuid` ^11.1.1、`yaml` ^2.8.1、`cookie` ^0.7.2 强制升级传递依赖 |

> 说明：修复前存在 **99 条告警**（2 critical / 43 high / 42 medium / 12 low），全部位于前端 `frontend/package-lock.json`；修复后仅剩 2 moderate，不影响 Go 后端服务。跟踪地址：https://github.com/Te-River/Gensokyo-NewQQ/security/dependabot

---

## ✅ 提交记录

```
8d71a1d  chore(deps): bump form-data from 4.0.0 to 4.0.6 in /frontend (#22)
3c6e196  chore(deps): bump golang.org/x/crypto from 0.23.0 to 0.52.0 (#21)
9089784  chore(deps): bump google.golang.org/grpc (#23)
2afff2a  chore(deps): bump golang.org/x/net in /botgo/examples (#20)
0b43e2e  Merge branch 'Pr-Edit'
59a8455  chore: gitignore 新增 .qoder/ 忽略项
0b4ee93  ci: 新增 Dependabot 配置覆盖 Go/npm/GitHub Actions 依赖
3e7bb6d  docs: 新建 CHANGELOG_v012 记录 Dependabot 依赖更新
6377d85  Add CodeQL analysis workflow configuration
c06ce56  chore(deps): bump golang.org/x/image in /botgo (#26)
8c0cf7c  chore(deps): bump actions/setup-node from 6 to 7 (#28)
2aa3c52  chore(deps): bump actions/setup-go from 6 to 7 (#29)
3735fe9  fix: 升级前端 @typescript-eslint 并修复 lint 构建失败
6d7e45a  docs: 更新 CHANGELOG_v012 记录前端依赖协调修复
53d88c5  chore(deps): bump the go_modules group across 2 directories with 3 updates (#43)
8ffe2af  chore(deps): bump the go_modules group across 2 directories with 2 updates (#44)
7a631cb  deps(deps-dev): bump @types/node from 20.8.10 to 26.1.2 in /frontend (#40)
```
