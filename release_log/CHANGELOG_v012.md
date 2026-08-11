# Changelog — Release012

> 自 Release011 以来的所有变更。本轮合并 main 与 Pr-Edit 两条线：Pr-Edit 侧补齐 QQ 官方 API v2 全部群聊接口（文档全量遍历确认共 17 个）并接入入群申请事件（`GROUP_JOIN_REQUEST`）；main 侧包含语音上传修复、@Bot 剥离改进、分阶段重构基础设施（PR #49）等既有变更。

---

## 🆕 新功能

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

### 群聊管理 API 暴露为 OneBot action

全部 11 个群聊管理类接口现已注册为 OneBot action，下游插件可直接调用：

| Action | 对应 botgo API | 说明 |
|--------|---------------|------|
| `get_group_info`（真实化） | `GroupInfo` | 返回真实群名/简介/成员数（原为 MOCK 假数据） |
| `set_group_ban`（真实化） | `SetRestrictChatSetting` | `duration` 秒禁言，0=解除（原为"暂未开放"骨架） |
| `set_group_whole_ban`（真实化） | `SetRestrictChatSetting(AllMute)` | `enable` 开关全员禁言，保留已有成员级禁言 |
| `get_group_join_request_list`（新增） | `JoinRequestList` | 入群申请列表，`next_index` 分页；返回的 `group_id`/`user_id`/`flag` 可直接回传审批 |
| `get_group_bot_state`（新增） | `BotInGroupState` | 机器人群内状态（入群时间/可推送/角色） |
| `join_approval_strategy_create/list/update/execute/whitelist/delete`（新增） | 6 个策略 API | 入群自动审批策略全生命周期管理 |

`set_group_ban`/`set_group_whole_ban` 同时保留旧 action 名 `get_group_ban`/`get_group_whole_ban` 兼容。`callapi.ParamsContent` 同步新增 `next_index`/`cursor`/`limit`/`strategy_id`/`group_openids`/`group_ids`/`is_enable`/`expire_at`/`remark`/`op`/`whitelist_users` 参数。

---

## 🐛 Bug 修复

### 语音上传失败修复

**文件：** `handlers/send_group_msg.go`

`url_record`/`url_records` 处理路径调用了 `UploadBase64RecordToServer`（上传到本地服务器），当 `server_dir` 是私有地址时会失败。

**修复：** 改为与 `local_record` 一致，直接调用 `CreateAndUploadMediaMessage` 上传 QQ CDN。

---

### 媒体消息 `url` 字段兼容修复

**文件：** `handlers/message_parser.go`

部分客户端（如 Koishi）使用 `url` 字段而非 `file` 字段传递媒体路径，导致本地文件路径被当作 `unknown_*` 处理，触发 SSRF 阻止。

**修复：** 在 `image`、`voice/record`、`video` 的解析中，当 `file` 字段为空时，回退读取 `url` 字段。

---

### lazy_message_id 多段回复偶发 40054005 msgseq 去重（Issue #19）

**文件：** `echo/messageidmap.go`

`lazy_message_id=true` 下一条命令触发多段独立回复时，偶发某段发送失败（QQ API 返回 `40054005 消息被去重`）。

**根因：** `GetLazyMessagesId`/`GetLazyMessagesIdv2` 选中一条 record 后执行 `usageCount++`，导致同一命令的多段回复每次选中不同 record，各段拿到不同 `msg_id`。QQ API 视两 `msg_id` 为同一回复链，要求 `msg_seq` 连续递增，但不同 `msg_id` 各自独立计数导致 seq 冲突。

**修复：** 移除选中后的 `usageCount++`，让同一回复链的多段回复复用同一 `msg_id`，配合 `GetMappingSeq`/`AddMappingSeq` 连续递增 `msg_seq`。

---

### 私聊 reply 跨场景 msg_id 越权（40034024）

**文件：** `handlers/send_private_msg.go`、`handlers/send_group_msg.go`

私聊 reply 时，虚拟 ID 反查的真实 ID 格式校验存在缺陷（`UserID MessageID` 格式不匹配），导致群聊 `msg_id` 被引用到私聊场景，触发 QQ API `40034024` 越权错误。

**修复：** `applyPrivateReply` 改用反查前 `RetrieveRowByIDv2` 校验虚拟 ID 归属是否为当前私聊目标 UserID，不一致则跳过 reply 避免引用群聊 msg_id 越权。富媒体（msg_type=7）/纯文本/markdown 三处 reply 段统一改用 `applyPrivateReply` 辅助函数。

