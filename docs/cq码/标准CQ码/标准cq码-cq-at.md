# [CQ:at]

## 用途

`[CQ:at]` 用于在群聊中 @ 提及某个用户或机器人。在 QQ Bot API 中，@ 行为通过 `<@OpenID>` 语法实现，Gensokyo 负责在虚拟 ID 和真实 OpenID 之间进行转换。

范围：`q群 (Group Chat)`

## 语法

```text
[CQ:at,qq=虚拟ID或特殊值]
```

## 参数

| 参数名 | 必填 | 说明 |
|--------|------|------|
| `qq` | ✅ | 要 @ 的用户虚拟 ID（int64），或特殊值 `all`（@全体成员） |

## 解析行为

### 出站（OneBot → QQ）

1. `[CQ:at,qq=12345]` 中的 `12345` 是 Gensokyo 的虚拟 ID。
2. Gensokyo 通过 idmap 系统将虚拟 ID 反查为真实 OpenID。
3. 转换为 QQ Bot API 的 `<@OpenID>` 语法嵌入消息正文。
4. 如果反查失败，@ 标签会被忽略或保留原文。

### 入站（QQ → OneBot）

1. QQ 消息中的 `<@OpenID>` 由 Gensokyo 自动识别。
2. 通过 idmap 系统将 OpenID 映射为虚拟 ID。
3. 根据配置决定是否剥离 @Bot 自身：
   - `remove_at=true`：剥离入站消息中的 @Bot
   - `remove_bot_at_group=true`：出站时隐藏 Bot 发出的 @

### 数组模式

入站数组模式下，@ 会被上报为独立的 `at` 段：

```json
[
  {"type": "at", "data": {"qq": "12345"}},
  {"type": "text", "data": {"text": "你好"}}
]
```

## 发送行为

### 群聊（普通文本路径，msg_type=0）

`[CQ:at,qq=虚拟ID]` → 反查 OpenID → 转为 `<@OpenID>` 嵌入消息正文。

### 群聊（Markdown 路径，msg_type=2）

`[CQ:at,qq=虚拟ID]` → 反查 OpenID → 转为 Markdown @ 标签 `<qqbot-at-user id=OpenID>`。

### 图文混合（msg_type=7）

同普通文本路径，@ 转为 `<@OpenID>` 嵌入正文。

### add_at_group 配置

当 `add_at_group=true` 时，出站群消息会自动在开头添加 `[CQ:at,qq=AppID]`（@Bot 自身），仅在全量群消息（`GROUP_MESSAGE_CREATE`）中生效。

## 使用示例

### NoneBot 插件

```python
from nonebot.adapters.onebot.v11 import Message, MessageSegment

await bot.send_group_msg(
    group_id=123456,
    message=Message([
        MessageSegment.at(12345),  # @ 虚拟 ID 为 12345 的用户
        MessageSegment.text(" 你好！"),
    ]),
)
```

### CQ 码字符串格式

```text
[CQ:at,qq=12345]
[CQ:at,qq=all]    // @全体成员（QQ Bot API 可能不支持）
```

## 注意

- `[CQ:at]` 仅在群聊场景有效，私聊（C2C）不支持 @ 功能。
- @ 的目标必须是已建立 idmap 映射的用户，否则反查失败。
- 全量群消息和被动群消息的 @Bot 剥离行为不同，详见 `remove_at` 和 `remove_bot_at_group` 配置。
- QQ Bot API 的 @ 语法与 go-cqhttp 等第三方实现不同，Gensokyo 已自动处理差异。
