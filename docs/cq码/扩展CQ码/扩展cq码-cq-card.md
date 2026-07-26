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
| `pic` | 否 | 卡片图片。支持公网 URL 或本地文件路径，本地路径会自动通过 OSS 图床上传为 CDN 链接 |
| `url` | 否 | 点击卡片跳转的链接（与 `pic` 同为 QQ API 必填字段，至少传一个） |

## 解析行为

- 从消息文本中提取 `[CQ:card,...]` 并移除，最终用户不会看到该 CQ 码。
- 参数顺序无关，内部自动解析为 `key=value` 键值对。
- `pic` 和 `url` 同时为空时卡片会被跳过（QQ API 要求两者至少传一个）。
- 数组消息段格式（`{"type":"card","data":{"title":"...","desc":"..."}}`）同样支持。

## 发送行为

当 `[CQ:card]` 出现在 `send_group_msg` 的出站消息中时：

1. CQ 码从消息文本中移除。
2. 消息类型切换为 `msg_type=8`，发送图文卡片。
3. QQ 群聊中将展示一张带标题、描述、图片和跳转链接的卡片。
4. 如果同一消息同时包含 `[CQ:markdown]`，markdown 优先级更高（卡片不会生效）。
5. `pic` 为本地路径时自动通过当前配置的 OSS 图床（`oss_type`）上传，替换为 CDN 链接。

## 使用示例

NoneBot 插件发送群聊卡片（使用 URL）：

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

NoneBot 插件发送群聊卡片（使用本地路径，自动 OSS 上传）：

```python
card_msg = Message(
    "[CQ:card,title=今日推荐,desc=点击查看详情,pic=D:/images/cover.png,url=https://example.com]"
)
await bot.send_group_msg(
    group_id=821404315,
    message=card_msg,
)
```

消息段格式（适合 NoneBot 的 MessageSegment 构造方式）：

```python
from nonebot.adapters.onebot.v11 import Message, MessageSegment

card_msg = Message([
    MessageSegment("card", {
        "title": "今日推荐",
        "desc": "点击查看详情",
        "pic": "https://example.com/cover.png",
        "url": "https://example.com",
    })
])
await bot.send_group_msg(group_id=821404315, message=card_msg)
```

## 注意

- 卡片消息仅支持群聊场景，单聊和频道不支持。
- `pic` 和 `url` 至少需要传一个，否则卡片会被跳过。
- `pic` 支持公网 URL 和本地文件路径。本地路径会自动通过 `oss_type` 配置的图床上传。
- 消息中同时携带 `[CQ:markdown]` 时，markdown 优先生效。
- `[CQ:card]` 不会与文本内容拼接，发送卡片时文本内容会被清空。
