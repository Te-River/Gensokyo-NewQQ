# [CQ:video]

## 用途

`[CQ:video]` 用于发送视频消息。QQ Bot API v2 支持 `file_type=2`（视频）的富媒体上传。

范围：`q群 (Group Chat)` / `C2C (私聊)`

## 语法

```text
[CQ:video,file=file:///path/to/video.mp4]      // 本地视频文件
[CQ:video,file=http://example.com/video.mp4]    // HTTP 远程视频
[CQ:video,file=https://example.com/video.mp4]   // HTTPS 远程视频
```

## 参数

| 参数名 | 必填 | 说明 |
|--------|------|------|
| `file` | ✅ | 视频来源，支持 `file:///`（本地）、`http(s)://`（远程） |

## 解析行为

根据 `file` 值的前缀自动判断视频来源类型：

| 前缀 | foundItems key | 处理方式 |
|------|---------------|----------|
| `file:///` | `local_video` | 本地路径，读取后走 QQ CDN 上传 |
| `http://` | `url_video` | HTTP URL，直接作为 `RichMediaMessage.URL` 发送 |
| `https://` | `url_videos` | HTTPS URL，同上 |

### SSRF 防护

所有 URL 类型的视频在发送时都会经过 SSRF 防护检查：
- 默认阻止私有地址（`10.x.x.x`、`192.168.x.x`、`127.x.x.x` 等）
- 可通过配置 `url_whitelist` 添加域名白名单来放行特定域名

## 发送行为

### URL 视频

直接将 URL 作为 `RichMediaMessage.URL`（`file_type=2`）发送给 QQ，由 QQ 服务器自行拉取视频内容。

### 本地视频

读取本地文件后通过 QQ CDN 上传（`file_type=2`）。

## 使用示例

### NoneBot 插件发送本地视频

```python
from nonebot.adapters.onebot.v11 import Message, MessageSegment

await bot.send_group_msg(
    group_id=123456,
    message=Message([
        MessageSegment.video("file:///D:/videos/clip.mp4"),
    ]),
)
```

### NoneBot 插件发送网络视频

```python
await bot.send_group_msg(
    group_id=123456,
    message=Message([
        MessageSegment.video("https://example.com/video.mp4"),
    ]),
)
```

### CQ 码字符串格式

```text
[CQ:video,file=file:///D:/videos/clip.mp4]
[CQ:video,file=https://example.com/video.mp4]
```

## 入站上报

### 字符串模式

收到的视频会以 `[CQ:video,file=xxx]` 形式追加在消息文本中。

### 数组模式

```json
[
  {"type": "video", "data": {"file": "xxx", "url": "https://..."}}
]
```

## 注意

- URL 视频受 SSRF 防护限制，内网地址默认被阻止，可通过 `url_whitelist` 配置放行。
- URL 视频的域名需要通过 QQ 平台域名校验（`identify_file` 机制），否则 QQ 可能拒绝拉取。
- 视频格式建议为 MP4（H.264 + AAC），其他格式取决于 QQ 服务端的兼容性。
