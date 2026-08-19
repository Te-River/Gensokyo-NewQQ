# `foundItems` 隐式协议审计

## 现状

`handlers/message_parser.go:676-1414` 的 `parseMessageContent` 返回 `(string, map[string][]string)`，以字符串 key 和字符串切片承载解析结果。代码同时处理 segment 数组、消息 map、CQ 字符串三类输入，媒体和控制字段在多个分支重复写入。

生产 key 包括：

- 控制/上下文：`reply_msg_id`、`active`、`active_type`、`active_sub_type`、`markdown`、`card`、`input_notify`、`stream`、`file_name`。
- 图片：`local_image`、`url_image`、`url_images`、`base64_image`、`unknown_image`。
- 语音：`local_record`、`url_record`、`url_records`、`base64_record`、`unknown_record`。
- 视频：`local_video`、`url_video`、`url_videos`、`base64_video`、`unknown_video`。
- 文件：`local_file`、`url_file`、`url_files`、`base64_file`、`unknown_file`。
- 其他：`qqmusic`、`unknown_*`。

## 消费者

- `send_group_msg.go:77-975` 读取并路由多种 key，`generateGroupMessage` 与 `generatePrivateMessage` 分别在约 1019-1755、1758-2496 处理媒体。
- `send_private_msg.go` 有私聊和唤醒路径的重复消费。
- `send_group_msg_raw.go`、`reply_helpers.go` 读取相同媒体 key。
- 控制字段在发送循环中显式跳过，说明 map 中同时承载控制信息和可发送内容。

## 已确认问题

1. **协议无类型约束**：拼写、互斥关系、必填字段和优先级都只能在运行时维护。
2. **输入分支重复**：同一个媒体语义在 segment、map、CQ 三条路径重复解析。
3. **发送逻辑重复**：群聊和私聊生成器有大量重复的图片/语音/视频/文件分支。
4. **解析层触碰外部行为**：parser 接收 client/api/apiv2，并调用 URL 转换/解析辅助路径，边界不再是纯语法解析。

## 迁移建议

第一步定义不可变的 `ParsedMessage`、`MessagePart`、`AttachmentRef`、`ReplyRef`、`ActiveAction` 和 `UnknownPart`，保留 `foundItems` 兼容适配器。第二步让群聊、私聊、raw、wakeup 共享一个 media planner。第三步删除 map 直读，最后再删除兼容层。

## 本轮结论

这是首个业务拆分热点，但本轮不改 parser 或 sender，避免在没有回归用例的情况下改变 OneBot 兼容行为。
