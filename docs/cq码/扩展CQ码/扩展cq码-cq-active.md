# [CQ:active]

> 🧪 **测试插件**：[CQ-active](https://github.com/Te-River/Gensokyo-NewQQ-Test-plugins/tree/main/CQ-active) — 发送「推送状态 / 群列表 / 推送」体验

## 用途

`[CQ:active]` 用于标记出站消息为主动推送模式。当消息内容中包含此 CQ 码时，Gensokyo 会强制走主动消息通道（清空缓存的 `msg_id` 和 `event_id`），即使有可用的被动回复上下文。

`[CQ:active]` 是 Gensokyo-NewQQ 用于标记主动或召回消息的扩展消息段。Release008 重点修复了它在 `send_private_msg_wakeup` 中的处理。

范围：`q群 (Group Chat)` / `私聊 (C2C)`（`send_group_msg` 与 `send_private_msg` 均识别该标记并强制走主动推送；`send_private_msg_wakeup` 消费并跳过该标记）

## 语法

支持两种格式：

```text
[CQ:active]                          // 裸标记：无参数
[CQ:active,type=<值>,sub_type=<值>]  // 带参数
```

数组消息段：

```json
{"type":"active","data":{"type":"<值>","sub_type":"<值>"}}
```

无论哪种格式，`[CQ:active]` 都会从消息文本中移除，最终用户不会看到。

## 解析行为

- `[CQ:active]`（裸标记）→ 识别为主动标记，从文本中移除。
- `[CQ:active,type=xxx,sub_type=yyy]`（带参数）→ 同上，且 `type` 写入 `foundItems["active_type"]`，`sub_type` 写入 `foundItems["active_sub_type"]`。
- 数组消息段的 `"type":"active"` 同样支持。
- 解析后会在 `foundItems` 中产生 `active`、`active_type` 和 `active_sub_type` 三个控制字段（实现：`handlers/cqcode.go:145-172` 的 `ProcessCQActive`，带参分支写 `active_type` / `active_sub_type`，`:164` 与 `:170` 均写 `active`）。

## 发送行为

当 `[CQ:active]` 出现在出站消息中时：

1. CQ 码从消息文本中移除。
2. 清空缓存的 `msg_id`（如果有），强制走主动推送通道。
3. 清空 `event_id`（如果有），避免消耗被动回复次数。
4. `send_private_msg_wakeup` 遍历消息内容时会跳过 `active` 相关字段，不会把它们错误地当作图片、语音或其他富媒体类型发送（`handlers/send_private_msg_wakeup.go:184-185`）。
5. 消息只包含 `[CQ:active]`、没有文本或其他媒体时，召回 Action 仍会补发一次唤醒请求，以确保 QQ 侧仍能收到互动召回（`handlers/send_private_msg_wakeup.go:283-286`）。

## 使用示例

NoneBot 插件构造主动群推送：

```python
from nonebot.adapters.onebot.v11 import Message, MessageSegment

def active_msg(text: str) -> Message:
    segs = [MessageSegment.text("[CQ:active]")]
    if text:
        segs.append(MessageSegment.text(text))
    return Message(segs)

# 主动推送到指定群
await bot.send_group_msg(
    group_id=821404315,
    message=active_msg("服务器将在十分钟后维护"),
)
```

C2C 唤醒消息：

```python
await bot.call_api(
    "send_private_msg_wakeup",
    user_id="<OpenID>",
    message=active_msg("你好，这是一条唤醒消息"),
)
```

## 与普通发送 Action 的区别

- `send_private_msg_wakeup` 会明确消费 active 标记，并强制设置 `is_wakeup=true`。
- 普通 `send_private_msg`、`send_group_msg` **同样会消费该标记**：两者在发送前检查 `foundItems["active"]`，命中即清空 `messageID` 与 `eventID`，强制走主动推送（`handlers/send_group_msg.go:312-316`、`handlers/send_private_msg.go:351-355`；新链同一语义见 `handlers/outbound_bridge.go:170-172` 置 `message.ModeActive`）。标记的产生点是 `handlers/cqcode.go:145-172` 的 `ProcessCQActive`——裸码与带参形态都会写入 `active`，带参时额外写入 `active_type` / `active_sub_type`。
- 需要 C2C 互动召回时，应直接调用 [`send_private_msg_wakeup`](../../api/扩展api/扩展api-send_private_msg_wakeup.md)。

## 返回结果

Action 处理结束后返回标准成功响应；实际 QQ 发送结果会另行上报：

```json
{
  "post_type": "notice",
  "notice_type": "c2c_wakeup_resp",
  "status": "success",
  "real_user_id": "目标 OpenID",
  "message_id": "QQ 消息 ID"
}
```

发送失败时 `status` 为 `failed`，并包含 `error_msg`。

## 注意

- 裸 `[CQ:active]`（精确写法、无参数）会被文本正则识别并从正文移除：legacy 字符串管道匹配 `\[CQ:active\]`（`handlers/cqcode.go:167-172`），数组消息段与 cqparse 解析路径同样支持。
- `[CQ:active]` 只切换发送通道，不改变消息内容；需要 C2C 互动召回（`is_wakeup=true`）时，请直接使用 [`send_private_msg_wakeup`](../../api/扩展api/扩展api-send_private_msg_wakeup.md)。
