# [CQ:share]

## 用途

`[CQ:share]` 用于发送链接分享卡片，包含标题、内容摘要、图片和链接。

范围：`-`（QQ Bot API 暂不支持发送）

## 语法

```text
[CQ:share,url=https://example.com,title=标题,content=摘要,image=https://example.com/pic.png]
```

## 参数

| 参数名 | 必填 | 说明 |
|--------|------|------|
| `url` | ✅ | 分享的链接 |
| `title` | ✅ | 标题 |
| `content` | ❌ | 摘要内容 |
| `image` | ❌ | 图片 URL |

## 与标准 OneBot V11 的差异

QQ Bot API（官方 API）**不提供**发送链接分享卡片的接口。`[CQ:share]` 在 Gensokyo 中**无法发送**。

| 差异点 | 标准 OneBot V11 | Gensokyo 实现 |
|--------|----------------|---------------|
| 发送分享 | ✅ 支持 | ❌ QQ Bot API 不支持 |
| 入站上报 | ✅ 收到分享时上报 | ❌ QQ Bot 不推送分享消息 |

## 发送行为

不支持。如果消息中包含 `[CQ:share]`，该 CQ 码会被忽略或作为文本原样发出。

## 替代方案

如需在消息中分享链接，可直接将 URL 作为文本发送。配合 `transfer_url=true` 配置，URL 会自动转换为短链。

```python
await bot.send_group_msg(
    group_id=123456,
    message="推荐一篇文章：https://example.com/article",
)
```

## 注意

- QQ Bot API 与 go-cqhttp 等第三方实现不同，不支持分享卡片功能。
- 如需富媒体卡片效果，可考虑使用扩展 CQ 码 `[CQ:card]`（仅群聊，msg_type=8）。
