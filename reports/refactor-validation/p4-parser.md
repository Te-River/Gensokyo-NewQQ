# P4 — 消息解析类型化 验证报告

```
PHASE: P4（消息解析类型化）
STATUS: PASS
```

---

## 目标

把 `handlers/message_parser.go` 的 `foundItems map[string][]string` 解析逻辑收敛为
typed `ParsedMessage`，String 与 Array 两种入站形态输出同一模型。

---

## 实现

### 新增 `internal/domain/message/`（纯函数，无副作用）

| 文件 | 职责 |
|------|------|
| `part.go` | `MessagePart` 接口 + `TextPart`/`MentionPart`/`ImagePart`/`AudioPart`/`VideoPart`/`FilePart`/`ReplyPart`/`MarkdownPart`/`KeyboardPart`/`QQMusicPart`/`UnknownPart` |
| `parsed.go` | `ParsedMessage`（Parts + Reply + `DeliveryMode`：active/passive 独立于内容） |
| `media.go` | `MediaSource`（LocalFile/RemoteURL/Base64/Bytes）+ `MediaSourceFromFile`（P5 复用） |
| `segment.go` | `Segment` 纯数据（OneBot 消息段）+ `FromMap` |
| `cq.go` | CQ 码解析器：转义/反转义 + 转义感知参数拆分 |
| `parse_string.go` | `ParseOneBotString`：String → ParsedMessage |
| `parse_array.go` | `ParseOneBotSegments`：Array → ParsedMessage |
| `compat.go` | compat bridge：`Canonicalize`（typed→段）+ `ToLegacyFoundItems` |

### 关键设计

- **Parser 纯函数化**：无 HTTP / QQ API / idmap / config / goroutine / 上传，
  一切 IO 由调用方在解析后处理。
- **String/Array 统一**：两个入口输出同一 `ParsedMessage` 模型（canonicalize 后逐字节一致）。
- **DeliveryMode 独立**：active/passive 是投递模式，不塞入 Parts。
- **UnknownPart**：未识别段保留原始参数，防止信息丢失。
- **CQ 解析器**：基于扫描器而非正则，支持转义（`&#44;` 不拆分参数）、malformed 容错。

### foundItems 冻结（P4.1）

- 本包不新增任何 `foundItems["..."]` key；`ToLegacyFoundItems` 仅做既有 key 的反向映射（compat）。
- 新功能一律进入 typed model。

---

## 验收清单

| 验收项 | 状态 |
|--------|------|
| Typed parser coverage >= 90% | ✅ PASS（96.9%） |
| String/Array 输出同一 ParsedMessage 模型 | ✅ PASS（canonical 一致） |
| 新功能禁止添加 foundItems key | ✅ PASS（未新增） |
| legacy bridge 集中在 compat | ✅ PASS（compat.go） |

---

## 变更文件

- 新增：`internal/domain/message/{part,parsed,media,segment,cq,parse_string,parse_array,compat}.go`
- 新增：`internal/domain/message/{cq,parse_string,parse_array,canonical,compat}_test.go`

---

## 验证结果

| 命令 | 结果 |
|------|------|
| `go build ./...` | ✅ PASS |
| `go vet ./...` | ✅ PASS |
| `go test ./internal/domain/message/` | ✅ PASS（96.9% coverage） |
| `go test ./...` | ✅ PASS |
| `go test -race ./...` | NOT_RUN（计划 V2/V3） |
| `govulncheck ./...` | NOT_RUN（计划 V2/V3） |
| frontend | NOT_RUN（不涉及） |

---

## KNOWN_LIMITATIONS

- 新 parser 尚未接入生产（`parseMessageContent` 仍为主路径），接入属 P13。
- `KeyboardPart`/`MarkdownPart` 暂保留原始 Content（JSON/base64），结构化解析属 P5/P6。
- malformed CQ（无结束括号）被拆为独立文本段（行为已在测试固化）。

---

## LEGACY_REMAINING

- `handlers/message_parser.go` 的 `foundItems` 全流程仍在使用（P13 删除）。
- 现有 2 套（String/Array）解析路径未收敛（P13）。

---

## NEXT_PHASE

- P5 媒体管线统一（`MediaService`，消费 `MediaSource`）。
