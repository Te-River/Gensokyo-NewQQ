# [CQ:wakeup]

> 🧪 **测试插件**：[gsk-wakeup](https://github.com/Te-River/Gensokyo-NewQQ-Test-plugins/tree/main/gsk-wakeup) — 发送「召回测试」体验

## 用途

`[CQ:wakeup,userid=<目标用户>]` 用于在出站私聊消息中标记 **C2C 互动召回（唤醒）消息**。当消息内容中包含此 CQ 码时，Gensokyo 会将这条消息作为召回消息（`is_wakeup=true`）发送给 CQ 码指定的目标用户，而不是发送给 `send_private_msg` 的 `user_id` 参数指向的用户。

范围：`私聊 (C2C)`

## 语法

```text
[CQ:wakeup,userid=<目标用户>]
```

- `<目标用户>`：目标用户的 **32 位 OpenID** 或 **虚拟数字 ID**（虚拟 ID 会自动转换为真实 OpenID）。
- `[CQ:wakeup]` 会从消息文本中移除，最终用户不会看到。

## 与 [CQ:active] 的区别

| CQ 码 | 行为 | 适用 |
|-------|------|------|
| `[CQ:active]` | 清空 `msg_id` / `event_id`，强制走**主动推送**通道（不设置 `is_wakeup`） | 群聊 / 私聊 |
| `[CQ:wakeup]` | 设置 `is_wakeup=true`，作为 **C2C 召回消息**发送给指定用户 | 私聊 (C2C) |

## 解析行为

- 字符串形式 `[CQ:wakeup,userid=xxx]` → `userid` 写入 `foundItems["wakeup"]`，CQ 码从文本中移除。
- 数组消息段的 `"type":"wakeup"` 同样支持（`data.userid`）。
- TRSS 格式 `{"type":"wakeup","data":{"userid":"xxx"}}` 同样支持。

## 发送行为

当 `[CQ:wakeup,userid=xxx]` 出现在 `send_private_msg` 出站消息中时：

1. CQ 码从消息文本中移除，`userid` 存入 `foundItems["wakeup"]`。
2. `send_private_msg` 检测到 `wakeup` 标记，将目标用户覆盖为 CQ 码指定的用户。
3. 消息按召回通道发送：`IsWakeup=true`、`MsgID=""`、`EventID=""`（互斥），支持纯文本 / 图文混合 / Markdown / 富媒体。
4. 发送结果以 `notice` 类型（`notice_type=c2c_wakeup_resp`）推送给应用端。

> ⚠️ 群聊消息（`send_group_msg`）中出现的 `[CQ:wakeup]` 会被跳过，不会发送（召回仅支持 C2C）。

## 使用示例

NoneBot 插件向指定用户发送召回消息：

```python
from nonebot.adapters.onebot.v11 import Message, MessageSegment

def wakeup_msg(user_id: str, text: str) -> Message:
    segs = [MessageSegment.text(f"[CQ:wakeup,userid={user_id}]")]
    if text:
        segs.append(MessageSegment.text(text))
    return Message(segs)

# 向指定用户发送 C2C 召回消息
await bot.send_private_msg(
    user_id="<虚拟用户ID>",  # 会被 [CQ:wakeup] 中的目标用户覆盖
    message=wakeup_msg("0123456789abcdef0123456789abcdef", "你好，这是一条召回消息"),
)
```

数组消息段格式：

```json
{"type":"wakeup","data":{"userid":"0123456789abcdef0123456789abcdef"}}
```

## 注意

- `[CQ:wakeup]` 的目标用户若为虚拟数字 ID，会先通过 idmap 转换为真实 OpenID；无法转换时消息发送失败。
- 也可以直接使用独立 API [`send_private_msg_wakeup`](../../api/扩展api/扩展api-send_private_msg_wakeup.md) 发送召回消息，两者效果等价。

范围：`私聊 (C2C)`
