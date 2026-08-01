# Changelog — Release010

> 自 Release009 以来的所有变更。

---

## 🐛 Bug 修复

### CQ 码处理修复

**文件：** `handlers/message_parser.go`、`handlers/send_group_msg.go`、`handlers/send_private_msg.go`、`handlers/send_guild_channel_msg.go`、`handlers/send_private_msg_wakeup.go`、`botgo/dto/message_create.go`、`botgo/openapi/v2/message.go`

#### 消息段格式缺失

1. **`[CQ:card]` 消息段格式未处理**：`[]interface{}` 和 `map[string]interface{}` 两个路径缺少 `case "card"`，日志报 `Unhandled segment type: card`。已补充。

2. **`[CQ:input_notify]` 消息段格式未处理**：同上问题，日志报 `Unhandled segment type: input_notify`。已补充。

#### 纯 CQ 码不发送

3. **`[CQ:card]` 纯卡片消息无文本时被跳过**：card 处理代码位于 `if messageText != ""` 块内，纯卡片（无文本）不执行。已在 text 路径外添加独立的卡片发送逻辑。

4. **`[CQ:input_notify]` 后空白文本被当作普通消息发送**：`messageText != ""` 判断未过滤纯空格，导致空消息被发出。已改为 `strings.TrimSpace`。

#### keyMap 补齐

5. **`url_record` / `url_video` 未加入 keyMap**：`sendGroupMsgKeyMap`、`sendPrivateMsgKeyMap`、send_private_msg_wakeup 的 keyMap 均缺少这两个 key，导致 HTTP 语音/视频在 MessageToCreate 路径无法发送。已同步补全。

#### channel 扩展 CQ 码支持

6. **频道消息不支持 markdown/qqmusic**：`send_guild_channel_msg.go` 的 foundItems 循环跳过了 `markdown` 和 `qqmusic`，但 `GenerateReplyMessage` 实际已支持这两个类型。已释放并新增 `sendGuildChannelMsgKeyMap` 对齐群聊/私聊的 keyMap 模式。

### 错误处理与回执修复

**文件：** `handlers/send_private_msg.go`

#### 错误处理

7. **私聊文本路径缺少 40034025/超时重试**：只处理了 `22009`（频控），缺少 event_id 无效重试和超时重试。已补齐。同时修复重试成功后仍无条件 `return nil` 的控制流问题。

#### 回执缺失

8. **`[CQ:input_notify]` `PostC2CMessage` 无回执**：API 调用成功或失败后均未向 OneBot 客户端返回 `SendC2CResponse`，导致插件超时。已修复。

9. **`[CQ:stream]` 多处缺失回执**：`start` 失败/成功但 resp 为空、`mid` 续片、`finish` 失败的 `SendC2CResponse` 均缺失。已统一修复：无论成功还是失败，都返回回执。

10. **`PostC2CStreamMessage` 反序列化错误**：`SetResult(dto.C2CMessageResponse{})` 无法解析 API 响应（该结构体无 json tag），`resp.Message` 始终为 nil，`stream_msg_id` 无法存储。已改为 `SetResult(dto.Message{})` + 手动构造，对齐现有模式。

### /me 命令修复

**文件：** `Processor/Processor.go`、`Processor/processor_test.go`、`config/config.go`

13. **`/me` 命令误报错误**：`HandleFrameworkCommand` 中 `/me` 命令路径包含不必要的 `err != nil` 检查，ID 映射失败时（如 idmaps-pro 模式下无映射）会错误地发送错误信息并退出。`/me` 是状态查询命令，映射失败不应阻断命令执行。已移除多余检查。

14. **新增 `/me` 命令自动化测试**：添加 `Processor/processor_test.go`，覆盖命令匹配、数据提取、前缀配置、边界情况等场景。`config` 包新增测试辅助 setter 函数（`SetMePrefix`、`SetIdmapPro`、`SetStatusPrefix`、`SetBroadcastPrefix`）。

### 其他修复

**文件：** `handlers/message_parser.go`、`handlers/send_guild_channel_msg.go`

