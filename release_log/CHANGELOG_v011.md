# v011 频道功能完全剔除

## 变更摘要

本次版本将 QQ 频道（guild/channel/forum/DM）相关功能从项目中完全剔除，仅保留群聊（group/C2C/private）部分。所有频道入站事件处理、出站 API 调用、配置项、botgo SDK Fork 中的频道 DTO/API/事件类型、idmap/echo 频道映射逻辑均已删除。

## Breaking Changes

- **`global_private_to_channel` 配置项删除**：单聊（C2C）不再转换为频道私信，直接走 C2C 接口。依赖此配置的用户需调整行为预期。
- **频道相关 handler 全部移除**：`send_guild_channel_msg`、`send_guild_channel_forum`、`send_guild_private_msg`、`get_guild_list`、`get_guild_channel_list`、`get_guild_service_profile` 等 action 不再可用。
- **频道相关事件不再上报**：`GUILD_CREATE/UPDATE/DELETE`、`CHANNEL_CREATE/UPDATE/DELETE`、`GUILD_MEMBER_ADD/UPDATE/REMOVE`、`MESSAGE_CREATE/DELETE`（私域）、`AT_MESSAGE_CREATE`、`DIRECT_MESSAGE_CREATE/DELETE`、`FORUM_*`、`AUDIO_*`、`MESSAGE_REACTION_*`、`MESSAGE_AUDIT_*` 等事件不再处理。
- **用户旧 `config.yml` 中残留的频道配置字段**（如 `global_channel_to_group`、`guild_url_image_to_base64`、`server_temp_qqguild`、`get_g_list_all_guilds` 等）会被 YAML 宽松解析忽略，不会报错，但不生效。新版本生成的配置文件仅包含实际有效的配置项。

## 删除的 Handler 文件（6 个）

- `handlers/send_guild_channel_msg.go`（667 行）
- `handlers/send_guild_channel_forum.go`（305 行）
- `handlers/send_guild_private_msg.go`（264 行）
- `handlers/get_guild_list.go`（82 行）
- `handlers/get_guild_channel_list.go`（81 行）
- `handlers/get_guild_service_profile.go`（60 行）

## 删除的 Processor 文件（5 个）

- `Processor/ProcessGuildATMessage.go`
- `Processor/ProcessGuildNormalMessage.go`
- `Processor/ProcessGuildMember.go`
- `Processor/ProcessChannelDirectMessage.go`
- `Processor/ProcessThreadMessage.go`

## botgo SDK Fork 精简

### dto 删除文件（15 个）

`guild.go`、`channel.go`、`channel_permissions.go`、`direct_message.go`、`forum.go`、`announces.go`、`api_permissions.go`、`audio.go`、`message_audit.go`、`message_reaction.go`、`message_setting.go`、`mute.go`、`pins.go`、`schedule.go`、`member.go`、`role.go`

### dto 修改文件

- `websocket_payload.go`：删除 `WSGuildData`/`WSGuildMemberData`/`WSChannelData`/`WSMessageData`/`WSATMessageData`/`WSDirectMessageData`/`WSMessageDeleteData`/`WSPublicMessageDeleteData`/`WSDirectMessageDeleteData`/`WSAudioData`/`WSMessageReactionData`/`WSMessageAuditData`/`WSThreadData`/`WSPostData`/`WSReplyData`/`WSForumAuditData` 等 16 个频道 payload 类型
- `message.go`：删除 `Member`、`DirectMessage`、`SeqInChannel`、`SrcGuildID` 字段

### openapi 删除文件（v1+v2 共 30 个）

v1：`guilds.go`、`channels.go`、`channel_permissions.go`、`direct_message.go`、`pins.go`、`audio.go`、`schedule.go`、`announces.go`、`mute.go`、`message_reaction.go`、`member.go`、`role.go`、`api_permissions.go`、`message_setting.go`、`webhook.go`（后恢复为通用 HTTP 回调）

v2：同上对称删除

