# [CQ:reply]

> 🧪 **测试插件**：[CQ-reply](https://github.com/Te-River/Gensokyo-NewQQ-Test-plugins/tree/main/CQ-reply) — 发送「引用」体验

## 用途

`[CQ:reply]` 用于回复指定的消息。在 QQ Bot API 中，回复通过设置 `MessageReference` 或 `MsgID`/`EventID` 实现，Gensokyo 负责将虚拟消息 ID 映射回真实的 QQ 消息 ID。

范围：`q群 (Group Chat)` / `C2C (私聊)`

## 语法

```text
[CQ:reply,id=虚拟消息ID]
```

## 参数

| 参数名 | 必填 | 说明 |
|--------|------|------|
| `id` | ✅ | 要回复的消息的虚拟 ID（由 Gensokyo idmap 系统分配） |

## 解析行为

### 字符串模式

`[CQ:reply,id=12345]` 通过正则 `replyRe` 匹配，提取 `id` 值，存入 `foundItems["reply_msg_id"]`。该 CQ 码会从消息文本中移除。

### 数组模式

```json
{"type": "reply", "data": {"id": "12345"}}
```

同样提取 `id` 存入 `foundItems["reply_msg_id"]`。

### ID 映射流程

1. 应用端传入 Gensokyo 分配的虚拟消息 ID。
2. Gensokyo 通过 echo 系统反查真实消息 ID（格式 `"GroupID MessageID"` 或 `"UserID MessageID"`）。
3. 将真实消息 ID 设置到发送消息的 `MessageReference` 和 `MsgID` 字段。

## 发送行为

### 群聊回复

回复通过 `MessageReference` 字段实现：

```go
messageToCreate.MessageReference = &dto.MessageReference{
    MessageID: 真实消息ID,
}
```

### 私聊回复

回复通过 `MsgID` 字段实现：

```go
messageToCreate.MsgID = 真实消息ID
```

### 安全校验

- **私聊 reply 越权防护**（v012 修复）：私聊 reply 时会校验虚拟 ID 反查出的真实 ID 是否属于当前私聊目标 UserID，不一致则跳过 reply，避免群聊 msg_id 被引用到私聊场景触发 `40034024` 越权错误。

### 富媒体回复

`reply` 会被应用到每个媒体消息（图片、语音、视频、文件等）的 `MessageReference` 和 `MsgID` 字段上，确保每条消息都携带回复引用。

## 使用示例

### NoneBot 插件

```python
from nonebot.adapters.onebot.v11 import Message, MessageSegment

# 回复一条群消息
await bot.send_group_msg(
    group_id=123456,
    message=Message([
        MessageSegment.reply(12345),  # 回复虚拟 ID 为 12345 的消息
        MessageSegment.text("收到！"),
    ]),
)
```

### CQ 码字符串格式

```text
[CQ:reply,id=12345]收到！
```

## 入站上报

### 字符串模式

收到的回复消息会在文本开头追加 `[CQ:reply,id=虚拟ID]`。

### 数组模式

```json
[
  {"type": "reply", "data": {"id": "12345"}},
  {"type": "text", "data": {"text": "原始消息内容"}}
]
```

## 注意

- 回复的目标消息必须在 Gensokyo 的 echo 缓存中存在，否则无法反查真实 ID。
- 缓存有效期受 `msgid_ttl_seconds` 配置控制（默认 3600 秒）。
- 私聊场景下，reply 只能回复同一私聊对话中的消息，不能跨场景引用群聊消息（v012 安全修复）。
- `[CQ:reply]` 在消息中的位置不影响行为——它总是被提取并应用到所有发送的媒体段上。
