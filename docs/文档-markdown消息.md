# Markdown 消息

Gensokyo 支持 QQ 官方 API 的 Markdown 卡片消息（`msg_type=2`），是对 OneBot V11 的扩展。

## 发送方式

### CQ 码

```
[CQ:markdown,data=xxx]
```

`data` 是经 base64 编码的 JSON 数据，支持与其他 CQ 码拼接。

### Message Segment（数据对象）

```json
{
    "type": "markdown",
    "data": {
        "data": "文本内容"
    }
}
```

data 支持：`string`（纯文本）、`base64://` 编码的 JSON，或 `map` 对象（自动序列化）。

### 对象格式

```json
{
    "type": "markdown",
    "data": {
        "data": { ... }
    }
}
```

内部结构：`data.data.markdown` + `data.data.keyboard` 双层嵌套。

### 直接调用 API

若直接调用 QQ API，msg_type 设为 2：

```json
{
    "content": "markdown",
    "msg_type": 2,
    "msg_id": "xxx",
    "markdown": { ... },
    "keyboard": { ... }
}
```

## Markdown 格式

> **按钮简写**：Gensokyo 支持 `keyboard.rows` 简写（推荐），无需 `keyboard.content` 嵌套。
> 两种格式都可使用，下文中会标注。

### 自定义 Markdown

**简写格式**（推荐，`keyboard.rows` 顶层）：

```json
{
  "markdown": {
    "content": "你好"
  },
  "keyboard": {
    "rows": [
      {
        "buttons": [
          {
            "id": "btn_ok",
            "render_data": {
              "label": "✅ 了解",
              "visited_label": "已了解"
            }
          }
        ]
      }
    ]
  }
}
```

**标准格式**（兼容，`keyboard.content.rows` 嵌套）：

```json
{
  "markdown": {
    "content": "你好"
  },
  "keyboard": {
    "content": {
      "rows": [
        {
          "buttons": [
            {
              "render_data": {
                "label": "再来一份",
                "visited_label": "正在绘图",
                "style": 1
              },
              "action": {
                "type": 2,
                "permission": {
                  "type": 2,
                  "specify_role_ids": ["1", "2", "3"]
                },
                "click_limit": 10,
                "unsupport_tips": "编辑-兼容文本",
                "data": "你好",
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

### 模板 Markdown

```json
{
  "markdown": {
    "custom_template_id": "101993071_1658748972",
    "params": [
      { "key": "text", "values": ["标题"] },
      { "key": "image", "values": ["https://example.com/img.png"] }
    ]
  },
  "keyboard": {
    "rows": [
      {
        "buttons": [
          {
            "render_data": { "label": "再来一份", "visited_label": "再来一份" },
            "action": {
              "type": 1,
              "permission": { "type": 1, "specify_role_ids": ["1"] },
              "click_limit": 10,
              "unsupport_tips": "兼容文本",
              "data": "data",
              "at_bot_show_channel_list": true
            }
          }
        ]
      }
    ]
  }
}
```

### 纯按钮（无 Markdown）

**简写格式**（推荐）：

```json
{
  "keyboard": {
    "rows": [
      {
        "buttons": [
          {
            "id": "btn_ok",
            "render_data": { "label": "✅ 了解", "visited_label": "已了解" }
          }
        ]
      }
    ]
  }
}
```

**标准格式**（兼容）：

```json
{
  "keyboard": {
    "content": {
      "rows": [
        {
          "buttons": [
            {
              "render_data": { "label": "再来一份", "visited_label": "再来一份" },
              "action": {
                "type": 1,
                "permission": { "type": 1, "specify_role_ids": ["1"] },
                "click_limit": 10,
                "unsupport_tips": "兼容文本",
                "data": "data",
                "at_bot_show_channel_list": true
              }
            }
          ]
        }
      ]
    }
  }
}
```

### 图文混排

```
{{.text}}![{{.image_info}}]({{.image_url}})
```

注意：`{{}}` 中不能使用 `![]()` 这类 Markdown 格式关键字。

### 图片自动上传

Markdown 内容中的 `![](path)` 图片，Gensokyo 会自动处理：

| path 类型 | 行为 |
|-----------|------|
| `https://...` / `http://...` | 直接保留原链接|
| 本地文件路径 | 读取文件 → base64 上传 → QQ CDN → 替换为 CDN 直链 |

```markdown
# URL 直接保留
![](https://example.com/image.png)

# 本地文件自动上传
![](C:\Users\xxx\Pictures\photo.png)
![](file:///C:/Users/xxx/Pictures/photo.png)
```

### QQ Markdown 图片尺寸语法

QQ 官方 Markdown 支持在图片中指定显示尺寸，语法为：

```markdown
![text #100px #100px](图片链接)
```

`#100px` 分别表示宽度和高度，单位 `px`，必须两个都指定

| 语法 | 效果 |
|------|------|
| `![text #100px #100px](图片链接)` | 宽高均为 100px |
| `![text #100px #200px](图片链接)` | 宽度 100px，高度 200px |

Gensokyo 自动上传本地图片到 QQ CDN 后，会保留该尺寸语法不变，确保图片按预期尺寸展示。

### Keyboard 按钮中的图片

按钮 `render_data.label` 和 `render_data.visited_label` 中的本地图片路径同样会被自动上传到图床并替换为 CDN URL，支持与 markdown 内容相同的路径格式：

```markdown
![text #20px #20px](D:\path\to\cover.png)按钮文字
![text #20px #20px](file:///C:/path/to/cover.png)按钮文字
```

## 参考

- [QQ 官方 Markdown 文档](https://bot.q.qq.com/wiki/develop/api/openapi/message/post_keyboard_messages.html)
- 推荐使用 [gensokyo-qqmd 模板](https://github.com/hoshinonyaruko/gensokyo-qqmd)
