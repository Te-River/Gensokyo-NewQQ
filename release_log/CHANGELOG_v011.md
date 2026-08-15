# Changelog — Release011

> 自 Release010 (`7682288`) 以来的所有变更。本轮将 QQ 频道（guild/channel/forum/DM）相关功能从项目中完全剔除，仅保留群聊（group/C2C/private）部分。

---

## 🚪 移除功能

### 频道出站 handler（6 个文件删除）

| 文件 | 行为 |
|------|------|
| `handlers/send_guild_channel_msg.go`（667 行） | 向指定子频道发送消息 |
| `handlers/send_guild_channel_forum.go`（305 行） | 向论坛/帖子入口发送内容 |
| `handlers/send_guild_private_msg.go`（264 行） | 频道私信（已废弃，改用 `send_private_msg`） |
| `handlers/get_guild_list.go`（82 行） | 获取频道列表 |
| `handlers/get_guild_channel_list.go`（81 行） | 获取子频道列表 |
| `handlers/get_guild_service_profile.go`（60 行） | 获取频道服务信息 |

### 频道入站 processor（5 个文件删除）

| 文件 | 事件 |
|------|------|
| `Processor/ProcessGuildATMessage.go` | `AT_MESSAGE_CREATE` 频道 @ 消息 |
| `Processor/ProcessGuildNormalMessage.go` | `MESSAGE_CREATE` 私域频道消息 |
| `Processor/ProcessGuildMember.go` | `GUILD_MEMBER_ADD/UPDATE/REMOVE` |
| `Processor/ProcessChannelDirectMessage.go` | `DIRECT_MESSAGE_CREATE` 频道私信 |
| `Processor/ProcessThreadMessage.go` | `FORUM_THREAD_*` 帖子事件 |

### botgo SDK Fork 频道精简

**dto 删除（15 个文件）：** `guild.go`、`channel.go`、`channel_permissions.go`、`direct_message.go`、`forum.go`、`announces.go`、`api_permissions.go`、`audio.go`、`message_audit.go`、`message_reaction.go`、`message_setting.go`、`mute.go`、`pins.go`、`schedule.go`、`member.go`、`role.go`

**dto 修改：**
- `websocket_payload.go`：删除 `WSGuildData`/`WSChannelData`/`WSMessageData`/`WSATMessageData`/`WSDirectMessageData`/`WSMessageDeleteData`/`WSPublicMessageDeleteData`/`WSDirectMessageDeleteData`/`WSAudioData`/`WSMessageReactionData`/`WSMessageAuditData`/`WSThreadData`/`WSPostData`/`WSReplyData`/`WSForumAuditData`/`WSGuildMemberData` 共 16 个频道 payload 类型
- `message.go`：删除 `Member`、`DirectMessage`、`SeqInChannel`、`SrcGuildID` 字段

**openapi 删除（v1+v2 共 30 个文件）：** `guilds.go`、`channels.go`、`channel_permissions.go`、`direct_message.go`、`pins.go`、`audio.go`、`schedule.go`、`announces.go`、`mute.go`、`message_reaction.go`、`member.go`、`role.go`、`api_permissions.go`、`message_setting.go`、`webhook.go`（后恢复为通用 HTTP 回调实现）

**openapi 修改：**
- `iface.go`：删除 13 个频道 API 接口块（GuildAPI/ChannelAPI/ChannelPermissionsAPI/AudioAPI/RoleAPI/MemberAPI/DirectMessageAPI/AnnouncesAPI/ScheduleAPI/APIPermissionsAPI/PinsAPI/MessageReactionAPI/MessageSettingAPI），保留 UserAPI/MessageAPI（群聊）/WebhookAPI/InteractionAPI
- `event/register.go`：删除 16 个频道 EventHandler 类型（Guild/GuildMember/Channel/Message/MessageReaction/ATMessage/DirectMessage/MessageAudit/MessageDelete/PublicMessageDelete/DirectMessageDelete/Audio/Thread/Post/Reply/ForumAudit）及注册逻辑
- `event/event.go`：删除频道事件路由表和处理函数，补回群聊 handler（groupAtMessage/c2cMessage/groupAddBot/groupDelBot/groupMemberAdd/groupMemberRemove）

