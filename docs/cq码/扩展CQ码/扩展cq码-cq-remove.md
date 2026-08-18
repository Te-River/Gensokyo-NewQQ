# [CQ:remove]

## 用途

`[CQ:remove]` 用于在出站 `send_group_msg` 消息中撤回指定群消息。必须同时提供 `user_id` 和 `msg_id`，缺少任一参数时该 CQ 码不做任何处理。

```text
[CQ:remove,user_id=<虚拟用户ID>,msg_id=<虚拟消息ID>]
```

| 参数 | 必填 | 说明 |
|------|------|------|
| `user_id` | 是 | 虚拟用户 ID |
| `msg_id` | 是 | 虚拟消息 ID（Gensokyo 分配的整数 ID） |

范围：`q群 (Group Chat)`

## 出站发送

在 `send_group_msg` 的消息内容中携带 `[CQ:remove]`：

```text
[CQ:remove,user_id=791838020,msg_id=1823]
```

发送时 Gensokyo 会：

1. 从消息文本中移除 `[CQ:remove]` CQ 码
2. 将 `user_id` 虚拟 ID 反查为 OpenID，将 `msg_id` 虚拟 ID 反查为 QQ 官方消息 ID
3. 调用 QQ API `RetractGroupMessage` 撤回该消息
4. 如果消息正文只有 `[CQ:remove]`（无其他内容），则不发送文本到 QQ，仅执行撤回并返回客户端回执

## 参数说明

- **`user_id`**：消息发送者的虚拟 ID。必须是此前已记录在 idmap 中的有效映射。
- **`msg_id`**：消息的虚拟 ID（整数）。由 Gensokyo 在收到消息时自动分配，接收端可以从 `message_id` 字段获取。

## 限制

- 仅支持出站（outbound）场景，通过 `send_group_msg` 发送。没有入站 `[CQ:remove]`。
- 需要同时提供 `user_id` 和 `msg_id`。不支持仅提供 `user_id` 自动撤回最新一条消息（该功能由 `delete_group_msg` API action 提供）。
- 范围仅限 `q群 (Group Chat)`。

## nonebot2 示例

```python
# 在 send_group_msg 的消息中嵌入 [CQ:remove]
# user_id 和 msg_id 来自收到消息时 event 中的字段

from nonebot.adapters.onebot.v11 import Bot, GroupMessageEvent

async def remove_message(bot: Bot, event: GroupMessageEvent, target_user_id: int, target_msg_id: int):
    await bot.send_group_msg(
        group_id=event.group_id,
        message=f"[CQ:remove,user_id={target_user_id},msg_id={target_msg_id}]"
    )
```
