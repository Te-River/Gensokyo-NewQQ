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

## ✨ 新功能

### QQ API 错误码中文提示

**文件：** `handlers/qq_error_codes.go`（新增）、`handlers/send_group_msg.go`、`handlers/send_group_msg_raw.go`、`handlers/send_private_msg.go`、`handlers/send_guild_channel_msg.go`、`handlers/send_private_msg_wakeup.go`

**背景：** 此前 QQ OpenAPI 调用失败时，控制台只输出原始 `err.Error()`（如 `{"code":22009,"message":"频控"}`），开发者需要自行查阅官方文档才知道错误码含义，排查成本高。

**实现：**

- 新增 `handlers/qq_error_codes.go`，内置错误码 → 中文描述/排查建议映射表（数据来源：[QQ 官方错误码文档](https://bot.q.qq.com/wiki/develop/api-v2/dev-prepare/api-call-guide.html#openapi-错误码)）
- `ExtractQQErrorCode(err)`：正则提取 `err.Error()` 中的 `"code":xxxxx`
- `FormatQQError(err)`：格式化为开发者可读提示

**接入方式（非侵入式）：** 在 5 个 handler 的现有 `if err != nil` 分支内，于 `mylog.Printf("...失败: %v", err)` 之后追加一行 `mylog.Printf("%s", FormatQQError(err))`。不改变现有错误处理流程（22009 入队、40034025 重试、超时重试等保持不变）。

**示例输出：**

```text
发送文本群组信息失败: {"code":22009,"message":"频控"}
[QQ API 错误码 22009] 主动消息频控超限。排查建议：降低发送频率或等待配额恢复
```

未识别的错误码会提示查阅官方文档；无错误码的原始错误也会打印，方便调试。

---

## 📝 文档同步

- `docs/cq码/扩展CQ码/扩展cq码-cq-at.md`：新增"图文混合消息（msg_type=7）"章节
- `docs/本版新增功能.md`：出站 @ 列表补充图文混合路径覆盖说明；新增"错误码提示"章节
- `docs/Gensokyo语法参考.md`：`[CQ:at]` 行内补充图文混合转换说明
- `docs/cq码/扩展CQ码汇总.md`：`[CQ:at]` 行内补充图文混合转换说明
- `readme.md`：功能亮点新增"QQ API 错误码中文提示"
- `release_log/CHANGELOG_v011.md`：本次变更独立 CHANGELOG
