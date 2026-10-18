# `get_group_bot_state`

> 🧪 **测试插件**：[gsk-group_bot_state](https://github.com/Te-River/Gensokyo-NewQQ-Test-plugins/tree/main/gsk-group_bot_state) — 发送「群状态」体验

范围：`q群 (Group Chat)`

获取机器人在指定群内的状态（QQ 官方 v2 接口 `GET /v2/groups/{group_openid}/bot_state`，2026-08 文档），包括入群时间、是否接收主动推送、接受消息设置与群内角色。

## 参数

| 参数 | 类型 | 必填 | 说明 |
|------|------|:----:|------|
| `group_id` | string/int | 是 | 群虚拟 ID 或实际 Group OpenID。 |

## 返回

`data` 字段（**兼容双写**：旧字段与新字段同时返回，值均取自官方新字段）：

| 字段 | 类型 | 说明 |
|------|------|------|
| `group_id` | int | 群虚拟 ID |
| `joined_at` | string | 机器人入群时间戳（RFC3339，官方字段原样透传） |
| `allow_proactive_msg` | bool | 是否接收主动推送（官方新字段） |
| `recv_msg_setting` | string | 接受消息的类型：`all` / `only_mention` / `mention_and_context` |
| `member_role` | string | 机器人群内角色：`member` / `owner` / `admin` |
| `member_openid` | string | 机器人的 OpenID（官方 `member_openid` 原样透传） |
| `join_time` | int | **旧字段（兼容保留）**：入群时间（秒级时间戳），由 `joined_at` 解析得到，解析失败为 `0` |
| `can_push` | bool | **旧字段（兼容保留）**：等同 `allow_proactive_msg` |
| `push_msg_setting` | string | **旧字段（兼容保留）**：等同 `recv_msg_setting` |
| `role` | string | **旧字段（兼容保留）**：等同 `member_role` |

> 旧字段名（`join_time` / `can_push` / `push_msg_setting` / `role`）与官方现文档不一致，仅为下游插件兼容保留；新代码建议使用官方字段名。

错误码：`11253` 应用无接口访问权限（该接口仅白名单机器人可用）。

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
  "joined_at": "2026-08-05T14:19:09+08:00",
  "allow_proactive_msg": true,
  "recv_msg_setting": "all",
  "member_role": "admin",
  "member_openid": "A1B2C3D4E5F6A1B2C3D4E5F6A1B2C3D4",
  "join_time": 1785910749,
  "can_push": true,
  "push_msg_setting": "all",
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
    await event.finish(
        f"机器人在本群角色：{data['member_role']}"
        f"（兼容别名 {data['role']}），入群于 {data['joined_at']}"
    )
```
