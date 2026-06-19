# [CQ:at] — Markdown 消息中的 @ 支持

## 说明

标准 OneBot 的 `[CQ:at,qq=虚拟ID]` 在普通文本消息中会转换为 `<qqbot-at-user>` 标签放在 `content` 字段。但在 **Markdown 消息**（`msg_type=2`）中，`content` 字段被忽略，@ 需要嵌入到 `markdown.content` 内。

Gensokyo 自动处理这一转换——当消息同时包含 `[CQ:at]` 和 `[CQ:markdown]` 时，自动将 @ 标签合并到 markdown 内容头部。

## 格式

```
[CQ:at,qq=虚拟用户ID]
```

| 参数 | 类型 | 说明 |
|------|------|------|
| `qq` | int64 | Gensokyo 转化的虚拟用户 ID |

## 行为

当消息**不含** `[CQ:markdown]` 时，`[CQ:at]` 行为与标准 OneBot 一致，转换为 `<qqbot-at-user>` 标签放在 `content` 字段。

当消息**同时包含** `[CQ:at]` 和 `[CQ:markdown]` 时，Gensokyo 将 @ 标签提取并嵌入到 markdown 内容头部，消息以 `msg_type=2` 发送：

```
发送: [CQ:at,qq=3607918353][CQ:markdown,data=base64://...]
  ↓
实际请求:
{
  "msg_type": 2,
  "markdown": {
    "content": "<qqbot-at-user id=\"真实OPENID\" />\n## 欢迎消息..."
  }
}
```

## 适用范围

🏷️ 群聊
