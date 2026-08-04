# Changelog — Release010

> 自 Release009 (`9d6ca70`) 以来的所有变更。

---

## 🚀 新增功能

### [CQ:card] 群聊图文卡片消息

**文件：** `botgo/dto/message_create.go`、`handlers/message_parser.go`、`handlers/send_group_msg.go`

新增 `msg_type=8` 卡片消息支持：

- botgo SDK 新增 `GroupCard` / `GroupCardContent` 结构体，`MessageToCreate` 新增 `Card` 字段
- 字符串格式：`[CQ:card,title=xxx,desc=xxx,pic=xxx,url=xxx]`（参数顺序无关，`cardPattern` 改为顺序无关的 `key=value` 提取）
- 消息段格式：`{"type":"card","data":{...}}`
- 本地路径 `pic` 自动通过 `oss_type` 配置的图床上传为 CDN 链接
- `pic` 或 `url` 为空时跳过发送（QQ API 要求两者至少传一个）
- `pic_url` 默认值改为真实图片 URL，为空时 QQ API 拒绝 (40011021)

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
- 字符串格式：`[CQ:stream,type=start,qq:虚拟ID]`（使用 `:` 分隔参数）
- 三段式生命周期：`start`（首片）→ `mid`（续片，可多次）→ `finish`（终片）
- 内部 `sync.Map` 缓存 `stream_msg_id`，按 `qq` 关联同一用户分片

### 智能分片上传

**文件：** `botgo/dto/message_create.go`、`botgo/openapi/iface.go`、`botgo/openapi/v1/message.go`、`botgo/openapi/v2/message.go`、`botgo/openapi/v2/resource.go`、`handlers/upload_helper.go`（新建）、`handlers/send_group_msg.go`、`handlers/send_private_msg.go`

新增完整分片上传流程：

- 软限制：图片 20MB、视频 30MB、语音 20MB、文件 200MB
- 超过软限制的 base64 数据自动走 `upload_prepare` → `PUT` 预签名 URL → `upload_part_finish` → 合并获取 `file_info`
- 未超过软限制或 URL 直传文件保持原有路径（不变）
- 单聊和群聊隔离上传（`/v2/users/` vs `/v2/groups/`），不冲突独立

### QQ API 错误码中文提示

**文件：** `handlers/qq_error_codes.go`（新增）、`handlers/send_group_msg.go`、`handlers/send_group_msg_raw.go`、`handlers/send_private_msg.go`、`handlers/send_guild_channel_msg.go`、`handlers/send_private_msg_wakeup.go`

QQ API 调用失败时，控制台输出错误码对应的中文描述和排查建议。新增 `qqErrorCodes` 映射表（数据来源：QQ 官方错误码文档）、`ExtractQQErrorCode`、`FormatQQError`，在 5 个 handler 的错误分支非侵入式追加一行提示。

**示例：**

```
发送文本群组信息失败: {"code":22009,"message":"频控"}
[QQ API 错误码 22009] 主动消息频控超限。排查建议：降低发送频率或等待配额恢复
```

### idmap 强制解绑工具

**文件：** `idmap/service.go`、`server/getIDHandler.go`

迁移异常或历史数据残留导致 2 个虚拟 ID 指向同一 OpenID 时，可用强制解绑工具按 OpenID 直接清理全部映射：

- **`ForceUnbindID(id)`**：Go 内部函数，入参支持 **OpenID 字符串**或**虚拟 ID** 两种形式。OpenID 时直接删正向 + 扫删所有指向同一 OpenID 的逆向条目；虚拟 ID 时先通过逆向条目 `uin:row-<N>` 反查 OpenID（新库+旧库回退），再按 OpenID 清理。返回清理条数
- **`getid type=18`**：HTTP API `GET /getid?type=18&id=<OpenID或虚拟ID>`，返回 `{"status":"success","unbound_count":N}`
- **`UpdateVirtualValue(old, 0)` 解绑行为修复**：`newRowValue=0` 解绑时已同步彻底清理正向 + 重复逆向条目（此前只删单条逆向不删正向，导致解绑失效）

