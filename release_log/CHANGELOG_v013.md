# Changelog — Release013

> 自 Release012 以来的所有变更。

---

## 🐛 Bug 修复

### QQ WebSocket 鉴权失败无限重连 + access_token 获取健壮性修复

修复连接 QQ 官方网关时 `InvalidSession(d=false)` + `close 4004 Authentication fail` 无限循环的问题：

- `botgo/token/authtoken.go`：access_token 响应解析兼容扁平结构（`{"access_token":...,"expires_in":7200}`）与 api-v2 `data` 信封结构，校验 HTTP 状态码与错误包络（`code`/`message`），缺失 `expires_in` 时默认 7200s；获取失败不再静默写入空 token，初次获取失败重试 3 次后向上返回明确错误（含响应体便于排查）
- `botgo/token/authtoken.go`：access_token 定时刷新对齐官方文档「凭证有效期与刷新」——由"等满整个 TTL（7200s）才刷新"改为**提前 45s 刷新**（官方建议在接近过期 60 秒内获取新 token，旧 token 在该窗口内仍有效；提前 45s 预留网络延迟余量，实现无缝切换）；**刷新失败后 30s 快速重试**（原为失败后继续等待完整 TTL，最长 2 小时空窗期内所有 WS 鉴权均会 4004 掉线）；新增刷新计划/刷新成功日志（输出有效期与距下次刷新倒计时，token 沿用脱敏）
- `botgo/token/token.go`：`InitToken` 失败时向调用方返回错误，启动阶段即提示"无法获取AccessToken"并终止，而不是带着空 token 进入 4004 无限重连
- `botgo/websocket/client/client.go`：Identify/Resume 日志对 token 字段脱敏（补全 GSK-006 遗漏路径）；连续 3 次鉴权失败（InvalidSession `d=false` / close 4004）后转为不可鉴权错误并终止重连，提示检查 `appid/client_secret/token` 配置与机器人类型；鉴权成功（Ready）后重置失败计数
- **停电自愈（整轮重试 5 次）**：连续 3 次鉴权失败达到上限后，不再立即永久停止重连——`botgo/sessions/local/local.go` / `botgo/sessions/multi/multi.go` 的 `newConnect` 在 `CanNotIdentify` 分支新增整轮重试逻辑：`dto.Session` 新增 `AuthRetryCount` 字段，`sessions/manager` 新增 `MaxAuthRetryCount = 5` 常量；未达上限时清零 `AuthFailCount` 并将 session 放回队列自动从头整轮重试（重新连接 → 重新鉴权），覆盖服务器停电重启后陈旧 access_token 的瞬时鉴权失败；已重试满 5 轮仍未成功才真正停止重连，并输出"已重试%d轮仍未成功"日志提示检查配置
- `botgo/sessions/local/local.go` / `botgo/sessions/multi/multi.go`：不可鉴权错误（含 4914/4915 封禁下架）不再把 session 放回队列无限重连，输出明确的配置排查提示后停止该分片

### SSM 补发链入队路径补齐共享关联标识日志

- `echo.PushGlobalStack` 内部新增 `[SSM][cid] 已入队 group=<群> 队列长度=<n>` 日志，入队事件不再依赖调用方样板日志，任何入队路径（含未来新增调用点）均可通过共享关联标识观测
- `echo.NextSSMCorrelationID` 对短 groupID 做安全截断（长度不足 8 时取全部），避免切片越界 panic
- 入队（`PushGlobalStack`）与补发（`SendStackMessages`）两侧日志沿用同一 `CorrelationID`，跨边界关联方式不变，发送与补发行为零改动

---

## ✨ 新功能 / 变更

### `[CQ:reply]` 支持 REFIDX 引用（引用非机器人消息）

QQ v2 群聊/C2C 发送 API 要求引用「非机器人发的消息」时，`message_reference.message_id` 必须使用事件 `message_scene.ext[]` 中的 `REFIDX_*` 索引（官方 Go SDK 的 `MessageReference` 未暴露该字段，此为自扩展）：

