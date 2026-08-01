# Changelog — Release011

> 自 Release010 以来的所有变更。

---

## 🐛 Bug 修复

### 图文混合消息 `[CQ:at]` 原文显示

**文件：** `handlers/send_group_msg.go`、`handlers/send_group_msg_raw.go`、`handlers/send_private_msg.go`、`handlers/send_guild_channel_msg.go`

**问题：** 开启全量群消息（`GROUP_MESSAGE_CREATE`）后，发送图文混合消息（`msg_type=7`）时，文本段中的 `[CQ:at,qq=数字]` 未被转换为 `@用户名`，QQ 官方 API 不识别 CQ 码，导致原文显示为 `图片[CQ:at,qq=123456]`。纯文本消息路径正常，仅图文混合路径漏处理。

**根因：** 图文混合路径在构造 `dto.MessageToCreate{Content: messageText, ...}` 时，直接将含 `[CQ:at]` 的 `messageText` 塞进 `Content` 发送，从未调用 `resolvePlainTextAtMentions`，与纯文本路径行为不一致。

**修复：** 在以下四个 handler 的图文混合路径中，构造 `MessageToCreate` 之前补一次 `messageText = resolvePlainTextAtMentions(messageText)`，与纯文本路径对齐：

1. `send_group_msg.go` — 群聊图文混合 `!transmd` 分支
2. `send_group_msg_raw.go` — raw 变体图文混合 `!transmd` 分支
3. `send_private_msg.go` — 私聊图文混合分支
4. `send_guild_channel_msg.go` — 频道图文混合 `!isbase64` 分支与 base64 multipart 分支

**影响范围：** Bug fix，单点补齐函数调用，不涉及接口变更，不影响现有功能兼容性。

**关联 Issue：** [Te-River/Gensokyo-NewQQ#15](https://github.com/Te-River/Gensokyo-NewQQ/issues/15)

---

## 📝 文档同步

- `docs/cq码/扩展CQ码/扩展cq码-cq-at.md`：新增"图文混合消息（msg_type=7）"章节
- `docs/本版新增功能.md`：出站 @ 列表补充图文混合路径覆盖说明
- `docs/Gensokyo语法参考.md`：`[CQ:at]` 行内补充图文混合转换说明
- `docs/cq码/扩展CQ码汇总.md`：`[CQ:at]` 行内补充图文混合转换说明
- `release_log/CHANGELOG_v011.md`：本次变更独立 CHANGELOG
