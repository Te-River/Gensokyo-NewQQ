# Changelog — Release014

> 自 Release013 以来的所有变更。

---

## ✨ 新功能 / 变更

### `[CQ:wakeup,userid=xxx]` C2C 互动召回消息标记

新增 `[CQ:wakeup]` CQ 码，可在 `send_private_msg` 出站消息中直接指定目标用户发送 **C2C 互动召回（唤醒）消息**，无需单独调用 `send_private_msg_wakeup` API：

- **解析**（`handlers/cqcode.go`）：新增 `wakeupPattern` 正则与 `ProcessCQWakeup` 函数，从文本中提取 `userid` 存入 `foundItems["wakeup"]` 并移除 CQ 码；统一管道 `ProcessCQCodePipeline` 第 7 步接入
- **三路径补齐**（`handlers/message_parser.go`）：`[]interface{}` 数组段（koishi）与 `map[string]interface{}`（TRSS）两条路径新增 `case "wakeup"`，与 string 路径解析行为一致
- **发送**（`handlers/send_private_msg.go`）：检测到 `wakeup` 标记后，目标用户覆盖为 CQ 码指定用户（虚拟 ID 自动转 OpenID），交由召回 handler 按 `IsWakeup=true`、`MsgID=""`、`EventID=""`（互斥）通道发送，支持纯文本/图文混合/Markdown/富媒体
- **结果回执**：发送结果以 `notice`（`notice_type=c2c_wakeup_resp`）推送给应用端，与 `send_private_msg_wakeup` API 行为一致
- **群聊防护**（`handlers/send_group_msg.go`）：群聊消息中的 `[CQ:wakeup]` 被跳过，不发送（召回仅支持 C2C）
- **测试**：`handlers/cqcode_pipeline_test.go` 新增 wakeup 字符串路径用例；`handlers/message_parser_test.go` 新增数组段与 TRSS 路径用例

文档：[CQ wakeup](../docs/cq码/扩展CQ码/扩展cq码-cq-wakeup.md)

---

## 🐛 修复

### `get_friend_list` 不再过滤虚拟数字 ID

`handlers/get_friend_list.go` 原先用 `!isNumeric(user.UserID)` 过滤掉纯数字的 user_id，导致**所有 C2C 私聊用户（虚拟数字 ID）被全部滤掉**，接口一直返回空列表。而 `StoreUserInfo` 全项目只在 `ProcessC2CMessage.go`（C2C 私聊处理）中调用，UserInfoBucket 里存的本来就是私聊用户，过滤逻辑属于误伤。

- **修改**：移除 `isNumeric` 过滤（顺带删除 `regexp` 导入），仅过滤空 UserID，保留全部私聊用户（含虚拟数字 ID）
- **影响**：`get_friend_list` 现在能正常返回所有 C2C 私聊用户，插件可遍历后配合 `send_private_msg_wakeup` 实现私聊广播

### Windows 下 Markdown/Keyboard 本地图片 `file:///` 路径解析错误

`handlers/message_parser.go` 中 `ResolveMarkdownImages` 与 `ResolveKeyboardImages`（Label/VisitedLabel 两个回调）直接用 `strings.TrimPrefix(mediaPath, "file://")` 剥离协议前缀。Windows 上路径为 `file:///C:/Users/...`（三斜杠），剥掉 `file://` 后残留前导 `/`，`safeLocalPath` 经 `filepath.Abs` 将其拼到工作目录后，实际读取路径变成 `D:\C:\Users\...`，导致 `Error reading local image for markdown: open D:\C:\...: The filename, directory name, or volume label syntax is incorrect`，Markdown 图片无法上传 CDN、显示空白。

- **修改**：三处统一改用仓库已有的 `trimFilePrefix()`（Windows 剥 `file:///`、Unix 剥 `file://`），与 `resolveLocalMedia` 等其他 file:// 处理路径行为一致
- **影响**：Windows 下 Markdown 与 keyboard 按钮中的 `file:///C:/...` 本地图片可正确读取并上传替换为 CDN 直链

