# Gensokyo API 介绍

## 标准 OneBot V11 API

以下 API 符合 [OneBot V11 标准](https://github.com/botuniverse/onebot-11)。

| Action | 文件 | 场景 | 行为 |
|--------|------|------|------|
| `send_msg` | send_msg.go | `私聊 (C2C)` / `q群 (Group Chat)` / `q頻 (QQ Guild)` | 按 echo/msgType 缓存和 ID 映射分流到对应发送处理器。 |
| `send_msg_async` | send_msg_async.go | 同 `send_msg` | 注册到 `HandleSendMsg`，处理逻辑同 `send_msg`。 |
| `send_group_msg` | send_group_msg.go | `q群 (Group Chat)` | 发送消息，支持 Gensokyo 的消息解析和 ID 转换。 |
| `send_group_msg_async` | send_group_msg_async.go | `q群 (Group Chat)` | 注册到 `HandleSendGroupMsg`，处理逻辑同 `send_group_msg`。 |
| `send_group_msg_raw` | send_group_msg_raw.go | `q群 (Group Chat)` | 发送消息，保留更多原始参数，少做预处理。 |
| `send_private_msg` | send_private_msg.go | `私聊 (C2C)` | 发送 C2C 私聊消息。 |
| `send_private_msg_async` | send_private_msg_async.go | `私聊 (C2C)` | 注册到 `HandleSendPrivateMsg`，处理逻辑同 `send_private_msg`。 |
| `delete_msg` | delete_msg.go | `私聊 (C2C)` / `q群 (Group Chat)` / `q頻 (QQ Guild)` | 按消息所属场景调用对应撤回接口。 |
| `get_login_info` | get_login_info.go | `-` | 获取当前机器人登录信息。 |
| `get_friend_list` | get_friend_list.go | `私聊 (C2C)` | 获取好友列表。 |
| `get_group_list` | get_group_list.go | `q群 (Group Chat)` | 获取列表。 |
| `get_group_info` | get_group_info.go | `q群 (Group Chat)` | 调用 QQ 官方 v2 群信息接口返回真实群名/简介/成员数。 |
| `get_group_member_info` | get_group_member_info.go | `q群 (Group Chat)` | 获取成员信息。 |
| `get_group_member_list` | get_group_member_list.go | `q群 (Group Chat)` | 获取成员列表。 |
| `get_status` | get_status.go | `-` | 获取运行状态。 |
| `get_version_info` | get_version_info.go | `-` | 获取版本信息。 |
| `get_online_clients` | get_online_clients.go | `-` | 获取在线客户端。 |
| `send_group_forward_msg` | send_group_forward_msg.go | `q群 (Group Chat)` | 发送合并转发消息。 |
| `set_group_ban` | set_group_ban.go | `q群 (Group Chat)` | 单人禁言，`duration` 秒数（0=解除禁言）。与 `[CQ:set_group,action=ban]` 共享实现底层。 |
| `set_group_whole_ban` | set_group_whole_ban.go | `q群 (Group Chat)` | 全员禁言，`enable` 开关。与 `[CQ:set_group,action=whole_ban]` 共享实现底层。 |
| `.handle_quick_operation` | handle_quick_operation.go | `-` | OneBot 快速操作。 |
| `.handle_quick_operation_async` | handle_quick_operation_async.go | `-` | OneBot 快速操作的 async action 名称。 |
| `mark_msg_as_read` | mark_msg_as_read.go | `-` | 标记消息已读。 |

> **delete_msg 差异说明：**
>
> | 参数 | 类型 | 说明 |
> |------|------|------|
> | `message_id` | int32 | 消息 ID |
> | `user_id` | int64/string | 私聊 (C2C) 时需要 |
> | `group_id` | int64/string | q群 (Group Chat) 时需要 |

## Gensokyo 扩展 API

| Action | 文件 | 场景 | 行为 |
|--------|------|------|------|
| [`get_avatar`](./扩展api/扩展api-get_avatar.md) | get_avatar.go | `-` | 按虚拟用户 ID 反查 OpenID 并返回头像直链。 |
| [`get_robot_share_link`](./扩展api/扩展api-get_robot_share_link.md) | get_robot_share_link.go | `-` | 获取机器人资料页分享链接。 |
| [`put_interaction`](./扩展api/扩展api-put_interaction.md) | put_interaction.go | `q群 (Group Chat)` / `q頻 (QQ Guild)` | 回复按钮交互结果。 |
| [`send_private_msg_wakeup`](./扩展api/扩展api-send_private_msg_wakeup.md) | send_private_msg_wakeup.go | `私聊 (C2C)` | 发送 `is_wakeup=true` 的 C2C 召回消息。 |
| `send_private_msg_sse` | send_private_msg_sse.go | `私聊 (C2C)` | SSE 私聊消息。 |
| `get_group_ban` | set_group_ban.go | `q群 (Group Chat)` | 兼容入口，处理逻辑等同 `set_group_ban`。 |
| `get_group_whole_ban` | set_group_whole_ban.go | `q群 (Group Chat)` | 兼容入口，处理逻辑等同 `set_group_whole_ban`。 |
| `set_group_add_request` | set_group_add_request.go | `q群 (Group Chat)` | 入群申请审批。与 `[CQ:set_group,action=add_request]` 共享实现底层。 |
| `send_to_group` | send_group_msg.go | `q群 (Group Chat)` | `send_group_msg` 别名。 |
| [`delete_group_msg`](./扩展api/扩展api-delete_group_msg.md) | delete_group_msg.go | `q群 (Group Chat)` | 撤回群内指定用户或 Bot 自身的消息；支持自动查找最后一条消息。 |
| [`get_group_join_request_list`](./扩展api/扩展api-get_group_join_request_list.md) | get_group_join_request_list.go | `q群 (Group Chat)` | 拉取入群申请列表，`next_index` 分页；返回的 `group_id`/`user_id`/`flag` 可直接回传 `set_group_add_request` 审批。 |
| [`get_group_bot_state`](./扩展api/扩展api-get_group_bot_state.md) | get_group_bot_state.go | `q群 (Group Chat)` | 获取机器人在群内状态（入群时间/可推送/角色）。 |
| [`join_approval_strategy_create`](./扩展api/扩展api-join_approval_strategy.md) | join_approval_strategy.go | `-` | 创建入群自动审批策略（`group_openids`/`group_ids` 二选一）。 |
| [`join_approval_strategy_list`](./扩展api/扩展api-join_approval_strategy.md) | join_approval_strategy.go | `-` | 查询策略列表（`cursor`/`limit` 分页）。 |
| [`join_approval_strategy_update`](./扩展api/扩展api-join_approval_strategy.md) | join_approval_strategy.go | `-` | 修改策略（`strategy_id` + `is_enable`/`expire_at`/`remark`/`op`+关联群）。 |
| [`join_approval_strategy_execute`](./扩展api/扩展api-join_approval_strategy.md) | join_approval_strategy.go | `-` | 执行策略全量扫描（异步约 10 分钟）。 |
| [`join_approval_strategy_whitelist`](./扩展api/扩展api-join_approval_strategy.md) | join_approval_strategy.go | `-` | 修改策略白名单（`strategy_id` + `op` + `whitelist_users`）。 |
| [`join_approval_strategy_delete`](./扩展api/扩展api-join_approval_strategy.md) | join_approval_strategy.go | `-` | 删除策略（`strategy_id`）。 |

## 消息事件扩展字段

### `to_me` 字段

Gensokyo 在 q群 消息事件中增加了 `to_me` 字段，标识消息是否 @ 了机器人。

```json
{
    "group_id": 123456,
    "message": [...],
    "to_me": true,   // 本版新增
    "user_id": 789012,
    ...
}
```

**取值逻辑：**
- `GROUP_AT_MESSAGE_CREATE`（@消息事件）→ `to_me: true` **始终为 true**
- `GROUP_MESSAGE_CREATE`（普通 q群 消息事件）→ 检测 `Mentions` 中是否有 `IsYou=true`，有为 `true`，否则为 `false`

**nonebot2 获取方式：**
```python
from nonebot import on_message
from nonebot.adapters.onebot.v11 import GroupMessageEvent

@on_message().handle()
async def handler(event: GroupMessageEvent):
    # 直接通过 event.to_me 获取
    if event.to_me:
        await event.finish("你@我了")
```
或者通过 raw event 获取：
```python
event.json()  # 查看完整字段
```

### 非自身 @ 处理

在 `GROUP_MESSAGE_CREATE`（普通 q群 消息）中，若内容包含对其他用户/机器人的 `@`，Gensokyo 会自动将原始 `<@OpenID>` 替换为标准的 `[CQ:at,qq=虚拟ID]` 格式，并建立 OpenID 与虚拟 ID 的映射。

这是一种 **双向转换**：

```
入站（QQ API → 后端）：
  QQ API:   <@B1FE88...> 贴贴
  后端收到: [CQ:at,qq=713011248] 贴贴

出站（后端 → QQ API）：
  后端发送 [CQ:at,qq=713011248] 贴贴
  QQ API:   <@!B1FE88...> 贴贴
```

**规则：**
- 入站 `@bot` 从 content 中剥离（`to_me = true`）
- 入站 `@其他人` 转为 `[CQ:at,qq=虚拟ID]`
- 出站 `[CQ:at,qq=数字]`：纯文本消息转为 `@用户名 `（带空格），Markdown 消息转为 `<qqbot-at-user id="OpenID" />`
- 纯文本出站时 @ 转为 `@用户名` 格式，用户名来自内存缓存（入站时自动缓存，10 分钟 TTL）

### Markdown 中的 @ 能力

在 Markdown 卡片（`msg_type=2`）内容中嵌入 `[CQ:at,qq=数字]`，Gensokyo 会自动将其转换为 QQ API 的 `<qqbot-at-user>` 标签，并且**整个 `messageText`**（含 `<qqbot-at-user>` 标签和文本内容）会合并到 Markdown 内容头部，确保文本不丢失。

**nonebot2 示例：**

```python
from nonebot import on_command
from nonebot.adapters.onebot.v11 import Bot, Event, Message

@on_command("md_at").handle()
async def _(bot: Bot, event: Event):
    md_content = f"你好 [CQ:at,qq={event.user_id}]，欢迎使用！"
    md_seg = {"type":"markdown","data":{"data":md_content}}
    await bot.send(event, Message(md_seg))
```

### Sender.Nickname 自动填充

Gensokyo 在群消息和私聊消息中，当 `card_nick` 配置为空时，会自动从 QQ API 返回的 `data.Author.Username` 中提取用户名填充到 `event.sender.nickname`。

```json
{
    "sender": {
        "nickname": "NIKO",   // 自动从 Author.Username 获取
        "user_id": 123456789,
        ...
    }
}
```

优先顺序：`card_nick` 配置 > `Author.Username` > 空

### Sender.Role 身份标识

群消息中 `sender.role` 字段会优先使用 QQ API 返回的 `member_role`，不再仅依赖 `master_id` 配置。

```json
{
    "sender": {
        "user_id": 10000001,
        "role": "owner",
        "nickname": "群主"
    }
}
```

取值：
- `owner` → 群主
- `admin` → 管理员
- `member` → 普通群员

API 未返回时回退到 `master_id` 配置判断，保持兼容。详见[本版新增功能](./本版新增功能.md#群消息-senderrole-支持-member_role)。
```
