# Changelog — Release009

> 自 Release008 (`74d405f`) 以来的所有变更。

---

## 🐛 Bug 修复

### CQ 码处理逻辑修复与 API 规范对齐

**文件：** `handlers/send_group_msg.go`、`handlers/send_private_msg.go`、`handlers/send_guild_channel_msg.go`、`handlers/send_private_msg_wakeup.go`、`handlers/send_private_msg_sse.go`

#### API 字段规范违反修复

1. **`MsgType=2` 时 `Content` 字段违反 API 规范**：QQ API 明确要求"传了 markdown 后此字段必须为空"，但代码中 7 处（`generateGroupMessage`/`generatePrivateMessage` 的 markdown 和 qqmusic 路径、combo transmd 路径）设置的 `Content: "markdown"` 字符串。已全部改为 `Content: ""`。

2. **`msg_id` 与 `event_id` 同时设置违反"二选一"**：API 文档明确 `event_id` 与 `msg_id` 互斥，但代码在 `[CQ:reply]` 设置 `MsgID` 后未清空已有的 `EventID`。已在所有 handler（group/private/channel）的 reply 处理块中，设置 `MsgID` 后增加 `groupMessage.EventID = ""`。

#### 安全修复

3. **`url_video`/`url_videos` 缺少 SSRF 防护**：`url_image`/`url_images`/`url_record`/`url_records` 均有 `isPrivateOrLoopback` 检查，但 `url_video`/`url_videos`（http/https 各 2 处，共 4 处）没有。已按同样模式添加 SSRF 检查。

#### 功能修复

4. **`auto_md` nil 指针解引用风险**：combo 路径（line 320-328）中当 `groupReply` 断言为 `*dto.MessageToCreate` 时 `richMediaMessage` 为 nil，但 `auto_md` 仍被无条件调用，内部直接访问 `richMediaMessage.URL` 会 panic。已增加 `&& richMediaMessage != nil` 条件保护。

5. **群聊图文混合（combo）路径缺失 `[CQ:reply]` 处理**：`send_private_msg.go` 的图文混合路径有 reply 处理，但 `send_group_msg.go` 的 combo 路径（msg_type=7 和 transmd→msg_type=2）完全没有。已补齐。

6. **富媒体消息（纯媒体无文本）缺失 `[CQ:reply]` 处理**：群聊和私聊的 foundItems 循环中，富媒体消息（图片/视频/语音/文件）上传后构造 msg_type=7 消息时没有设置 `MessageReference`。已补齐。

7. **Wakeup handler 图文混合检查缺少 `url_images`**：`local_image`/`url_image`/`base64_image` 都有检查，但 `url_images`（https 图片）遗漏。已补充。

8. **频道纯文本路径缺失 `[CQ:at]` 处理**：频道 handler 的纯文本路径没有调用 `resolvePlainTextAtMentions`，导致 `[CQ:at,qq=xxx]` 原样发送。已在 markdown 处理分支后添加 `else` 分支处理。

#### 错误处理修复

9. **私聊 22009（主动消息频控）错误处理缺失**：群聊有 `PushGlobalStack` 补发机制，但私聊纯文本路径直接 `return ""`、KeyMap 路径只有 `TODO`、RichMediaMessage 路径完全没有检查。已统一添加 22009 日志落盘。

10. **`fmt.Printf` 替换为 `mylog.Printf`**：`send_private_msg_sse.go` 中 2 处使用了标准库 `fmt.Printf` 而非项目统一的 `mylog.Printf`。已替换。

#### 代码优化

11. **keyMap 从循环内提取为包级变量**：`send_group_msg.go` 和 `send_private_msg.go` 的 foundItems 遍历循环中，每次迭代都创建一个新的 map 字面量。已提取为包级变量 `sendGroupMsgKeyMap`/`sendPrivateMsgKeyMap`。

### GROUP_MESSAGE_CREATE 中 `bot:true` 的提及也注册到 selfAtIDs

**文件：** `Processor/ProcessGroupNormalMessage.go`

QQ 开放平台 `GROUP_MESSAGE_CREATE`（全量群消息）的 `Mentions` 数组中，`is_you` 字段在某些场景（如多实例）下可能不准确。此前仅当 `mention.IsYou` 为 true 时才注册 bot 自身 OpenID 到 `selfAtIDs`，导致 `is_you` 缺失时 `RevertTransformedText` 无法识别 @Bot。

**修复**：遍历 `Mentions` 时条件改为 `mention.IsYou || mention.Bot`，将 `bot:true` 的 OpenID 也注册到 `selfAtIDs`，确保下游能正确识别并转换为 `[CQ:at,qq=AppID]`。`toMe` 标记仍只由 `IsYou` 决定。

### 仅含 `@bot` 的群消息被误判为黑白名单拦截

**文件：** `Processor/ProcessGroupNormalMessage.go`、`handlers/message_parser.go`

