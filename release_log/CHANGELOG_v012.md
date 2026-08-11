# Changelog — Release012

> 自 Release011 (`fb16b48`) 以来的所有变更。本轮补齐 QQ 官方 API v2 全部群聊接口（文档全量遍历确认共 17 个，11 个群聊管理/审批类接口均为新增），接入入群申请事件（`GROUP_JOIN_REQUEST`），`set_group_add_request` 由 MOCK 改为真实审批调用，并修复多段回复 msgseq 去重与私聊 reply 越权等问题。

---

## 🚀 新功能

### 群聊 API 全量补齐（11 个接口，12 个方法）

基于官方文档实现全部群聊管理类接口，其中 **6 个入群自动审批策略接口为本轮新增**，其余 5 个为上一轮开发、本轮随提交落库：

| 接口 | HTTP 路径 | 方法 | 说明 |
|------|-----------|------|------|
| 获取群基本信息 | `/v2/groups/{group_openid}/info` | GET | 群名、简介、分类、标签、成员数 |
| 获取机器人群内状态 | `/v2/groups/{group_openid}/bot_state` | GET | 入群时间、是否可推送、角色 |
| 入群申请列表拉取 | `/v2/groups/{group_openid}/join_request_list` | GET | 分页拉取，需群管理员身份 |
| 入群申请审批 | `/v2/groups/{group_openid}/approval_join_request/{member_openid}` | POST | approve/decline，支持拒绝理由与拉黑 |
| 查询群禁言状态 | `/v2/groups/{group_openid}/restrict_chat_setting` | GET | 全员禁言 + 成员级禁言列表 |
| 设置群成员禁言 | `/v2/groups/{group_openid}/restrict_chat_setting` | POST | 单次最多 10 个成员 |
| 创建入群自动审批策略 | `/v2/groups/join_approval_strategy` | POST | 一个机器人最多 20 个策略 |
| 查询策略列表 | `/v2/groups/join_approval_strategy` | GET | cursor 分页，limit 默认 20 最大 100 |
| 修改策略 | `/v2/groups/join_approval_strategy/{strategy_id}` | PATCH | 启停/过期/增删关联群/备注 |
| 执行策略 | `/v2/groups/join_approval_strategy/{strategy_id}/execute` | POST | 对关联群全量扫描，异步约 10 分钟 |
| 修改策略白名单 | `/v2/groups/join_approval_strategy/{strategy_id}/whitelist_users` | POST | 单次最多 10000 个 QQ 号码 |
| 删除策略 | `/v2/groups/join_approval_strategy/{strategy_id}` | DELETE | — |

既有群聊消息类接口（发送/撤回群消息、富媒体上传/预上传/分片完成）保持不变，至此 17 个群聊接口全部覆盖。

### GROUP_JOIN_REQUEST 入群申请事件

官方文档确认该事件属于 `GROUP_AND_C2C_EVENT`（1<<25）Intent，机器人需为群管理员才可收到：

- **botgo 事件层**：事件常量、intent 映射、`ParseData` 事件 ID 注入、`GroupJoinRequestEventHandler` 注册（含未注册时的告警日志）
- **Processor 接入**：新增 `ProcessGroupJoinRequest`，将 `group_openid`/`member_openid` 映射为虚拟 ID，上报 OneBot `request` 事件（`request_type=group`、`sub_type=add`），`flag` 携带 `join_request_id` 供审批使用，`comment` 携带验证信息
- **配置**：`text_intent` 新增 `GroupJoinRequestEventHandler`（需群管理员身份）

### set_group_add_request 真实化

- `handlers/set_group_add_request.go` 由 MOCK 占位改为真实调用 v2 审批接口：虚拟 ID 经 idmap 反查真实 OpenID，`flag` 作为 `join_request_id`，`approve` 映射为 `op=approve/decline`
- `callapi.ParamsContent` 新增扩展参数：`approve`、`flag`、`reason`（拒绝理由）、`add_to_member_blacklist`（是否拉黑）

---

## 🔧 修复

| 文件 | 修复内容 |
|------|----------|
| `handlers/send_private_msg.go` | 私聊富媒体（msg_type=7）reply 跨场景 `msg_id` 越权（错误码 40034024），富媒体路径补齐 `MessageReference`/`MsgID` 设置 |
| `handlers/send_private_msg.go` / `send_group_msg.go` / `echo/echo.go` | 私聊 reply 越权、群聊 Markdown 发送 panic、多段回复 msgseq 去重 |
| `echo/messageidmap.go` | `GetLazyMessagesId`/`GetLazyMessagesIdv2` 移除选中后的 `usageCount++`，同一回复链复用同一 `msg_id`，配合 `msg_seq` 连续递增，避免偶发 `40054005 msgseq 去重`（Issue #19） |
| `botgo/dto/group.go` | `GroupJoinRequestEvent` 补 `ID`/`EventID` 字段（修复编译错误），`ApplyAt` 改为 `interface{}` 兼容不同类型时间戳 |
| `botgo/openapi/v2/group.go` | `ApprovalJoinRequest` 请求体字段名修正（`action` → `op`），并支持 `join_request_id`/`reject_reason`/`add_to_member_blacklist` |

---

## 📦 构建与工程

| 文件 | 变更 |
|------|------|
| `.github/dependabot.yml` | 新增 Dependabot 配置，覆盖 Go/npm/GitHub Actions 依赖 |
| `.gitignore` | 新增 `.qoder/` 忽略项 |

---

## 📝 文档同步

| 文件 | 变更 |
|------|------|
| `release_log/CHANGELOG_v012.md` | 本文档（新建） |
| `template/config_template.go` | `text_intent` 注释新增 `GroupJoinRequestEventHandler` |
| `readme.md` | 已实现 Intent 列表新增 `GroupJoinRequestEventHandler`（入群申请） |
| `AGENTS.md` | botgo Fork 描述补充群聊管理 API（GroupAPI）与入群申请事件 |
| `AGENTS.md` / `docs/本版新增功能.md` / `CHANGELOG_v010.md` | lazy_message_id msgseq 去重修复的文档同步（`2c0baf9`） |

---

## 🧪 验证

| 命令 | 结果 |
|------|------|
| `go build ./...` | ✅ 通过 |
| `go vet ./...` | ✅ 通过 |
| `go test ./handlers/` | ✅ 通过 |
| `go test ./Processor/` | ✅ 通过 |

---

## ✅ 提交记录

```
43b816e  feat: 补齐群聊 API 并接入入群申请事件
0b4ee93  ci: 新增 Dependabot 配置覆盖 Go/npm/GitHub Actions 依赖
59a8455  chore: gitignore 新增 .qoder/ 忽略项
d5c780b  Merge pull request #17 from Te-River/Refactor
b9da907  Merge remote-tracking branch 'origin/main' into Refactor
2c0baf9  docs: lazy_message_id 多段回复 msgseq 去重修复的文档同步
1ed26a5  fix: lazy_message_id 多段回复偶发 40054005 msgseq 去重
5a9c36d  fix: 修复私聊reply越权/群聊Markdown panic/多段回复msgseq去重
e72e04f  fix: 修复私聊富媒体reply跨场景msg_id越权(40034024)
```
