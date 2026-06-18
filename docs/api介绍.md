# Gensokyo API 介绍

## 标准 OneBot V11 API

以下 API 符合 [OneBot V11 标准](https://github.com/botuniverse/onebot-11)：

| Action | 文件 | 说明 |
|--------|------|------|
| `send_msg` | send_msg.go | 发送消息（自动判断群/私聊/频道） |
| `send_msg_async` | send_msg_async.go | 异步发送消息 |
| `send_group_msg` | send_group_msg.go | 发送群消息 |
| `send_group_msg_async` | send_group_msg_async.go | 异步发群消息 |
| `send_group_msg_raw` | send_group_msg_raw.go | 发送原始群消息（不做预处理） |
| `send_private_msg` | send_private_msg.go | 发送私聊消息 |
| `send_private_msg_async` | send_private_msg_async.go | 异步发私聊消息 |
| `delete_msg` | delete_msg.go | 撤回消息（见下方说明） |
| `get_login_info` | get_login_info.go | 获取登录号信息 |

> **delete_msg 差异说明：**
> QQ 官方 API 的撤回需要 `message_id` 及对应的 `user_id`/`group_id`/`channel_id`/`guild_id` 来定位消息所属场景
>
> | 参数 | 类型 | 说明 |
> |------|------|------|
> | `message_id` | int32 | 消息 ID |
> | `user_id` | int64 | 私聊时需要 |
> | `group_id` | int64 | 群聊时需要 |
> | `channel_id` | int64 | 频道消息时需要 |
> | `guild_id` | int64 | 频道私信时需要 |
| `get_friend_list` | get_friend_list.go | 获取好友列表 |
| `get_group_list` | get_group_list.go | 获取群列表 |
| `get_group_info` | get_group_info.go | 获取群信息 |
| `get_group_member_info` | get_group_member_info.go | 获取群成员信息 |
| `get_group_member_list` | get_group_member_list.go | 获取群成员列表 |
| `get_status` | get_status.go | 获取运行状态 |
| `get_version_info` | get_version_info.go | 获取版本信息 |
| `get_online_clients` | get_online_clients.go | 获取在线客户端 |
| `send_group_forward_msg` | send_group_forward_msg.go | 发送合并转发消息 |
| `set_group_ban` | set_group_ban.go | 群组单人禁言 |
| `set_group_whole_ban` | set_group_whole_ban.go | 群组全员禁言 |
| `.handle_quick_operation` | handle_quick_operation.go | 快速操作 |
| `.handle_quick_operation_async` | handle_quick_operation_async.go | 异步快速操作 |
| `mark_msg_as_read` | mark_msg_as_read.go | 标记消息已读 |

## 频道/ guild 扩展 API

| Action | 文件 | 说明 |
|--------|------|------|
| `send_guild_channel_msg` | send_guild_channel_msg.go | 发送频道消息 |
| `send_guild_channel_forum` | send_guild_channel_forum.go | 发送频道帖子 |
| ~~`send_guild_private_msg`~~ | ~~send_guild_private_msg.go~~ | ~~发送频道私信~~（已废弃，请使用 `send_private_msg` 代替） |
| `get_guild_list` | get_guild_list.go | 获取频道列表 |
| `get_guild_channel_list` | get_guild_channel_list.go | 获取频道子频道列表 |
| `get_guild_service_profile` | get_guild_service_profile.go | 获取频道服务信息 |

## Gensokyo 扩展 API

| Action | 文件 | 说明 |
|--------|------|------|
| [`get_avatar`](./额外api-get_avatar.md) | get_avatar.go | 获取用户头像直链 |
| `get_robot_share_link` | [get_robot_share_link.go](./扩展api-get_robot_share_link.md) | 获取机器人分享链接 |
| `put_interaction` | [put_interaction.go](./扩展api-put_interaction.md) | 处理按钮交互回调 |
| `send_private_msg_wakeup` | [send_private_msg_wakeup.go](./扩展api-send_private_msg_wakeup.md) | 发送被动唤醒私聊消息 |
| `send_private_msg_sse` | send_private_msg_sse.go | SSE 私聊消息 |
| `get_group_ban` | set_group_ban.go | 群组单人禁言（等同于 `set_group_ban`） |
| `get_group_whole_ban` | set_group_whole_ban.go | 群组全员禁言（等同于 `set_group_whole_ban`） |
| `send_to_group` | send_group_msg.go | `send_group_msg` 别名 |

## 消息事件扩展字段

### `to_me` 字段

Gensokyo 在群消息事件中增加了 `to_me` 字段，标识消息是否 @ 了机器人：

```json
{
    "group_id": 123456,
    "message": [...],
    "to_me": true,   // ← 新增
    "user_id": 789012,
    ...
}
```

**取值逻辑：**
- `GROUP_AT_MESSAGE_CREATE`（@消息事件）→ `to_me: true` **始终为 true**
- `GROUP_MESSAGE_CREATE`（普通群消息事件）→ 检测 `Mentions` 中是否有 `IsYou=true`，有则 `true`，否则 `false`

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

在 `GROUP_MESSAGE_CREATE`（普通群消息）中，若内容包含对其他用户/机器人的 `@`，Gensokyo 会自动将原始 `<@OpenID>` 替换为标准的 `[CQ:at,qq=虚拟ID]` 格式，并建立 OpenID ↔ 虚拟 ID 的映射，确保同一用户多次出现的 @ 虚拟 ID 始终一致。

这是一个**双向转换**：

```
入站（QQ API → 后端）:
  QQ API:   <@B1FE88...> 贴贴
  后端收到: [CQ:at,qq=713011248] 贴贴

出站（后端 → QQ API）:
  后端发送: [CQ:at,qq=713011248] 贴贴
  QQ API:   <@!B1FE88...> 贴贴
```

**规则：**
- 入站 `@bot` → 从 content 中剥离（`to_me = true`）
- 入站 `@其他人` → 转为 `[CQ:at,qq=虚拟ID]`
- 出站 `[CQ:at,qq=数字]` → 转为 `<qqbot-at-user id="OpenID" />`（最新 QQ 格式，频道场景渲染为蓝色 @）
- 群聊纯文本出站消息中 @ 不渲染（API 限制）

### Markdown 中的 @ 能力

在 Markdown 卡片（`msg_type=2`）内容中嵌入 `[CQ:at,qq=数字]`，Gensokyo 会自动将其转换为 QQ API 的 `<qqbot-at-user>` 标签，在群聊和频道中渲染为蓝色 @。

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

Gensokyo 在群消息和私聊消息中，当 `card_nick` 配置为空时，会自动从 QQ API 返回的 `data.Author.Username` 中提取用户名填充到 `event.sender.nickname`：

```json
{
    "sender": {
        "nickname": "NIKO",   // ← 自动从 Author.Username 获取
        "user_id": 123456789,
        ...
    }
}
```

优先顺序：`card_nick` 配置值 > `Author.Username` > 空

### Sender.Role 身份标识

群消息中 `sender.role` 字段会优先使用 QQ API 返回的 `member_role`，不再仅依赖 `master_id` 配置：

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
- `owner` — 群主
- `admin` — 管理员
- `member` — 普通群员

API 未返回时回退到 `master_id` 配置判断，保持兼容。详见[本版新增功能](./本版新增功能.md#群消息-senderrole-支持-member_role)。
```