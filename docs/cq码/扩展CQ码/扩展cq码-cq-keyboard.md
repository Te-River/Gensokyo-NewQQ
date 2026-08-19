# [CQ:keyboard]

> 🧪 **测试插件**：[CQ-keyboard](https://github.com/Te-River/Gensokyo-NewQQ-Test-plugins/tree/main/CQ-keyboard) — 发送「键盘测试」体验

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

## 完整构建（按钮 JSON 全字段诠释）

以下字段结构与 QQ 开放平台官方文档一致，官网参考：

- 发送群聊消息：https://bot.q.qq.com/wiki/develop/api-v2/autogen/api/v2_groups_group_openid_messages.post.html
- 发送单聊消息：https://bot.qq.com/wiki/develop/api-v2/autogen/api/v2_users_user_openid_messages.post.html

### Keyboard（键盘对象）

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `id` | string | 否 | 内嵌键盘模板 ID。使用平台预设模板时填写 |
| `content` | KeyboardContent | 否 | 自定义键盘布局。与 `id` 互斥，用于自定义按钮 |

### KeyboardContent（键盘内容）

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `rows` | Row[] | 否 | 按钮行列表 |

### Row（按钮行）

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `buttons` | Button[] | 否 | 行内按钮，从左到右排列 |

### Button（按钮）

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `id` | string | 否 | 按钮 ID。同一键盘内唯一 |
| `render_data` | RenderData | 否 | 按钮渲染 |
| `action` | Action | 否 | 按钮点击行为 |

### RenderData（按钮渲染）

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `label` | string | 否 | 按钮文字，最多 10 字符 |
| `visited_label` | string | 否 | 点击后文字，不传则保持不变 |
| `style` | integer | 否 | 0=灰线框, 1=蓝线框, 2=白字, 3=蓝底白字 |

### Action（点击行为）

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `type` | integer | 否 | 0：跳转按钮（http 或小程序）；1：回调按钮（回调后台接口，data 传给后台）；2：指令按钮（自动在输入框插入 `@bot data`） |
| `permission` | Permission | 否 | 操作权限 |
| `data` | string | 否 | 回调数据。type=1/2 时必填 |
| `click_limit` | integer | 否 | 【已废弃】可点击次数限制。0=无限 |
| `unsupport_tips` | string | 否 | 版本过低时提示文案 |
| `enter` | boolean | 否 | 指令按钮可用，点击按钮后直接自动发送 data，仅单聊可用，默认 false。支持版本 8983 |
| `reply` | boolean | 否 | 指令按钮可用，指令是否带引用回复本消息，默认 false。支持版本 8983 |
| `anchor` | integer | 否 | 仅指令按钮下有效，设置后忽略 action.enter。1=点击按钮自动唤起手Q选图器，其他值无效果（仅手机端 8983+ 单聊，桌面端不支持） |

### Permission（权限）

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `type` | integer | 否 | 0=指定用户, 1=管理员, 2=所有人 |
| `specify_user_ids` | string[] | 否 | 有权限的用户 id 列表（Gensokyo 自动将数字虚拟 ID 转为 OpenID） |
| `specify_role_ids` | string[] | 否 | 有权限的身份组 id 列表（仅频道可用） |

### 完整示例（三按钮：指令 + 回调 + 跳转）

```json
{
  "content": {
    "rows": [
      {
        "buttons": [
          {
            "id": "btn_cmd",
            "render_data": { "label": "签到", "style": 1 },
            "action": {
              "type": 2,
              "data": "/签到",
              "permission": { "type": 2 },
              "enter": true
            }
          },
          {
            "id": "btn_cb",
            "render_data": { "label": "回调", "style": 0 },
            "action": {
              "type": 1,
              "data": "callback_key",
              "permission": { "type": 1 }
            }
          },
          {
            "id": "btn_url",
            "render_data": { "label": "官网", "style": 2 },
            "action": {
              "type": 0,
              "data": "https://example.com",
              "permission": { "type": 2 },
              "unsupport_tips": "请升级QQ版本"
            }
          }
        ]
      }
    ]
  }
}
```

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
