# Changelog — Release014

> 🚀 **已封版**：Release014 已封版，后续变更请记录到 [CHANGELOG_v015.md](./CHANGELOG_v015.md)。

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

### 入群通知回复被误判为群私聊（code:11255）

`GROUP_MEMBER_ADD` / `GROUP_MEMBER_REMOVE` 通知经 `Processor/ProcessGroupMember.go` 处理时，群 OpenID 通过 `idmap.StoreIDv2` 生成虚拟 group_id，但**未登记该虚拟 ID 的消息类型**（对比常规群消息 `ProcessGroupNormalMessage.go` 会写 `echo.AddMsgType(..., "group")` 与 `idmap.WriteConfigv2(id, "type", "group")`）。于是插件针对入群通知回包 `send_group_msg` 时，`HandleSendGroupMsg` 的 `GetMessageTypeByGroupid` / `GetMessageTypeByGroupidV2` 均查不到类型，落入未知类型兜底逻辑（`send_group_msg.go` 中 `echo.AddMsgType(..., "group_private")`）被误判为群私聊，进而把群 OpenID 当作用户走 C2C 接口 `POST /v2/users/{群OpenID}/messages`，报 `11255 请求的资源不存在(用户/群已注销)`。

- **修改**：`Processor/ProcessGroupMember.go` 在生成虚拟 group_id 后，补充 `echo.AddMsgType(config.GetAppIDStr(), groupID, "group")` 与 `idmap.WriteConfigv2(fmt.Sprint(groupID), "type", "group")`，与常规群消息路径保持一致
- **影响**：入群/退群欢迎语等针对成员变动的 `send_group_msg` 回包将正确走群聊 `POST /v2/groups/{群OpenID}/messages`，不再误发 C2C 导致 11255

### `safeLocalPath` 先拒绝 `..` 再 Clean，修复 Linux 下路径穿越校验绕过

`handlers/message_parser.go` 的 `safeLocalPath` 原先先 `filepath.Clean(decoded)` 再检查 `..` 字符串。`filepath.Clean` 会把根路径下的 `/../` 折叠成 `/`（如 `/../secret.txt` → `/secret.txt`），先 Clean 再检查的字符串匹配在 Linux（CI）上被绕过，`file:///../secret.txt` 得以进入 `local_file` / `local_image` 本地读取路径，形成路径穿越。

- **修改**：`..` 检查移到 `filepath.Clean` 之前，对 URL 解码后的原始路径判断；Clean 仅用于移除 `.` 与规整路径，`filepath.Abs` 相对路径防护保留
- **影响**：Windows/Linux 行为一致，路径穿越校验不再依赖平台，`file:///../secret.txt` 等恶意路径被统一拒绝

### `c2cMessageHandler` 按消息 ID 去重，修复私聊消息偶发收到两遍

群消息 handler（`groupAtMessageHandler` / `groupMessageHandler`）早已通过 `processedIDs.LoadOrStore` 按消息 ID 去重，挡住 QQ 网关断线重连 / RESUME 时的重复投递；`c2cMessageHandler` 缺少同样的去重，导致私聊消息偶发重复上报。

- **修改**：`botgo/event/event.go` 的 `c2cMessageHandler` 在 `ParseData` 后补充 `processedIDs.LoadOrStore(data.ID, struct{}{})` 去重，命中已处理 ID 直接返回 nil
- **影响**：C2C 私聊消息与群消息行为对齐，网关重连恢复时不再重复上报

---

## 📝 工程与文档变更

### 入群/退群通知登记群类型时增加诊断日志

`Processor/ProcessGroupMember.go` 在每次 `GROUP_MEMBER_ADD` / `GROUP_MEMBER_REMOVE` 登记虚拟 group_id 为群聊类型时打印一条 INFO 日志（`[ProcessGroupMember] 虚拟 group_id %d 已登记为群聊类型`），便于确认 11255 修复（见上）已部署生效。

### Gensokyo语法参考新增图文混排（非 Markdown）说明

`docs/Gensokyo语法参考.md` 补充 `msg_type=7` 图文混合消息的触发条件、范围与图片来源：

- 示例覆盖 CQ 码字符串与消息段数组两种写法
- 说明多图/纯图行为及 `two_way_echo` 转 Markdown 的边界

### reply REFIDX 与 keyboard 共存行为同步到 CQ 码文档

封版前补齐相关 CQ 码文档，与实际代码行为对齐：

- `标准cq码-cq-reply.md`：ID 映射流程补充 REFIDX 优先说明（非机器人消息引用使用 `message_scene.ext[].msg_idx`，无 REFIDX 回退普通消息 ID，`MsgID` 始终为普通消息 ID）
- `扩展cq码-cq-keyboard.md`：修正 3 处过时描述（"markdown 优先、独立 keyboard 被忽略" → markdown 未内嵌 keyboard 时独立 `[CQ:keyboard]` 仍会附加，v013 起共存）
- `本版新增功能.md`：reply 补充 REFIDX 说明；keyboard 补充消息段路径支持与共存行为

---

## ✅ 提交记录

```
452bca8  fix: c2cMessageHandler 按消息ID去重，修复私聊消息偶发收到两遍
a8f4e61  docs: Gensokyo语法参考新增图文混排（非 Markdown）说明
f98f6df  fix: safeLocalPath 先拒绝 .. 再 Clean，修复 Linux 下路径穿越校验被 /../ 折叠绕过
deb119a  chore: 入群/退群通知登记群类型时增加诊断日志
3276f0a  fix: 入群/退群通知回包 send_group_msg 被误判为群私聊 (code:11255)
5d40b32  fix: 私聊图文混合消息图片发送失败导致 send_msg 超时
fd2ccc3  fix: Windows 下 Markdown/Keyboard 本地图片 file:/// 路径解析错误
a68cd65  fix: get_friend_list 不再过滤虚拟数字 ID
c563e68  feat: 新增 [CQ:wakeup,userid=xxx] C2C 互动召回消息标记
9f9cc0a  docs: 同步 reply REFIDX 与 keyboard 共存行为到 CQ 码文档
```
