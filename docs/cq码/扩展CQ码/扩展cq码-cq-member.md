# [CQ:member]

## 用途

`[CQ:member]` 用于 q群成员入群/退群事件和后续回复路由。`group_id`、`user_id` 都是 Gensokyo 映射后的虚拟 ID。

```text
[CQ:member,type=add/remove,group_id=<虚拟群ID>,user_id=<虚拟用户ID>]
```

| 参数 | 说明 |
|------|------|
| `type` | `add` 表示入群，`remove` 表示退群。 |
| `group_id` | 虚拟 q群 ID。 |
| `user_id` | 虚拟用户 ID。 |

范围：`q群 (Group Chat)`

## 入站事件

启用以下 handler 后，Gensokyo 会把 QQ 群成员事件转为 OneBot V11 notice：

```yaml
text_intent:
  - "GroupMemberAddEventHandler"
  - "GroupMemberRemoveEventHandler"
```

| QQ 事件 | OneBot 事件 | `sub_type` | `message` |
|---------|-------------|------------|-----------|
| `GROUP_MEMBER_ADD` | `notice.group_increase` | `approve` | `[CQ:member,type=add,...]` |
| `GROUP_MEMBER_REMOVE` | `notice.group_decrease` | `leave` | `[CQ:member,type=remove,...]` |

入群事件会保存 `event_id`，用于后续被动回复。退群事件没有可用 `event_id`，后续发送会走主动消息。

## 出站回复

后端收到 notice 后，使用普通 `send_group_msg` 回复，并把原 `message` 中的 `[CQ:member]` 放回消息开头：

```text
[CQ:member,type=add,group_id=821404315,user_id=3607918353]欢迎入群
```

发送时 Gensokyo 会：

1. 移除 `[CQ:member]`。
2. 将虚拟 `group_id`、`user_id` 反查为 OpenID。
3. `type=add` 时尝试使用入群事件保存的 `event_id`。
4. `type=remove` 时清空 `event_id`，按主动消息发送。

## nonebot2 示例

```python
from nonebot import on_notice
from nonebot.adapters.onebot.v11 import Bot, GroupIncreaseNoticeEvent, GroupDecreaseNoticeEvent

member = on_notice(priority=1, block=False)

@member.handle()
async def handle_member(bot: Bot, event: GroupIncreaseNoticeEvent | GroupDecreaseNoticeEvent):
    cq = getattr(event, "message", "") or ""
    if isinstance(event, GroupIncreaseNoticeEvent):
        await bot.send_group_msg(group_id=event.group_id, message=f"{cq}欢迎入群")
    else:
        await bot.send_group_msg(group_id=event.group_id, message=f"{cq}离开了我们")
```
