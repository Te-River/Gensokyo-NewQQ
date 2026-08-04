# Changelog — Release012

> 自 Release011 (`d5c780b`) 以来的所有变更。

---

## 🐛 Bug 修复

### 非全量群重复 @ 修复

**文件：** `Processor/ProcessGroupMessage.go`

**问题：** 未开启全量群消息（仅订阅 `GROUP_AT_MESSAGE_CREATE` 被动消息）的群中，`add_at_group` 配置会无条件添加 `[CQ:at,qq=AppID]`，但被动消息本身已包含对 bot 的 @（`remove_at` 配置未剥离时），导致出站消息出现重复 @。

**修复：** `add_at_group` 添加 `[CQ:at,qq=AppID]` 的条件增加 `GetRemoveAt()` 判断——仅当 `remove_at` 开启（即 @bot 已被剥离需要补回）时才添加，避免已包含 @bot 的消息再重复添加。

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