- `botgo/dto/message.go`：`dto.Message` 新增 `MessageScene` 字段（`source` + `ext []string`，`key=value` 格式），入站事件自动解析 `msg_idx=REFIDX_*`
- `echo/echo.go`：新增内存映射 `globalRefIdxMap`（消息ID → REFIDX），提供 `StoreRefIdx` / `GetRefIdx` / `DeleteRefIdx` / `StoreRefIdxFromScene`，并接入 30min 清理例程；`StoreRefIdxFromScene` 从 `MessageScene.Ext` 提取 `msg_idx=` 前缀存入
- `Processor/ProcessGroupMessage.go` / `ProcessGroupNormalMessage.go` / `ProcessC2CMessage.go`：入站时调用 `echo.StoreRefIdxFromScene(data.ID, data.MessageScene)`，把每条消息的 REFIDX 与消息 ID 关联存储
- `handlers/reply_helpers.go`：新增 `ResolveReplyRefID`——反查 `[CQ:reply]` 目标真实消息 ID 后，若该消息关联了 REFIDX 则优先返回 REFIDX，否则回退普通消息 ID
- `handlers/send_group_msg.go` / `handlers/send_private_msg.go`：全部 6 处 reply 处理（文本/图文混合/transmd/Markdown/富媒体/私聊）的 `MessageReference.MessageID` 改用 `ResolveReplyRefID(refID)`；`MsgID` 字段保持普通消息 ID（被动回复语义不变）
- 行为：引用机器人自己发的消息时（发送响应 `ext_info.ref_idx`）与无 REFIDX 的老消息，回退原有 `message_id` 逻辑，零回归
- 测试：`echo/echo_test.go` 新增 `TestStoreRefIdxAndGetRefIdx` / `TestStoreRefIdxFromScene`

### 独立 `[CQ:keyboard]` 可与 markdown 消息共存

`[CQ:markdown]` + 独立 `[CQ:keyboard,data=base64://...]` 组合发送时，按钮不再被忽略：

- `handlers/send_group_msg.go` / `handlers/send_private_msg.go`：独立 `[CQ:keyboard]` 的合并条件由 `md == nil` 改为 `groupMessage.Keyboard == nil`——markdown 消息未内嵌 keyboard（`parseMarkdownFromMessage` 返回的 `kb` 为 nil）时，独立 `[CQ:keyboard]` 仍会附加到 `groupMessage.Keyboard`，实现「MD 内容 + 独立键盘按钮」；**foundItems 循环路径同步补齐**——消息段（koishi/TRSS）发送 markdown 走 foundItems 循环而非主文本路径，循环内 markdown 分支此前只合并 reply 未合并独立 keyboard，现同样按 `groupMessage.Keyboard == nil` 合并（群聊 `ResolveKeyboardImages`，私聊另含 `ResolvePlaceholderUserIDs`），发送请求体正确携带 `keyboard` 字段
- 行为：markdown JSON 已内嵌 keyboard 时优先使用内嵌键盘（`Keyboard` 非 nil 跳过合并），无 markdown 时行为与原先完全一致，零回归
- 私聊路径保留 `ResolvePlaceholderUserIDs`（`__USER_ID__` 占位符替换）与 `ResolveKeyboardImages`
- `handlers/message_parser.go`：**消息段路径补齐 keyboard case**——`[]interface{}` 数组段（koishi）与 `map[string]interface{}`（TRSS）两条路径此前缺失 `case "keyboard"`（日志 `Unhandled segment type: keyboard`，按钮被静默丢弃），现按 string 路径 `ProcessCQKeyboard` 语义补齐（`base64://` 解码 / 原始 JSON 直接存入 `foundItems["keyboard"]`，不残留 messageText，避免统一管道重复处理）；三路径（string / 数组段 / TRSS）解析行为一致

### set_group 系列 CQ 码统一为 `[CQ:set_group,action=...]`

