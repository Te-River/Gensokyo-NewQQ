# [CQ:member] — 群成员变动

## 说明

用于标记群成员入群/退群事件的 CQ 码。`group_id` 和 `user_id` 均为 Gensokyo 对 OpenID 转换后的虚拟 ID。

整个流程对后端完全透明——入站是普通 `message.group.normal` 事件，出站是普通 `send_group_msg`，后端无需特殊处理。

## 格式

```
[CQ:member,type=add/remove,group_id=虚拟群ID,user_id=虚拟用户ID]
```

| 参数 | 类型 | 说明 |
|------|------|------|
| `type` | string | `add` = 成员入群，`remove` = 成员离群 |
| `group_id` | int64 | Gensokyo 转化的虚拟群 ID |
| `user_id` | int64 | Gensokyo 转化的虚拟用户 ID |

## 流程

### type=add（成员入群）

```
① Gsk 捕获事件 → 推送普通消息事件，message 为 CQ 码
   [message.group.normal]: [CQ:member,type=add,group_id=821404315,user_id=3607918353]

② 后端收到消息事件，解析 CQ 码 → 用普通 send_group_msg 回复
   send_group_msg(group_id=821404315, message="[CQ:member,type=add,group_id=821404315,user_id=3607918353]欢迎入群！")

③ Gsk 收到消息 → 解析 CQ 码，group_id 转 GroupOpenID 确定目标群
   user_id 转 OpenID，用 event_id 被动回复，发送"欢迎入群！"
```

### type=remove（成员退群）

```
① Gsk 捕获事件 → 推送普通消息事件，message 为 CQ 码
   [message.group.normal]: [CQ:member,type=remove,group_id=821404315,user_id=3607918353]

② 后端收到消息事件 → 用普通 send_group_msg 回复
   send_group_msg(group_id=821404315, message="[CQ:member,type=remove,group_id=821404315,user_id=3607918353]离开了呢")

③ Gsk 收到消息 → 解析 CQ 码，group_id 转 GroupOpenID 确定目标群
   user_id 转 OpenID，无 event_id，直接主动消息发送"离开了呢"
```

## 后端示例（nonebot2）

```python
from nonebot import on_message
from nonebot.adapters.onebot.v11 import Bot, GroupMessageEvent, Message

@on_message().handle()
async def handle_member_cq(bot: Bot, event: GroupMessageEvent):
    # 判断是否为 [CQ:member] 消息
    if not event.raw_message.startswith("[CQ:member"):
        return

    # 解析 CQ 码
    import re
    match = re.search(r'type=(\w+)', event.raw_message)
    cq_type = match.group(1)  # 'add' 或 'remove'

    if cq_type == "add":
        # 入群欢迎： [CQ:member] + [CQ:at] + [CQ:markdown] 或文本
        # [CQ:at] 在 markdown 消息下自动合并到内容（详见 [CQ:at] Markdown 文档）
        reply_msg = Message(
            f"{event.raw_message}"
            f"[CQ:at,qq={event.user_id}]"
            f"[CQ:markdown,data=<base64>]"  # 替换为实际 markdown
        )
    elif cq_type == "remove":
        # 退群通知： [CQ:member] + 文本
        reply_msg = Message(f"{event.raw_message}离开了我们")
    else:
        return

    await bot.send_group_msg(group_id=event.group_id, message=reply_msg)
```

## 配置

需在 `config.yml` 的 `text_intent` 中启用：

```yaml
text_intent:
  - "GroupMemberAddEventHandler"
  - "GroupMemberRemoveEventHandler"
```

## 适用范围

🏷️ 群聊
