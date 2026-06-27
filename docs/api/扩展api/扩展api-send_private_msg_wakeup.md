# send_private_msg_wakeup

## 说明

向用户发送 C2C 召回消息。发送时会设置 `is_wakeup=true`。

范围：`私聊 (C2C)`

## 请求参数

与 `send_private_msg` 一致：

| 参数 | 类型 | 说明 |
|------|------|------|
| `user_id` | string | 目标用户 OpenID；如果不是 32 位 OpenID，会先尝试按虚拟用户 ID 反查。 |
| `message` | array/string | 消息内容，支持文本、图片、Markdown 等 |

无法反查为 32 位 OpenID 时，请求会停止发送。

## 返回方式

推送一个伪造的 `notice` 事件：

```json
{
    "post_type": "notice",
    "notice_type": "c2c_wakeup_resp",
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
        user_id="目标用户OpenID或虚拟用户ID",
        message="这是一条 C2C 召回消息"
    )
```