解绑后该 OpenID 彻底无映射，下次 `storeIdentity` 会重新分配唯一虚拟 ID。详见 [idmap 文档](../docs/idmap.md)。

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

5. **`url_record` / `url_video` 未加入 keyMap**：`sendGroupMsgKeyMap`、`sendPrivateMsgKeyMap`、`send_private_msg_wakeup` 的 keyMap 均缺少这两个 key，导致 HTTP 语音/视频在 MessageToCreate 路径无法发送。已同步补全。

#### channel 扩展 CQ 码支持

6. **频道消息不支持 markdown/qqmusic**：`send_guild_channel_msg.go` 的 foundItems 循环跳过了 `markdown` 和 `qqmusic`，但 `GenerateReplyMessage` 实际已支持这两个类型。已释放并新增 `sendGuildChannelMsgKeyMap` 对齐群聊/私聊的 keyMap 模式。

#### 正则参数顺序依赖

7. **`cardPattern` 正则参数顺序依赖**：使用固定位置分组捕获，用户传入不同顺序时静默失败。已改为顺序无关的 `key=value` 提取。

### 错误处理与回执修复

**文件：** `handlers/send_private_msg.go`、`handlers/send_private_msg_sse.go`

#### 错误处理

8. **私聊文本路径缺少 40034025/超时重试**：只处理了 `22009`（频控），缺少 event_id 无效重试和超时重试。已补齐。同时修复重试成功后仍无条件 `return nil` 的控制流问题。

#### 回执缺失

9. **`[CQ:input_notify]` `PostC2CMessage` 无回执**：API 调用成功或失败后均未向 OneBot 客户端返回 `SendC2CResponse`，导致插件超时。已修复。
10. **`[CQ:stream]` 多处缺失回执**：`start` 失败/成功但 resp 为空、`mid` 续片、`finish` 失败的 `SendC2CResponse` 均缺失。已统一修复：无论成功还是失败，都返回回执。
11. **`PostC2CStreamMessage` 反序列化错误**：`SetResult(dto.C2CMessageResponse{})` 无法解析 API 响应（该结构体无 json tag），`resp.Message` 始终为 nil，`stream_msg_id` 无法存储。已改为 `SetResult(dto.Message{})` + 手动构造，对齐现有模式。

### /me 命令修复

**文件：** `Processor/Processor.go`、`Processor/processor_test.go`、`config/config.go`

12. **`/me` 命令误报错误**：`HandleFrameworkCommand` 中 `/me` 命令路径包含不必要的 `err != nil` 检查，ID 映射失败时（如 idmaps-pro 模式下无映射）会错误地发送错误信息并退出。`/me` 是状态查询命令，映射失败不应阻断命令执行。已移除多余检查。
13. **新增 `/me` 命令自动化测试**：添加 `Processor/processor_test.go`，覆盖命令匹配、数据提取、前缀配置、边界情况等场景。`config` 包新增测试辅助 setter 函数（`SetMePrefix`、`SetIdmapPro`、`SetStatusPrefix`、`SetBroadcastPrefix`）。

### 纯文本 `[CQ:at]` 用户名缓存失效

**文件：** `handlers/message_parser.go`

14. **纯文本 `[CQ:at]` 用户名缓存失效时原样发送 CQ 码**：`resolvePlainTextAtMentions` 在 `idmap.GetUserName()` 返回空时直接保留原始 `[CQ:at,qq=...]` 文本，导致 QQ API 收到未解析的 CQ 码。已修复：缓存失效时回退为 `<@OpenID>` 格式（QQ API 原生 @ 语法），ID 映射也失败时移除该标记。

### 图文混合消息 `[CQ:at]` 原文显示（Issue #15）

**文件：** `handlers/send_group_msg.go`、`handlers/send_group_msg_raw.go`、`handlers/send_private_msg.go`、`handlers/send_guild_channel_msg.go`

