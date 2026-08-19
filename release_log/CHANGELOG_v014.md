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
