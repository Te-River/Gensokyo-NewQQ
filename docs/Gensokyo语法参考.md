# Gensokyo 语法参考

本文只列 Gensokyo 相对 OneBot V11 增加或改变的消息语法。

范围说明：

| 标记 | 含义 |
|------|------|
| `-` | 通用解析；是否能发送取决于调用的 Action 和 QQ API 限制 |
| `私聊 (C2C)` | QQ C2C 单聊 |
| `q群 (Group Chat)` | QQ 群 |

## 扩展 CQ 码

| CQ 码 | 写法 | 范围 | 行为 |
|-------|------|------|------|
| Markdown | `[CQ:markdown,data=base64://<base64-json>]` 或 `[CQ:markdown,data=<json>]` | `-` | 解析为 QQ Markdown 消息。 |
| 头像 | `[CQ:avatar,qq=<虚拟用户ID>]` | `-` | 替换为该用户 QQ 头像图片。 |
| QQ 音乐 | `[CQ:music,type=qq,id=<歌曲ID>]` | `-` | 转为 QQ 音乐 Markdown 卡片。 |
| 回复 | `[CQ:reply,id=<消息ID>]` | `q群 (Group Chat)` | `send_group_msg` 会尝试转换为 `message_reference`；QQ q群可能接受但不渲染引用样式。 |
| 成员变动 | `[CQ:member,type=add/remove,group_id=<虚拟群ID>,user_id=<虚拟用户ID>]` | `q群 (Group Chat)` | 群成员入群/退群通知和后续回复路由。见 [CQ member](./cq码/扩展CQ码/扩展cq码-cq-member.md)。 |
| 主动标记 | `[CQ:active,type=<值>,sub_type=<值>]` | `-` | 当前只解析并移除该 CQ 码，记录 `type` / `sub_type`；没有后续发送逻辑。见 [CQ active](./cq码/扩展CQ码/扩展cq码-cq-active.md)。 |
| 召回标记 | `[CQ:wakeup,userid=<目标用户>]` | `私聊 (C2C)` | 将 `send_private_msg` 消息作为 C2C 互动召回（`is_wakeup=true`）发送给 `userid` 指定用户（OpenID 或虚拟 ID）。见 [CQ wakeup](./cq码/扩展CQ码/扩展cq码-cq-wakeup.md)。 |
| @ | `[CQ:at,qq=<虚拟用户ID>]` | `q群 (Group Chat)` | 纯文本出站转为 `@用户名 `，Markdown 出站转为 `<qqbot-at-user id="OpenID" />`。图文混合消息（msg_type=7）同样走纯文本转换路径；图文混合走 Markdown 路径（auto_md，msg_type=2）同样走 Markdown 转换路径（2026-08 修复）。见 [CQ at](./cq码/扩展CQ码/扩展cq码-cq-at.md)。 |
| 消息撤回 | `[CQ:remove,user_id=<虚拟用户ID>,msg_id=<虚拟消息ID>]` | `q群 (Group Chat)` | 在 `send_group_msg` 中携带，撤回指定群消息。需要同时提供 `user_id` 和 `msg_id`。见 [CQ remove](./cq码/扩展CQ码/扩展cq码-cq-remove.md)。 |
| 卡片消息 | `[CQ:card,title=<标题>,desc=<描述>,pic=<图片URL>,url=<跳转链接>]` | `q群 (Group Chat)` | 群聊图文卡片消息（`msg_type=8`）。参数顺序无关，`title` 必填，其余可选。见 [CQ card](./cq码/扩展CQ码/扩展cq码-cq-card.md)。 |
| 输入状态 | `[CQ:input_notify,type=<类型>,second=<秒数>]` | `私聊 (C2C)` | 在发送正文前先发送"正在输入"状态（`msg_type=6`）。`type` 默认 `1`，`second` 最大 `60`。见 [CQ input_notify](./cq码/扩展CQ码/扩展cq码-cq-input_notify.md)。 |
| 流式消息 | `[CQ:stream,type:start,qq:<虚拟用户ID>]` | `私聊 (C2C)` | 流式消息，分 start→mid→finish 三阶段逐片发送，实现打字机效果。见 [CQ stream](./cq码/扩展CQ码/扩展cq码-cq-stream.md)。 |

## 消息段

数组消息段也会进入同一套解析逻辑：

| 段类型 | data 字段 | 行为 |
|--------|-----------|------|
| `markdown` | `data` | 接受 JSON、base64 JSON 或 `base64://` 前缀。 |
| `avatar` | `qq` | 等同 `[CQ:avatar]`。 |
| `active` | `type`, `sub_type` | 解析后不写入文本。 |
| `wakeup` | `userid` | 等同 `[CQ:wakeup,userid=xxx]`，解析后不写入文本。 |
| `member` | `type`, `group_id`, `user_id` | 等同 `[CQ:member]`。 |

## Markdown 图片尺寸

Gensokyo 支持 QQ 官方 Markdown 的图片尺寸语法：

```markdown
![#100px](图片链接)        # 宽度 100px，高度自适应
![#100px #100px](图片链接)  # 宽高均为 100px
```

详见 [Markdown 消息文档](./文档-markdown消息.md)。

## 图文混排（非 Markdown）

文本 + 单张图片会自动合并为一条 QQ 富媒体消息（`msg_type=7`）发送，不需要 Markdown：

| 项 | 说明 |
|----|------|
| 触发条件 | 恰好 1 张图片，且文本非空 |
| 范围 | `q群 (Group Chat)`、`私聊 (C2C)`（含 `send_group_msg_raw`、`send_private_msg_wakeup`） |
| 图片来源 | `file://` 本地路径、`http(s)://` 链接、`base64://` 数据（对应 `local_image` / `url_image` / `url_images` / `base64_image`） |
| `[CQ:at]` | 文本段中的 `[CQ:at,qq=虚拟ID]` 转为 `@用户名 `，与纯文本路径一致 |
| `[CQ:reply]` | 支持回复引用，转换为 `message_reference` 并设置 `msg_id`（群聊与私聊均支持） |

示例：

```text
[CQ:image,file=https://example.com/pic.png]这是图片说明
```

消息段模式写法（`[CQ:image,file=...]` 与 `{"type":"image","data":{"file":"..."}}` 等价，均进入同一套解析逻辑）：

```json
[
  {"type": "image", "data": {"file": "https://example.com/pic.png"}},
  {"type": "text", "data": {"text": "这是图片说明"}}
]
```

注意：

- 多张图片（或纯图片无文本）不会合并，会按 foundItems 逐条作为富媒体消息发送。
- 群聊中若开启 `two_way_echo`，图文混排可能被自动转为 Markdown（`msg_type=2`）发送。
