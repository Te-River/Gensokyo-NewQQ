# Changelog — Release012

> 自 Release011 (`d5c780b`) 以来的所有变更。

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

## 📝 文档更新

### 标准 CQ 码文档完善

**新增文件：** `docs/cq码/` 下 12 个标准 CQ 码文档 + 1 个统一汇总

为 OneBot V11 标准 CQ 码编写了完整的文档，包括：

- `CQ码汇总.md`：统一索引页，汇总标准 CQ 码和扩展 CQ 码
- `标准CQ码/标准cq码-cq-text.md`：纯文本
- `标准CQ码/标准cq码-cq-face.md`：QQ 表情
- `标准CQ码/标准cq码-cq-image.md`：图片（含 SSRF 防护说明）
- `标准CQ码/标准cq码-cq-record.md`：语音（含 silk 转码流程和 SSRF 防护说明）
- `标准CQ码/标准cq码-cq-video.md`：视频（含 SSRF 防护说明）
- `标准CQ码/标准cq码-cq-at.md`：@ 标签（含 idmap 转换和剥离逻辑说明）
- `标准CQ码/标准cq码-cq-share.md`：链接分享（标注 QQ Bot API 不支持）
- `标准CQ码/标准cq码-cq-location.md`：位置（标注 QQ Bot API 不支持）
- `标准CQ码/标准cq码-cq-music.md`：音乐（仅 QQ 音乐）
- `标准CQ码/标准cq码-cq-reply.md`：回复（含私聊越权防护说明）
- `标准CQ码/标准cq码-cq-forward.md`：合并转发（标注 QQ Bot API 不支持）

同步更新了 `docs/更多文档.md` 文档索引，新增标准 CQ 码章节。

---

## 📦 依赖更新

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

## 🧪 验证

| 命令 | 结果 |
|------|------|
| `go build ./...` | ✅ 通过 |
| `go vet ./...` | ✅ 通过 |
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
86888fb  fix: 非全量群重复 @ 修复（add_at_group 增加 remove_at 判断）
158c33c  fix: 被动消息中移除 add_at_group 添加 @ 逻辑
bef1750  fix: transformMessageTextAt 根据 remove_bot_at_group 移除 @bot
961daf4  refactor: 正则直接匹配 AppID，简化 @bot 移除逻辑
6421bcd  移除非全量消息@用户重复（PR #47）
2a6315d  fix: 全量群消息(DisableErrorChan=true)时 @Bot 未被剥离
4bbaf17  fix: 全量群消息 @Bot 剥离失败 — IsSelfAtID 因 ID 格式不匹配返回 false
1ed26a5  fix: lazy_message_id 多段回复偶发 40054005 msgseq 去重（Issue #19）
e72e04f  fix: 修复私聊富媒体reply跨场景msg_id越权(40034024)
5a9c36d  fix: 修复私聊reply越权/群聊Markdown panic/多段回复msgseq去重
bcba13d  修复图文消息cqat未转换（PR #16）
9240987  fix: bound HTTP resources and atomize message sequences
b93b01d  refactor: centralize delivery retry classification
21d0015  refactor: route handler errors through retry classifier
15cef7c  refactor(config): introduce immutable runtime snapshots（P2）
cb75b38  refactor(identity): add typed identity resolver（P3）
b751e65  refactor(message): introduce typed parsed message model（P4）
762ab11  refactor(media): unify media pipeline with safe fetcher（P5）
2f1cbe6  refactor(outbound): introduce unified outbound service（P6）
fe4a4a7  refactor(queue): add bounded queue with session ordering（P7）
444ae1f  refactor(processor): introduce inbound event pipeline（P8）
514be8c  refactor(callapi): add typed actions and explicit registry（P9）
9e58ce9  refactor(storage): repository-ize idmap and echo state（P10）
8cb1c6b  refactor(config): remove global config reads from new architecture（P11）
a305fea  refactor(adapter): isolate botgo and media SDKs（P12）
a926dda  refactor(legacy): document P13 deletion gating（P13）
87d34f8  security: remove embedded cloud credentials from Nature provider
d030501  revert: restore nature public image hosting credentials
ecff462  feat: 合并 PR #49 — 分阶段重构基础设施（P2-P12）
```