15. **纯文本 `[CQ:at]` 用户名缓存失效时原样发送 CQ 码**：`resolvePlainTextAtMentions` 在 `idmap.GetUserName()` 返回空时直接保留原始 `[CQ:at,qq=...]` 文本，导致 QQ API 收到未解析的 CQ 码。已修复：缓存失效时回退为 `<@OpenID>` 格式（QQ API 原生 @ 语法），ID 映射也失败时移除该标记。

11. **`cardPattern` 正则参数顺序依赖**：使用固定位置分组捕获，用户传入不同顺序时静默失败。已改为顺序无关的 `key=value` 提取。

12. **`sendGuildChannelMsgKeyMap` 死代码**：声明了但未在循环中使用。已在 foundItems 循环中添加 keyMap 检查，使其实际生效。

---

## 🚀 新增功能

### [CQ:card] 群聊图文卡片消息

**文件：** `botgo/dto/message_create.go`、`handlers/message_parser.go`、`handlers/send_group_msg.go`

新增 `msg_type=8` 卡片消息支持：
- botgo SDK 新增 `GroupCard` / `GroupCardContent` 结构体，`MessageToCreate` 新增 `Card` 字段
- 字符串格式：`[CQ:card,title=xxx,desc=xxx,pic=xxx,url=xxx]`（参数顺序无关）
- 消息段格式：`{"type":"card","data":{...}}`
- 本地路径 `pic` 自动通过 `oss_type` 配置的图床上传为 CDN 链接
- `pic` 或 `url` 为空时跳过发送（QQ API 要求两者至少传一个）

### [CQ:input_notify] 单聊输入状态通知

**文件：** `botgo/dto/message_create.go`、`handlers/message_parser.go`、`handlers/send_private_msg.go`

新增 `msg_type=6` 输入状态支持：
- botgo SDK 新增 `InputNotify` 结构体，`MessageToCreate` 新增 `InputNotify` 字段
- 字符串格式：`[CQ:input_notify,type=1,second=60]`
- 消息段格式：`{"type":"input_notify","data":{...}}`
- 在正文发送前先发送键入状态通知

### [CQ:stream] 单聊流式消息

**文件：** `botgo/dto/message_create.go`、`botgo/openapi/iface.go`、`botgo/openapi/v2/resource.go`、`botgo/openapi/v2/message.go`、`botgo/openapi/v1/message.go`、`handlers/message_parser.go`、`handlers/send_private_msg.go`

新增流式消息支持（`POST /v2/users/{user_openid}/stream_messages`）：
- botgo SDK 新增 `StreamChunk` 结构体、`PostC2CStreamMessage` 接口及 v2/v1 实现
- 字符串格式：`[CQ:stream,type:start,qq:虚拟ID]`（使用 `:` 分隔参数）
- 三段式生命周期：`start`（首片）→ `mid`（续片，可多次）→ `finish`（终片）
- 内部 `sync.Map` 缓存 `stream_msg_id`，按 `qq` 关联同一用户分片

### 智能分片上传

**文件：** `botgo/dto/message_create.go`、`botgo/openapi/iface.go`、`botgo/openapi/v1/message.go`、`botgo/openapi/v2/message.go`、`botgo/openapi/v2/resource.go`、`handlers/upload_helper.go`（新建）、`handlers/send_group_msg.go`、`handlers/send_private_msg.go`

新增完整分片上传流程：
- 软限制：图片 20MB、视频 30MB、语音 20MB、文件 200MB
- 超过软限制的 base64 数据自动走 `upload_prepare` → `PUT` 预签名 URL → `upload_part_finish` → 合并获取 `file_info`
- 未超过软限制或 URL 直传文件保持原有路径（不变）
- 单聊和群聊隔离上传（`/v2/users/` vs `/v2/groups/`），不冲突独立

---

## 📋 其他

### AGENTS.md 规范更新

- 新增「🌿 大更改 → 提 PR」章节
- 分支名固定为 `Pr-Edit`，内容体现在 commit 与 PR 中
- `foundItems` 表格新增 `card`、`input_notify`、`stream` key
- `msg_type` 陷阱说明补充 `MsgType=6`、`MsgType=8`
- 新增连接模式与 Processor 初始化章节（四种 OneBot 连接方式）
- 新增本地 Fork 依赖（go.mod replace）说明
- 补充 Handler 签名和参数含义
- 重构构建章节为命令表格，补充单测运行方式和产物路径
- 重组陷阱章节为分类子章节（Fork/配置/消息系统）

