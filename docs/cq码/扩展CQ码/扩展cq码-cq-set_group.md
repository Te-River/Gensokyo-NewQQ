# [CQ:set_group]

> 🧪 **测试插件**：[CQ-set_group](https://github.com/Te-River/Gensokyo-NewQQ-Test-plugins/tree/main/CQ-set_group)（发送「群禁言测试 / 群全员禁言测试 / 入群审批测试」）、[CQ-set_group_ban](https://github.com/Te-River/Gensokyo-NewQQ-Test-plugins/tree/main/CQ-set_group_ban)（发送「禁言测试」）、[CQ-set_group_whole_ban](https://github.com/Te-River/Gensokyo-NewQQ-Test-plugins/tree/main/CQ-set_group_whole_ban)（发送「全员禁言测试」）、[join_approval](https://github.com/Te-River/Gensokyo-NewQQ-Test-plugins/tree/main/join_approval)（入群申请转发审批）体验

## 概述

`[CQ:set_group]` 是出站单向动作 CQ 码：后端（OneBot 客户端）在发送群消息时，通过它触发 Gensokyo 执行群管理操作（禁言 / 全员禁言 / 入群审批 / 审批策略 / 踢人 / 黑名单），执行后该 CQ 码从消息文本中移除，不会发送到群里。

本 CQ 码由原来的 4 个独立 CQ 码合并而来，统一了参数解析、ID 反查与底层实现：

| 旧 CQ 码 | 新 CQ 码 | 状态 |
|----------|----------|------|
| `[CQ:set_group_ban,...]` | `[CQ:set_group,action=ban,...]` | 旧码已移除 |
| `[CQ:set_group_whole_ban,...]` | `[CQ:set_group,action=whole_ban,...]` | 旧码已移除 |
| `[CQ:set_group_add_request,...]` | `[CQ:set_group,action=add_request,...]` | 旧码已移除 |
| `[CQ:strategy,action=execute/delete,...]` | `[CQ:set_group,action=strategy_execute/strategy_delete,...]` | 旧码已移除 |

范围：`q群 (Group Chat)`

## 格式定义

```text
[CQ:set_group,action=ban,group_id=<虚拟群ID>,user_id=<虚拟用户ID>,duration=<秒>]
[CQ:set_group,action=whole_ban,group_id=<虚拟群ID>,enable=<true/false>]
[CQ:set_group,action=add_request,group_id=<虚拟群ID>,user_id=<虚拟用户ID>,flag=<申请ID>,approve=<true/false>,reason=<理由>,add_to_member_blacklist=<true/false>]
[CQ:set_group,action=strategy_execute,strategy_id=<策略ID>]
[CQ:set_group,action=strategy_delete,strategy_id=<策略ID>]
[CQ:set_group,action=kick,group_id=<虚拟群ID>,user_id=<虚拟用户ID>|user_ids=<逗号分隔批量>,add_blacklist=<true/false>]
[CQ:set_group,action=blacklist_add,group_id=<虚拟群ID>,user_id=<虚拟用户ID>|user_ids=<逗号分隔批量>]
[CQ:set_group,action=blacklist_del,group_id=<虚拟群ID>,user_id=<虚拟用户ID>|user_ids=<逗号分隔批量>]
```

参数采用**顺序无关**的 `key=value` 提取方式，参数顺序可以任意排列；未知参数忽略；缺少必要参数时行为见「行为细节」。

## 参数总表

| 字段 | 适用动作 | 必填 | 类型 | 说明 |
|------|---------|:----:|------|------|
| `action` | 全部 | ✅ | string | 动作类型：`ban` / `whole_ban` / `add_request` / `strategy_execute` / `strategy_delete` / `kick` / `blacklist_add` / `blacklist_del` |
| `group_id` | ban / whole_ban / add_request / kick / blacklist_add / blacklist_del | ❌ | string | 目标群：虚拟数字 ID 或 32 位原生 OpenID；省略时回退到消息发送的目标群（支持跨群路由） |
| `user_id` | ban / add_request / kick / blacklist_add / blacklist_del | 二选一* | string | 目标成员：虚拟数字 ID 或 32 位原生 OpenID |
| `user_ids` | kick / blacklist_add / blacklist_del | 二选一* | string | 批量目标成员，逗号分隔；与 `user_id` 同时存在时合并去重，超 20 截断并警告 |
| `duration` | ban | ❌ | int | 禁言时长（秒）；`>0` 禁言至当前时间 + duration，省略或 `0` 表示解除禁言 |
| `enable` | whole_ban | ❌ | bool | 全员禁言开关：`true` 开启 / `false` 关闭；省略按 `false` 处理 |
| `flag` | add_request | ✅ | string | 入群申请 ID（`join_request_id`），来自入站 request 事件或 `get_group_join_request_list` 返回 |
| `approve` | add_request | ❌ | bool | `true` = 通过（op=approve）/ `false` = 拒绝（op=decline）；省略按 `true` 处理 |
| `reason` | add_request | ❌ | string | 拒绝理由，仅 `approve=false` 时生效 |
| `add_to_member_blacklist` | add_request | ❌ | bool | 审批时是否同时将申请人加入群黑名单 |
| `add_blacklist` | kick | ❌ | bool | 移出群的同时加入群黑名单；省略按 `false` 处理 |
| `strategy_id` | strategy_execute / strategy_delete | ✅ | string | 审批策略 ID，来自 `join_approval_strategy_create` / `join_approval_strategy_list` 返回 |

\* `user_id` 与 `user_ids` 在 kick / blacklist_add / blacklist_del 中二选一（至少提供一个）。

## 参数详解

### `action`（必填）

8 个枚举值，决定执行什么操作：

| 值 | 操作 | 调用 QQ API |
|----|------|-------------|
| `ban` | 群成员禁言 / 解禁 | `SetRestrictChatSetting` |
| `whole_ban` | 群全员禁言开关 | `SetRestrictChatSetting`（AllMute） |
| `add_request` | 审批入群申请 | `ApprovalJoinRequest` |
| `strategy_execute` | 执行自动审批策略（全量扫描，异步约 10 分钟） | `ExecuteJoinApprovalStrategy` |
| `strategy_delete` | 删除自动审批策略 | `DeleteJoinApprovalStrategy` |
| `kick` | 单个/批量移出群成员（≤20，可同步拉黑） | `BatchRemoveMembers` |
| `blacklist_add` | 加入群黑名单（≤20） | `UpdateMemberBlacklist`（op=add） |
| `blacklist_del` | 移出群黑名单（≤20） | `UpdateMemberBlacklist`（op=del） |

> 说明：策略动作使用 `strategy_execute` / `strategy_delete` 作为顶层枚举值，而非 `action=strategy,strategy_action=execute` 的形式，避免与顶层 `action` 参数撞名。

### `group_id`

- 接受两种格式：虚拟数字 ID（Gensokyo 通过 idmap 映射）或 32 位原生 OpenID。
- 省略时回退到消息发送的目标群（即 `send_group_msg` 的 `group_id` 参数）。
- 提供时支持**跨群路由**：CQ 码中的群与发送目标群不同时，动作作用在 CQ 码指定的群上。
- 反查规则：32 位直接使用；非 32 位通过 idmap 反查真实 OpenID。

### `user_id`

- 接受两种格式：虚拟数字 ID 或 32 位原生 OpenID。
- 反查规则：32 位直接使用；非 32 位通过 idmap 反查真实 OpenID，反查失败则动作不执行。

### `duration`（仅 ban）

- `> 0`：禁言该成员至「当前时间 + duration 秒」（`RestrictUntil`）。
- `= 0` 或省略：解除禁言——先查询群当前禁言设置，仅移除该成员的条目，**不影响其他成员的禁言状态**。

### `enable`（仅 whole_ban）

- `true`：全员禁言开启；`false`：关闭。
- 提交时保留群当前的成员禁言列表（`MemberRestrict`），不会清掉已设置的单项禁言。

### `flag`（仅 add_request）

- 入群申请 ID，来自入站 request 事件（`request_type=group`）的 `flag` 字段，或 `get_group_join_request_list` 返回的 `flag`，两者完全一致可直接回传。

### `approve`（仅 add_request）

- `true` → `op=approve`；`false` → `op=decline`。

### `reason`（仅 add_request）

- 拒绝理由，仅在 `approve=false` 时传给 QQ API。

### `add_to_member_blacklist`（仅 add_request）

- `true` 时审批同时将申请人加入群黑名单。

### `user_ids`（仅 kick / blacklist_add / blacklist_del）

- 逗号分隔的批量成员列表，每个成员接受虚拟数字 ID 或 32 位原生 OpenID。
- 与 `user_id` 同时存在时**合并后去重保序**；空项自动过滤。
- 超过官方单批上限 20 人时截断到前 20 人并记录警告日志。
- 逐个反查 OpenID，反查失败的单个成员跳过并记录日志，**不中断整批**。

### `add_blacklist`（仅 kick）

- `true` 时移出群聊的同时将成员加入群黑名单（对应官方 `add_to_member_blacklist` 字段）。

### `strategy_id`（仅 strategy_execute / strategy_delete）

- 审批策略 ID，格式如 `st_d83eca11e9`。

## 行为细节

- **纯动作消息**：若消息仅含 CQ 码、处理后无文本且无媒体（foundItems 为空），Gensokyo 不向 QQ 发送任何消息，仅向 OneBot 客户端返回成功回执。
- **参数缺失/无效**：`action` 缺失或未知 → CQ 码原样保留在文本中；`group_id` 缺失 → 先回退当前会话群（legacy 与 new 一致），仅 `group_id` 与 `user_id` 双缺才保留原文；`user_id`/`flag`/`strategy_id` 缺失 → 记录日志、动作不执行，CQ 码从文本移除（`cq_parse_mode: new` 及 2026-09 重构后行为；legacy 模式仍保留原文）；`enable`/`approve` 解析失败 → 记录日志、动作不执行、码不泄漏（`cq_parse_mode: new` 行为，legacy 模式 whole_ban 仍保留原文）。
- **反查失败**：ID 反查 OpenID 失败 → 动作不执行，CQ 码从文本移除（不发送原文）；kick / blacklist 动作逐个反查，失败的单个成员跳过不中断整批。
- **批量上限**：kick / blacklist 单批最多 20 人（官方接口上限），超出截断并记录警告日志；单个成员也走批量接口。
- **黑名单 add 限制**：群内成员加入黑名单会被官方拒绝（群内成员应使用 `kick` 的 `add_blacklist`），错误透传日志，不影响消息发送。
- **执行失败**：QQ API 调用失败仅记录日志，不阻断消息中其余文本的发送。
- **移除语义**：CQ 码执行后一律从文本移除，无论成败都不会把 CQ 码原文发到群里。
- **动作型 CQ 码仅群聊生效**：`[CQ:set_group]` 在群聊路径（`send_group_msg`）执行；私聊（C2C）/转发节点路径一律**拦截**——码从文本移除、记录日志、不执行不发送（2026-09 修复，此前私聊/转发会原样泄漏为聊天文本）。
- **与 OneBot API 的关系**：`/set_group_ban`、`/set_group_whole_ban`、`/set_group_add_request`、`/set_group_kick` 等 API handler 与 `[CQ:set_group]` 共享同一底层实现，行为一致；API 路径有结构化回执（retcode），CQ 码路径无结构化回执。

## 数据流（传输模式）

### 模式一：字符串 CQ 码

```
后端插件：send_group_msg(group_id=<群>, message="[CQ:set_group,action=ban,user_id=<用户>,duration=60] 已禁言")
  → parseMessageContent（动作 CQ 码保留在 messageText）
  → ProcessOutboundCQCodes（单次扫描，case "set_group"）
  → cqSetGroupAction（顺序无关参数解析 + action 分发）
  → 共享底层：ID 反查 → QQ API 调用
  → CQ 码从文本移除 → 剩余文本正常发送
```

### 模式二：消息段数组（arrayValue 模式）

```
后端插件：message=[{"type":"set_group","data":{"action":"ban","group_id":"<群>","user_id":"<用户>","duration":"60"}}, {"type":"text","data":{"text":"已禁言"}}]
  → parseMessageContent 的 []interface{} 路径，case "set_group"
  → 还原为 CQ 码字符串 [CQ:set_group,action=ban,...] 拼入 messageText
  → 与模式一汇合，走 ProcessOutboundCQCodes 统一分发
```

### 模式三：TRSS map 格式

```
后端插件：message={"type":"set_group","data":{...}}
  → parseMessageContent 的 map 路径，case "set_group"
  → 还原为 CQ 码字符串拼入 messageText → 统一分发
```

三种模式最终都归一化为 CQ 码字符串，进入同一个分发入口执行。

## 示例

### 禁言（带文本）

```text
[CQ:set_group,action=ban,group_id=821404315,user_id=3607918353,duration=60] 这位请安静一分钟
```

发送后：文本"这位请安静一分钟"正常发送，成员 `3607918353` 被禁言 60 秒。

### 解除禁言

```text
[CQ:set_group,action=ban,user_id=3607918353,duration=0] 解禁了
```

### 全员禁言开关

```text
[CQ:set_group,action=whole_ban,group_id=821404315,enable=true] 全员禁言已开启
```

### 审批入群（通过）

```text
[CQ:set_group,action=add_request,group_id=821404315,user_id=3607918353,flag=f_12345,approve=true] 欢迎入群
```

### 审批入群（拒绝 + 拉黑）

```text
[CQ:set_group,action=add_request,group_id=821404315,user_id=3607918353,flag=f_12345,approve=false,reason=群已满员,add_to_member_blacklist=true]
```

### 执行/删除策略

```text
[CQ:set_group,action=strategy_execute,strategy_id=st_d83eca11e9] 已发起白名单全量扫描，约10分钟完成
[CQ:set_group,action=strategy_delete,strategy_id=st_d83eca11e9] 策略已删除
```

### 踢出单个成员

```text
[CQ:set_group,action=kick,user_id=3607918353] 请自省后再回来
```

### 批量踢出（可同步拉黑）

```text
[CQ:set_group,action=kick,user_ids=3607918353,821404315,555666777,add_blacklist=true] 广告小号清理完毕
```

### 黑名单增删

```text
[CQ:set_group,action=blacklist_add,user_id=3607918353] 已加入黑名单
[CQ:set_group,action=blacklist_del,user_ids=3607918353,821404315] 已移出黑名单
```

## nonebot2 示例

### 禁言

```python
from nonebot import on_command
from nonebot.adapters.onebot.v11 import Bot, GroupMessageEvent

@on_command("ban").handle()
async def _(bot: Bot, event: GroupMessageEvent, args):
    seconds = str(args).strip()
    await bot.send_group_msg(group_id=event.group_id,
        message=f"[CQ:set_group,action=ban,user_id={event.user_id},duration={seconds}] 已禁言")
```

### 全员禁言

```python
@on_command("mute_all").handle()
async def _(bot: Bot, event: GroupMessageEvent):
    await bot.send_group_msg(group_id=event.group_id,
        message="[CQ:set_group,action=whole_ban,enable=true] 全员禁言已开启")
```

### 入群审批（配合 get_group_join_request_list）

```python
from nonebot import on_command
from nonebot.adapters.onebot.v11 import Bot

@on_command("approve_all").handle()
async def _(bot: Bot, event):
    result = await bot.call_api("get_group_join_request_list", group_id=event.group_id)
    for item in result.get("data", {}).get("items", []):
        cq = (f"[CQ:set_group,action=add_request,group_id={item['group_id']},"
              f"user_id={item['user_id']},flag={item['flag']},approve=true]")
        await bot.send_group_msg(group_id=event.group_id, message=cq)
```

### 执行策略

```python
@on_command("scan").handle()
async def _(bot: Bot, event: GroupMessageEvent, args):
    sid = str(args).strip()
    await bot.send_group_msg(group_id=event.group_id,
        message=f"[CQ:set_group,action=strategy_execute,strategy_id={sid}] 已发起全量扫描")
```

## 迁移指南

将旧 CQ 码替换为新格式（其余参数不变）：

| 旧写法 | 新写法 |
|--------|--------|
| `[CQ:set_group_ban,group_id=1,user_id=2,duration=60]` | `[CQ:set_group,action=ban,group_id=1,user_id=2,duration=60]` |
| `[CQ:set_group_whole_ban,group_id=1,enable=true]` | `[CQ:set_group,action=whole_ban,group_id=1,enable=true]` |
| `[CQ:set_group_add_request,group_id=1,user_id=2,flag=f,approve=true]` | `[CQ:set_group,action=add_request,group_id=1,user_id=2,flag=f,approve=true]` |
| `[CQ:strategy,action=execute,strategy_id=st]` | `[CQ:set_group,action=strategy_execute,strategy_id=st]` |
| `[CQ:strategy,action=delete,strategy_id=st]` | `[CQ:set_group,action=strategy_delete,strategy_id=st]` |

`kick` / `blacklist_add` / `blacklist_del` 为新增能力，无旧码需要迁移：

- 此前 Gensokyo 的 `/set_group_kick` 为不支持/mock 实现，现已升级为真实批量移出接口；原通过其他 OneBot 实现调用 `/set_group_kick` 的插件可继续使用标准 API，或改用本 CQ 码获得批量能力。
- 群黑名单此前无任何入口，`blacklist_add` / `blacklist_del` 与 `/get_group_member_blacklist`、`/set_group_member_blacklist` 为全新能力。
- 注意迁移语义差异：`kick` 走官方批量移出接口（单个成员也走批量 API），`user_id` 传虚拟数字 ID 或 32 位原生 OpenID 均可。

步骤：

1. 全局替换插件代码中的 5 种旧 CQ 码写法。
2. 逐个动作验证（禁言/解禁/全员/审批/策略）。
3. 若使用消息段数组（arrayValue 模式），将 `{"type":"set_group_ban",...}` 改为 `{"type":"set_group","data":{"action":"ban",...}}`。

## 常见问题 FAQ

**Q: 为什么策略动作不用 `action=strategy,strategy_action=execute`？**
A: 避免与顶层 `action` 参数撞名，直接用 `strategy_execute` / `strategy_delete` 两个枚举值更干净，解析也更简单。

**Q: 为什么 `[CQ:set_group]` 只在群聊生效？**
A: 它是群管理动作，QQ 官方 API 仅提供群聊场景接口（群禁言/审批/策略均针对群），私聊无对应能力。

**Q: 虚拟 ID 和 OpenID 什么时候互转？**
A: 你（插件）永远只接触虚拟数字 ID；Gensokyo 内部通过 idmap 反查真实 OpenID 后再调用 QQ API。32 位原生 OpenID 也可直接传入，会被识别并跳过反查。

**Q: 纯动作消息（无文本）为什么群里收不到？**
A: CQ 码执行后即从文本移除，若移除后无剩余文本和媒体，Gensokyo 判定为纯动作消息，只回执不发送，避免发出一条空消息。

**Q: 为什么踢人单个成员也走批量接口？**
A: QQ 官方仅提供批量移出接口 `batch_remove_members`（≤20），单个成员走同一接口；黑名单增删同理。

**Q: 为什么群内成员无法直接加入黑名单？**
A: 官方限制：仅已移出/已退群成员可加入黑名单，群内成员 add 会被官方拒绝并透传错误日志。需要"移出并拉黑"请使用 `kick` 动作配合 `add_blacklist=true`。

**Q: 执行失败会怎样？**
A: 仅记录日志，不影响消息中其余文本的发送；CQ 码同样会被移除，不会把原始 CQ 码文本发到群里。
