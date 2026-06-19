# [CQ:active] — 主动推送标记

## 说明

用于标记消息为**主动推送模式**。当后端在群聊或私聊中发送主动消息时，加入此 CQ 码，Gensokyo 收到后自动清空 `msg_id`/`event_id`，不走被动回复逻辑。

## 格式

```
[CQ:active]
```

无参数。直接放在消息文本中任意位置即可。

## 用法

### 群主动推送

后端调用 `send_group_msg` 时在消息中嵌入：

```
[CQ:active]大家好，这是一条主动推送的消息
```

Gensokyo 行为：
1. 通过 `parseMessageContent` 解析并移除 `[CQ:active]`
2. 清空 `msg_id`（避免使用过期的被动回复消息 ID）
3. 发送时不含 `msg_id`、`event_id`，走群聊主动消息通道

### C2C 唤醒私聊

与 `send_private_msg_wakeup` 配合使用（自动通过插件 `active_msg()` 函数添加）：

```python
def active_msg(text: str) -> Message:
    return Message(segs)
```

## 注意事项

- `[CQ:active]` 仅标记消息模式，不含消息内容
- 群聊主动推送需要群管理员已在机器人资料页开启"群消息推送"
- 使用主动模式时，公域机器人每月仅有 4 次主动推送额度

## 适用范围

🌐 全场景（群聊 + 频道 + C2C）
