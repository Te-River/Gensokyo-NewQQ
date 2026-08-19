# [CQ:forward]

## 用途

`[CQ:forward]` 用于发送合并转发消息，将多条消息折叠为一条转发消息。

范围：`-`（QQ Bot API 暂不支持）

## 语法

```text
[CQ:forward,id=合并转发ID]
```

## 参数

| 参数名 | 必填 | 说明 |
|--------|------|------|
| `id` | ✅ | 合并转发消息 ID（由 `send_forward_msg` 等接口返回） |

## 与标准 OneBot V11 的差异

QQ Bot API（官方 API）**不提供**合并转发消息的接口。`[CQ:forward]` 在 Gensokyo 中**无法发送**。

| 差异点 | 标准 OneBot V11 | Gensokyo 实现 |
|--------|----------------|---------------|
| 发送合并转发 | ✅ 支持 | ❌ QQ Bot API 不支持 |
| 入站上报 | ✅ 收到转发时上报 | ❌ QQ Bot 不推送转发消息 |
| `[CQ:node]` 节点 | ✅ 支持 | ❌ 不支持 |

## 发送行为

不支持。如果消息中包含 `[CQ:forward]`，该 CQ 码会被忽略。

## 替代方案

如需发送多条消息，可逐条发送：

```python
for msg in messages:
    await bot.send_group_msg(group_id=123456, message=msg)
```

如需发送折叠消息，可考虑使用扩展 CQ 码 `[CQ:card]`（群聊图文卡片，msg_type=8）将内容整合为一条卡片消息。

## 注意

- QQ Bot API 与 go-cqhttp 等第三方实现不同，不支持合并转发功能。
- `forward_msg_limit` 配置项（默认 3）控制的是 SSM（Send Stack Messages）补发队列的折叠阈值，与合并转发消息无关。
