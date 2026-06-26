# `[CQ:remove]`

范围：`q群 (Group Chat)`（出站）

撤回目标用户在当前群的消息。未传 `message_id` 时撤回该用户最后一条仍在缓存期内的消息；传入时撤回指定消息。

## 格式

```text
[CQ:remove,user_id=<用户ID>]
[CQ:remove,user_id=<用户ID>,message_id=<消息ID>]
```

| 参数 | 类型 | 必填 | 说明 |
|------|------|:--:|------|
| `user_id` | string | 是 | 目标用户 ID。默认自动解析 vUIN；OpenID 可直接传入。 |
| `message_id` | string | 否 | 指定消息 ID。默认自动解析虚拟消息 ID；真实消息 ID 可直接传入。 |
| `msg_id` | string | 否 | `message_id` 的兼容别名。 |

未指定 `message_id` 时，Gensokyo 使用接收群消息时维护的 `(群 OpenID, 用户 OpenID)` 索引查找最后一条消息。该索引与消息 ID 缓存使用相同的 `msgid_ttl_seconds`。

## 限制

| 限制 | 说明 |
|------|------|
| 时效 | 只能查找消息缓存有效期内的最后一条消息，默认 1 小时。 |
| 范围 | 仅群聊 |
| 权限 | 需机器人为群管理员 |

## 示例

```python
@on_command("撤回").handle()
async def recall_msg(bot: Bot, event: GroupMessageEvent):
    await bot.send_group_msg(
        group_id=event.group_id,
        message=f"[CQ:remove,user_id={event.user_id}]",
    )
```

需要指定消息时，将事件的虚拟消息 ID 作为 `message_id` 传入：

```text
[CQ:remove,user_id=3607918353,message_id=5678]
```
