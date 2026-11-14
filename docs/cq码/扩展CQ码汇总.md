# 扩展 CQ 码

本页汇总 Gensokyo-NewQQ 在标准 OneBot 消息段基础上增加或扩展的行为，与 `docs/cq码/扩展CQ码/` 下的 11 张扩展码页面逐条对应。

| CQ 码 | 范围 | 说明 | 文档 |
|-------|------|------|------|
| `[CQ:member]` | `q群 (Group Chat)` | 群成员入群/退群通知及回复路由。 | [查看](./扩展CQ码/扩展cq码-cq-member.md) |
| `[CQ:active]` | `q群 (Group Chat)` / `私聊 (C2C)` | 主动推送标记：写入 `active` / `active_type` / `active_sub_type` 三个控制字段，群聊与私聊发送前命中即清空 `msg_id` 与 `event_id` 强制走主动通道；召回 Action 额外消费该标记。 | [查看](./扩展CQ码/扩展cq码-cq-active.md) |
| `[CQ:wakeup]` | `私聊 (C2C)` | C2C 互动召回（唤醒）消息标记，`userid` 指定目标用户（OpenID 或虚拟 ID）。 | [查看](./扩展CQ码/扩展cq码-cq-wakeup.md) |
| `[CQ:remove]` | `q群 (Group Chat)` | 撤回指定群消息（出站单向，需 `user_id` + `msg_id`）。 | [查看](./扩展CQ码/扩展cq码-cq-remove.md) |
| `[CQ:at]` | `q群 (Group Chat)` | @ 标签扩展行为：纯文本出站转为 `@用户名 `，Markdown 出站转为 `<qqbot-at-user>` 标签。 | [查看](./扩展CQ码/扩展cq码-cq-at.md) |
| `[CQ:card]` | `q群 (Group Chat)` | 图文卡片消息（`msg_type=8`），参数顺序无关；`title` / `desc` 缺失或超长整条消息回 `1400`，`pic` / `url` 缺失时卡片被丢弃、正文照常发送。 | [查看](./扩展CQ码/扩展cq码-cq-card.md) |
| `[CQ:input_notify]` | `私聊 (C2C)` | 输入状态通知（`msg_type=6`），正文发送前先展示「正在输入」；字符串、数组消息段与 cqparse 三种输入形态均支持。 | [查看](./扩展CQ码/扩展cq码-cq-input_notify.md) |
| `[CQ:stream]` | `私聊 (C2C)` | 流式消息，逐片展示实现打字机效果（start→mid→finish）。 | [查看](./扩展CQ码/扩展cq码-cq-stream.md) |
| `[CQ:keyboard]` | `q群 (Group Chat)` / `私聊 (C2C)` | 文本消息附加内嵌键盘（按钮消息）。⚠️ 按钮仅在 Markdown 消息下渲染，建议与 `[CQ:markdown]` 配合。 | [查看](./扩展CQ码/扩展cq码-cq-keyboard.md) |
| `[CQ:set_group]` | `q群 (Group Chat)` | 出站动作统一码：成员禁言/解禁、⚠️ 全员禁言（`action=whole_ban` **固定失败**）、入群申请审批、审批策略执行/删除、成员踢出、黑名单增删。 | [查看](./扩展CQ码/扩展cq码-cq-set_group.md) |
| `[CQ:group_info]` | `q群 (Group Chat)` / `私聊 (C2C)` | 群信息内容展开码：正文展开群名/群简报/成员数或三字段 JSON，支持指定群与失败 fallback，同群多码合并一次取数。⚠️ 需 `cq_parse_mode: new`；legacy/shadow 下该码不被识别会原文发出。 | [查看](./扩展CQ码/扩展cq码-cq-group_info.md) |

---

## 标准码中被扩展的行为

以下 CQ 码本身属于 OneBot V11 标准码，但 Gensokyo 为它们增加了官方 API 特有的出站能力，文档仍归在标准 CQ 码目录下：

| CQ 码 | 扩展行为 | 文档 |
|-------|----------|------|
| `[CQ:file]` | 标准协议中只用于入站上报，Gensokyo 扩展为**支持出站发送文件**（本地路径 / URL / `base64://`），并支持 `file_name` 自定义文件名（已生效）。 | [查看](./标准CQ码/标准cq码-cq-file.md) |
| `[CQ:record]` | 语音自动 silk 转码后上传 CDN；`[CQ:voice]` 为其同义别名。 | [查看](./标准CQ码/标准cq码-cq-record.md) |
