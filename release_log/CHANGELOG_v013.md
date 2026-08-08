# Changelog — Release013

> 自 Release012 以来的所有变更。

---

## 🔒 安全：移除源码内置云凭据（S0）

**背景：** `imagehosting/nature.go` 中曾硬编码一组腾讯 COS 的 SecretId/SecretKey（base64 编码），
用于 "Nature" 免费图床（oss_type=10）直传。内置真实云凭据属于高危安全缺陷，必须移除。

**变更：**

- `imagehosting/nature.go` 删除 base64 硬编码凭据及 `mustB64` 函数，改为从配置读取。
- `structs.Settings.Nature` 类型由空结构 `ImageHostingSimple` 改为 `ImageHostingNature`
  （含 `secret_id` / `secret_key` / `region` / `bucket` / `domain` 字段）。
- `config.GetImageHostingNature()` 返回类型同步更新。
- 凭据缺失时 **fail closed**（返回错误），不再回退到任何内置凭据。
- 默认域名不再指向旧存储桶的 CDN，改为与 `cos.go` 一致：留空时使用 COS 默认域名。
- `template/config_template.go`、`readme.md`、`imagehosting/README.md` 同步更新配置示例。

**⚠️ 破坏性变更：** `oss_type=10`（Nature）不再开箱即用，必须自行填写 COS 凭据。

**安全注意：** 请到腾讯云控制台 revoke 旧凭据（被内置在历史版本源码中的那组），
并确认旧凭据已无法继续使用。删除源码中的 Secret 不等于修复，**旧凭据失效**才是真正的修复点。

---

## 🏗 配置基础设施：Immutable Snapshot 管线（P2）

**背景：** 旧 `config/config.go` 全局 singleton 同时负责读取、补全、写入、重载，
缺少独立的 parse → migrate → validate → snapshot 管线，直接覆盖写存在截断风险。

**新增 `internal/infrastructure/config/`（与旧 config/ 双轨并存，未接入生产）：**

- `loader.go`：读取/解析 `config.yml` → `ConfigDTO`（只对磁盘格式负责）
- `migrator.go`：`Migrator` 接口（基于 `yaml.Node`，禁止字符串/行号/缩进 hack）；v0(legacy) → v1 自动补 `version`
- `validator.go`：`ValidateSchema` + `ValidateSemantic`
  - Schema：app_id 必填、端口范围、URL/地址格式、oss_type 枚举、超时/媒体上限非负，错误带具体路径（如 `config.idmap.grpc_port`）
  - Semantic：TLS 开启但证书缺失、图床开启但凭据缺失、Lotus 开启但 endpoint 缺失
- `runtime.go`：`ConfigDTO → RuntimeConfig`（QQ/OneBot/Transport/IDMap/Media 分组，slice 深拷贝）
- `snapshot.go`：`Snapshot`（不可变）+ `Manager`（重载失败保留旧快照，不置零不崩溃）
- `writer.go`：`AtomicWrite`（前置校验 + 临时文件 + fsync + `.bak` 备份 + rename，失败时原配置可用）
- `watcher.go`：fsnotify + debounce（事件风暴合并为一次重载）
- `errors.go`：分类错误（Parse/Migration/Validation/IO）+ 字段路径

**Golden fixtures（`testdata/config/`）：** `legacy-basic` / `legacy-full` / `v1-basic` /
`invalid-port` / `invalid-url` / `missing-secret` / `unknown-fields` / `malformed`

**说明：** 本阶段只建基础设施与测试，业务层切换（Snapshot 读取、watcher 接入）由后续 P11 完成，
保证旧行为完全兼容。

---

## 🆔 Identity 类型化（P3）

**背景：** 旧代码用 `len(id)==32` 长度启发式推断 OpenID/虚拟 ID，身份语义散落各处。

**新增 `internal/domain/identity/`：**

- `types.go`：`OpenID` / `OpenGroupID` / `VirtualUserID` / `VirtualGroupID` / `UIN` / `AppID` 类型 + 转换
- `classify.go`：legacy adapter `IsOpenID` / `IsVirtualID`（长度启发式唯一收敛点）
- `resolver.go`：`IdentityResolver` 接口 + `UserRef`/`GroupRef`/`ResolvedUser`/`ResolvedGroup`
- `target.go`：`TargetKind`（Group/Private）+ `ResolvedTarget`
- 新增 `adapter/identity/`：基于 idmap 的真实 Resolver 实现

