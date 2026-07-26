# Changelog — Release010

> 自 Release009 以来的所有变更。

---

## 修复

### CQ 码处理完整性与对齐

**涉及文件：** `handlers/message_parser.go`、`handlers/send_group_msg.go`、`handlers/send_private_msg.go`、`handlers/send_guild_channel_msg.go`、`handlers/send_private_msg_wakeup.go`

#### [CQ:card] 消息段格式缺失
`[]interface{}` 和 `map[string]interface{}` 两个路径都缺少 `case "card"` 处理，导致通过消息段数组发送卡片时被静默丢弃（日志显示 `Unhandled segment type: card`）。已补充。

#### 纯卡片消息不发送
卡片处理代码位于 `if messageText != ""` 块内部，纯卡片（无文本）时 messageText 为空，不会执行。已在 text 路径外新增独立的纯卡片发送逻辑。

#### 卡片 pic/url 为空时 QQ API 拒绝
QQ API 要求 `tuwen` 类型卡片的 `url` 和 `pic_url` 为有效链接，空值时报 `40011021`。已增加验证：`pic` 或 `url` 为空时跳过卡片发送并记录日志，不设置默认值。

#### 卡片 pic 支持本地路径自动 OSS 上传
`pic_url` 为本地文件路径时，自动通过当前配置的 OSS 图床（`imagehosting.UploadBytes`）上传并替换为 CDN 链接。

#### [CQ:input_notify] 消息段格式缺失
与 card 相同问题，`[]interface{}` 和 `map[string]interface{}` 两个路径缺少 `case "input_notify"` 处理。已补充。

#### [CQ:input_notify] 空白文本被发送
`[CQ:input_notify]` 后只剩空格/空白时，仍以普通文本消息发送。已改为 `strings.TrimSpace` 判断，纯空白不发送。

#### 频道消息不支持扩展 CQ 码
`send_guild_channel_msg.go` 的 `foundItems` 循环跳过了 `markdown`、`qqmusic` 等 key，导致频道消息无法发送这些媒体类型。`GenerateReplyMessage` 实际已支持，只是被跳过列表拦截。已释放 `markdown` 和 `qqmusic`，并添加 `sendGuildChannelMsgKeyMap` 对齐群聊/私聊的 keyMap 模式。

#### keyMap 缺 url_record / url_video
`sendGroupMsgKeyMap`、`sendPrivateMsgKeyMap`、`send_private_msg_wakeup.go` 的 keyMap 都缺少 `url_record` 和 `url_video`，导致 HTTP 语音/视频在 MessageToCreate 路径无法发送。已同步补全。

### 错误处理与回执

#### 私聊文本路径 40034025 / 超时重试缺失
`send_private_msg.go` 文本路径只处理了 `22009`（频控），缺少 `40034025`（event_id 无效）重试和超时重试。已补齐。

#### 重试成功后仍 return nil
`send_private_msg.go` 文本路径重试后仍无条件 `return "", nil`，即使重试成功。已改为仅在重试也失败时 return。

#### stream / input_notify 路径缺失回执
`[CQ:stream]` 的 `mid` 续片、`start` 失败/成功但 resp 为空、`finish` 失败，以及 `[CQ:input_notify]` 的 API 调用，均未向 OneBot 客户端返回 `SendC2CResponse`，导致 nonebot 插件超时。已统一修复：无论 API 成功还是失败，都发送回执。

#### PostC2CStreamMessage 反序列化错误
`SetResult(dto.C2CMessageResponse{})` 无法解析 API 返回的 JSON（该结构体无 json tag），导致 `resp.Message` 始终为 nil，`stream_msg_id` 无法存储。已改为 `SetResult(dto.Message{})` + 手动构造 `C2CMessageResponse`，对齐现有 `PostC2CMessage` 写法。

### 其他修复

#### cardPattern 正则参数顺序依赖
card 参数解析使用固定位置的正则分组，用户传入不同顺序时静默失败。已改为顺序无关的 `key=value` 提取。

#### sendGuildChannelMsgKeyMap 死代码
声明的 keyMap 未在循环中使用，对实际行为无影响。已在 `foundItems` 循环中添加 keyMap 检查，使其实际生效。

---

## 新增功能

### [CQ:card] 群聊图文卡片消息（msg_type=8）

