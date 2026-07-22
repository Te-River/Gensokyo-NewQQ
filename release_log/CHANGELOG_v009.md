# Changelog — Release009

> 自 Release008 (`6b38e78`) 以来的所有变更。

---

## 🐛 Bug 修复

### Keyboard 按钮中的本地图片路径未上传图床

**文件：** `handlers/message_parser.go`、`handlers/send_group_msg.go`、`handlers/send_guild_channel_msg.go`、`handlers/send_private_msg.go`

按钮 `render_data.label` 和 `render_data.visited_label` 中的本地图片路径（如 `![text #20px #20px](D:\path\to\cover.png)`）未被图床处理，导致发送到 QQ API 时仍包含本地路径，图片无法显示。

**修复**：新增 `ResolveKeyboardImages` 函数，遍历 keyboard 所有按钮，将 label/visited_label 中的本地图片通过 `resolveMarkdownMediaReferences` 上传到 CDN 并替换为 URL，与 markdown 内容中的图片处理保持一致。三个 send handler（`send_group_msg`、`send_guild_channel_msg`、`send_private_msg`）在发送前均调用此函数。

### GROUP_MESSAGE_CREATE 全量群消息中 @Bot 未剥离（依赖 remove_at 配置）

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

在 Windows 系统上，端口 5700 可能被 Hyper-V 或其他系统服务保留，导致 `http_address` 配置的 HTTP API 服务器启动时出现：

```
http apilisten: listen tcp 127.0.0.1:5700: bind: An attempt was made to access a socket in a way forbidden by its access permissions.
```

此前程序会调用 `log.Fatalf` 直接终止整个进程，即使主 Gin 服务器（配置端口）和 WebSocket 连接已经正常运行。

**修复**：将 `log.Fatalf` 改为 `mylog.Printf`，仅记录错误日志但不终止进程。HTTP API 服务器是可选附加功能，其绑定失败不影响主程序运行。

---

## 📝 文档

- `docs/本版新增功能.md` — 更新 `GroupMessageEventHandler` 描述为"全量群消息（含 @Bot 的，其中 @Bot 始终剥离）"；非自身 @ 节明确 @Bot 不依赖 `remove_at` 配置；新增 Keyboard 按钮本地图片节；新增 HTTP API 绑定失败行为变更说明
- `docs/文档-markdown消息.md` — 新增 Keyboard 按钮中的图片章节
- `AGENTS.md` — `removeAt` 说明补充全量群消息例外
- `release_log/CHANGELOG_v009.md` — 本文档

---

## 📦 文件变更清单

| 文件 | 变更 |
|------|------|
| `handlers/message_parser.go` | 新增 `ResolveKeyboardImages` 函数；`RevertTransformedText` 新增 `isFullGroupMsg` 标记，全量群消息 @Bot 始终剥离；`ConvertToSegmentedMessage` 同步；移除 `safeLocalPath` 基础目录限制 |
| `handlers/send_group_msg.go` | 发送前调用 `ResolveKeyboardImages` |
| `handlers/send_guild_channel_msg.go` | 发送前调用 `ResolveKeyboardImages` |
| `handlers/send_private_msg.go` | 发送前调用 `ResolveKeyboardImages` |
| `main.go` | `GroupMessageEventHandler` 日志区分 @Bot 与普通消息 |
| `Processor/ProcessGroupNormalMessage.go` | 注释更新 |
| `AGENTS.md` | `removeAt` 说明补充全量群消息例外 |
| `docs/本版新增功能.md` | 多处更新 |
| `docs/文档-markdown消息.md` | 新增 Keyboard 按钮图片章节 |
| `release_log/CHANGELOG_v009.md` | 本文档 |

---

## ✅ 提交记录

```
6b38e78 fix: http apilisten 绑定失败时不终止进程
db900d4 docs: 添加 HTTP API 绑定失败行为变更说明
918b4e3 fix: 移除 Markdown 本地图片的基础目录限制
8c244a3 fix: keyboard 按钮中的本地图片路径未上传图床
32818f7 fix: GROUP_MESSAGE_CREATE 全量群消息中 @Bot 始终剥离，不依赖 remove_at 配置
3f80c56 docs: 明确 GROUP_MESSAGE_CREATE 中 @Bot 始终剥离，不依赖 remove_at
8291e4d fix: ConvertToSegmentedMessage 中全量群消息的 @Bot 始终剥离
```