---

### 群聊 Markdown nil pointer panic

**文件：** `handlers/send_group_msg.go`

`parseMarkdownFromMessage` 解析失败返回 nil 时，`md.Content = messageText + md.Content` 触发空指针 panic。

**修复：** 补 `if md != nil` 校验，nil 时跳过文本合并，走纯文本路径。

---

### 图文消息 CQ:at 未转换（PR #16）

**文件：** `handlers/message_parser.go`

图文消息走 Markdown 路径（`auto_md`）时，`[CQ:at,qq=...]` 未被正确转换为 QQ @ 语法，导致原文显示。

**修复：** 在 Markdown 消息处理路径中补充 CQ 码 @ 转换逻辑。

---

### botgo 层修复

**文件：** `botgo/dto/group.go`、`botgo/openapi/v2/group.go`

- `GroupJoinRequestEvent` 补 `ID`/`EventID` 字段（修复编译错误），`ApplyAt` 改为 `interface{}` 兼容不同类型时间戳
- `ApprovalJoinRequest` 请求体字段名修正（`action` → `op`），并支持 `join_request_id`/`reject_reason`/`add_to_member_blacklist`

---

## 🔧 @Bot 剥离改进

### 全量群消息 @Bot 剥离修复

**背景：** QQ 平台在不同场景使用不同 ID 格式（全局 OpenID vs 群特定 OpenID），导致 `handlers.BotID`（来自 `/users/@me`）与群消息 Mentions 中的 ID 可能不一致，`IsSelfAtID` 无法识别为自身 @。

**修复（`2a6315d`）：** 全量群消息（`DisableErrorChan=true`）时 `RevertTransformedText` 不会被调用，`<@BotOpenID>` 原样上报。在 `ProcessGroupNormalMessage` 中添加独立的 @Bot 剥离步骤，无论 `DisableErrorChan` 如何设置均执行。导出 `handlers.IsSelfAtID` 供 Processor 包调用。

**修复（`4bbaf17`）：** 在 `resolveIncomingAtID` 解析出 atID 后，额外检查 atID 是否等于 bot 的 UIN 或 AppID，若是则认定为 @Bot 自身并剥离。同时修复 `RevertTransformedText`（string 模式）和 `ConvertToSegmentedMessage`（array 模式）两处。

---

### @bot 移除逻辑简化

**文件：** `handlers/message_parser.go`

用正则直接匹配 AppID 简化 @bot 移除逻辑，不再依赖复杂的 ID 格式判断。`transformMessageTextAt` 根据 `remove_bot_at_group` 配置移除 @bot。

---

### 非全量群消息 @用户重复修复

**文件：** `handlers/message_parser.go`、`handlers/send_group_msg.go`、`handlers/send_group_msg_raw.go`

#### 非全量群重复 @ 修复（`86888fb`）

未开启全量群消息（仅订阅 `GROUP_AT_MESSAGE_CREATE` 被动消息）的群中，`add_at_group` 配置会添加 `[CQ:at,qq=AppID]`，但被动消息本身已包含对 bot 的 @，导致出站消息出现重复 @。

**修复：** 被动消息（`GROUP_AT_MESSAGE_CREATE`）中直接移除 `add_at_group` 添加 `[CQ:at,qq=AppID]` 的逻辑。`add_at_group` 仅在全量群消息（`GROUP_MESSAGE_CREATE`）中生效。

#### 非全量消息 @用户重复修复（PR #47）

非全量群消息中，出站回复时出现 @用户重复。进一步修复了非全量消息场景下 @用户重复的问题，并同步更新了 `send_group_msg` 和 `send_group_msg_raw` 的处理逻辑。

---

## 🔧 稳定性修复

### HTTP 资源边界与消息序列原子化（`9240987`）

**涉及：** `echo/`、`handlers/`、`idmap/`、`imagehosting/`、`server/`、`Processor/` 等 26 个文件

- HTTP 客户端增加超时和连接资源边界控制，防止资源泄漏
- 消息序列号生成原子化，避免并发冲突
- idmap HTTP 客户端增加安全边界
- imagehosting 各后端统一资源管理
- server 端 HTTP/webhook 增加请求边界控制
- echo 系统并发安全改进

### 投递重试分类统一

- `b93b01d`：集中投递重试分类逻辑
- `21d0015`：handler 错误路由到重试分类器

---

## 🆕 新增接口