上一项修复（`0b73926`）在注册 bot OpenID 到 `selfAtIDs` 的同时，**额外用正则把 `<@OpenID>` 从 `data.Content` 中剥离**。当用户在群里只发送 `@bot` 而无其他文字时，content 剥离后只剩空格，`TrimSpace` 后变 `""`，被空内容检查误判为"被自定义黑白名单拦截"而丢弃——即使**未配置任何黑白名单**。

**修复：**

1. 移除 `ProcessGroupNormalMessage` 中的前置正则剥离，@ 格式转换统一交由 `RevertTransformedText` 处理，与 `GROUP_AT_MESSAGE_CREATE` 处理器保持一致。这样仅含 `@bot` 的消息仍能转换为 `[CQ:at,qq=...]`，产生非空 messageText 正常上报。
2. `resolveIncomingAtID` 中自身 @ 的返回值现在根据 `use_uin` 选择 UIN 或 AppID，与消息 `SelfID` 字段保持一致，避免下游因 `[CQ:at]` 的 qq 与 `self_id` 不匹配而无法识别 `@` 的是自己。

### Keyboard 按钮中的本地图片路径未上传图床

**文件：** `handlers/message_parser.go`、`handlers/send_group_msg.go`、`handlers/send_guild_channel_msg.go`、`handlers/send_private_msg.go`

按钮 `render_data.label` 和 `render_data.visited_label` 中的本地图片路径（如 `![text #20px #20px](D:\path\to\cover.png)`）未被图床处理，导致发送到 QQ API 时仍包含本地路径，图片无法显示。

**修复**：新增 `ResolveKeyboardImages` 函数，遍历 keyboard 所有按钮，将 label/visited_label 中的本地图片通过 `resolveMarkdownMediaReferences` 上传到 CDN 并替换为 URL，与 markdown 内容中的图片处理保持一致。三个 send handler（`send_group_msg`、`send_guild_channel_msg`、`send_private_msg`）在发送前均调用此函数。

### GROUP_MESSAGE_CREATE 全量群消息中 @Bot 剥离改为不依赖 remove_at 配置

**文件：** `handlers/message_parser.go`、`main.go`、`Processor/ProcessGroupNormalMessage.go`

QQ 开放平台的 `GROUP_MESSAGE_CREATE`（全量群消息）事件包含群内所有消息，Content 中可能带有 `<@OpenID>` 原文。此前 @Bot 的剥离依赖 `remove_at` 配置，但全量群消息中的 @ 只是原文一部分，而非 `GROUP_AT_MESSAGE_CREATE` 的专门 @ 事件，本应始终剥离。

**修复：**

1. **`RevertTransformedText`** — 新增 `isFullGroupMsg` 标记，来自 `WSGroupMessageData` 时设为 true。@Bot 处理逻辑改为 `isFullGroupMsg || config.GetRemoveAt()`，全量群消息始终剥离，不依赖 `remove_at`。配套的空格清理同步适用。

2. **`ConvertToSegmentedMessage`**（消息段模式 `arrayValue: true`）— 同样独立处理 @ 提及，之前只检查了 `config.GetRemoveAt()`，同步添加 `isFullGroupMsg` 标记，与 `RevertTransformedText` 保持一致。

3. **`GroupMessageEventHandler` 日志** — 改为检查 `data.Mentions` 中是否有 `is_you`/`bot` 为 true 的条目，含 @Bot 时打印 `"收到群消息"`，不含时打印 `"收到非@群消息"`，消除误导。

### 移除 Markdown 本地图片的基础目录限制

**文件：** `handlers/message_parser.go`

移除 `safeLocalPath` 中的 `baseDir` 前缀检查，允许从任意本地路径加载 Markdown 中的图片/媒体文件，仅保留 `..` 路径穿越防护。

### HTTP API 端口绑定失败不再终止进程

**文件：** `main.go`

在 Windows 系统上，端口 5700 可能被 Hyper-V 或其他系统服务保留，导致 `http_address` 配置的 HTTP API 服务器启动时出现：

```
http apilisten: listen tcp 127.0.0.1:5700: bind: An attempt was made to access a socket in a way forbidden by its access permissions.
```

此前程序会调用 `log.Fatalf` 直接终止整个进程，即使主 Gin 服务器（配置端口）和 WebSocket 连接已经正常运行。

**修复**：将 `log.Fatalf` 改为 `mylog.Printf`，仅记录错误日志但不终止进程。HTTP API 服务器是可选附加功能，其绑定失败不影响主程序运行。

---

## 📝 文档