### 配置项删除

**`structs/structs.go`（7 个字段）：**

| 字段 | yaml | 说明 |
|------|------|------|
| `GlobalChannelToGroup` | `global_channel_to_group` | 频道转群 |
| `GlobalPrivateToChannel` | `global_private_to_channel` | 私聊转频道私信（**Breaking**） |
| `GlobalForumToChannel` | `global_forum_to_channel` | 帖子转子频道 |
| `GuildUrlImageToBase64` | `guild_url_image_to_base64` | 频道 URL 图转 base64 |
| `GlobalServerTempQQguild` / `ServerTempQQguild` / `ServerTempQQguildPool` | `*_qqguild*` | V3 临时频道发图 |
| `GetGroupListAllGuilds` / `GetGroupListGuilds` / `GetGroupListReturnGuilds` / `GetGroupListGuidsType` | `get_g_list_*_guilds*` | 群列表轮询频道 |

**`config/config.go`：** 对应 8 个 `GetXxx()` 访问器全部删除。

**`template/config_template.go`：** 删除频道 handler 注释、3 个转换配置、3 个频道发图配置、4 个 `get_g_list` guild 配置。**新版本生成的配置文件仅包含实际有效的配置项。**

### 其他删除

| 文件 | 删除内容 |
|------|----------|
| `images/upload_api.go` | V3 频道发图分支、`UploadBehaviorV3`/`postImageToServerV3` 函数 |
| `server/uploadpic.go` | `UploadBase64ImageHandlerV3` 频道 multipart 端点 |
| `main.go` | 5 个频道 EventHandler、频道 intent 抑制逻辑、`uploadpicv3` 路由 |
| `echo/messageidmap.go` | `strings.HasPrefix(msgType, "guild")` 判断（剔除后 msgType 不再有 guild 前缀，直接返回 "2000"） |
| `webui/api.go` | `handleSendGuildChannelMessage`/`handleGetGuildList`/`handleGetChannelList` 端点 |
| `httpapi/httpapi.go` | `send_guild_channel_msg` 端点 |

---

## 🔧 重构

### handler 频道分支剔除（保留群聊路径）

| 文件 | 变更 |
|------|------|
| `handlers/send_group_msg.go` | 删除 `case "guild"`/`case "guild_private"`/`case "forum"` 分支；递归枚举 `tryMessageTypes` 由 `["group","guild","guild_private"]` 改为 `["group"]` |
| `handlers/send_group_msg_raw.go` | 同上 |
| `handlers/send_msg.go` | 删除 guild/guild_private/forum 路由分支 |
| `handlers/send_private_msg.go` | 删除 `case "guild_private", "guild"` 分支；递归枚举精简为 `["group"]` |
| `handlers/message_parser.go` | 删除 `case "guild"`/`case "guild_private"` 分支和频道 payload case（`WSATMessageData`/`WSMessageData`/`WSDirectMessageData`） |
| `handlers/delete_msg.go` | 删除频道消息撤回和频道私信撤回分支 |
| `handlers/get_group_info.go` | 删除 `ConvertGuildToGroupInfo` 函数，重写为纯群聊实现（175→88 行） |
| `handlers/get_group_list.go` | 删除频道轮询和"频道→群"转换逻辑，仅保留 idmap 群列表（313→164 行） |
| `handlers/get_group_member_list.go` | 删除 `case "guild"` 分支 |
| `handlers/set_group_ban.go` / `set_group_whole_ban.go` | 删除频道禁言分支，重写为纯群聊骨架 |
| `handlers/reply_helpers.go`（**新建**） | 从已删除的 `send_guild_channel_msg.go` 迁移 `GenerateReplyMessage`/`downloadImageAndConvertToBase64` 函数 |

