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

当 `[CQ:input_notify]` 出现在 `send_private_msg` 的出站消息中时，本条消息**整体被替换**为一条 `msg_type=6` 的输入状态通知（`input_notify{input_type, input_second}`）：

1. CQ 码从消息文本中移除。
2. 只发送这一条通知消息，`second` 缺省补 `60`（对齐 Go `sendInputNotify` 的默认值）。
3. 输入状态超时后自动消失，无需手动关闭。

**硬约束（一律 `retcode=1400`，整条不发送）**，见 `crates/gensokyo-runtime/src/deferred.rs` 的 `compose_input_notify`：

| 场景 | 结果 |
|------|------|
| 群目标（`send_group_msg` / `send_to_group`）携带 `[CQ:input_notify]` | 1400「只有单聊支持（官方 msg_type=6 仅 C2C）」 |
| 与正文（非空 text）同条 | 1400「必须单独成条」 |
| 与 markdown / 卡片 / 键盘 / 音乐 / 任何媒体段同条 | 1400「必须单独成条」（msg_type=6 不承载这些内容） |

也就是说：**想发通知就单独发一条纯 `[CQ:input_notify]` 消息，正文另起一条**。

## 与 Go 主线的差异（刻意分侧）

> Go 主线为两段式（先通知后正文）：Go 的实现是收到带 `[CQ:input_notify]` 的消息后，**先**单独发一条 `msg_type=6` 通知、**再**照常发送正文，插件可以 `"[CQ:input_notify] 正文..."` 一条搞定。本仓 Rust 链一次计划只发一条消息体（`msg_type` 只能取一个值，正文 / 媒体 / markdown / 卡片都带不进 `msg_type=6`），因此选择**硬拒混发**而不是清空正文静默丢弃。该差异属本仓刻意分侧口径，登记见 `reports/refactor/17-go-rust-parity.md` §F / B4（不要按 bug 修）；代码证据为 `compose_input_notify` 的函数文档注释。

## 使用示例

单发一条输入状态通知（本仓口径，正文另起一条）：

```python
from nonebot.adapters.onebot.v11 import Message

# 第一条：纯通知，不带任何正文
await bot.send_private_msg(
    user_id=123456,
    message=Message("[CQ:input_notify,type=1,second=30]"),
)

# 第二条：实际内容
await bot.send_private_msg(
    user_id=123456,
    message=Message("正在为您查询，请稍候..."),
)
```

> ⚠️ 以下写法在 Go 主线可用，但**本仓会回 1400**（通知与正文混发）：
>
> ```python
> # 本仓不支持：input_notify 与正文同条
> msg = Message("[CQ:input_notify,type=1,second=30] 正在为您查询，请稍候...")
> ```

## 注意

- 输入状态通知**仅支持单聊（C2C）场景**，群聊和频道不支持（群目标直接 1400）。
- `second` 未提供时按 `60` 秒发送；官方 UI 展示效果，不影响其他消息的发送逻辑。
- `[CQ:input_notify]` 本身不发送正文：**必须单独成条**，正文需在另一条消息里提供。
- 多条消息连续发送时，只需在正文之前另发一条纯通知消息即可。
