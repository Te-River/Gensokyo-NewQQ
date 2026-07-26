# [CQ:input_notify]

## 用途

`[CQ:input_notify]` 用于在单聊（C2C）消息发送前，先向用户展示"正在输入"的状态通知（`msg_type=6`）。适合需要等待后端处理较长时间的场景，给用户即时反馈。

范围：`私聊 (C2C)`

## 语法

```text
[CQ:input_notify,type=<类型>,second=<秒数>]
```

| 参数 | 必填 | 说明 |
|------|------|------|
| `type` | 否 | 输入类型，默认 `1` |
| `second` | 否 | 状态持续时间（秒），最长 `60`，默认 `60` |

## 解析行为

- 从消息文本中提取 `[CQ:input_notify,...]` 并移除，最终用户不会看到该 CQ 码。
- 数组消息段的 `"type":"input_notify"` 暂不支持，仅支持字符串格式。
- 参数有效范围：`type` 固定为 `1`，`second` 为 `1~60`。

## 发送行为

当 `[CQ:input_notify]` 出现在 `send_private_msg` 的出站消息中时：

1. CQ 码从消息文本中移除。
2. **在正文发送之前**，先向 QQ API 发送一条 `msg_type=6` 的输入状态通知。
3. 用户端会看到"正在输入..."的提示。
4. 随后发送正文消息（正文内容不变）。
5. 输入状态超时后自动消失，无需手动关闭。

## 使用示例

NoneBot 插件发送输入状态通知：

```python
from nonebot.adapters.onebot.v11 import Message

# 先展示"正在输入"，再发送实际内容
msg = Message("[CQ:input_notify,type=1,second=30] 正在为您查询，请稍候...")
await bot.send_private_msg(
    user_id=123456,
    message=msg,
)
```

用于耗时操作（如 AI 生成）前给用户反馈：

```python
# 在消息开头标记输入状态
await bot.send_private_msg(
    user_id=user_id,
    message=Message(
        "[CQ:input_notify]"
        "已收到您的请求，正在生成回答..."
    ),
)
```

## 注意

- 输入状态通知**仅支持单聊（C2C）场景**，群聊和频道不支持。
- `second` 最大值为 `60`，超过会被截断。
- 输入状态通知只是 UI 展示效果，不影响实际消息的发送逻辑。
- `[CQ:input_notify]` 本身不会发送实际消息，正文仍需在 CQ 码之后提供。
- 多条消息连续发送时，只需在第一条消息前带上此 CQ 码即可。
