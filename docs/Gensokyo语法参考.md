# Gensokyo 语法参考

本文只列 Gensokyo 相对 OneBot V11 增加或改变的消息语法。

范围说明：

| 标记 | 含义 |
|------|------|
| `-` | 通用解析；是否能发送取决于调用的 Action 和 QQ API 限制 |
| `私聊 (C2C)` | QQ C2C 单聊 |
| `q群 (Group Chat)` | QQ 群 |
| `q頻 (QQ Guild)` | QQ 频道/子频道 |

## 扩展 CQ 码

| CQ 码 | 写法 | 范围 | 行为 |
|-------|------|------|------|
| Markdown | `[CQ:markdown,data=base64://<base64-json>]` 或 `[CQ:markdown,data=<json>]` | `-` | 解析为 QQ Markdown 消息。 |
| 头像 | `[CQ:avatar,qq=<虚拟用户ID>]` | `-` | 替换为该用户 QQ 头像图片。 |
| QQ 音乐 | `[CQ:music,type=qq,id=<歌曲ID>]` | `-` | 转为 QQ 音乐 Markdown 卡片。 |
| 回复 | `[CQ:reply,id=<消息ID>]` | `q群 (Group Chat)` | `send_group_msg` 会尝试转换为 `message_reference`；QQ q群可能接受但不渲染引用样式。 |
| 成员变动 | `[CQ:member,type=add/remove,group_id=<虚拟群ID>,user_id=<虚拟用户ID>]` | `q群 (Group Chat)` | 群成员入群/退群通知和后续回复路由。见 [CQ member](./cq码/扩展CQ码/扩展cq码-cq-member.md)。 |
| 主动标记 | `[CQ:active,type=<值>,sub_type=<值>]` | `-` | 当前只解析并移除该 CQ 码，记录 `type` / `sub_type`；没有后续发送逻辑。见 [CQ active](./cq码/扩展CQ码/扩展cq码-cq-active.md)。 |
| @ | `[CQ:at,qq=<虚拟用户ID>]` | `q群 (Group Chat)` / `q頻 (QQ Guild)` | 纯文本出站转为 `@用户名 `，Markdown 出站转为 `<qqbot-at-user id="OpenID" />`。见 [CQ at](./cq码/扩展CQ码/扩展cq码-cq-at.md)。 |
| 消息撤回 | `[CQ:remove,user_id=<虚拟用户ID>,msg_id=<虚拟消息ID>]` | `q群 (Group Chat)` | 在 `send_group_msg` 中携带，撤回指定群消息。需要同时提供 `user_id` 和 `msg_id`。见 [CQ remove](./cq码/扩展CQ码/扩展cq码-cq-remove.md)。 |

## 消息段

数组消息段也会进入同一套解析逻辑：

| 段类型 | data 字段 | 行为 |
|--------|-----------|------|
| `markdown` | `data` | 接受 JSON、base64 JSON 或 `base64://` 前缀。 |
| `avatar` | `qq` | 等同 `[CQ:avatar]`。 |
| `active` | `type`, `sub_type` | 解析后不写入文本。 |
| `member` | `type`, `group_id`, `user_id` | 等同 `[CQ:member]`。 |

## Markdown 图片尺寸

Gensokyo 支持 QQ 官方 Markdown 的图片尺寸语法：

```markdown
![#100px](图片链接)        # 宽度 100px，高度自适应
![#100px #100px](图片链接)  # 宽高均为 100px
```

详见 [Markdown 消息文档](./文档-markdown消息.md)。
