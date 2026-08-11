# [CQ:set_group_whole_ban]

## 用途

出站单向动作 CQ 码：在发送群消息时切换群全员禁言，执行后该 CQ 码从文本中移除，不会发送到群里。`group_id` 是 Gensokyo 映射后的虚拟 ID（也接受 32 位原生 OpenID）。

```text
[CQ:set_group_whole_ban,group_id=<虚拟群ID>,enable=<true/false>]
```

| 参数 | 必填 | 说明 |
|------|:----:|------|
| `group_id` | 否 | 虚拟 q群 ID；省略时使用消息发送的目标群（支持跨群路由）。 |
| `enable` | 是 | `true` 开启全员禁言，`false` 解除全员禁言（支持 `1`/`0`）。 |

范围：`q群 (Group Chat)`

## 行为

- 切换全员禁言时保留群内已有的成员级禁言设置（先查询当前设置再提交）。
- 参数缺失或 `enable` 无效时：CQ 码原样保留在文本中。
- 执行失败仅记录日志，不阻断其余文本发送。

## 示例

```text
[CQ:set_group_whole_ban,group_id=821404315,enable=true] 全员禁言，安静一下
```

解除全员禁言：

```text
[CQ:set_group_whole_ban,group_id=821404315,enable=false] 解除了
```

## nonebot2 示例

```python
from nonebot import on_command
from nonebot.adapters.onebot.v11 import Bot, GroupMessageEvent

@on_command("mute_all").handle()
async def _(bot: Bot, event: GroupMessageEvent):
    await bot.send_group_msg(group_id=event.group_id,
        message="[CQ:set_group_whole_ban,enable=true] 全员禁言中")
```
