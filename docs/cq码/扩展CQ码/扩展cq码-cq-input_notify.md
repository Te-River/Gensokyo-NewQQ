# [CQ:input_notify]

> 🧪 **测试插件**：[CQ-input_notify](https://github.com/Te-River/Gensokyo-NewQQ-Test-plugins/tree/main/CQ-input_notify) — 发送「输入测试」体验

## 用途

`[CQ:input_notify]` 用于向单聊（C2C）用户发送"正在输入"的状态通知（`msg_type=6`）。适合需要等待后端处理较长时间的场景，给用户即时反馈。

范围：`私聊 (C2C)`

## 语法

```text
[CQ:input_notify,type=<类型>,second=<秒数>]
```

| 参数 | 必填 | 说明 |
|------|------|------|
| `type` | 是 | 输入类型，本仓按官方口径透传数字；常用值 `1` |
| `second` | 否 | 状态持续时间（秒），缺省 `60` |

## 解析行为

- 从消息文本中提取 `[CQ:input_notify,...]` 并移除，最终用户不会看到该 CQ 码。
- 数组消息段的 `"type":"input_notify"` 同样支持（段 JSON 载荷须为合法 JSON 且带数字 `type`）。
- `type` 缺失或非数字时不命中，整码按字面留在正文（不报错、也不产生通知）。

## 发送行为（本仓 Rust 实现口径）

当 `[CQ:input_notify]` 出现在 `send_private_msg` 的出站消息中时，本仓**先发一条 `msg_type=6` 通知、再照常发正文**（与 Go 主线同口径）：

1. CQ 码从消息文本中移除。
2. 通知条：`msg_type=6` + `input_notify{input_type, input_second}`，`second` 缺省补 `60`（对齐 Go `sendInputNotify` 的默认值）。
3. 正文条：去掉通知后剩下的正文 / markdown / 媒体等按原有优先级正常发送；**回执只有一条**，取后发的正文那条（`data.message_id` 是正文消息的 ID）。
4. 输入状态超时后自动消失，无需手动关闭。

纯通知消息（消息里只有 `[CQ:input_notify]`）仍然只发一条通知，回执就是通知的 `message_id`。

**唯一的硬拒**，见 `crates/gensokyo-runtime/src/deferred.rs` 的 `compose_input_notify`：

| 场景 | 结果 |
|------|------|
| 群目标（`send_group_msg` / `send_to_group`）携带 `[CQ:input_notify]` | 1400「只有单聊支持（官方 msg_type=6 仅 C2C）」 |

> 群聊这条与 Go 一致但形态不同：Go 群链根本不调用 `sendInputNotify`，该码在群消息里会被静默忽略（`Error: Expected RichMediaMessage type for key input_notify` 日志）；本仓选择明确 `1400`，避免插件以为通知发出去了。

## 与 Go 主线的对照

> Go 主线（`handlers/send_private_msg.go:473-477`）收到带 `[CQ:input_notify]` 的消息后**先**单独发一条 `msg_type=6` 通知、**再**照常发送正文，插件 `"[CQ:input_notify] 正文..."` 一条消息即可搞定。**本仓 2026-11-25 起已对齐这个双发口径**（此前是「混发一律 `1400` 硬拒」）；登记见 `reports/refactor/17-go-rust-parity.md` §F 的 P77-9。

## 使用示例

一条消息搞定（通知 + 正文，本仓与 Go 同口径）：

```python
from nonebot.adapters.onebot.v11 import Message

await bot.send_private_msg(
    user_id=123456,
    message=Message("[CQ:input_notify,type=1,second=30] 正在为您查询，请稍候..."),
)
```

也可以拆成两条独立消息（等价的旧写法，仍然支持）：

```python
# 纯通知
await bot.send_private_msg(
    user_id=123456,
    message=Message("[CQ:input_notify,type=1,second=30]"),
)
# 正文
await bot.send_private_msg(
    user_id=123456,
    message=Message("正在为您查询，请稍候..."),
)
```

## 注意

- 输入状态通知**仅支持单聊（C2C）场景**，群聊和频道不支持（群目标直接 1400）。
- `second` 未提供时按 `60` 秒发送；官方 UI 展示效果，不影响其他消息的发送逻辑。
- 混发时上游会收到**两条**消息（通知 + 正文），但插件只拿到**一条**回执（正文那条）。
- 通知条不承载正文：正文永远在紧随其后的那条消息里。
