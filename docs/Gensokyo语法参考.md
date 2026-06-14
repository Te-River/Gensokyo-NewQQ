# Gensokyo 语法参考

Gensokyo 对 OneBot V11 的扩展语法汇总。

## CQ 码（扩展）

| CQ 码 | 格式 | 适用范围 | 说明 |
|-------|------|:--------:|------|
| Markdown | `[CQ:markdown,data=base64]` | 🌐 全场景 | Markdown 卡片消息 |
| 头像 | `[CQ:avatar,qq=数字]` | 🌐 全场景 | 在消息中嵌入用户头像图片 |
| QQ 音乐 | `[CQ:music,type=qq,id=数字]` | 🌐 全场景 | 分享 QQ 音乐（自动转为 Markdown 卡片） |
| 回复 | `[CQ:reply,id=数字]` | 📡 仅频道 | 引用回复标记。**频道端**渲染引用样式，群聊端 API 接受但不显示；**Markdown 消息**不支持 |

> 图例: 🌐 全场景 | 🏷️ 群聊 | 📡 频道 | 💬 C2C

## Message Segment 类型

### `markdown` — Markdown 卡片

```json
{
    "type": "markdown",
    "data": {
        "data": "文本内容"
    }
}
```

data 支持三种形式：

| data 类型 | 示例 | 说明 |
|-----------|------|------|
| string | `"纯文本"` | 普通文本 |
| base64:// | `"base64://eyJtYXJrZG93biI6..."` | base64 编码的 JSON |
| map/object | `{"markdown":{...},"keyboard":{...}}` | JSON 对象（自动序列化） |

嵌套结构：`data → data → markdown / keyboard`

### `avatar` — 头像

```json
{
    "type": "avatar",
    "data": {
        "qq": "123456"
    }
}
```

在消息中插入用户头像图片。

### `text` — 文本（含 CQ 码解析）

```json
{
    "type": "text",
    "data": {
        "text": "你好 [CQ:at,qq=123456]"
    }
}
```

文本内容中的 CQ 码会被自动解析（at、image、markdown 等）。

### `at` — @ 某人

```json
{
    "type": "at",
    "data": {
        "qq": "123456"
    }
}
```

出站时自动转为 `<qqbot-at-user id="OpenID" />`。

### `image` — 图片

```json
{
    "type": "image",
    "data": {
        "file": "base64://...",
        "file": "http://...",
        "file": "file://..."
    }
}
```

支持 base64、HTTP(S) URL、本地文件路径。

### `voice` / `record` — 语音

```json
{
    "type": "record",
    "data": {
        "file": "base64://..."
    }
}
```

支持 base64、HTTP(S) URL、本地文件路径。

## Markdown 卡片格式

### 自定义 Markdown

```json
{
  "markdown": {
    "content": "### 标题\n内容"
  },
  "keyboard": {
    "content": {
      "rows": [
        {
          "buttons": [
            {
              "render_data": {
                "label": "按钮文字",
                "visited_label": "点击后文字",
                "style": 1
              },
              "action": {
                "type": 2,
                "permission": { "type": 2 },
                "data": "回调数据",
                "unsupport_tips": "兼容文本",
                "click_limit": 10,
                "at_bot_show_channel_list": false
              }
            }
          ]
        }
      ]
    }
  }
}
```

### 按钮 action.type

| type | 说明 |
|------|------|
| 0 | 跳转链接 |
| 1 | 回调（reply） |
| 2 | 回调（带输入框） |

### 按钮 permission.type

| type | 说明 |
|------|------|
| 0 | 指定用户（specify_user_ids） |
| 1 | 指定角色（specify_role_ids） |
| 2 | 所有人 |

> 注意：C2C 私聊场景下，permission.type=0 会被自动改为 2（QQ API 限制）。

### 按钮虚拟 ID 自动转化

`specify_user_ids` 中的虚拟数字 ID 会自动转为真实 QQ OpenID。

### 模板 Markdown

```json
{
  "markdown": {
    "custom_template_id": "模板ID",
    "params": [
      { "key": "text", "values": ["标题"] },
      { "key": "image", "values": ["https://..."] }
    ]
  },
  "keyboard": { ... }
}
```

### Markdown 中的 @

在 Markdown 文本内容中嵌入 `[CQ:at,qq=数字]`，Gensokyo 自动转换为 QQ 官方 @ 标签：

```
[CQ:at,qq=713011248]
→ <qqbot-at-user id="真实OpenID" />
```

### Markdown 中的图片

在 Markdown 文本内容中使用 `![](path)` 嵌入图片，Gensokyo 自动处理：

| path 类型 | 行为 |
|-----------|------|
| `https://...` / `http://...` | 直接保留，不做处理 |
| `file:///C:/...` 或 `C:/...` | 读取本地文件 → base64 上传 QQ CDN → 替换为 CDN URL |
| 其他本地路径 | 同上 |

**示例：**

```markdown
### 标题
这是说明文字

![](https://example.com/public.png)          ← 保留 URL
![](C:\\Users\\xxx\\Pictures\\photo.png)     ← 自动上传
![](file:///C:/Users/xxx/Pictures/photo.png)  ← 自动上传
```

## 消息事件扩展字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `to_me` | bool | 消息是否 @ 了机器人 |
| `real_message_type` | string | 真实消息类型（group/guild/guild_private） |
| `real_user_id` | string | 真实用户 QQ OpenID |
| `real_group_id` | string | 真实群 OpenID |
| `is_binded_group_id` | bool | 群号是否经过 bind 映射 |
| `is_binded_user_id` | bool | 用户号是否经过 bind 映射 |
| `avatar` | string | 发送者头像 URL |

## @ 自动转换

```
入站: <@OpenID> → [CQ:at,qq=虚拟ID]（非自身 @）
入站: <@BotOpenID> → 从内容剥离，to_me=true
出站: [CQ:at,qq=数字] → <qqbot-at-user id="OpenID" />
MD 内: [CQ:at,qq=数字] → <qqbot-at-user id="OpenID" />
```

## Sender.Nickname 自动填充

优先顺序：`card_nick` 配置值 > `Author.Username` > 空

## 扩展 API

| Action | 说明 |
|--------|------|
| `get_avatar` | 获取用户头像直链（[文档](./额外api-get_avatar.md)） |
| `get_robot_share_link` | 获取机器人分享链接 |
| `send_private_msg_wakeup` | 发送被动唤醒私聊消息 |
| `put_interaction` | 处理按钮交互回调 |
