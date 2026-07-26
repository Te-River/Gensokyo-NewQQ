# [CQ:card]

## 用途

`[CQ:card]` 用于在群聊中发送图文卡片消息（`msg_type=8`）。卡片消息会展示标题、描述、图片和跳转链接。

范围：`q群 (Group Chat)`

## 语法

```text
[CQ:card,title=<标题>,desc=<描述>,pic=<图片URL>,url=<跳转链接>]
```

所有参数顺序无关，`title` 为必填，其余可选。

| 参数 | 必填 | 说明 |
|------|------|------|
| `title` | 是 | 卡片标题 |
| `desc` | 否 | 卡片描述文字 |
| `pic` | 否 | 卡片图片 URL（需公网可访问） |
| `url` | 否 | 点击卡片跳转的链接 |

## 解析行为

- 从消息文本中提取 `[CQ:card,...]` 并移除，最终用户不会看到该 CQ 码。
- 参数顺序无关，内部自动解析为 `key=value` 键值对。
- 只有 `title` 是必填参数，缺少 `title` 时卡片会被忽略。
- 数组消息段的 `"type":"card"` 暂不支持，仅支持字符串格式。

## 发送行为

当 `[CQ:card]` 出现在 `send_group_msg` 的出站消息中时：

1. CQ 码从消息文本中移除。
2. 消息类型切换为 `msg_type=8`，发送图文卡片。
3. QQ 群聊中将展示一张带标题、描述、图片和跳转链接的卡片。
4. 如果同一消息同时包含 `[CQ:markdown]`，markdown 优先级更高（卡片不会生效）。

## 使用示例

NoneBot 插件发送群聊卡片：

```python
from nonebot.adapters.onebot.v11 import Message

card_msg = Message(
    "[CQ:card,title=今日推荐,desc=点击查看详情,pic=https://example.com/cover.png,url=https://example.com]"
)
await bot.send_group_msg(
    group_id=821404315,
    message=card_msg,
)
```

## 注意

- 卡片消息仅支持群聊场景，单聊和频道不支持。
- `pic` 需要使用公网可访问的图片 URL。
- 消息中同时携带 `[CQ:markdown]` 时，markdown 优先生效。
- `[CQ:card]` 不会与文本内容拼接，发送卡片时文本内容会被清空。