**涉及文件：** `botgo/dto/message_create.go`、`handlers/message_parser.go`、`handlers/send_group_msg.go`

- botgo SDK 新增 `GroupCard` / `GroupCardContent` 结构体，`MessageToCreate` 新增 `Card` 字段
- CQ 码格式：`[CQ:card,title=xxx,desc=xxx,pic=xxx,url=xxx]`（参数顺序无关）
- 消息段格式：`{"type":"card","data":{"title":"...","desc":"...","pic":"...","url":"..."}}`
- 发送时自动切换 `msg_type=8`，QQ 群聊展示图文卡片
- `pic` 支持公网 URL 和本地文件路径（本地路径自动 OSS 上传）

### [CQ:input_notify] 单聊输入状态通知（msg_type=6）

**涉及文件：** `botgo/dto/message_create.go`、`handlers/message_parser.go`、`handlers/send_private_msg.go`

- botgo SDK 新增 `InputNotify` 结构体，`MessageToCreate` 新增 `InputNotify` 字段
- CQ 码格式：`[CQ:input_notify,type=1,second=60]`
- 消息段格式：`{"type":"input_notify","data":{"type":"1","second":"60"}}`
- 在正文发送前先发送输入状态通知，再发正文

### [CQ:stream] 单聊流式消息

**涉及文件：** `botgo/dto/message_create.go`、`botgo/openapi/iface.go`、`botgo/openapi/v2/resource.go`、`botgo/openapi/v2/message.go`、`botgo/openapi/v1/message.go`、`handlers/message_parser.go`、`handlers/send_private_msg.go`

- botgo SDK 新增 `StreamChunk` 结构体、`PostC2CStreamMessage` 接口及 v2/v1 实现
- CQ 码格式：`[CQ:stream,type:start,qq:虚拟ID]`（使用 `:` 分隔参数）
- 三个生命周期：`start`（首片）→ `mid`（续片，可多次）→ `finish`（终片）
- 内部通过 `sync.Map` 缓存 `stream_msg_id`，按 `qq` 关联同一用户的分片
- `start`：`input_state=1, index=0`，返回 `stream_msg_id` 并缓存
- `mid`：`input_state=1, index=N+1`，携带缓存的 `stream_msg_id`
- `finish`：`input_state=10, index=N+1`，发送后清理缓存

### 智能分片上传

**涉及文件：** `botgo/dto/message_create.go`、`botgo/openapi/iface.go`、`botgo/openapi/v1/message.go`、`botgo/openapi/v2/message.go`、`botgo/openapi/v2/resource.go`、`handlers/upload_helper.go`（新建）、`handlers/send_group_msg.go`、`handlers/send_private_msg.go`

- 新增完整的分片上传流程：`upload_prepare` → `PUT` 预签名 URL → `upload_part_finish` → 合并获取 `file_info`
- 软限制检测：图片 20MB、视频 30MB、语音 20MB、文件 200MB
- 自动切换：`base64` 文件数据超过软限制时走分片上传，否则保持 URL 直传
- 支持单聊和群聊两个隔离场景

---

## 文档更新

### AGENTS.md
- 新增「🌿 大更改 → 提 PR」章节
- 分支名固定为 `Pr-Edit`，内容体现在 commit 与 PR 中
- `foundItems` 表格新增 `card`、`input_notify`、`stream` key
- `msg_type` 陷阱说明更新：添加 `MsgType=6`、`MsgType=8`

### docs/
- `Gensokyo语法参考.md`：新增 card / input_notify / stream 三行 CQ 码
- `扩展CQ码汇总.md`：新增 card / input_notify / stream 三行
- 新建 `扩展cq码-cq-card.md`（卡片消息文档）
- 新建 `扩展cq码-cq-input_notify.md`（输入状态文档）
- 新建 `扩展cq码-cq-stream.md`（流式消息文档）
- `本版新增功能.md`：新增 card / input_notify / stream 三节
- `更多文档.md`：扩展 CQ 码索引新增三条链接
- `readme.md`：功能亮点 + CQ 码列表 + 拓展表格均更新

---

## 文件变更统计

```
14 个文件被修改，+190/-27 行（核心功能）
+8 个文档文件新建/修改
= 共约 22 个文件
```
