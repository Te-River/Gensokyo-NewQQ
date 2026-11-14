# [CQ:card]

> 🧪 **测试插件**：[CQ-card](https://github.com/Te-River/Gensokyo-NewQQ-Test-plugins/tree/main/CQ-card) — 发送「card」体验

## 用途

`[CQ:card]` 用于在群聊中发送图文卡片消息（`msg_type=8`）。卡片消息会展示标题、描述、图片和跳转链接。

范围：`q群 (Group Chat)`（单聊与频道不支持）

## 语法

```text
[CQ:card,title=<标题>,desc=<描述>,pic=<图片URL>,url=<跳转链接>]
```

参数顺序无关，四个参数都应填写：`title` / `desc` 缺失或超长时官方接口回 `1400`、整条消息不发送；`pic` 或 `url` 缺失时 Gensokyo 在发送前直接丢弃卡片、正文照常以文本发出（本地保护，不会走到 1400）。

| 参数 | 必填 | 说明 |
|------|------|------|
| `title` | 是 | 卡片标题，官方接口要求 1~50 字符。 |
| `desc` | 是 | 卡片描述，官方接口要求 1~200 字符。 |
| `pic` | 是 | 卡片图片：`https://` 链接（官方要求 ≤1024 字节），或本地文件路径——本地路径自动经配置的图床（`oss_type`）上传并替换为 CDN 外链（`handlers/send_group_msg.go:3136-3154` 的 `resolveCardPic`）。 |
| `url` | 是 | 点击卡片跳转的链接，官方接口要求 ≤1024 字节。 |

## 解析行为

- 从消息文本中提取 `[CQ:card,...]` 并移除，最终用户不会看到该 CQ 码。
- 参数顺序无关，内部自动解析为 `key=value` 键值对；Go 侧组装只识别 `title` / `desc` / `pic` / `url` 这四个键名。
- 数组消息段格式（`{"type":"card","data":{"title":"...","desc":"..."}}`）同样支持。
- 同一消息携带多份卡片时只发送第一份，其余被丢弃（各组装点只取 `cardItems[0]`）。

## 校验与失败行为

卡片载荷在解析阶段只做「参数收集 → JSON 编码」，三条解析链各有一个构造点（字符串 legacy 管道 `handlers/cqcode_pipeline.go:115` 的 `processCQCardPipeline`、cqparse 新解析器 `handlers/cqparse/handlers_content.go:115` 的 `resolveCard`、数组消息段 `handlers/message_parser.go:1023-1032`），最终统一由 `handlers/outbound_bridge.go:1000-1025` 的 `applyGroupCard`（新链）与 `handlers/send_group_msg.go:644`、`:756`（legacy 链）组装成 `card{type:"tuwen"}`。Gensokyo 侧不做本地字符数与 URL 格式校验，失败口径如下：

| 失败场景 | 结果 |
|----------|------|
| `title` 缺失或为空 | 卡片照常组装，官方接口以 `retcode=1400`（参数不合法）拒绝，**整条消息不发送** |
| `title` 超过 50 字符 | 同上（平台校验） |
| `desc` 缺失或为空 | 同上（平台校验） |
| `desc` 超过 200 字符 | 同上（平台校验） |
| `pic` 或 `url` 参数缺失 | 卡片被丢弃、正文照常以文本发送（`handlers/send_group_msg.go:637-639`、`handlers/outbound_bridge.go:1009-1011`） |
| `pic_url` / `url` 超过 1024 字节或协议不被平台接受 | 平台校验拒绝，Gensokyo 侧不预先拦截 |
| `[CQ:card]` 的 JSON 参数解析失败 | 卡片载荷无法反序列化时该卡片被丢弃、正文照常发送（`handlers/outbound_bridge.go:1006-1008`） |

> 消息中同时包含 `[CQ:markdown]` 时 markdown 优先：卡片直接丢弃、不参与组装（`applyGroupCard` 在 `MsgType == 2` 时立即返回，`handlers/outbound_bridge.go:1002`）。

## 发送行为

当 `[CQ:card]` 出现在 `send_group_msg` / `send_to_group` 的出站消息中时：

1. CQ 码从消息文本中移除。
2. 消息类型切换为 `msg_type=8`，发送图文卡片（`card{type:"tuwen",content{title,description,pic_url,url}}`）。
3. QQ 群聊中展示一张带标题、描述、图片和跳转链接的卡片。
4. 消息正文（`content`）会被清空，卡片不会与文本拼接（`handlers/outbound_bridge.go:1014`）。
5. 如果同一消息同时包含 `[CQ:markdown]`，markdown 优先级更高，卡片不生效。

## 使用示例

NoneBot 插件发送群聊卡片（使用 URL）：

```python
from nonebot.adapters.onebot.v11 import Message

card_msg = Message(
    "[CQ:card,title=今日推荐,desc=点击查看详情,pic=https://example.com/cover.png,url=https://example.com]"
)
await bot.send_group_msg(
    group_id=821404315,
    message=card_msg,
)
```

NoneBot 插件发送群聊卡片（使用本地路径，自动图床上传）：

```python
card_msg = Message(
    "[CQ:card,title=今日推荐,desc=点击查看详情,pic=D:/images/cover.png,url=https://example.com]"
)
await bot.send_group_msg(
    group_id=821404315,
    message=card_msg,
)
```

消息段格式（适合 NoneBot 的 MessageSegment 构造方式）：

```python
from nonebot.adapters.onebot.v11 import Message, MessageSegment

card_msg = Message([
    MessageSegment("card", {
        "title": "今日推荐",
        "desc": "点击查看详情",
        "pic": "https://example.com/cover.png",
        "url": "https://example.com",
    })
])
await bot.send_group_msg(group_id=821404315, message=card_msg)
```

## 注意

- `pic` 支持 `https://` 公网链接与本地文件路径；本地路径自动通过 `oss_type` 配置的图床上传并替换为 CDN 链接（`resolveCardPic`，`handlers/send_group_msg.go:3136-3154`）。
- 官方接口要求 `title` 1~50 字符、`desc` 1~200 字符、`pic_url` / `url` ≤1024 字节；违反时整条消息回 `1400`，Gensokyo 侧不做本地预校验。
- `pic` 或 `url` 缺失时卡片被丢弃、正文以纯文本照常发出。
- `[CQ:card]` 不会与文本内容拼接，发送卡片时文本内容会被清空。
- 消息中同时携带 `[CQ:markdown]` 时，markdown 优先生效，卡片被丢弃。
- 卡片消息仅支持群聊场景，单聊和频道不支持。