### openapi 修改文件

- `iface.go`：删除 13 个频道 API 接口块（GuildAPI/ChannelAPI/ChannelPermissionsAPI/AudioAPI/RoleAPI/MemberAPI/DirectMessageAPI/AnnouncesAPI/ScheduleAPI/APIPermissionsAPI/PinsAPI/MessageReactionAPI/MessageSettingAPI），保留 UserAPI/MessageAPI（群聊）/WebhookAPI/InteractionAPI
- `event/register.go`：删除 16 个频道 EventHandler 类型和注册逻辑
- `event/event.go`：删除频道事件路由表和处理函数

## 删除的配置项

### structs.go（7 个字段）

- `GlobalChannelToGroup`
- `GlobalPrivateToChannel`
- `GlobalForumToChannel`
- `GuildUrlImageToBase64`
- `GlobalServerTempQQguild` / `ServerTempQQguild` / `ServerTempQQguildPool`
- `GetGroupListAllGuilds` / `GetGroupListGuilds` / `GetGroupListReturnGuilds` / `GetGroupListGuidsType`

### config.go（8 个访问器）

对应上述字段的 `GetXxx()` 访问器全部删除。

### template/config_template.go

删除频道 handler 注释、3 个转换配置、3 个频道发图配置、4 个 `get_g_list` guild 配置。新版本配置模板仅包含实际有效的配置项。

## 其他修改

- `images/upload_api.go`：删除 V3 频道发图分支和 `UploadBehaviorV3`/`postImageToServerV3` 函数
- `server/uploadpic.go`：删除 `UploadBase64ImageHandlerV3` 频道 multipart 端点
- `main.go`：删除 5 个频道 EventHandler、频道 intent 抑制逻辑、`uploadpicv3` 路由
- `Processor/Processor.go`：删除 `OnebotChannelMessage` 结构体、频道 downtimemessage/GetIDAndType2/unlock/Autobind/SendMessage/SendMessageMd 分支
- `Processor/ProcessC2CMessage.go`：剔除 `GlobalPrivateToChannel` 频道转换分支
- `Processor/ProcessInlineSearch.go`：剔除频道回调分支
- `echo/messageidmap.go`：删除 `strings.HasPrefix(msgType, "guild")` 判断
- `webui/api.go`：删除频道端点（`handleSendGuildChannelMessage`/`handleGetGuildList`/`handleGetChannelList`）
- `httpapi/httpapi.go`：删除 `send_guild_channel_msg` 端点
- `handlers/message_parser.go`：删除 `case "guild"`/`case "guild_private"` 分支和频道 payload case
- `handlers/send_group_msg.go`/`send_group_msg_raw.go`/`send_msg.go`/`send_private_msg.go`：删除 guild/guild_private/forum 分支和递归枚举中的 guild 项
- `handlers/delete_msg.go`：删除频道消息撤回分支
- `handlers/get_group_info.go`：删除 `ConvertGuildToGroupInfo` 函数
- `handlers/get_group_list.go`：删除频道轮询逻辑，仅保留 idmap 群列表
- `handlers/get_group_member_list.go`：删除 `case "guild"` 分支
- `handlers/set_group_ban.go`/`set_group_whole_ban.go`：删除频道禁言分支
- `handlers/reply_helpers.go`（新增）：从已删除的 `send_guild_channel_msg.go` 迁移 `GenerateReplyMessage` 函数

## 保留项

- **QQ 频道图床（oss_type=6）**：保留枚举位和 `imagehosting/qq_channel.go` 实现，作为通用图床后端保留。
- **`InlineSearch`/`InteractionHandler`**：群聊也支持互动按钮回调，保留。
- **idmap 数据库残留的 guild/channel 映射数据**：无影响，可保留。

## 验证结果

- `go build ./...` 通过
- `go vet ./...` 通过
- `go test ./handlers/` 通过
- `go test ./Processor/` 环境依赖 FAIL（`instance is nil`，与本次改动无关）