- `docs/本版新增功能.md` — 更新 `GroupMessageEventHandler` 描述为"全量群消息（含 @Bot 的，其中 @Bot 始终剥离）"；非自身 @ 节明确 @Bot 不依赖 `remove_at` 配置；新增出站 @ 纯文本/Markdown 双行为说明；新增 Keyboard 按钮本地图片节；新增 HTTP API 绑定失败行为变更说明
- `docs/文档-markdown消息.md` — 新增 Keyboard 按钮中的图片章节
- `docs/api/api介绍.md` — 同步出站 @ 行为变更（纯文本转 `@用户名`、Markdown 整个 messageText 合并到头部）
- `docs/cq码/扩展CQ码/扩展cq码-cq-at.md` — 标题由"Markdown 专用"改为通用，新增纯文本 `@用户名` 行为说明
- `docs/Gensokyo语法参考.md`、`docs/cq码/标准CQ码/标准CQ码汇总.md` — @ 行为描述同步
- `readme.md` — 小幅修订
- `AGENTS.md` — `removeAt` 说明补充全量群消息例外
- `release_log/CHANGELOG_v008.md` — 移除已删除的 `is_private` 字段记录；移除不属于本项目的 NoneBot 插件条目；修复提交记录缩进不一致
- `release_log/CHANGELOG_v009.md` — 本文档

---

## 🔧 杂项

- `.gitignore` — 新增忽略 AI 编辑器与助手工具本地目录（`.atomcode`、`.zcode` 等）；修正一处笔误
- 从版本库移除误提交的 `.atomcode/` 目录

---

## 📦 文件变更清单

| 文件 | 变更 |
|------|------|
| `Processor/ProcessGroupNormalMessage.go` | `Mentions` 遍历条件增加 `bot:true`；移除前置正则剥离 `<@OpenID>`，统一交由 `RevertTransformedText`；注释更新 |
| `handlers/message_parser.go` | `resolveIncomingAtID` 自身 @ 返回值按 `use_uin` 选择；新增 `ResolveKeyboardImages`；`RevertTransformedText`/`ConvertToSegmentedMessage` 新增 `isFullGroupMsg` 标记；移除 `safeLocalPath` 基础目录限制 |
| `handlers/send_group_msg.go` | 发送前调用 `ResolveKeyboardImages` |
| `handlers/send_guild_channel_msg.go` | 发送前调用 `ResolveKeyboardImages` |
| `handlers/send_private_msg.go` | 发送前调用 `ResolveKeyboardImages` |
| `handlers/send_private_msg_wakeup.go` | 补充 `url_images` 图文混合检查 |
| `handlers/send_private_msg_sse.go` | `fmt.Printf` → `mylog.Printf` |
| `main.go` | HTTP API 绑定失败 `log.Fatalf` → `mylog.Printf`；`GroupMessageEventHandler` 日志区分 @Bot 与普通消息 |
| `AGENTS.md` | `removeAt` 说明补充全量群消息例外 |
| `docs/本版新增功能.md` | 多处更新 |
| `docs/文档-markdown消息.md` | 新增 Keyboard 按钮图片章节 |
| `docs/api/api介绍.md` | 出站 @ 行为同步 |
| `docs/cq码/扩展CQ码/扩展cq码-cq-at.md` | 新增纯文本 @ 行为说明 |
| `docs/Gensokyo语法参考.md` | @ 行更新 |
| `docs/cq码/标准CQ码/标准CQ码汇总.md` | @ 行更新 |
| `readme.md` | 小幅修订 |
| `.gitignore` | 忽略 AI 工具目录、修正笔误 |
| `release_log/CHANGELOG_v008.md` | 移除失效条目、修复缩进 |
| `release_log/CHANGELOG_v009.md` | 本文档 |

---

## ✅ 提交记录

```
9d6ca70 docs: 更新文档，同步出站 @ 行为变更
1efac59 style: 修复 CHANGELOG_v008 提交记录缩进不一致
6b1aaf7 docs: 移除 CHANGELOG_v008 中不属于本项目的 NoneBot 插件条目
0b73926 fix: GROUP_MESSAGE_CREATE 中 bot:true 的提及也注册到 selfAtIDs
fdb8faf fix: @bot 单独提及不再被误判为黑白名单拦截
9a3d4fe chore: 忽略 AI 编辑器与助手工具本地目录
2e12db2 chore: 从版本库移除 .atomcode，修正 .gitignore 笔误
86aff80 Update readme.md
3578fda docs: 从CHANGELOG_v008移除已删除的is_private字段记录
6b38e78 fix: http apilisten 绑定失败时不终止进程
db900d4 docs: 添加 HTTP API 绑定失败行为变更说明
918b4e3 fix: 移除 Markdown 本地图片的基础目录限制
8c244a3 fix: keyboard 按钮中的本地图片路径未上传图床
32818f7 fix: GROUP_MESSAGE_CREATE 全量群消息中 @Bot 始终剥离，不依赖 remove_at 配置
3f80c56 docs: 明确 GROUP_MESSAGE_CREATE 中 @Bot 始终剥离，不依赖 remove_at
8291e4d fix: ConvertToSegmentedMessage 中全量群消息的 @Bot 始终剥离
81b4078 docs: 更新 CHANGELOG_v009 补充 release008 以来的所有变更
```
