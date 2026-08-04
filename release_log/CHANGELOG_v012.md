# Changelog — Release012

> 自 Release011 (`d5c780b`) 以来的所有变更。

---

## 🐛 Bug 修复

### 非全量群重复 @ 修复

**文件：** `Processor/ProcessGroupMessage.go`

未开启全量群消息（仅订阅 `GROUP_AT_MESSAGE_CREATE` 被动消息）的群中，`add_at_group` 配置会添加 `[CQ:at,qq=AppID]`，但被动消息本身已包含对 bot 的 @，导致出站消息出现重复 @。

**修复：** 被动消息（`GROUP_AT_MESSAGE_CREATE`）中直接移除 `add_at_group` 添加 `[CQ:at,qq=AppID]` 的逻辑，因为消息本身已包含 @bot。`add_at_group` 仅在全量群消息（`GROUP_MESSAGE_CREATE`）中生效，全量消息中的 @bot 始终被剥离，需要补回。

---

## 🧪 验证

| 命令 | 结果 |
|------|------|
| `go build ./...` | ✅ 通过 |
| `go vet ./...` | ✅ 通过 |
| `go test ./handlers/ ./Processor/` | ✅ 通过 |

---

## ✅ 提交记录

```
<commit hash>
```