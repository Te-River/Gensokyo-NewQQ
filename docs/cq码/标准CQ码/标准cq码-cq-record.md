# [CQ:record]

## 用途

`[CQ:record]` 用于发送和接收语音消息。Gensokyo 会将语音文件自动转码为 silk 格式后上传至 QQ CDN。

范围：`q群 (Group Chat)` / `C2C (私聊)`

## 语法

```text
[CQ:record,file=file:///path/to/audio.mp3]      // 本地音频文件
[CQ:record,file=http://example.com/audio.mp3]    // HTTP 远程音频
[CQ:record,file=https://example.com/audio.mp3]   // HTTPS 远程音频
[CQ:record,file=base64://<base64_data>]          // base64 编码音频
```

## 参数

| 参数名 | 必填 | 说明 |
|--------|------|------|
| `file` | ✅ | 语音来源，支持 `file:///`（本地）、`http(s)://`（远程）、`base64://`（编码数据） |

## 解析行为

根据 `file` 值的前缀自动判断语音来源类型：

| 前缀 | foundItems key | 处理方式 |
|------|---------------|----------|
| `file:///` | `local_record` | 本地路径，读取后走 silk 转码，再上传 QQ CDN |
| `http://` | `url_record` | HTTP URL，下载后走 silk 转码，再上传 QQ CDN |
| `https://` | `url_records` | HTTPS URL，同上 |
| `base64://` | `base64_record` | 去除前缀后直接走 silk 转码，再上传 QQ CDN |

### SSRF 防护

所有 URL 类型的语音在发送时都会经过 SSRF 防护检查：
- 默认阻止私有地址（`10.x.x.x`、`192.168.x.x`、`127.x.x.x` 等）
- 可通过配置 `url_whitelist` 添加域名白名单来放行特定域名

### 转码流程

1. 获取原始音频数据（本地读取 / URL 下载 / base64 解码）
2. 使用 silk 编码器转码（采样率和比特率可通过 `record_sampleRate` 和 `record_bitRate` 配置）
3. 转码后的 silk 数据进行 base64 编码
4. 通过 `CreateAndUploadMediaMessage` 上传至 QQ CDN（`file_type=3`）

> ⚠️ silk 转码依赖 ffmpeg。请确保运行环境已安装 ffmpeg，否则转码会失败并返回错误提示。

## 发送行为

语音消息统一走 QQ CDN 上传（`file_type=3`），不支持 URL 直链发送。上传成功后 QQ 会自动处理语音的播放。

## 使用示例

### NoneBot 插件发送本地语音

```python
from nonebot.adapters.onebot.v11 import Message, MessageSegment

await bot.send_group_msg(
    group_id=123456,
    message=Message([
        MessageSegment.record("file:///D:/audio/hello.mp3"),
    ]),
)
```

### NoneBot 插件发送网络语音

```python
await bot.send_group_msg(
    group_id=123456,
    message=Message([
        MessageSegment.record("https://example.com/audio.mp3"),
    ]),
)
```

### CQ 码字符串格式

```text
[CQ:record,file=file:///D:/audio/hello.mp3]
[CQ:record,file=https://example.com/audio.mp3]
[CQ:record,file=base64://SUQzBAAAA...]
```

## 入站上报

### 字符串模式

收到的语音会以 `[CQ:record,file=xxx]` 形式追加在消息文本中。

### 数组模式

```json
[
  {"type": "record", "data": {"file": "xxx", "url": "https://..."}}
]
```

## 配置项

| 配置项 | 默认值 | 说明 |
|--------|--------|------|
| `record_sampleRate` | `24000` | 语音采样率（Hz），最高 48000 |
| `record_bitRate` | `24000` | 语音比特率（bps），默认 25000 |

## 注意

- 语音**必须**经过 silk 转码才能被 QQ 正确播放，Gensokyo 会自动完成此步骤。
- URL 语音受 SSRF 防护限制，内网地址默认被阻止，可通过 `url_whitelist` 配置放行。
- 支持的输入格式取决于 ffmpeg 的编解码能力，常见格式（mp3、wav、ogg、aac、flac）均可。