**长度启发式收敛：** 全仓 `len(id)==32 / !=32` 身份判断归零（handlers 7 文件 + idmap 3 文件），
全部替换为 `identity.IsOpenID(...)`（纯等价，零行为差异）。

**遗留（P13 处理）：** 撤回/还原路径的 `params.GroupID = realOpenID` 式字段覆盖。

---

## 💬 消息解析类型化（P4）

**背景：** `handlers/message_parser.go` 的 `foundItems map[string][]string` 解析逻辑庞大且无类型。

**新增 `internal/domain/message/`（纯函数，无副作用）：**

- `MessagePart` 类型系统 + `ParsedMessage`（Parts/Reply/DeliveryMode）
- `ParseOneBotString`（String）与 `ParseOneBotSegments`（Array）输出同一模型
- CQ 码扫描器（转义感知参数拆分、malformed 容错）
- `MediaSource`（LocalFile/RemoteURL/Base64/Bytes，P5 复用）
- compat bridge：`Canonicalize` + `ToLegacyFoundItems`（仅迁移期）

**测试：** golden corpus（text/image/record/video/file/at/reply/markdown/keyboard/music/mixed/escaped/malformed/empty），
String/Array 一致性 + canonical 比较，覆盖率 96.9%。

**约束：** 冻结 foundItems（不新增 key），新功能一律进入 typed model。

---

## 🖼 媒体管线统一（P5）

**新增 `internal/application/media/`：**

- `MediaService.Prepare(ctx, source, policy)`：Local/URL/Base64/Bytes 统一入口
- `SafeHTTPFetcher`：timeout / max bytes / 重定向限制 / SSRF 检查 / 状态码 / 签名；大媒体流式落临时文件
- Base64 decode 前长度限制（防 OOM）；图片尺寸/像素校验（防解压炸弹）
- 本地文件：AllowedDirs + 扩展名 + regular + 大小校验（防任意文件读取）
- `PreparedMedia.Close()` 临时文件生命周期（幂等）+ 测试
- `MediaUploader` / `AudioTranscoder` 接口边界（云 SDK / FFmpeg / go-silk 封装点）

---

## 📤 出站消息模型（P6）

**新增 `internal/application/outbound/`：**

- `OutboundService.Send(ctx, OutboundCommand)`：唯一发送主链（Build→Send→Classify→Retry→fail）
- `OutboundMessage` / `OutboundCommand` / `DeliveryPolicy`（active/wakeup/fallback）
- `QQSender` 接口（Application 不 import botgo DTO）
- `RetryPolicy` + `ErrorClassifier`（解耦 QQ 错误码；P1 legacy 待 P13 收敛）

---

## ⏳ 队列与 WebSocket 生命周期（P7）

**新增 `internal/application/queue/`：**

- 有界队列：容量均分分区、session hash 分区保证顺序
- 背压必须显式选择：Block / Drop / Reject（Drop 有计数，不静默丢）
- 重试走 delay scheduler（不占 worker Sleep）
- Close/Wait 可预测 shutdown；Metrics（Capacity/Depth/Rejected/Processed/Active）

---

## 📥 Processor 分层（P8）

**新增 `internal/domain/event/` + `internal/application/inbound/` + `adapter/onebot/`：**

- `DomainEvent`（ID/Time/Source/Actor/Target/Message）+ `EventSource`
- `EventNormalizer` / `EventPublisher` 接口（QQ Adapter 边界）
- `IsSelfMention` / `NormalizeMentions`：@Bot 唯一 canonical 实现
- OneBot serializer：`SerializeString` / `SerializeArray`（纯表示层，无业务判断）

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
| `go test ./...` | ✅ 通过 |

---

## ✅ 提交记录

```
<commit hash>（S0）
<commit hash>（P2）
<commit hash>（P3）
<commit hash>（P4）
<commit hash>（P5）
<commit hash>（P6）
<commit hash>（P7）
<commit hash>（P8）
```
