# [CQ:strategy]

## 用途

出站单向动作 CQ 码：在发送群消息时对入群自动审批策略执行操作（全量扫描执行 / 删除），执行后该 CQ 码从文本中移除，不会发送到群里。策略 ID 来自 `join_approval_strategy_create` / `join_approval_strategy_list` 返回的 `strategy_id`。

```text
[CQ:strategy,action=<execute/delete>,strategy_id=<策略ID>]
```

| 参数 | 必填 | 说明 |
|------|:----:|------|
| `action` | 是 | `execute` 对策略关联的全部群发起全量扫描（异步约 10 分钟）；`delete` 删除策略。 |
| `strategy_id` | 是 | 策略 ID（如 `st_d83eca11e9`）。 |

范围：`-`（不区分群，作用于策略本身）

## 行为

- `strategy_id` 缺失或 `action` 未知时：CQ 码原样保留在文本中（不会被吞掉）。
- 执行失败仅记录日志，不阻断其余文本发送。

## 示例

```text
[CQ:strategy,action=execute,strategy_id=st_d83eca11e9] 已发起白名单全量扫描，约10分钟完成
```

删除策略：

```text
[CQ:strategy,action=delete,strategy_id=st_d83eca11e9] 策略已删除
```

## nonebot2 示例

```python
from nonebot import on_command
from nonebot.adapters.onebot.v11 import Bot, GroupMessageEvent

@on_command("scan_strategy").handle()
async def _(bot: Bot, event: GroupMessageEvent, args):
    sid = str(args).strip()
    await bot.send_group_msg(group_id=event.group_id,
        message=f"[CQ:strategy,action=execute,strategy_id={sid}] 已发起全量扫描")
```