### Processor 频道分支剔除

| 文件 | 变更 |
|------|------|
| `Processor/Processor.go` | 删除 `OnebotChannelMessage` 结构体、频道 downtimemessage/GetIDAndType2/unlock/Autobind/SendMessage/SendMessageMd 分支 |
| `Processor/ProcessC2CMessage.go` | 剔除 `GlobalPrivateToChannel` 频道转换分支（467→189 行） |
| `Processor/ProcessInlineSearch.go` | 剔除频道回调分支；`ConvertInteractionToMessage` 删除 `ChannelID`/`GuildID`/`DirectMessage` 字段映射 |

---

## ⚠️ Breaking Changes

- **`global_private_to_channel` 删除**：单聊（C2C）不再转换为频道私信，直接走 C2C 接口。依赖此配置的用户需调整行为预期。
- **频道相关 handler 全部移除**：`send_guild_channel_msg`、`send_guild_channel_forum`、`send_guild_private_msg`、`get_guild_list`、`get_guild_channel_list`、`get_guild_service_profile` 等 action 不再可用。
- **频道相关事件不再上报**：`GUILD_*`、`CHANNEL_*`、`GUILD_MEMBER_*`、`MESSAGE_CREATE/DELETE`（私域）、`AT_MESSAGE_CREATE`、`DIRECT_MESSAGE_*`、`FORUM_*`、`AUDIO_*`、`MESSAGE_REACTION_*`、`MESSAGE_AUDIT_*` 等事件不再处理。
- **用户旧 `config.yml` 残留的频道配置字段**（如 `global_channel_to_group`、`guild_url_image_to_base64`、`server_temp_qqguild`、`get_g_list_all_guilds` 等）会被 YAML 宽松解析忽略，不会报错，但不生效。**不强制要求用户更新配置文件**，新版本生成的配置文件仅包含实际有效的配置项。

---

## ✅ 保留项

- **QQ 频道图床（oss_type=6）**：保留枚举位和 `imagehosting/qq_channel.go` 实现，作为通用图床后端保留。
- **`InlineSearch`/`InteractionHandler`**：群聊也支持互动按钮回调，保留。
- **idmap 数据库残留的 guild/channel 映射数据**：无影响，可保留。

---

## 📝 文档同步

| 文件 | 变更 |
|------|------|
| `release_log/CHANGELOG_v011.md` | 本文档（新建） |
| `readme.md` | 删除 `send_guild_channel_msg` API 表项、6 个频道 EventHandler、频道 handler 注释、3 个转换配置、`ThreadEventHandler` |
| `docs/api/api介绍.md` | 删除频道撤回说明和"q頻 扩展 API"段 |
| `docs/idmap.md` | 删除"频道相关 ID 暂不纳入本轮重构"说明 |
| `docs/开始使用.md` | "频道类型" → "机器人类型" |
| `AGENTS.md` | 删除 QQ 频道图床举例、`api` 频道相关、handler `group/private/channel` 的 channel |

---

## 🧪 验证

| 命令 | 结果 |
|------|------|
| `go build ./...` | ✅ 通过 |
| `go vet ./...` | ✅ 通过 |
| `go test ./handlers/` | ✅ 通过（5.8s） |
| `go test ./Processor/` | ✅ 通过（修复 `processor_test.go` 频道剔除残留导致的群聊测试回归后） |

---

## ✅ 提交记录

```
2d44ef8  fix: 修复 processor_test 频道剔除残留导致的群聊测试回归
3259757  docs: 同步剔除频道文档并新建 CHANGELOG_v011
f9209ca  refactor: 清理idmap/echo频道映射
db54a10  refactor: 精简botgo SDK Fork频道部分
c771acf  refactor: 剔除频道入站processor
d53a66d  refactor: 剔除频道配置系统
8b69368  refactor: 剔除频道出站handler
```
