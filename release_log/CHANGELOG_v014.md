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
