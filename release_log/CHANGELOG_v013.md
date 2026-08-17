# Changelog — Release013

> 自 Release012 以来的所有变更。

---

## 🐛 Bug 修复

### QQ WebSocket 鉴权失败无限重连 + access_token 获取健壮性修复

修复连接 QQ 官方网关时 `InvalidSession(d=false)` + `close 4004 Authentication fail` 无限循环的问题：

- `botgo/token/authtoken.go`：access_token 响应解析兼容扁平结构（`{"access_token":...,"expires_in":7200}`）与 api-v2 `data` 信封结构，校验 HTTP 状态码与错误包络（`code`/`message`），缺失 `expires_in` 时默认 7200s；获取失败不再静默写入空 token，初次获取失败重试 3 次后向上返回明确错误（含响应体便于排查）
- `botgo/token/token.go`：`InitToken` 失败时向调用方返回错误，启动阶段即提示"无法获取AccessToken"并终止，而不是带着空 token 进入 4004 无限重连
- `botgo/websocket/client/client.go`：Identify/Resume 日志对 token 字段脱敏（补全 GSK-006 遗漏路径）；连续 3 次鉴权失败（InvalidSession `d=false` / close 4004）后转为不可鉴权错误并终止重连，提示检查 `appid/client_secret/token` 配置与机器人类型；鉴权成功（Ready）后重置失败计数
- `botgo/sessions/local/local.go` / `botgo/sessions/multi/multi.go`：不可鉴权错误（含 4914/4915 封禁下架）不再把 session 放回队列无限重连，输出明确的配置排查提示后停止该分片

### SSM 补发链入队路径补齐共享关联标识日志

- `echo.PushGlobalStack` 内部新增 `[SSM][cid] 已入队 group=<群> 队列长度=<n>` 日志，入队事件不再依赖调用方样板日志，任何入队路径（含未来新增调用点）均可通过共享关联标识观测
- `echo.NextSSMCorrelationID` 对短 groupID 做安全截断（长度不足 8 时取全部），避免切片越界 panic
- 入队（`PushGlobalStack`）与补发（`SendStackMessages`）两侧日志沿用同一 `CorrelationID`，跨边界关联方式不变，发送与补发行为零改动

### 剔除频道 handler 后消息类型递归枚举越界 panic 修复

修复 `send_private_msg` / `send_group_msg` 在直接向 Gensokyo 发送 action（`msgType` 为空触发类型递归）时的 `panic: runtime error: index out of range` 崩溃：

- 提交 8b69368 剔除频道出站 handler 时，把 `tryMessageTypes` 枚举数组由 `{"group", "guild", "guild_private"}` 缩减为 `{"group"}`，但递归计数器仍为 `AddMapping(idInt64, 4)`，递归索引 `tryMessageTypes[GetMapping-1]` 可达 `[2]` 而数组长度仅 1，触发越界
- 修复：`handlers/send_group_msg.go` 与 `handlers/send_private_msg.go` 的递归计数 `AddMapping(idInt64, 4)` 调整为 `AddMapping(idInt64, 2)`，与 1 元素枚举数组匹配（仅剩 `group_private` + `group` 两种类型可尝试），索引最大为 `[0]`，不再越界
- 行为不变：正常带 `Echo` 类型的 OneBot 请求不经过该递归路径；仅直接投递无类型 action 时触发，修复后递归 1 次尝试 `group` 类型

---

## 🧪 测试

### 群消息发送与解析热路径聚焦测试

**文件：** `handlers/send_group_msg_test.go`、`handlers/message_parser_test.go`

- 新增 `send_group_msg_test.go`：mock openapi 桩直接驱动 `postGroupMessageWithRetry` / `postGroupRichMediaMessageWithRetry`，覆盖超时重试恢复、非超时错误不重试、持续超时耗尽重试次数，以及富媒体重试前清空 EventID 的行为边界
- 新增 `message_parser_test.go`：表驱动覆盖 `parseMessageContent` 消息段数组路径的 foundItems 解析（text/image/voice/record/video/file/music/markdown/card/input_notify/stream/active/reply 段），含路径穿越拒绝、未知段类型跳过、markdown 非法 base64 跳过等负面用例
- 测试桩仅覆盖 PostGroupMessage，退避注入为 0 避免真实 sleep

### vitest 替换前端占位测试脚本

**文件：** `frontend/package.json`、`frontend/vitest.config.mts`、`frontend/src/pages/qdvc-utils.test.ts`、`frontend/src/pages/qdvc-utils.ts`

- `package.json` 的 `test` 脚本由自定义语法脚本改为 `vitest run`，原语法门控保留为 `test:syntax`
- 新增 `vitest.config.mts` 与 `qdvc-utils.test.ts`（4 个真实断言）
- 修复 QDVC base64 编解码对中文 device 抛 `InvalidCharacterError` 的缺陷，改为对称的 UTF-8 编解码（旧 ASCII 链接兼容）
- 同步更新 `frontend/README.md` 与 `AGENTS.md` 命令表

---

## 📦 构建与工程

### PR 触发路径 Test & Vet 修复（webui embed 占位）

**文件：** `.github/workflows/cross_compile.yml`

`webui/dist` 被 `.gitignore` 忽略，checkout 后目录不存在，`go:embed dist/*` 编译报 `pattern dist/*: no matching files found`，导致 test job 自加入以来在 CI 中必然失败（gh run 最近 4 次 PR 均 failure）。

**修复：** 在 Test job 中新增 `Create webui embed placeholders` 步骤，创建与 `build.ps1` 的 `Ensure-WebUIDist` 一致的占位文件（覆盖 dist 及 css/fonts/icons/js 五个 embed pattern）；9 平台编译矩阵与 `contents: read` 最小权限保持不变。

**验证：** 按 workflow 等效命令本地执行 `go vet ./...` 与 `go test ./... -count=1` 均通过（37 包全绿），`go build ./...` 通过。

### .mcp.json 移除 context7 MCP 服务

**文件：** `.mcp.json`

v012（P3-4.6）曾固定两个 MCP 服务版本，本轮移除 `context7`（`@upstash/context7-mcp@4.0.1`），仅保留 `github`（`@modelcontextprotocol/server-github@2025.4.8` 固定版本），并统一 args 数组换行格式。

---

## ✅ 提交记录

```
08fdd39  fix: QQ网关鉴权失败无限重连与access_token获取健壮性
67913eb  ci: 修复 PR 触发路径 Test & Vet 在 CI 中的 embed 编译失败
8d05a1a  test: 为群消息发送与解析热路径添加聚焦测试
d303879  fix: SSM 补发链入队路径补齐共享关联标识日志
68c6921  test: 用 vitest 替换前端占位测试脚本并补充真实断言
cedb2f6  docs: 修正 CHANGELOG 依赖章节为 Dependabot 停用与依赖回退
```

## 🧪 验证

| 命令 | 结果 |
|------|------|
| `go test ./handlers/`（含 8d05a1a 聚焦测试） | ✅ 通过 |
| `cd frontend && npm test`（68c6921 vitest） | ✅ 通过（4 个真实断言） |
| `cd frontend && npm run test:syntax` | ✅ 通过（语法门控保留） |
| `go vet ./...` + `go test ./... -count=1`（67913eb CI 等效） | ✅ 通过（37 包全绿） |
