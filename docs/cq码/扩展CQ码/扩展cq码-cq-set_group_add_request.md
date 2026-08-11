# [CQ:set_group_add_request]

## 用途

出站单向动作 CQ 码：在发送群消息时审批入群申请，执行后该 CQ 码从文本中移除，不会发送到群里。`flag` 为入站 `request` 事件（或 `get_group_join_request_list`）返回的申请 ID。机器人需拥有群管理员身份。

```text
[CQ:set_group_add_request,group_id=<虚拟群ID>,user_id=<虚拟用户ID>,flag=<申请ID>,approve=<true/false>]
```

| 参数 | 必填 | 说明 |
|------|:----:|------|
| `group_id` | 否 | 虚拟 q群 ID；省略时使用消息发送的目标群（支持跨群路由）。 |
| `user_id` | 是 | 申请者虚拟用户 ID 或真实 OpenID。 |
| `flag` | 是 | 申请 ID（`join_request_id`），来自入站 request 事件或申请列表。 |
| `approve` | 是 | `true` 通过，`false` 拒绝（支持 `1`/`0`）。 |
| `reason` | 否 | 拒绝理由（`approve=false` 时生效）。 |
| `add_to_member_blacklist` | 否 | `true` 同时加入群黑名单（`approve=false` 时生效）。 |

范围：`q群 (Group Chat)`

## 行为

- `approve=true` 映射为 `op=approve`，`approve=false` 映射为 `op=decline`。
- 必填参数缺失或 `approve` 无效时：CQ 码原样保留在文本中。
- 执行失败仅记录日志，不阻断其余文本发送。

## 示例

```text
[CQ:set_group_add_request,user_id=3607918353,flag=AURi8Rr6MfGdUNedupWf2uV5Xiay...,approve=true] 欢迎入群
```

拒绝并拉黑：

```text
[CQ:set_group_add_request,user_id=3607918353,flag=AURi8Rr6MfGdUNedupWf2uV5Xiay...,approve=false,reason=广告勿扰,add_to_member_blacklist=true] 已拒绝
```

## nonebot2 示例

配合入群申请事件自动审批：

```python
from nonebot import on_request
from nonebot.adapters.onebot.v11 import GroupRequestEvent

join_req = on_request(priority=1, block=False)

@join_req.handle()
async def handle_join(bot, event: GroupRequestEvent):
    if event.sub_type == "add" and "学习" in (event.comment or ""):
        # 自动通过并欢迎
        await bot.send_group_msg(group_id=event.group_id,
            message=f"[CQ:set_group_add_request,user_id={event.user_id},flag={event.flag},approve=true] 欢迎入群")
```
