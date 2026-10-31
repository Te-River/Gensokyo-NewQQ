# [CQ:card]

> 🧪 **测试插件**：[CQ-card](https://github.com/Te-River/Gensokyo-NewQQ-Test-plugins/tree/main/CQ-card) — 发送「card」体验

## 用途

`[CQ:card]` 用于发送图文卡片消息（`msg_type=8`）。卡片消息会展示标题、描述、图片和跳转链接。

范围：`q群 (Group Chat)` / `私聊 (C2C)`（官方 2026-10-28 起单聊同样支持，群聊与单聊载荷同构）

## 语法

```text
[CQ:card,title=<标题>,description=<描述>,pic_url=<图片URL>,url=<跳转链接>]
```

参数顺序无关；**`title` / `description` / `url` / `pic_url` 四项全部必填**，缺任一项整条消息回 `1400`。

| 参数 | 别名 | 必填 | 说明 |
|------|------|------|------|
| `title` | — | 是 | 卡片标题，1~50 字符（按 Unicode 字符计，首尾空白会被去除）。 |
| `desc` | `description` | 是 | 卡片描述，1~200 字符（按 Unicode 字符计，首尾空白会被去除）。 |
| `pic` | `pic_url` | 是 | 卡片图片，必须是 `https://` 链接且 ≤1024 字节。**不支持本地路径与 `http://`**，也不接受 `qqbot-media://`。 |
| `url` | — | 是 | 点击卡片跳转的链接，必须是 `https://` 且 ≤1024 字节。 |

## 解析行为

- 从消息文本中提取 `[CQ:card,...]` 并移除，最终用户不会看到该 CQ 码。
- 参数顺序无关，内部自动解析为 `key=value` 键值对；别名 `description` / `pic_url` 与 `desc` / `pic` 等价。
- 数组消息段格式（`{"type":"card","data":{"title":"...","desc":"..."}}`）同样支持。
- 同一消息携带多份卡片时只发送第一份，其余记录日志后丢弃。

## 校验与失败行为

所有构造点共用同一套校验（`handlers/card_payload.go` 的 `buildTuwenCard`），任一项不通过即返回 `retcode=1400`，**整条消息不发送**：

| 失败场景 | 结果 |
|----------|------|
| `title` 缺失或去空白后为空 | 1400 |
| `title` 超过 50 字符 | 1400 |
| `description` 缺失或去空白后为空 | 1400 |
| `description` 超过 200 字符 | 1400 |
| `url` / `pic_url` 缺失 | 1400 |
| `url` / `pic_url` 不是 `https://`（含 `http://` 与本地路径） | 1400 |
| `url` / `pic_url` 超过 1024 **字节** | 1400 |
| `[CQ:card]` 的 JSON 参数解析失败 | 1400 |

> 卡片与 @全员 同现（markdown 未优先）同样回 `1400`：卡片会清空正文，@全员 标签无法承载。markdown 与卡片同现时 markdown 优先，卡片被丢弃且不参与校验。

> 2026-10 行为变更：此前 `pic` 为本地路径会自动走 OSS 图床上传、非法卡片会被跳过并继续发送文本；现在本地路径与 `http` 一律 1400，且整条消息失败（不再静默降级为纯文本）。

## 发送行为

当 `[CQ:card]` 出现在 `send_group_msg` / `send_to_group` / `send_private_msg` 的出站消息中时：

1. CQ 码从消息文本中移除。
2. 消息类型切换为 `msg_type=8`，发送图文卡片（`card{type:"tuwen",content{title,description,pic_url,url}}`）。
3. 群聊与单聊都展示同一张带标题、描述、图片和跳转链接的卡片。
4. 消息正文（`content`）会被清空，卡片不会与文本拼接。
5. 如果同一消息同时包含 `[CQ:markdown]`，markdown 优先级更高：卡片直接丢弃且不参与校验（避免一张坏卡片拦住合法的 markdown）。

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

发送单聊卡片（同一 CQ 码）：

```python
await bot.send_private_msg(
    user_id=123456789,
    message=Message(
        "[CQ:card,title=今日推荐,desc=点击查看详情,pic=https://example.com/cover.png,url=https://example.com]"
    ),
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

- `pic` / `url` 必须为 `https://` 且 ≤1024 字节；本地图片需先转存为公网 `https://` 地址（卡片 `pic` 不接受 `qqbot-media://`）。
- 四字段必填：`title` 1~50 字符、`description` 1~200 字符、`url` / `pic_url` 均为 https 链接。
- 与 @全员 同现（markdown 未优先）回 `1400`；需要同时 @全员 时请改用 markdown 消息。
- `[CQ:card]` 不会与文本内容拼接，发送卡片时文本内容会被清空。
- 消息中同时携带 `[CQ:markdown]` 时，markdown 优先生效，卡片被丢弃。
- 频道场景不支持卡片消息。
