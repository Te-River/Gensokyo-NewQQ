# [CQ:image]

## 用途

`[CQ:image]` 用于发送和接收图片消息。QQ Bot API v2 支持 `file_type=1`（图片）的富媒体上传，可通过 CDN 直接上传或通过 URL 链接发送。

范围：`q群 (Group Chat)` / `C2C (私聊)`

## 语法

```text
[CQ:image,file=file:///path/to/image.png]      // 本地图片路径
[CQ:image,file=http://example.com/image.png]    // HTTP 远程图片
[CQ:image,file=https://example.com/image.png]   // HTTPS 远程图片
[CQ:image,file=base64://<base64_data>]          // base64 编码图片
```

## 参数

| 参数名 | 必填 | 说明 |
|--------|------|------|
| `file` | ✅ | 图片来源，支持 `file:///`（本地）、`http(s)://`（远程）、`base64://`（编码数据） |

## 解析行为

根据 `file` 值的前缀自动判断图片来源类型：

| 前缀 | foundItems key | 处理方式 |
|------|---------------|----------|
| `file:///` | `local_image` | 本地路径，URL 解码后 `os.ReadFile` 读取，压缩后走 QQ CDN 上传 |
| `http://` | `url_image` | HTTP URL，根据 `url_pic_transfer` 决定直接发送链接还是下载后重新上传 |
| `https://` | `url_images` | HTTPS URL，同上 |
| `base64://` | `base64_image` | 去除前缀后直接走 QQ CDN 上传 |

### URL 图片的处理分支

当 `url_pic_transfer=true` 时：
1. 先从 URL 下载图片数据
2. 转为 base64 后通过 QQ CDN 上传
3. 使用上传后的 URL 发送

当 `url_pic_transfer=false`（默认）时：
- 直接将原始 URL 作为 `RichMediaMessage.URL` 发出
- ⚠️ 此时 URL 域名需通过 QQ 平台域名校验（`identify_file` 机制），否则可能被拦截

### SSRF 防护

所有 URL 类型的图片在发送时都会经过 SSRF（服务端请求伪造）防护检查：
- 默认阻止私有地址（`10.x.x.x`、`192.168.x.x`、`127.x.x.x` 等）
- 可通过配置 `url_whitelist` 添加域名白名单来放行特定域名

## 发送行为

### 群聊

图片通过 `RichMediaMessage`（`file_type=1`）发送，走 QQ CDN 上传或 URL 直链。

### 私聊

同群聊，通过 `RichMediaMessage` 发送。

### Markdown 路径

当消息被转换为 Markdown 格式发送时（`auto_md` / `native_md`），图片会被转换为 Markdown 图片语法 `![](url)`。

## 使用示例

### NoneBot 插件发送本地图片

```python
from nonebot.adapters.onebot.v11 import Message, MessageSegment

await bot.send_group_msg(
    group_id=123456,
    message=Message([
        MessageSegment.image("file:///D:/images/photo.png"),
    ]),
)
```

### NoneBot 插件发送网络图片

```python
await bot.send_group_msg(
    group_id=123456,
    message=Message([
        MessageSegment.image("https://example.com/photo.jpg"),
    ]),
)
```

### CQ 码字符串格式

```text
[CQ:image,file=file:///D:/images/photo.png]
[CQ:image,file=https://example.com/photo.jpg]
[CQ:image,file=base64://iVBORw0KGgo...]
```

## 入站上报

### 字符串模式

收到的图片会在消息文本末尾追加 `[CQ:image,file=xxx.image,url=xxx]`。

### 数组模式

```json
[
  {"type": "image", "data": {"file": "xxx.image", "subType": "0", "url": "https://gchat.qpic.cn/..."}}
]
```

## 注意

- 本地文件路径需使用 `file:///` 前缀（三个斜杠），Windows 路径如 `file:///D:/path/to/file`。
- 图片大小受 QQ API 限制，建议不超过 5MB。
- `url_pic_transfer` 配置仅影响 URL 图片，本地图片和 base64 图片始终走 CDN 上传。
- URL 图片受 SSRF 防护限制，内网地址默认被阻止，可通过 `url_whitelist` 配置放行。
