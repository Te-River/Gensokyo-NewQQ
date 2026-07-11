# [CQ:active]

## 用途

`[CQ:active]` 用于标记出站消息为主动推送模式。当消息内容中包含此 CQ 码时，Gensokyo 会强制走主动消息通道（清空缓存的 `msg_id` 和 `event_id`），即使有可用的被动回复上下文。

范围：`-`

## 语法

支持两种格式：

```text
[CQ:active]                          // 裸标记：无参数
[CQ:active,type=<值>,sub_type=<值>]  // 带参数
```

无论哪种格式，`[CQ:active]` 都会从消息文本中移除，最终用户不会看到。

## 解析行为

- `[CQ:active]`（裸标记）→ 识别为主动标记，从文本中移除。
- `[CQ:active,type=xxx,sub_type=yyy]`（带参数）→ 同上，且 `type` 写入 `foundItems["active_type"]`，`sub_type` 写入 `foundItems["active_sub_type"]`。
- 数组消息段的 `"type":"active"` 同样支持。

## 发送行为

当 `[CQ:active]` 出现在出站消息中时：

1. CQ 码从消息文本中移除。
2. 清空缓存的 `msg_id`（如果有），强制走主动推送通道。
3. 清空 `event_id`（如果有），避免消耗被动回复次数。

## 使用示例

NoneBot 插件构造主动群推送：

```python
from nonebot.adapters.onebot.v11 import Message, MessageSegment

def active_msg(text: str) -> Message:
    segs = [MessageSegment.text("[CQ:active]")]
    if text:
        segs.append(MessageSegment.text(text))
    return Message(segs)

# 主动推送到指定群
await bot.send_group_msg(
    group_id=821404315,
    message=active_msg("服务器将在十分钟后维护"),
)
```

C2C 唤醒消息：

```python
await bot.call_api(
    "send_private_msg_wakeup",
    user_id="<OpenID>",
    message=active_msg("你好，这是一条唤醒消息"),
)
```
{"type":"active","data":{"type":"<值>","sub_type":"<值>"}}
```

## 注意

裸 `[CQ:active]` 当前不会被文本正则匹配。需要 C2C 召回消息时，使用 [`send_private_msg_wakeup`](../../api/扩展api/扩展api-send_private_msg_wakeup.md)。

范围：`-`
