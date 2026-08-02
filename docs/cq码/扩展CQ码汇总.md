# 扩展 CQ 码

| CQ 码 | 范围 | 说明 | 文档 |
|-------|------|------|------|
| `[CQ:member]` | `q群 (Group Chat)` | 群成员入群/退群通知及回复路由。 | [查看](./扩展CQ码/扩展cq码-cq-member.md) |
| `[CQ:active]` | `-` | 主动消息标记，强制走主动推送通道。 | [查看](./扩展CQ码/扩展cq码-cq-active.md) |
| `[CQ:remove]` | `q群 (Group Chat)` | 撤回指定群消息（出站单向，需 user_id + msg_id）。 | [查看](./扩展CQ码/扩展cq码-cq-remove.md) |
| `[CQ:at]` | `q群 (Group Chat)` / `q頻 (QQ Guild)` | @ 标签：纯文本出站转为 `@用户名 `，Markdown 出站转为 `<qqbot-at-user>` 标签。图文混合消息（msg_type=7）同样走纯文本转换路径；图文混合走 Markdown 路径（auto_md，msg_type=2）同样走 Markdown 转换路径（2026-08 修复）。 | [查看](./扩展CQ码/扩展cq码-cq-at.md) |
| `[CQ:card]` | `q群 (Group Chat)` | 群聊图文卡片消息（msg_type=8），参数顺序无关。 | [查看](./扩展CQ码/扩展cq码-cq-card.md) |
| `[CQ:input_notify]` | `私聊 (C2C)` | 输入状态通知，正文发送前先展示"正在输入"。 | [查看](./扩展CQ码/扩展cq码-cq-input_notify.md) |
| `[CQ:stream]` | `私聊 (C2C)` | 流式消息，逐片展示实现打字机效果（start→mid→finish）。 | [查看](./扩展CQ码/扩展cq码-cq-stream.md) |
