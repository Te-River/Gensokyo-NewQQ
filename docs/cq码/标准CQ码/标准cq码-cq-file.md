# [CQ:file]

## 用途

`[CQ:file]` 用于发送文件消息。QQ Bot API v2 支持 `file_type=4`（文件）的富媒体上传，可通过 CDN 直接上传或通过 URL 链接发送。

范围：`q群 (Group Chat)` / `C2C (私聊)` / `C2C 互动召回`

## 语法

```text
[CQ:file,file=file:///path/to/file]                // 本地文件路径
[CQ:file,file=http://example.com/file.zip]          // HTTP 远程文件
[CQ:file,file=https://example.com/file.zip]         // HTTPS 远程文件
[CQ:file,file=base64://<base64_data>]               // base64 编码数据
[CQ:file,file=file:///path,file_name=myfile.txt]    // 可选指定文件名
```

所有格式均支持可选的 `file_name` 参数，用于自定义文件名。该参数会以 `file_name` 字段写入 QQ Bot API 的 `/files` 上传请求体中，QQ 收到后即以此名显示。

## 与标准 OneBot V11 的差异

标准 OneBot V11 协议中 `[CQ:file]` 仅用于接收消息时的文件信息上报，**不支持发送文件**。Gensokyo 扩展了 `[CQ:file]` 使其**支持出站发送**，利用 QQ Bot API v2 的富媒体文件上传能力。

| 差异点 | 标准 OneBot V11 | Gensokyo 扩展 |
|--------|----------------|---------------|
| 发送文件 | ❌ 不支持 | ✅ 完整支持 |
| 本地文件 | ❌ | ✅ `file:///` 路径 |
| URL 文件 | ❌ | ✅ `http(s)://` 链接 |
| base64 | ❌ | ✅ `base64://` 数据 |
| 自定义文件名 | ❌ | ✅ `file_name` 参数(预留) |

## 解析行为

根据 `file` 值的前缀自动判断文件来源类型：

| 前缀 | foundItems key | 处理方式 |
|------|---------------|----------|
| `file:///` | `local_file` | 本地路径，URL 解码后 `os.ReadFile` 读取，base64 编码后走 QQ CDN 上传 |
| `http://` | `url_file` | HTTP URL，直接作为 `RichMediaMessage.URL` 交给 QQ CDN 拉取 |
| `https://` | `url_files` | HTTPS URL，同上 |
| `base64://` | `base64_file` | 去除前缀后直接走 QQ CDN 上传 |

文件上传走 `POST /v2/groups/{group_id}/files` 或 `POST /v2/users/{user_id}/files` 接口，`file_type=4`。

## 文件名优先级

1. `file_name` 参数（如果填写）
2. `filepath.Base()` 自动从路径/URL 末尾提取
3. 空（base64 来源无法提取文件名）

> 💡 **注意：** 使用 `file_data`（base64）上传时，如需自定义文件名，请添加 `file_name` 参数。URL 方式发送时 QQ 默认从 URL 末尾提取文件名，也可用 `file_name` 覆盖。

## 使用示例

### NoneBot 插件发送本地文件

```python
from nonebot.adapters.onebot.v11 import Message, MessageSegment

# 发送本地文件到群
await bot.send_group_msg(
    group_id=821404315,
    message=Message([
        MessageSegment("file", {"file": "file:///D:/data/document.pdf"}),
    ]),
)

# 发送本地文件并指定文件名（预留）
await bot.send_group_msg(
    group_id=821404315,
    message=Message([
        MessageSegment("file", {
            "file": "file:///D:/data/document.pdf",
            "file_name": "报告.pdf",
        }),
    ]),
)

# 发送远程文件到私聊
await bot.send_private_msg(
    user_id="<OpenID>",
    message=Message([
        MessageSegment("file", {"file": "https://example.com/file.zip"}),
    ]),
)
```

### CQ 码字符串格式

```text
[CQ:file,file=file:///D:/data/report.zip]
[CQ:file,file=https://example.com/software.exe,file_name=installer.exe]
```

## 支持场景

- 群聊发送文件（`send_group_msg` / `send_to_group`）
- 私聊发送文件（`send_private_msg`）
- 私聊互动召回消息（`send_private_msg_wakeup`）