### 配置模板变更

**文件：** `template/config_template.go`

- `FriendAddEventHandler` 和 `FriendDelEventHandler` 默认启用（取消注释），新安装自动订阅好友添加/删除事件并转发给下游 OneBot 客户端

### 文档更新

- 新建 `docs/cq码/扩展CQ码/扩展cq码-cq-card.md`
- 新建 `docs/cq码/扩展CQ码/扩展cq码-cq-input_notify.md`
- 新建 `docs/cq码/扩展CQ码/扩展cq码-cq-stream.md`
- `Gensokyo语法参考.md`、`扩展CQ码汇总.md`、`本版新增功能.md`、`更多文档.md`、`readme.md` 同步更新

### 提交列表

```
49c3dd7 feat: QQ Bot API v2 适配修复 — CQ 码支持、卡片消息、输入状态、错误处理
ed3a240 docs: 添加大更改提 PR 的分支规范
0da3056 docs: 分支名固定为 Pr-Edit，内容体现在 commit 与 PR 中
e6acf7b docs: 新增 [CQ:card] / [CQ:input_notify] 文档及 CHANGELOG
06f9ec6 docs: 补全 readme 和 更多文档.md 中 card/input_notify 的引用
61e9c63 feat: 智能分片上传 — 文件超软限制时自动切换
584f171 fix: 消息段格式 [CQ:card] 未处理导致静默丢弃
6e5de80 fix: 纯卡片消息（无文本内容）未发送
5232956 fix: 卡片消息 url 为空时 QQ API 拒绝 (40011021)
b2aa5a9 fix: 卡片 pic_url 默认值改为真实图片 URL
84cd337 feat: 卡片 pic_url 支持本地路径自动 OSS 上传
7462bb9 docs: 补充卡片消息 pic 本地路径 OSS 上传说明
0466b81 fix: 消息段格式 [CQ:input_notify] 未处理导致静默丢弃
5f2a06f feat: 新增 [CQ:stream] 流式消息支持
7a7ee61 fix: PostC2CStreamMessage 响应反序列化错误
15b25fd fix: [CQ:input_notify] 后空白文本不发送普通消息
aaa2d94 fix: [CQ:stream] mid 续片缺少回执
17f6324 fix: 所有 stream/input_notify 路径统一补 SendC2CResponse
1df8256 docs: Release010 CHANGELOG 独立
5d88c4c docs: 重写 CHANGELOG_v010 对齐 v009 格式
e7efa02 docs: readme 补充 QQ 机器人官方文档引用
b95be7a5 docs: 改进 AGENTS.md 架构文档与构建指南
1a1d5cd fix: 修复 /me 命令报错问题并添加自动化测试
7983e84 docs: 同步更新 CHANGELOG 和文档反映 /me 修复与测试
```

---

## 🔨 后续追加变更（2026-08-01）

### idmap 迁移重复映射修复

**文件：** `idmap/service.go`、`idmap/new_service.go`、`server/getIDHandler.go`

**问题：** 用户迁移 idmap 后出现 2 个虚拟 ID 对应同一 OpenID 的重复映射，`getid` 的 `type=5`（`UpdateVirtualValue`）无法更新，`newRowValue=0` 也无法解绑。

**根因：**
1. `StartMigration` 内部 `go backgroundMigration()` 非阻塞，迁移与消息接收并行
2. 迁移期间 `storeIdentity` 双写新库 + `backgroundMigration` 迁入并发，`writeBatchToNewDB` 按 key 去重但不按 value 去重
3. `UpdateVirtualValue` 的 `newRowValue=0` 解绑分支只删单条逆向映射，不删正向映射，不扫重复逆向条目

