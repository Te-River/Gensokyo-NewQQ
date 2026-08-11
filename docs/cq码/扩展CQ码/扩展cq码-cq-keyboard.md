# [CQ:keyboard]

## 用途

`[CQ:keyboard]` 用于在文本消息上附加 QQ 官方内嵌键盘（按钮消息）。键盘按钮支持跳转、回调与指令三种行为，展示在消息下方。

范围：`q群 (Group Chat)` / `C2C (私聊)`

## 语法

```text
[CQ:keyboard,data=base64://<base64 编码的键盘 JSON>]
[CQ:keyboard,data=<键盘 JSON>]
```

| 参数 | 必填 | 说明 |
|------|------|------|
| `data` | 是 | 键盘 JSON，支持 base64 编码或原始 JSON 两种形式（与 `[CQ:markdown]` 一致） |

键盘 JSON 结构与官方 `keyboard` 参数一致，支持以下三种形态：

```json
{
  "id": "模板ID"
}
```

```json
{
  "content": {
    "rows": [
      {
        "buttons": [
          {
            "id": "btn_1",
            "render_data": { "label": "跳转", "style": 0 },
            "action": { "type": 0, "data": "https://example.com", "permission": { "type": 2 } }
          }
        ]
      }
    ]
  }
}
```

```json
{
  "rows": [
    {
      "buttons": [
        {
          "id": "btn_2",
          "render_data": { "label": "回调", "style": 1 },
          "action": { "type": 1, "data": "click_me", "permission": { "type": 2 } }
        }
      ]
    }
  ]
}
```

按钮 `action.type`：`0`=跳转按钮（http 或小程序）、`1`=回调按钮（触发 `INTERACTION_CREATE` 事件）、`2`=指令按钮（自动在输入框插入 `@bot data`）。

## 渲染说明（重要）

**QQ 客户端仅在 Markdown 消息（`msg_type=2`）下渲染内嵌键盘按钮。**

- 纯文本消息（`msg_type=0`）附带的 `keyboard` 参数会随消息发送，但 QQ 客户端**不显示按钮**（已实测确认）。
- 因此 `[CQ:keyboard]` 独立使用时，建议**与 `[CQ:markdown]` 配合**：将文本放入 markdown 内容，键盘随 markdown 一起渲染。
- 组合用法示例：

```text
[CQ:markdown,data={"content":"你好，欢迎使用签到功能"}][CQ:keyboard,data=base64://<键盘JSON>]
```

- 若同一消息同时携带 `[CQ:markdown]`，其附带键盘优先生效，独立 `[CQ:keyboard]` 被忽略（markdown 路径的 keyboard 解析自 markdown JSON 的 `keyboard` 字段）。

## 解析行为

- 从消息文本中提取 `[CQ:keyboard,...]` 并移除，最终用户不会看到该 CQ 码。
- base64 格式会先解码为原始 JSON，两种格式最终都存入 `foundItems["keyboard"]`。
- keyboard JSON 为空或无法解析时，该 CQ 码被移除且不附加键盘，不影响文本发送。

## 发送行为

当 `[CQ:keyboard]` 出现在 `send_group_msg` / `send_private_msg` / `send_group_msg_raw` 的出站消息中时：

1. CQ 码从消息文本中移除。
2. 剩余文本以 `msg_type=0` 文本消息发送，`keyboard` 字段携带按钮（文本 + 内嵌键盘）。
3. `specify_user_ids` 中的数字虚拟 ID 自动转换为 QQ 官方 OpenID；私聊场景下 `__USER_ID__` 占位符替换为实际用户 OpenID。
4. 按钮图片（`render_data` 中的本地路径）自动解析上传。
5. 如果同一消息同时包含 `[CQ:markdown]`，markdown 优先（其附带键盘生效，独立 keyboard 被忽略）。

## 使用示例

NoneBot 插件发送带按钮的群聊文本消息（base64 格式）：

```python
import base64, json
from nonebot.adapters.onebot.v11 import Message

keyboard = {
    "content": {
        "rows": [
            {
                "buttons": [
                    {
                        "id": "btn_signin",
                        "render_data": {"label": "签到", "style": 1},
                        "action": {"type": 2, "permission": {"type": 2}, "data": "/签到", "enter": True},
                    }
                ]
            }
        ]
    }
}
b64 = base64.b64encode(json.dumps(keyboard, ensure_ascii=False).encode()).decode()
msg = Message(f"[CQ:keyboard,data=base64://{b64}]欢迎使用签到功能")
await bot.send_group_msg(group_id=821404315, message=msg)
```

## 注意

- 键盘按钮的消息以文本消息（`msg_type=0`）发送，群聊与单聊均支持。
- `action.type=2` 指令按钮的 `enter`/`reply` 参数仅单聊可用（官方版本 8983+）。
- 同一键盘内按钮 `id` 需唯一。
- 消息中同时携带 `[CQ:markdown]` 时，markdown 优先，独立 keyboard 不生效。
