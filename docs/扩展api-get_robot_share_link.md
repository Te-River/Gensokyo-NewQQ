# 获取机器人分享链接

## 说明

调用 QQ API 生成机器人资料页的分享链接，结果通过 `notice` 事件推送回客户端。

## 请求参数

| 参数 | 类型 | 说明 |
|------|------|------|
| `callback_data` | string | 透传参数，会在返回的 notice 中带回，用于区分请求 |

## 返回方式

不通过标准 API 响应返回，而是推送一个伪造的 `notice` 事件：

```json
{
    "post_type": "notice",
    "notice_type": "share_link_generated",
    "url": "https://qun.qq.com/...",
    "callback_data": "你传的透传参数",
    "self_id": 123456,
    "time": 1700000000
}
```

## nonebot2 示例

```python
from nonebot import on_command
from nonebot.adapters.onebot.v11 import Bot, Event

@on_command("share_link").handle()
async def _(bot: Bot, event: Event):
    await bot.call_api("get_robot_share_link", callback_data="my_link_1")
```

在 bot 的 `notice` 事件处理器中接收结果：

```python
from nonebot import on_notice
from nonebot.adapters.onebot.v11 import PokeNotifyEvent

@on_notice().handle()
async def _(event: PokeNotifyEvent):
    if event.notice_type == "share_link_generated":
        url = event.url
        await event.finish(f"分享链接已生成：{url}")
```
