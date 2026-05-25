# Gensokyo Embed 消息扩展

Gensokyo 对 OneBot v11 的扩展，支持 QQ 官方 API 的 Embed 卡片消息（`msg_type=4`）。

> **📌 相关内容**
> - [Markdown 消息定义](./文档-markdown定义.md) — 富文本/模板消息（msg_type=2）
> - [Markdown Message Segment](./文档-markdown%20message%20segment.md) — segment 格式定义
> - [文档索引](./更多文档.md)

---

## Embed 结构说明

QQ Embed 消息由标题、描述、摘要（Prompt）、缩略图和字段列表组成，适合展示结构化信息。

| Embed 字段   | 类型     | 必填 | 说明                               |
|-------------|----------|------|------------------------------------|
| `title`       | string   | 否   | 卡片标题                           |
| `description` | string   | 否   | 卡片描述                           |
| `prompt`      | string   | **是** | 消息列表摘要/通知栏弹窗文本        |
| `thumbnail`   | object   | 否   | 缩略图，`{"url": "https://..."}`   |
| `fields`      | array    | 否   | 字段列表，每个有 `name` 和 `value` |

### 完整示例

```json
{
  "title": "今日天气",
  "description": "XX市XX区 2026-05-25",
  "prompt": "天气卡片",
  "thumbnail": {
    "url": "https://example.com/weather_icon.png"
  },
  "fields": [
    { "name": "温度", "value": "28°C" },
    { "name": "湿度", "value": "65%" },
    { "name": "风力", "value": "3级" }
  ]
}
```

---

## 发送方式

### 方式一：Message Segment 数组（推荐）

适用于 nonebot2 等使用 segment array 的 OneBot 客户端。

数据放在 `data.data` 中（双层 data），支持传 **map 对象** 或 **base64:// 编码的 JSON 字符串**。

#### 传对象 (map)

```json
{
    "type": "embed",
    "data": {
        "data": {
            "title": "今日天气",
            "description": "广州市 2026-05-25",
            "prompt": "天气卡片",
            "thumbnail": {
                "url": "https://example.com/icon.png"
            },
            "fields": [
                { "name": "温度", "value": "28°C" },
                { "name": "湿度", "value": "65%" }
            ]
        }
    }
}
```

#### 传 base64 编码的 JSON 字符串

```json
{
    "type": "embed",
    "data": {
        "data": "base64://eyJ0aXRsZSI6IuS7iuaXoeWksei0pSIsInByb21wdCI6IuWksei0peWNh+eahCJ9"
    }
}
```

### 方式二：CQ 码

```markdown
[CQ:embed,data=base64://eyJ0aXRsZSI6IuS7iuaXoW...]
```

`data` 的值为 Embed JSON 的 base64 编码（base64:// 前缀可选）。

### 方式三：直接发 msg_type=4 的 JSON（高级）

如果你直接调用 `/v2/groups/{group_id}/messages` API：

```json
{
    "content": "embed",
    "msg_type": 4,
    "msg_id": "xxx",
    "embed": {
        "title": "今日天气",
        "prompt": "天气卡片",
        "fields": [
            { "name": "温度", "value": "28°C" }
        ]
    }
}
```

---

## 支持的场景

| 场景             | 支持 | 说明                              |
|-----------------|------|-----------------------------------|
| 群聊发消息         | ✅   | `send_group_msg`                  |
| 私聊发消息         | ✅   | `send_private_msg`                |
| 频道发消息         | ✅   | `send_guild_channel_msg`          |
| 频道私信          | ✅   | `send_guild_private_msg`          |
| CQ 码格式        | ✅   | `[CQ:embed,data=base64://...]`    |
| Message Segment  | ✅   | `{"type":"embed","data":{...}}`   |

---

## 与 Markdown 的对比

| 特性          | Markdown (msg_type=2) | Embed (msg_type=4)      |
|--------------|----------------------|------------------------|
| 文本格式       | 支持 Markdown 语法      | 纯文本（title/desc/fields） |
| 按钮组件       | 支持 Keyboard         | ❌ 不支持                |
| 模板支持       | ✅ custom_template_id | ❌ 不支持                |
| 缩略图         | ❌                    | ✅ 支持                  |
| 通知栏摘要     | 依赖模板                | ✅ 独立的 `prompt` 字段   |
| 适用场景       | 富文本、互动卡片          | 结构化信息展示            |

---

## 注意事项

1. **`prompt` 为必填字段** — 用于消息列表的摘要显示和通知栏弹窗，不填可能发送失败
2. **`fields` 字段不宜过多** — 建议不超过 4 个，过多会被折叠
3. **公域机器人限制** — Embed 消息也需要 `msg_id` 或 `event_id` 被动消息通道，否则会被审核
4. **`thumbnail.url` 必须可公开访问** — QQ 客户端会直接拉取该图片

---

## 参考

- QQ 官方 API 文档：[消息类型 - Embed](https://bot.q.qq.com/wiki/develop/api-v2/)
- Gensokyo 实现源码：`handlers/message_parser.go` `parseEmbedData` 函数
- Markdown 消息参考：`docs/文档-markdown message segment.md`