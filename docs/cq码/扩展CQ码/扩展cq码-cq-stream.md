# [CQ:stream]

> 🧪 **测试插件**：[CQ-stream](https://github.com/Te-River/Gensokyo-NewQQ-Test-plugins/tree/main/CQ-stream) — 发送「流式测试」体验

## 用途

`[CQ:stream]` 用于在单聊（C2C）中发送流式消息，实现打字机效果。消息会逐片展示，而非一次性显示。分为三个生命周期阶段：`start`（首片）→ `mid`（续片）→ `finish`（终片）。

范围：`私聊 (C2C)`

## 语法

```text
[CQ:stream,type:start,qq:123456789]消息内容
[CQ:stream,type:mid,qq:123456789]消息内容
[CQ:stream,type:finish,qq:123456789]消息内容
```

> `cq_parse_mode: new` 下同时接受等号写法 `[CQ:stream,type=start,qq=123456789]`；legacy 模式仅支持冒号写法。

| 参数 | 必填 | 说明 |
|------|------|------|
| `type` | 是 | 流式阶段：`start`（首片）、`mid`（续片）、`finish`（终片） |
| `qq` | 是 | 接收者的 QQ 虚拟 ID，用于关联同一流的多个分片 |

> 取值宽松度按 Go 默认链：**`type` 或 `qq` 任一非空就吞码**（`handlers/cqparse/handlers_content.go:157-176` 的 `resolveStream` 只判「值非空」），缺 `qq` / `qq` 非数字 / `type` 不是三枚举都不会把码漏进正文，见下节「解析行为」。

## 生命周期

一个完整的流式消息由三次独立的 `send_private_msg` 调用组成：

```
① [CQ:stream,type:start,qq:xxx] 正在为您查询     → 首片，返回 stream_msg_id
② [CQ:stream,type:mid,qq:xxx]   正在为您查询结果  → 续片，携带 stream_msg_id
③ [CQ:stream,type:finish,qq:xxx] 查询完毕       → 终片，input_state=10
```

Gensokyo 内部维护 `stream_msg_id` 缓存（key=qq），自动关联同一用户的前后分片。

## 解析行为

- 从消息文本中提取 `[CQ:stream,...]` 并移除，用户不会看到该 CQ 码。
- `type` 和 `qq` 通过 JSON 编码存入 `foundItems["stream"]`（只写非空键：`[CQ:stream,type=start]` 产出 `{"type":"start"}`，`[CQ:stream,qq=123]` 产出 `{"qq":"123"}`）。
- **只有 `type` 与 `qq` 都空**（`[CQ:stream]`、`[CQ:stream,type=]`）才按普通文本原样留在正文里（Go 回填 `tok.Raw`）。
- 数组消息段格式（`{"type":"stream","data":{"type":"start","qq":"123456789"}}`）同样支持。

## 发送行为

当 `[CQ:stream]` 出现在 `send_private_msg` 的出站消息中时：

1. CQ 码从消息文本中移除。
2. 根据 `type` 值决定行为：

| type | 行为 | API 参数 |
|------|------|---------|
| `start` | 首次发送，服务器返回 `stream_msg_id` | `input_state=1, input_mode=replace, index=0` |
| `mid` | 续片，需之前返回的 `stream_msg_id` | `input_state=1, index=N, stream_msg_id=xxx` |
| `finish` | 结束片，清理缓存 | `input_state=10, index=N, stream_msg_id=xxx` |

3. 流式消息处理完毕后，不会走普通文本发送路径。
4. `type` 不是 `start` / `mid` / `finish`（例如只给了 `qq`）时，Go 的 `switch` 没有对应分支：码已从正文移除、**什么都不发**、也不回失败码（`handlers/send_private_msg.go:124-175`），回执是成功（`retcode 0` / `data.message_id 0`）。
5. **群聊目标**：Go 群链不消费 `foundItems["stream"]`（`handlers/outbound_bridge.go:484-630` 没有该键分支），码被静默忽略、正文照发、回执 `retcode 0`；群聊里不会真的发出流式消息。

## 使用示例

NoneBot 插件发送流式消息：

```python
from nonebot.adapters.onebot.v11 import Message

# 首片
await bot.send_private_msg(
    user_id=123456,
    message=Message("[CQ:stream,type:start,qq:123456] 正在为您查询，请稍候..."),
)

# 续片（根据实际生成进度多次调用）
await bot.send_private_msg(
    user_id=123456,
    message=Message("[CQ:stream,type:mid,qq:123456] 正在为您查询，已找到部分结果..."),
)

# 终片
await bot.send_private_msg(
    user_id=123456,
    message=Message("[CQ:stream,type:finish,qq:123456] 查询完毕，共找到 5 条结果"),
)
```

消息段格式：

```python
from nonebot.adapters.onebot.v11 import Message, MessageSegment

# 首片
await bot.send_private_msg(
    user_id=123456,
    message=Message([
        MessageSegment("stream", {"type": "start", "qq": "123456"}),
        MessageSegment.text("正在为您查询，请稍候..."),
    ]),
)
```

## 注意

- 流式消息**仅支持单聊（C2C）场景**，群聊和频道不会发出流式消息：群体目标带 `[CQ:stream]` 时该码被静默忽略（码消失、正文照发、`retcode 0`），不是参数错误。
- 同一个用户的流式消息必须按 `start` → `mid`（可多次）→ `finish` 顺序发送。
- 如果缺少 `start` 直接发 `mid` 或 `finish`，会因缺少 `stream_msg_id` 而被**跳过**：不向上游发任何请求、正文也不再发（Go `send_private_msg.go:141-142`/`:159-160` 的「缺少 stream_msg_id，跳过」同口径），回执是成功（`retcode 0` / `data.message_id 0`），不是参数错误。
- 缓存的生命周期由 Gensokyo 进程管理，重启后缓存丢失。
- 不同用户的流式消息相互独立，`qq` 参数用于区分缓存。