**修复：**
1. **`UpdateVirtualValue` 解绑分支彻底清理**：`newRowValue=0` 时删正向 `uin:<OpenID>` + 扫删所有指向同一 OpenID 的重复逆向条目 `uin:row-*`（新库+旧库）
2. **新增 `ForceUnbindID(openID)`**：按 OpenID 直接定位并清理全部映射，返回清理条数，适合批量清理重复映射
3. **`getIDHandler.go` 新增 `case 18`**：调 `ForceUnbindID`，返回 `{"status":"success","unbound_count":N}`
4. **`StartMigration` 改阻塞式迁移**：去 `go`，迁移完成才返回，确保 `main.go` 调用点之后才连 WS / 启动 HTTP
5. **`writeBatchToNewDB` 按 value 呱重**：逆向条目 `uin:row-*` 迁入前查正向 `uin:<OpenID>` 是否已存在，若已存在则跳过（双保险）

### 图文混合消息 `[CQ:at]` 原文显示

**文件：** `handlers/send_group_msg.go`、`handlers/send_group_msg_raw.go`、`handlers/send_private_msg.go`、`handlers/send_guild_channel_msg.go`

**问题：** 全量群消息下图文混合消息（msg_type=7）时 `[CQ:at,qq=数字]` 未转换，原文显示为 `图片[CQ:at,qq=123456]`。

**修复：** 四个 handler 的图文混合路径构造 `MessageToCreate` 前补 `resolvePlainTextAtMentions(messageText)`，与纯文本路径对齐。

**关联 Issue：** [Te-River/Gensokyo-NewQQ#15](https://github.com/Te-River/Gensokyo-NewQQ/issues/15)

### QQ API 错误码中文提示

**文件：** `handlers/qq_error_codes.go`（新增）、`handlers/send_group_msg.go`、`handlers/send_group_msg_raw.go`、`handlers/send_private_msg.go`、`handlers/send_guild_channel_msg.go`、`handlers/send_private_msg_wakeup.go`

**功能：** QQ API 调用失败时，控制台输出错误码对应的中文描述和排查建议。新增 `qqErrorCodes` 映射表（数据来源：QQ 官方错误码文档）、`ExtractQQErrorCode`、`FormatQQError`，在 5 个 handler 的错误分支非侵入式追加一行提示。

**示例：**
```
发送文本群组信息失败: {"code":22009,"message":"频控"}
[QQ API 错误码 22009] 主动消息频控超限。排查建议：降低发送频率或等待配额恢复
```

### 文档同步

- `docs/cq码/扩展CQ码/扩展cq码-cq-at.md`：新增"图文混合消息（msg_type=7）"章节
- `docs/本版新增功能.md`：出站 @ 补图文混合路径说明；新增"错误码提示"章节；idmap 章节追加强制解绑工具和迁移阻塞式说明
- `docs/Gensokyo语法参考.md`、`docs/cq码/扩展CQ码汇总.md`：`[CQ:at]` 补图文混合转换说明
- `docs/idmap.md`：迁移阻塞式说明、`ForceUnbindID` / type=18 强制解绑说明、`UpdateVirtualValue` 解绑行为变化说明
- `readme.md`：功能亮点新增"QQ API 错误码中文提示"
- 删除错误的 `release_log/CHANGELOG_v011.md`（当前仍在 Release010）

### ForceUnbindID 支持虚拟 ID 入参（2026-08-01 修复）

**文件：** `idmap/service.go`

**问题：** 用户反馈 `getid type=18` 和 `type=5` 都返回 false，且会阻塞消息一会才返回。

**根因：** `ForceUnbindID` 原仅接受 OpenID 入参，但用户实际常传虚拟 ID（row 值），导致 `b.Get([]byte(key))` 查正向 `uin:<虚拟ID>` 找不到直接返回 `unboundCount=0`（表现为返回 false）。"卡一会"的阻塞来自 `identityDB.Update` 里的 `c.Cursor()` 全桶扫描，1200 万条计数器扫一遍很慢。

**修复：**
1. `ForceUnbindID` 支持双形式入参：纯数字视为虚拟 ID，先通过逆向条目 `uin:row-<N>` 反查 OpenID（新库+旧库回退），再按 OpenID 清理；非纯数字视为 OpenID 直接清理
2. 去掉"正向条目存在检查"的提前 `return nil`：即便正向已被其他路径删除，也要扫删残留逆向条目，确保彻底清理重复映射

**验证：** `go build ./...` 编译通过 + `go vet ./idmap/ ./server/` 静态分析通过
