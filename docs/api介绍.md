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
| `send_private_msg_sse` | send_private_msg_sse.go | SSE 私聊消息 |
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
| `send_guild_private_msg` | send_guild_private_msg.go | 发送频道私信 |
| `get_guild_list` | get_guild_list.go | 获取频道列表 |
| `get_guild_channel_list` | get_guild_channel_list.go | 获取频道子频道列表 |
| `get_guild_service_profile` | get_guild_service_profile.go | 获取频道服务信息 |

## Gensokyo 扩展 API

| Action | 文件 | 说明 |
|--------|------|------|
| [`get_avatar`](./额外api-get_avatar.md) | get_avatar.go | 获取用户头像直链 |
| `get_robot_share_link` | get_robot_share_link.go | 获取机器人分享链接 |
| `put_interaction` | put_interaction.go | 处理按钮交互回调 |
| `send_private_msg_wakeup` | send_private_msg_wakeup.go | 发送被动唤醒私聊消息 |
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