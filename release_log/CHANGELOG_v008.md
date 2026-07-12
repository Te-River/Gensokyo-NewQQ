# Changelog — Release008

> 自 Release007 (`a44882c`) 以来的所有变更。

---

## 🚀 新增功能

### CQ:file 文件上传支持

QQ 机器人 API v2 现已开放 `file_type=4`（文件）的富媒体上传，可通过 `POST /v2/users/{openid}/files` 和 `POST /v2/groups/{group_openid}/files` 上传任意文件。

Gensokyo 新增 `[CQ:file]` CQ 码的完整支持：

- `[CQ:file,file=file:///path/to/file]` — 本地文件路径（自动 base64 编码后走 CDN 上传）
- `[CQ:file,file=http://example.com/file.zip]` — HTTP 远程文件链接
- `[CQ:file,file=https://example.com/file.zip]` — HTTPS 远程文件链接
- `[CQ:file,file=base64://<data>]` — base64 编码数据（走 CDN 上传）

支持可选参数 `file_name` 指定文件名，预留给未来 API 开放使用：
- `[CQ:file,file=file:///path/to/file,file_name=myfile.txt]`
- 数组段格式：`{"type":"file","data":{"file":"file:///path","file_name":"myfile.txt"}}`
- 不填时自动从路径/URL 末尾提取（`filepath.Base()`）

支持场景：
- 群聊发送文件（`send_group_msg`）
- 私聊发送文件（`send_private_msg`）
- 私聊互动召回消息（`send_private_msg_wakeup`）

> ⚠️ **已知限制：** QQ Bot API 的 `/files` 接口不支持自定义文件名。使用 `file_data`（base64）上传的文件在 QQ 中始终显示为"未命名"。如需要正确文件名，需自行先将文件托管到公开 URL，然后通过 `[CQ:file,file=http://...]` 方式发送（QQ 会从 URL 末尾提取文件名）。

### send_private_msg_wakeup API

新增 `send_private_msg_wakeup` API，用于向 QQ 用户发送 C2C 互动召回（唤醒）消息。OneBot 应用端可通过此接口主动唤醒用户会话，不受被动回复上下文限制。

---

## 🔧 改进

### send_private_msg_wakeup 处理优化

- 添加 `active` / `active_type` / `active_sub_type` key 遍历跳过逻辑，避免 `[CQ:active]` 内容被错误地作为媒体消息发送
- 纯 `[CQ:active]` 无实际内容时发送空白唤醒请求，确保用户收到互动通知
- 调用方获得真实的成功/失败返回（同步模式），不再因异步处理导致 WebSocket 超时

---

## 🐛 Bug 修复

### 语音 URL 未正确重命名引起的潜在 panic

**文件：** `handlers/send_group_msg.go`

`url_record` 分支中 `generateGroupMessage` 使用了外层作用域的 `imageURLs` 变量而非 `recordURLs`。编译通过但在实际语音发送路径下会因索引越界 panic。已修正为 `recordURLs[0]`。

### send_private_msg_wakeup 遍历 active key 时误发媒体

`foundItems` 遍历时未跳过 `"active"` key，导致 `[CQ:active]` 标记被当作媒体类型发送，产生 `"Expected RichMediaMessage type for key active"` 错误。已添加 key 过滤。

### NoneBot 插件 msg_text 截断

**文件：** `active_msg/__init__.py`

`handle_wakeup` 中 `on_command("唤醒")` 已将命令前缀剥离，但代码中仍使用 `text[len("唤醒"):].strip()` 再次裁剪，导致 `"唤醒 @target 123"` 的实际消息变为 `"3"`。已修复为直接使用 `text`。

### 富媒体消息 FileType 注释更新

`botgo/dto/message_create.go` 中 `RichMediaMessage.FileType` 注释新增 `4 文件` 类型。

### 文件消息段未处理导致静默丢弃

**文件：** `handlers/message_parser.go`

NoneBot 以 koishi 数组段格式 `{"type":"file","data":{"file":"file:///..."}}` 发送文件消息时，`parseMessageContent` 的 `switch segmentType` 中没有 `case "file":`，日志打印 `Unhandled segment type: file`，文件被静默丢弃。已在 koishi 和 TRSS 两种消息格式中均添加 `case "file":` 处理。

### 本地文件路径 URL 编码未解码

**文件：** `handlers/message_parser.go`

`file:///` 路径中的中文等字符经 URL 编码（如 `%E7%A5%9E` → `神`），去掉 `file:///` 前缀后路径仍为编码状态，`os.ReadFile` 找不到文件。已在两处 `case "file":` 中添加 `neturl.PathUnescape()` 解码。

### foundItems 遍历缺少文件类型 key

**文件：** `handlers/send_group_msg.go`、`handlers/send_private_msg.go`

`local_file` / `base64_file` 经 `generateGroupMessage` 上传 CDN 后返回 `MessageToCreate`，但遍历 `foundItems` 时 `keyMap` 中没有这些 key，导致上传成功的文件不会被发送。已添加 `local_file`、`url_file`、`url_files`、`base64_file` 到 `keyMap`。

---

## 📦 文件变更清单

| 文件 | 变更 |
|------|------|
| `botgo/dto/message_create.go` | FileType 注释新增 `4 文件` 类型 |
| `handlers/message_parser.go` | 新增 CQ:file 正则解析 + foundItems key 映射 + 数组段 `case "file"` + URL 解码 |
| `handlers/send_group_msg.go` | `generateGroupMessage`/`generatePrivateMessage` 文件处理分支 + keyMap 补充 + 文件名传递 |
| `handlers/send_private_msg.go` | keyMap 补充文件类型 + RichMediaMessage 上传后文件名透传 |
| `handlers/send_private_msg_wakeup.go` | 同步模式改造 + active key 跳过 + 空内容兜底 |