### idmap 批量身份映射导出（`/getid?type=19`）

**文件：** `idmap/service.go`、`server/getIDHandler.go`

新增 `GET /getid?type=19` 接口，一次性导出所有身份映射（OpenID <-> 虚拟 ID），方便迁移调试和数据核查。

**新增结构体：**
- `IdentityMapping`：单条映射记录（`real_id`、`virtual_id`、`username`）
- `IdentitySnapshot`：快照响应（`mappings`、`count`、`timestamp`）

**新增函数：**
- `ListAllIdentities() (*IdentitySnapshot, error)`：遍历 identityDB 导出所有正向映射

**快照机制：**
- 使用 bbolt MVCC `View` 事务，在请求到达时生成一致性快照
- 即使遍历期间有其他 goroutine 写入 identityDB，也不影响本次导出结果
- `timestamp` 字段记录快照生成时间（Unix 时间戳，秒）

**注意事项：**
- type=19 不走 lotus/gRPC 远程调用，始终查询本地 identityDB
- 受 `IDMapAuthMiddleware` 保护（lotus password 或本地回环限制）

详见 [docs/idmap.md](./docs/idmap.md#批量导出接口)。

---

## 🏗 分阶段重构基础设施（PR #49）

**说明：** 本轮重构在旧代码旁新增 `internal/` 分层架构包（双轨并存，未接入生产），生产路径（handlers / Processor / callapi / idmap / echo）行为完全不变。

### 配置基础设施：Immutable Snapshot 管线（P2）

**新增 `internal/infrastructure/config/`：**

- `loader.go`：读取/解析 `config.yml` → `ConfigDTO`
- `migrator.go`：`Migrator` 接口（基于 `yaml.Node`，v0→v1 自动补 `version`）
- `validator.go`：`ValidateSchema` + `ValidateSemantic`（错误带具体路径）
- `runtime.go`：`ConfigDTO → RuntimeConfig`（slice 深拷贝）
- `snapshot.go`：不可变 `Snapshot` + `Manager`（重载失败保留旧快照）
- `writer.go`：`AtomicWrite`（tmp + fsync + `.bak` 备份 + rename）
- `watcher.go`：fsnotify + debounce
- `errors.go`：分类错误（Parse/Migration/Validation/IO）+ 字段路径

### Identity 类型化（P3）

**新增 `internal/domain/identity/`：**

- `OpenID` / `OpenGroupID` / `VirtualUserID` / `VirtualGroupID` / `UIN` / `AppID` 类型 + 转换
- `IsOpenID` / `IsVirtualID`（长度启发式唯一收敛点）
- `IdentityResolver` 接口 + `UserRef`/`GroupRef`/`ResolvedUser`/`ResolvedGroup`
- 新增 `adapter/identity/`：基于 idmap 的真实 Resolver 实现

**长度启发式收敛：** 全仓 `len(id)==32 / !=32` 身份判断归零，替换为 `identity.IsOpenID(...)`。

### 消息解析类型化（P4）

**新增 `internal/domain/message/`：**

- `MessagePart` 类型系统 + `ParsedMessage`（Parts/Reply/DeliveryMode）
- `ParseOneBotString`（String）与 `ParseOneBotSegments`（Array）输出同一模型
- CQ 码扫描器（转义感知参数拆分、malformed 容错）
- `MediaSource`（LocalFile/RemoteURL/Base64/Bytes）
- compat bridge：`Canonicalize` + `ToLegacyFoundItems`（仅迁移期）

### 媒体管线统一（P5）

**新增 `internal/application/media/`：**

- `MediaService.Prepare(ctx, source, policy)`：Local/URL/Base64/Bytes 统一入口
- `SafeHTTPFetcher`：timeout / max bytes / 重定向限制 / SSRF 检查
- Base64 decode 前长度限制（防 OOM）；图片尺寸/像素校验（防解压炸弹）
- 本地文件：AllowedDirs + 扩展名 + regular + 大小校验
- `PreparedMedia.Close()` 临时文件生命周期（幂等）

### 出站消息模型（P6）

**新增 `internal/application/outbound/`：**

- `OutboundService.Send(ctx, OutboundCommand)`：唯一发送主链
- `OutboundMessage` / `OutboundCommand` / `DeliveryPolicy`
- `QQSender` 接口（Application 不 import botgo DTO）
- `RetryPolicy` + `ErrorClassifier`

### 队列与 WebSocket 生命周期（P7）

**新增 `internal/application/queue/`：**

- 有界队列：容量均分分区、session hash 分区保证顺序
- 背压显式选择：Block / Drop / Reject
- 重试走 delay scheduler（不占 worker Sleep）
- Close/Wait 可预测 shutdown；Metrics

### Processor 分层（P8）

**新增 `internal/domain/event/` + `internal/application/inbound/` + `adapter/onebot/`：**

- `DomainEvent` + `EventSource`
- `EventNormalizer` / `EventPublisher` 接口
- `IsSelfMention` / `NormalizeMentions`：@Bot 唯一 canonical 实现
- OneBot serializer：`SerializeString` / `SerializeArray`

### HTTP/WS/callapi Adapter（P9）

**新增 `internal/application/action/`：**

- `Registry` 显式注册表 + `Dispatcher`（HTTP/WS 共用）
- `Envelope` / `Handler` / typed `SendMessageAction`（int/string ID 兼容）

### idmap/echo 仓储化（P10）

**新增 `internal/application/state/`：**

- `SequenceRepository`：msgseq 只允许原子 Next
- `MessageContextRepository`：owner+key 隔离 + TTL 过期校验
- `IdentityRepository` 复用 P3 的 IdentityResolver

### 全局配置依赖收口（P11）

- 验证 `internal/` + `adapter/` 全部无 `config` import
- outbound 增加 `OutboundConfig` 构造注入

### SDK / Generated 边界隔离（P12）

- 新增 `adapter/qq/`（botgo → typed identity 转换边界）
- 验证 domain/application 无 botgo/go-silk
- 新增 fork inventory：`docs/forks/{botgo,go-silk}.md`
- 新增 `scripts/generate.{ps1,sh}`

### P13 — 条件未满足，破坏性删除 BLOCKED

- 生产路径尚未切换到新架构，无真实联调稳定周期 → **不执行破坏性删除**
- 已完成非破坏收尾：新架构无 foundItems（仅 compat bridge）、无 config/botgo/go-silk

---

## 🔒 Nature 图床凭据说明

`imagehosting/nature.go` 内置的腾讯 COS 凭据（`oss_type=10` Nature 免费图床）为**公开共享凭据**，并非用户私有密钥，无泄露风险，维持"开箱即用"行为。

- 曾一度将其误判为需移除的私有云凭据并改为配置注入（提交 `87d34f8`），经确认属于误判，已回滚（`d030501`）。
- 当前 `oss_type=10`（Nature）保持内置凭据、开箱即用；如需自定义 COS 请使用 `cos.go`（需配置）。

---

## 📦 构建与工程

| 文件 | 变更 |
|------|------|
| `.github/dependabot.yml` | 新增 Dependabot 配置，覆盖 Go/npm/GitHub Actions 依赖 |
| `.gitignore` | 新增 `.qoder/` 忽略项 |

### 依赖更新

| PR | 变更 |
|----|------|
| #20 | `golang.org/x/net` → 0.55.0（botgo/examples） |
| #21 | `golang.org/x/crypto` → 0.52.0 |
| #22 | `form-data` → 4.0.6（frontend） |
| #23 | `google.golang.org/grpc` → 1.82.1 |
| #26 | `golang.org/x/image` → 0.41.0（botgo） |
| #28 | `actions/setup-node` v6 → v7 |
| #29 | `actions/setup-go` v6 → v7 |
| #30 | go_modules minor-and-patch 14 项更新 |
| #40 | `@types/node` → 26.1.2（frontend） |
| #43 | go_modules 3 项更新 |
| #44 | go_modules 2 项更新 |

---

## 📝 文档同步

| 文件 | 变更 |
|------|------|
| `release_log/CHANGELOG_v012.md` | 本文档（新建，融合 main 与 Pr-Edit 两份变更记录） |
| `template/config_template.go` | `text_intent` 注释新增 `GroupJoinRequestEventHandler` |
| `readme.md` | 已实现 Intent 列表新增 `GroupJoinRequestEventHandler`（入群申请）；API 表新增 8 个群聊管理 action 并标注真实化 |
| `docs/api/api介绍.md` | 标准/扩展 API 表新增 8 个 action 及参数说明 |
| `docs/api/扩展API文档.md` | 扩展 API 索引新增 8 个 action |
| `docs/api/扩展api/` | 新增 `get_group_join_request_list`、`get_group_bot_state`、`join_approval_strategy`（6 action）详细文档 |
| `docs/本版新增功能.md` | 事件表新增 `GroupJoinRequestEventHandler`；API 表 8 个 action 加链接；`set_group_add_request` 过时 MOCK 描述修正 |
| `AGENTS.md` | botgo Fork 描述补充群聊管理 API（GroupAPI）与入群申请事件 |
| `docs/cq码/` | 标准 CQ 码文档完善（12 个标准 CQ 码文档 + 1 个统一汇总，同步 `docs/更多文档.md` 索引） |
| `docs/forks/` | fork inventory：`botgo.md`、`go-silk.md`（P12） |

---

## 🧪 验证

| 命令 | 结果 |
|------|------|
| `go build ./...` | ✅ 通过 |
| `go vet ./...` | ✅ 通过 |
| `go test ./handlers/` | ✅ 通过 |
| `go test ./Processor/` | ✅ 通过 |
| `go test ./imagehosting/ ./config/ ./structs/ ./template/` | ✅ 通过 |
| `go test ./internal/infrastructure/config/` | ✅ 通过（24 用例） |
| `go test ./internal/domain/identity/ ./adapter/identity/` | ✅ 通过（12 用例） |
| `go test ./internal/domain/message/` | ✅ 通过（96.9% coverage） |
| `go test ./internal/application/media/` | ✅ 通过（69.6% coverage） |
| `go test ./internal/application/outbound/` | ✅ 通过（90.9% coverage） |
| `go test ./internal/application/queue/` | ✅ 通过（86.1% coverage） |
| `go test ./internal/application/inbound/ ./adapter/onebot/` | ✅ 通过 |
| `go test ./internal/application/action/` | ✅ 通过（87.5% coverage） |
| `go test ./internal/application/state/` | ✅ 通过（97.5% coverage） |
| `go test ./...` | ✅ 通过 |

---

## ✅ 提交记录

```
ecff462  feat: 合并 PR #49 — 分阶段重构基础设施（P2-P12）
a926dda  refactor(legacy): document P13 deletion gating（P13）
a305fea  refactor(adapter): isolate botgo and media SDKs（P12）
8cb1c6b  refactor(config): remove global config reads from new architecture（P11）
9e58ce9  refactor(storage): repository-ize idmap and echo state（P10）
514be8c  refactor(callapi): add typed actions and explicit registry（P9）
444ae1f  refactor(processor): introduce inbound event pipeline（P8）
fe4a4a7  refactor(queue): add bounded queue with session ordering（P7）
2f1cbe6  refactor(outbound): introduce unified outbound service（P6）
762ab11  refactor(media): unify media pipeline with safe fetcher（P5）
b751e65  refactor(message): introduce typed parsed message model（P4）
cb75b38  refactor(identity): add typed identity resolver（P3）
15cef7c  refactor(config): introduce immutable runtime snapshots（P2）
21d0015  refactor: route handler errors through retry classifier
b93b01d  refactor: centralize delivery retry classification
9240987  fix: bound HTTP resources and atomize message sequences
bcba13d  修复图文消息cqat未转换（PR #16）
5a9c36d  fix: 修复私聊reply越权/群聊Markdown panic/多段回复msgseq去重
e72e04f  fix: 修复私聊富媒体reply跨场景msg_id越权(40034024)
1ed26a5  fix: lazy_message_id 多段回复偶发 40054005 msgseq 去重（Issue #19）
4bbaf17  fix: 全量群消息 @Bot 剥离失败 — IsSelfAtID 因 ID 格式不匹配返回 false
2a6315d  fix: 全量群消息(DisableErrorChan=true)时 @Bot 未被剥离
6421bcd  移除非全量消息@用户重复（PR #47）
961daf4  refactor: 正则直接匹配 AppID，简化 @bot 移除逻辑
bef1750  fix: transformMessageTextAt 根据 remove_bot_at_group 移除 @bot
158c33c  fix: 被动消息中移除 add_at_group 添加 @ 逻辑
86888fb  fix: 非全量群重复 @ 修复（add_at_group 增加 remove_at 判断）
d030501  revert: restore nature public image hosting credentials
87d34f8  security: remove embedded cloud credentials from Nature provider
f535855  docs: 为新增群聊管理扩展 API 编写详细文档
ce9791d  feat: 群聊管理 API 全部暴露为 OneBot action
43b816e  feat: 补齐群聊 API 并接入入群申请事件
0b4ee93  ci: 新增 Dependabot 配置覆盖 Go/npm/GitHub Actions 依赖
```
