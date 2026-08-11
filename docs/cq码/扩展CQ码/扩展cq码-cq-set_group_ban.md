# [CQ:set_group_ban]

## 用途

出站单向动作 CQ 码：在发送群消息时执行成员禁言/解禁操作，执行后该 CQ 码从文本中移除，不会发送到群里。`group_id`、`user_id` 都是 Gensokyo 映射后的虚拟 ID（也接受 32 位原生 OpenID）。

```text
[CQ:set_group_ban,group_id=<虚拟群ID>,user_id=<虚拟用户ID>,duration=<秒>]
```

| 参数 | 必填 | 说明 |
|------|:----:|------|
| `group_id` | 否 | 虚拟 q群 ID；省略时使用消息发送的目标群（支持跨群路由）。 |
| `user_id` | 是 | 虚拟用户 ID 或真实 OpenID。 |
| `duration` | 否 | 禁言时长（秒）；省略或为 `0` 表示解除禁言。 |

范围：`q群 (Group Chat)`

## 行为

- `duration > 0`：禁言该成员至当前时间 + `duration` 秒。
- `duration = 0`（或省略）：解除禁言（查询当前群禁言设置，仅移除该成员的条目，不影响其他成员禁言）。
- 参数缺失或反查 OpenID 失败时：CQ 码原样保留在文本中（不会被吞掉）。
- 执行失败仅记录日志，不阻断其余文本发送。

## 示例

```text
[CQ:set_group_ban,group_id=821404315,user_id=3607918353,duration=60] 这位请安静一分钟
```

发送后：文本"这位请安静一分钟"正常发送，成员 `3607918353` 被禁言 60 秒。

解除禁言：

```text
[CQ:set_group_ban,user_id=3607918353,duration=0] 解禁了
```

## nonebot2 示例

```python
from nonebot import on_command
from nonebot.adapters.onebot.v11 import Bot, GroupMessageEvent

@on_command("ban").handle()
async def _(bot: Bot, event: GroupMessageEvent):
    # 禁言 10 分钟并回复
    await bot.send_group_msg(group_id=event.group_id,
        message=f"[CQ:set_group_ban,user_id={event.user_id},duration=600] 已禁言 10 分钟")
```
