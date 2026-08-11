# `get_group_bot_state`

范围：`q群 (Group Chat)`

获取机器人在指定群内的状态（QQ 官方 v2 接口 `GET /v2/groups/{group_openid}/bot_state`），包括入群时间、是否可主动推送、消息推送设置与群内角色。

## 参数

| 参数 | 类型 | 必填 | 说明 |
|------|------|:----:|------|
| `group_id` | string/int | 是 | 群虚拟 ID 或实际 Group OpenID。 |

## 返回

`data` 字段：

| 字段 | 类型 | 说明 |
|------|------|------|
| `group_id` | int | 群虚拟 ID |
| `join_time` | int | 机器人入群时间（秒级时间戳） |
| `can_push` | bool | 是否可接收主动消息推送 |
| `push_msg_setting` | string | 推送设置（如 `notify` / `silence` 等，以 QQ 返回为准） |
| `role` | string | 机器人群内角色（`owner` / `admin` / `member`） |

## 示例

```json
{
  "action": "get_group_bot_state",
  "params": {
    "group_id": 870389197
  }
}
```

响应 `data`：

```json
{
  "group_id": 870389197,
  "join_time": 1720000000,
  "can_push": true,
  "push_msg_setting": "notify",
  "role": "admin"
}
```

## nonebot2 示例

```python
from nonebot import on_command
from nonebot.adapters.onebot.v11 import Bot, Event

@on_command("bot_role").handle()
async def _(bot: Bot, event: Event):
    data = await bot.call_api("get_group_bot_state", group_id=event.group_id)
    await event.finish(f"机器人在本群角色：{data['role']}，可推送：{data['can_push']}")
```
