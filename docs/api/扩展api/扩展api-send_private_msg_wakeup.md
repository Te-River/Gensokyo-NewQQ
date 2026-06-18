# 发送被动唤醒私聊消息

## 说明

用于向用户发送**互动召回消息**（被动唤醒私聊）。QQ 官方 API 中，机器人向未主动发过消息的用户发私聊需要标记 `IsWakeup`。

## 请求参数

与 `send_private_msg` 一致：

| 参数 | 类型 | 说明 |
|------|------|------|
| `user_id` | string | 目标用户的 **32 位 OpenID**（注意：不是虚拟数字 ID） |
| `message` | array/string | 消息内容，支持文本、图片、Markdown 等 |

> ⚠️ `user_id` 必须是 QQ 原生的 32 位 OpenID 字符串，不能使用虚拟数字 ID。

## 返回方式

推送一个伪造的 `notice` 事件：

```json
{
    "post_type": "notice",
    "notice_type": "wakeup_response",
    "user_id": 虚拟数字ID,
    "real_user_id": "32位OpenID",
    "status": "success",
    "message_id": "xxx",
    "error_msg": "",
    "self_id": 123456,
    "time": 1700000000
}
```

## nonebot2 示例

```python
from nonebot import on_command
from nonebot.adapters.onebot.v11 import Bot, Event

@on_command("wakeup").handle()
async def _(bot: Bot, event: Event):
    await bot.call_api(
        "send_private_msg_wakeup",
        user_id="目标用户的32位OpenID",
        message="这是一条被动唤醒消息"
    )
```