15. **图文混合消息（msg_type=7）`[CQ:at]` 未转换**：全量群消息下图文混合消息时 `[CQ:at,qq=数字]` 未转换，原文显示为 `图片[CQ:at,qq=123456]`。

**根因：** 四个 handler 的图文混合路径（`!transmd` 分支，MsgType=7）构造 `MessageToCreate` 前未调用 `resolvePlainTextAtMentions`，与纯文本路径不一致。

**修复：** 四个 handler 的图文混合路径构造 `MessageToCreate` 前补 `resolvePlainTextAtMentions(messageText)`，与纯文本路径对齐。

**关联 Issue：** [Te-River/Gensokyo-NewQQ#15](https://github.com/Te-River/Gensokyo-NewQQ/issues/15)

### 图文混合走 Markdown 路径（auto_md）未转换 `[CQ:at]`

**文件：** `handlers/send_group_msg.go`（`auto_md` 函数）

16. **图文混合走 Markdown 路径时 `[CQ:at]` 未转换**：图文混合消息走 `transmd=true` 的 Markdown 路径时，`[CQ:at,qq=数字]` 未被转换，QQ 官方 Markdown 渲染把它当纯文本显示，变形为 `[CO:at,qq=数字]`。

**根因：** `auto_md()` 把含 `[CQ:at]` 的 `messageText` 塞进 Markdown 参数（`text_end` 或原生 `content`），从未调用 `ResolveMarkdownAtMentions`。此前修复只覆盖了 `!transmd` 分支（MsgType=7），漏了 `transmd=true` 的 Markdown 分支（MsgType=2）。

**修复：** 在 `auto_md()` 内 `messageText` 塞进 Markdown 前调 `ResolveMarkdownAtMentions(messageText)`，将 `[CQ:at,qq=数字]` 转为 `<qqbot-at-user id="OpenID" />` 标签。修在 `auto_md` 内部一处即可覆盖三个共用 handler（`send_group_msg`、`send_group_msg_raw`、`send_guild_channel_msg`）。

### idmap 迁移重复映射修复

**文件：** `idmap/service.go`、`idmap/new_service.go`、`server/getIDHandler.go`

17. **idmap 迁移后出现 2 个虚拟 ID 对应同一 OpenID 的重复映射**：用户迁移 idmap 后，`getid` 的 `type=5`（`UpdateVirtualValue`）无法更新，`newRowValue=0` 也无法解绑。

**根因：**

1. `StartMigration` 内部 `go backgroundMigration()` 非阻塞，迁移与消息接收并行
2. 迁移期间 `storeIdentity` 双写新库 + `backgroundMigration` 迁入并发，`writeBatchToNewDB` 按 key 去重但不按 value 去重
3. `UpdateVirtualValue` 的 `newRowValue=0` 解绑分支只删单条逆向映射，不删正向映射，不扫重复逆向条目

**修复：**

1. **`UpdateVirtualValue` 解绑分支彻底清理**：`newRowValue=0` 时删正向 `uin:<OpenID>` + 扫删所有指向同一 OpenID 的重复逆向条目 `uin:row-*`（新库+旧库）
2. **新增 `ForceUnbindID(openID)`**：按 OpenID 直接定位并清理全部映射，返回清理条数，适合批量清理重复映射
3. **`getIDHandler.go` 新增 `case 18`**：调 `ForceUnbindID`，返回 `{"status":"success","unbound_count":N}`
4. **`StartMigration` 改阻塞式迁移**：去 `go`，迁移完成才返回，确保 `main.go` 调用点之后才连 WS / 启动 HTTP
5. **`writeBatchToNewDB` 按 value 去重**：逆向条目 `uin:row-*` 迁入前查正向 `uin:<OpenID>` 是否已存在，若已存在则跳过（双保险）

### ForceUnbindID 支持虚拟 ID 入参

**文件：** `idmap/service.go`

18. **`getid type=18` 和 `type=5` 都返回 false**：用户反馈两个类型都返回 false，且会阻塞消息一会才返回。

**根因：** `ForceUnbindID` 原仅接受 OpenID 入参，但用户实际常传虚拟 ID（row 值），导致 `b.Get([]byte(key))` 查正向 `uin:<虚拟ID>` 找不到直接返回 `unboundCount=0`（表现为返回 false）。"卡一会"的阻塞来自 `identityDB.Update` 里的 `c.Cursor()` 全桶扫描，1200 万条计数器扫一遍很慢。

**修复：**

1. `ForceUnbindID` 支持双形式入参：纯数字视为虚拟 ID，先通过逆向条目 `uin:row-<N>` 反查 OpenID（新库+旧库回退），再按 OpenID 清理；非纯数字视为 OpenID 直接清理
2. 去掉"正向条目存在检查"的提前 `return nil`：即便正向已被其他路径删除，也要扫删残留逆向条目，确保彻底清理重复映射

### lazy_message_id 多段回复偶发 40054005 msgseq 去重

**文件：** `echo/messageidmap.go`

19. **`lazy_message_id=true` 下多段回复偶发 `40054005`**：一条命令触发多段独立回复时，偶发某一段发送失败，QQ API 返回 `{"message":"消息被去重，请检查请求msgseq","code":40054005}`。不限于单一事件类型，`GROUP_AT_MESSAGE_CREATE` 和 `GROUP_MESSAGE_CREATE` 场景均出现过。

**根因：** `GetLazyMessagesId` / `GetLazyMessagesIdv2` 选中一条 record 后执行 `usageCount++`，导致同一条命令的多段回复每次调用都选中**不同的 record**（下次该条 `usageCount!=0`，选中另一条），两段拿到不同 `msg_id`。但 QQ API 视两 `msg_id` 为同一回复链（都回复同一 event），要求 `msg_seq` 在同一回复链内连续递增。`GetMappingSeq` 按 `msg_id` 字符串做 key 存 seq，两段拿到不同 `msg_id` 时第二段查不到第一段的 seq，seq 与第一段冲突，判去重 `40054005`。

**修复：** 移除 `GetLazyMessagesId` / `GetLazyMessagesIdv2` 选中后的 `usageCount++`，让同一回复链的多段回复都拿到同一 `msg_id`，配合 `GetMappingSeq` / `AddMappingSeq` 在该 `msg_id` 上连续递增 `msg_seq`。

**关联 Issue：** [Te-River/Gensokyo-NewQQ#19](https://github.com/Te-River/Gensokyo-NewQQ/issues/19)

**验证：** `go build ./...` 编译通过 + `go vet ./echo/` 静态分析通过

---

## 🔧 改进

### 默认启用 FriendAdd/FriendDel 事件转发

**文件：** `template/config_template.go`

`FriendAddEventHandler` 和 `FriendDelEventHandler` 默认启用（取消注释），新安装自动订阅好友添加/删除事件并转发给下游 OneBot 客户端。

### message_reference IgnoreGetMessageError 改为 false

**文件：** `handlers/send_group_msg.go`

`message_reference.IgnoreGetMessageError` 改为 `false`，对齐 QQ API 规范。

---

## 📝 文档

- **新建** `docs/cq码/扩展CQ码/扩展cq码-cq-card.md`、`docs/cq码/扩展CQ码/扩展cq码-cq-input_notify.md`、`docs/cq码/扩展CQ码/扩展cq码-cq-stream.md`
- `docs/cq码/扩展CQ码/扩展cq码-cq-at.md` — 新增"图文混合消息（msg_type=7）"章节、"图文混合消息走 Markdown 路径（auto_md，msg_type=2）"章节
- `docs/本版新增功能.md` — 出站 @ 补图文混合路径说明 + Markdown 路径说明；新增"错误码提示"章节；idmap 章节追加强制解绑工具和迁移阻塞式说明
- `docs/Gensokyo语法参考.md`、`docs/cq码/扩展CQ码汇总.md` — `[CQ:at]` 补图文混合转换说明 + Markdown 路径转换说明
- `docs/idmap.md` — 迁移阻塞式说明、`ForceUnbindID` / type=18 强制解绑说明、`UpdateVirtualValue` 解绑行为变化说明
- `readme.md` — 功能亮点新增"QQ API 错误码中文提示"、"idmap 迁移阻塞式 + 强制解绑工具"
- `AGENTS.md` — 新增「🌿 大更改 → 提 PR」章节；分支名固定为 `Pr-Edit`；`foundItems` 表格新增 `card`、`input_notify`、`stream` key；`msg_type` 陷阱说明补充 `MsgType=6`、`MsgType=8`；新增连接模式与 Processor 初始化章节；新增本地 Fork 依赖说明；补充 Handler 签名和参数含义；重构构建章节为命令表格；重组陷阱章节为分类子章节
- 删除错误的 `release_log/CHANGELOG_v011.md`（当前仍在 Release010）

---

## 📦 文件变更清单

| 文件 | 变更 |
|------|------|
| `botgo/dto/message_create.go` | 新增 `GroupCard`/`GroupCardContent`/`InputNotify`/`StreamChunk` 结构体，`MessageToCreate` 新增 `Card`/`InputNotify` 字段 |
| `botgo/openapi/iface.go` | 新增 `PostC2CStreamMessage` 接口 |
| `botgo/openapi/v1/message.go` | `PostC2CStreamMessage` v1 实现 |
| `botgo/openapi/v2/message.go` | `PostC2CStreamMessage` v2 实现 |
| `botgo/openapi/v2/resource.go` | 分片上传相关 |
| `handlers/message_parser.go` | 消息段格式补 `card`/`input_notify` case；`cardPattern` 改为顺序无关提取；`resolvePlainTextAtMentions` 缓存失效回退 `<@OpenID>` |
| `handlers/send_group_msg.go` | 卡片消息发送逻辑；纯卡片独立发送；`auto_md` 内补 `ResolveMarkdownAtMentions`；图文混合补 `resolvePlainTextAtMentions`；`FormatQQError` 接入；`message_reference.IgnoreGetMessageError` 改为 `false` |
| `handlers/send_group_msg_raw.go` | 图文混合补 `resolvePlainTextAtMentions`；`FormatQQError` 接入 |
| `handlers/send_private_msg.go` | 输入状态通知；流式消息三段式；回执补齐；40034025/超时重试；`FormatQQError` 接入；keyMap 补 `url_record`/`url_video` |
| `handlers/send_guild_channel_msg.go` | 释放 markdown/qqmusic；新增 `sendGuildChannelMsgKeyMap`；图文混合补 `resolvePlainTextAtMentions`；`FormatQQError` 接入 |
| `handlers/send_private_msg_wakeup.go` | `FormatQQError` 接入；keyMap 补 `url_record`/`url_video` |
| `handlers/send_private_msg_sse.go` | `fmt.Printf` → `mylog.Printf` |
| `handlers/upload_helper.go` | 新建，分片上传辅助 |
| `handlers/qq_error_codes.go` | 新建，错误码映射表 + `ExtractQQErrorCode`/`FormatQQError` |
| `idmap/service.go` | `UpdateVirtualValue` 解绑彻底清理；新增 `ForceUnbindID` |
| `idmap/new_service.go` | `StartMigration` 改阻塞式；`writeBatchToNewDB` 按 value 去重 |
| `server/getIDHandler.go` | 新增 `case 18` 强制解绑 |
| `Processor/Processor.go` | `/me` 命令移除多余 `err != nil` 检查 |
| `Processor/processor_test.go` | 新建，`/me` 命令自动化测试 |
| `config/config.go` | 新增测试辅助 setter 函数 |
| `template/config_template.go` | `FriendAdd`/`FriendDel` 默认启用 |
| `docs/cq码/扩展CQ码/扩展cq码-cq-card.md` | 新建 |
| `docs/cq码/扩展CQ码/扩展cq码-cq-input_notify.md` | 新建 |
| `docs/cq码/扩展CQ码/扩展cq码-cq-stream.md` | 新建 |
| `docs/cq码/扩展CQ码/扩展cq码-cq-at.md` | 新增图文混合 msg_type=7 + Markdown 路径章节 |
| `docs/本版新增功能.md` | 多处更新 |
| `docs/Gensokyo语法参考.md` | `[CQ:at]` 行更新 |
| `docs/cq码/扩展CQ码汇总.md` | `[CQ:at]` 行更新 |
| `docs/idmap.md` | 迁移阻塞式 + 强制解绑工具说明 |
| `readme.md` | 功能亮点更新 |
| `AGENTS.md` | 大更改 PR 流程、foundItems 表格、msg_type 陷阱等 |
| `release_log/CHANGELOG_v011.md` | 删除（错误创建） |
| `release_log/CHANGELOG_v010.md` | 本文档 |

---

## ✅ 提交记录

```
9d6ca70  docs: 更新文档，同步出站 @ 行为变更（Release009 末尾）
49c3dd7  feat: QQ Bot API v2 适配修复 — CQ 码支持、卡片消息、输入状态、错误处理
ed3a240  docs: 添加大更改提 PR 的分支规范
0da3056  docs: 分支名固定为 Pr-Edit，内容体现在 commit 与 PR 中
e6acf7b  docs: 新增 [CQ:card] / [CQ:input_notify] 文档及 CHANGELOG
06f9ec6  docs: 补全 readme 和 更多文档.md 中 card/input_notify 的引用
61e9c63  feat: 智能分片上传 — 文件超软限制时自动切换
e3174fb  fix: 消息段格式 [CQ:card] 未处理导致静默丢弃
6e5de80  fix: 纯卡片消息（无文本内容）未发送
5232956  fix: 卡片消息 url 为空时 QQ API 拒绝 (40011021)
b2aa5a9  fix: 卡片 pic_url 默认值改为真实图片 URL
5955888  fix: 卡片消息 pic_url 为空时 QQ API 拒绝 (40011021)
84cd337  feat: 卡片 pic_url 支持本地路径自动 OSS 上传
7462bb9  docs: 补充卡片消息 pic 本地路径 OSS 上传说明
0466b81  fix: 消息段格式 [CQ:input_notify] 未处理导致静默丢弃
5f2a06f  feat: 新增 [CQ:stream] 流式消息支持
7a7ee61  fix: PostC2CStreamMessage 响应反序列化错误
15b25fd  fix: [CQ:input_notify] 后空白文本不发送普通消息
aaa2d94  fix: [CQ:stream] mid 续片缺少回执
17f6324  fix: 所有 stream/input_notify 路径统一补 SendC2CResponse
1df8256  docs: Release010 CHANGELOG 独立
5d88c4c  docs: 重写 CHANGELOG_v010 对齐 v009 格式
e7efa02  docs: readme 补充 QQ 机器人官方文档引用
b95be7a5  docs: 改进 AGENTS.md 架构文档与构建指南
1a1d5cd  fix: 修复 /me 命令报错问题并添加自动化测试
7983e84  docs: 同步更新 CHANGELOG 和文档反映 /me 修复与测试
7ad9fb6  fix: message_reference IgnoreGetMessageError 改为 false
3f0f2de  feat: 默认启用 FriendAdd/FriendDel 事件转发
c982fc9  fix: 纯文本 [CQ:at] 用户名缓存失效时原样发送 CQ 码
44be5d9  fix: 图文混合消息(msg_type=7)未转换[CQ:at]导致原文显示
4ab2513  feat: QQ API 错误码中文提示 + 图文混合[CQ:at]修复
127c8f8  fix: idmap 迁移重复映射修复 + 强制解绑工具
7e3a91c  fix: ForceUnbindID 支持虚拟ID入参 + 修复 type18/type5 返回 false
518acb3  fix: 图文混合走Markdown路径(auto_md)未转换[CQ:at]导致原文显示
c30653f  docs: ForceUnbindID 支持虚拟ID入参的文档同步
7682288  docs: 图文混合走Markdown路径(auto_md)未转换[CQ:at]的文档同步
```