将原 4 个出站动作 CQ 码合并为 1 个统一 CQ 码，统一参数解析与 ID 反查，CQ 码路径与 OneBot API handler 共享同一底层实现：

- **新 CQ 码格式**：
  - `[CQ:set_group,action=ban,group_id=,user_id=,duration=]`：成员禁言/解禁
  - `[CQ:set_group,action=whole_ban,group_id=,enable=]`：全员禁言开关
  - `[CQ:set_group,action=add_request,group_id=,user_id=,flag=,approve=,reason=,add_to_member_blacklist=]`：入群申请审批
  - `[CQ:set_group,action=strategy_execute/strategy_delete,strategy_id=]`：审批策略执行/删除
- **破坏性变更**：旧 CQ 码 `set_group_ban` / `set_group_whole_ban` / `set_group_add_request` / `strategy` 已移除，插件需迁移（迁移对照见 [`docs/cq码/扩展CQ码/扩展cq码-cq-set_group.md`](../docs/cq码/扩展CQ码/扩展cq码-cq-set_group.md)）
- **新增 `cqParseParams`**：顺序无关的 `key=value` 参数解析器（handlers/cqcode.go），替换 4 个旧函数各自重复的解析循环
- **新增 `handlers/set_group_helpers.go`** 共享底层：`resolveGroupOpenID` / `resolveMemberOpenID` / `applyRestrictChatSetting` / `approveJoinRequest`，CQ 码动作与 `set_group_ban` / `set_group_whole_ban` / `set_group_add_request` handler 共用
- **消息段路径补齐**：`handlers/message_parser.go` 的 `[]interface{}` 与 map（TRSS）两条路径新增 `case "set_group"`，消息段自动还原为 CQ 码字符串后走统一分发；顺带补齐 `member` 在 TRSS map 路径缺失的 case
- **OneBot API 兼容**：`/set_group_ban`、`/get_group_ban`、`/set_group_whole_ban`、`/get_group_whole_ban`、`/set_group_add_request` 注册名与参数不变，仅内部实现切换为共享底层
- 文档：新增 `扩展cq码-cq-set_group.md`（含参数详解/数据流/迁移指南/FAQ），删除 4 个旧 CQ 码文档，同步更新 `readme.md`、`CQ码汇总.md`、`api介绍.md`、`本版新增功能.md`

### 剔除频道 handler 后消息类型递归枚举越界 panic 修复

修复 `send_private_msg` / `send_group_msg` / `send_group_msg_raw` / `send_msg` 在直接向 Gensokyo 发送 action（`msgType` 为空触发类型递归）时的 `panic: runtime error: index out of range` 崩溃：

- 提交 8b69368 剔除频道出站 handler 时，把 `tryMessageTypes` 枚举数组由 `{"group", "guild", "guild_private"}` 缩减为 `{"group"}`，但递归计数器仍为 `AddMapping(idInt64, 4)`，递归索引 `tryMessageTypes[GetMapping-1]` 可达 `[2]` 而数组长度仅 1，触发越界
- 修复：`handlers/send_group_msg.go` / `send_private_msg.go` / `send_group_msg_raw.go` / `send_msg.go` 的递归计数 `AddMapping(idInt64, 4)` 调整为 `AddMapping(idInt64, 2)`，枚举数组与 1 元素匹配（仅剩 `group_private` + `group` 两种类型可尝试），索引最大为 `[0]`，不再越界
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
d2b74fe  feat: 将 set_group 系列 CQ 码统一为 [CQ:set_group,action=...]
```

## 🧪 验证

| 命令 | 结果 |
|------|------|
| `go test ./handlers/`（含 8d05a1a 聚焦测试） | ✅ 通过 |
| `cd frontend && npm test`（68c6921 vitest） | ✅ 通过（4 个真实断言） |
| `cd frontend && npm run test:syntax` | ✅ 通过（语法门控保留） |
| `go vet ./...` + `go test ./... -count=1`（67913eb CI 等效） | ✅ 通过（37 包全绿） |