### 私聊图文混合消息图片发送失败（send_msg 超时）

`handlers/send_private_msg.go` 图文混合路径（`imageCount == 1 && messageText != ""`）对 `groupReply` 只做了 `*dto.RichMediaMessage` 断言：`local_image` / `base64_image` 等在 `generatePrivateMessage` 内已完成上传、返回的是 `*dto.MessageToCreate`，断言失败后直接 `return "", nil`，**既未发送消息也未给 OneBot 客户端回执**，插件侧表现为 `WebSocket call api send_msg timeout`。群聊同路径（`send_group_msg.go`）已有 `*dto.MessageToCreate` 兜底断言，因此群聊正常。

- **修改**：图文混合路径补齐 `*dto.MessageToCreate` 兜底断言——若返回已上传完成的 `MessageToCreate`，直接补文本（`resolvePlainTextAtMentions`）、`MsgID`/`EventID`/`Timestamp` 后发送；仅在 `RichMediaMessage` 时才调用 `uploadMediaPrivate` 上传
- **失败回执**：发送失败不再裸 `return`，按 22009 / 40034025 / 超时重试 分类处理并统一 `SendC2CResponse` 回执，避免客户端超时
- **影响**：私聊本地图片 / base64 图片 + 文本的图文混合消息可正常发送，不再超时

### send_group_msg 误入私聊路径（11255 用户/群已注销）

群成员入群等 notice 触发框架主动调用 `send_group_msg` 时（如入群欢迎语，group_id 为虚拟数字 ID），`HandleSendGroupMsg` 无法从 echo/idmap 缓存判断该群的消息类型（新群尚无群消息入站，`msgType` 为空），兜底递归逻辑硬编码猜测 `group_private` 并递归，命中 `case "group_private"` 后把 `UserID = GroupID`，经 idmap 将**群的虚拟 ID 还原成群的 OpenID**，当作 用户OpenID 发 `POST /v2/users/{群OpenID}/messages` 私聊，必然报 `11255 请求的资源不存在(用户/群已注销)`，同时消耗主动推送次数。

- **修改**（`handlers/send_group_msg.go`、`handlers/send_group_msg_raw.go`）：`send_group_msg`/`send_group_msg_raw` action 本身即群消息，`msgType` 未知时**默认按 `group` 处理**，不再兜底猜测 `group_private` 并递归
- **同步**（`handlers/send_msg.go`）：通用 `send_msg` action 带 `group_id` 时按群处理，仅带 `user_id` 时按私聊处理，与群消息语义一致
- **影响**：入群欢迎等主动群消息正常发送到群，不再误发私聊、不再报 11255；已能通过 echo/idmap 缓存识别出的 `group_private`（C2C 虚拟成群私聊）路径不受影响

### Markdown / Keyboard 中 `mqqapi://` 等协议链接被误当本地图片读取

欢迎语等 Markdown 内容含 `[/自定义欢迎语](mqqapi://aio/inlinecmd?command=...)` 形式链接时，`resolveMarkdownMediaReferences` 正则把 `mqqapi://...` 当作媒体路径传入，`ResolveMarkdownImages` / `ResolveKeyboardImages` 仅排除 `http(s)` 与 `data:`，其余一律走本地文件分支，导致 `os.ReadFile` 去读 `mqqapi:/aio/inlinecmd?...` 报 `Error reading local image for markdown: open ...: no such file or directory`（欢迎语主链路不受影响，但日志持续报错）。

- **修改**（`handlers/message_parser.go`）：三处 resolve 回调（Markdown、Keyboard Label、Keyboard VisitedLabel）在本地文件分支前增加 `://` 协议判断——非 `file://` 协议链接直接跳过，不再尝试读取本地文件
- **影响**：`mqqapi://`、`qun.qq.com://` 等协议链接原样保留在 Markdown/按钮文案中，不再产生误读错误日志
