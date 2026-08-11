# CQ 码汇总

Gensokyo 兼容 [OneBot V11](https://github.com/botuniverse/onebot-11) 标准 CQ 码协议，并在此基础上扩展了多种 CQ 码以适配 QQ Bot API 的特有能力。

---

## 标准 CQ 码

OneBot V11 协议定义的标准消息类型。

| CQ 码 | 范围 | 说明 | 文档 |
|-------|------|------|------|
| `[CQ:text]` | `全场景` | 纯文本消息段，最基本的消息类型。 | [查看](./标准CQ码/标准cq码-cq-text.md) |
| `[CQ:face]` | `全场景` | QQ 表情（经典小脸表情）。 | [查看](./标准CQ码/标准cq码-cq-face.md) |
| `[CQ:image]` | `q群 (Group Chat)` / `C2C (私聊)` | 图片消息，支持 URL / base64 / 本地路径。 | [查看](./标准CQ码/标准cq码-cq-image.md) |
| `[CQ:record]` | `q群 (Group Chat)` / `C2C (私聊)` | 语音消息，支持 URL / base64 / 本地路径，自动转码 silk。 | [查看](./标准CQ码/标准cq码-cq-record.md) |
| `[CQ:video]` | `q群 (Group Chat)` / `C2C (私聊)` | 视频消息，支持 URL / 本地路径。 | [查看](./标准CQ码/标准cq码-cq-video.md) |
| `[CQ:at]` | `q群 (Group Chat)` | @ 标签，用于提及群成员或机器人。 | [查看](./标准CQ码/标准cq码-cq-at.md) |
| `[CQ:share]` | `-` | 分享链接（QQ Bot API 暂不支持发送）。 | [查看](./标准CQ码/标准cq码-cq-share.md) |
| `[CQ:location]` | `-` | 位置信息（QQ Bot API 暂不支持发送）。 | [查看](./标准CQ码/标准cq码-cq-location.md) |
| `[CQ:music]` | `q群 (Group Chat)` / `C2C (私聊)` | 音乐分享，当前仅支持 QQ 音乐。 | [查看](./标准CQ码/标准cq码-cq-music.md) |
| `[CQ:reply]` | `q群 (Group Chat)` / `C2C (私聊)` | 回复指定消息。 | [查看](./标准CQ码/标准cq码-cq-reply.md) |
| `[CQ:forward]` | `-` | 合并转发（QQ Bot API 暂不支持）。 | [查看](./标准CQ码/标准cq码-cq-forward.md) |

---

## 扩展 CQ 码

Gensokyo 为适配 QQ Bot API 特有能力而扩展的 CQ 码类型。

| CQ 码 | 范围 | 说明 | 文档 |
|-------|------|------|------|
| `[CQ:member]` | `q群 (Group Chat)` | 群成员入群/退群通知及回复路由。 | [查看](./扩展CQ码/扩展cq码-cq-member.md) |
| `[CQ:active]` | `-` | 主动消息标记，强制走主动推送通道。 | [查看](./扩展CQ码/扩展cq码-cq-active.md) |
| `[CQ:remove]` | `q群 (Group Chat)` | 撤回指定群消息（出站单向，需 user_id + msg_id）。 | [查看](./扩展CQ码/扩展cq码-cq-remove.md) |
| `[CQ:at]` | `q群 (Group Chat)` / `q頻 (QQ Guild)` | @ 标签：纯文本出站转为 `@用户名 `，Markdown 出站转为 `<qqbot-at-user>` 标签。图文混合消息（msg_type=7）同样走纯文本转换路径；图文混合走 Markdown 路径（auto_md，msg_type=2）同样走 Markdown 转换路径（2026-08 修复）。 | [查看](./扩展CQ码/扩展cq码-cq-at.md) |
| `[CQ:card]` | `q群 (Group Chat)` | 群聊图文卡片消息（msg_type=8），参数顺序无关。 | [查看](./扩展CQ码/扩展cq码-cq-card.md) |
| `[CQ:input_notify]` | `私聊 (C2C)` | 输入状态通知，正文发送前先展示"正在输入"。 | [查看](./扩展CQ码/扩展cq码-cq-input_notify.md) |
| `[CQ:stream]` | `私聊 (C2C)` | 流式消息，逐片展示实现打字机效果（start→mid→finish）。 | [查看](./扩展CQ码/扩展cq码-cq-stream.md) |
| `[CQ:set_group_ban]` | `q群 (Group Chat)` | 出站动作：成员禁言/解禁（`duration` 秒，0=解除）。 | [查看](./扩展CQ码/扩展cq码-cq-set_group_ban.md) |
| `[CQ:set_group_whole_ban]` | `q群 (Group Chat)` | 出站动作：全员禁言开关（`enable`）。 | [查看](./扩展CQ码/扩展cq码-cq-set_group_whole_ban.md) |
| `[CQ:set_group_add_request]` | `q群 (Group Chat)` | 出站动作：入群申请审批（`flag`+`approve`）。 | [查看](./扩展CQ码/扩展cq码-cq-set_group_add_request.md) |
| `[CQ:strategy]` | `-` | 出站动作：入群自动审批策略执行/删除（`action`+`strategy_id`）。 | [查看](./扩展CQ码/扩展cq码-cq-strategy.md) |

---

## 范围说明

| 标记 | 含义 |
|------|------|
| `全场景` | 所有消息场景（群聊、私聊、频道等） |
| `q群 (Group Chat)` | QQ 群消息 |
| `C2C (私聊)` | QQ C2C 单聊消息 |
| `q頻 (QQ Guild)` | QQ 频道/子频道消息 |
| `-` | 通用解析；是否能发送取决于调用的 Action 和 QQ API 限制 |
