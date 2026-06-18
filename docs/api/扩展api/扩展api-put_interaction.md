# 处理按钮交互回调

## 说明

用于回复按钮交互（Button Interaction），向 QQ API 返回交互结果。

## 请求参数

| 参数 | 类型 | 说明 |
|------|------|------|
| `echo` | string | 按钮回调事件中的 `echo` 值，即 `interaction_id` |
| `post_type` | string | 操作结果码：`"0"`=成功、`"1"`=操作失败、`"2"`=操作频繁、`"3"`=重复操作、`"4"`=没有权限、`"5"`=仅管理员操作 |

## 响应

```json
{
    "data": "",
    "message": "",
    "retcode": 0,
    "status": "ok",
    "echo": "原echo值"
}
```

## nonebot2 示例

```python
from nonebot import on_command
from nonebot.adapters.onebot.v11 import Bot, Event

@on_command("reply_interaction").handle()
async def _(bot: Bot, event: Event):
    await bot.call_api(
        "put_interaction",
        echo="回调中的echo值",
        post_type="0"
    )
```

## 按钮回调事件获取 echo

按钮被点击时，Gensokyo 会推送一个消息事件，其中 `echo` 字段即为 `interaction_id`，直接传给 `put_interaction` 即可。
