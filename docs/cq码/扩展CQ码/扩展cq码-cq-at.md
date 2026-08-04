# [CQ:at]

## 说明

`[CQ:at,qq=<虚拟用户ID>]` 在出站时根据消息类型做不同处理。

## 行为

### Markdown 消息

在 Markdown 内容（`msg_type=2`）中写入 `[CQ:at,qq=<虚拟用户ID>]`，Gensokyo 会在发送前将其转换为 QQ 官方 @ 标签：

```text
<qqbot-at-user id="<真实OpenID>" />
```

此外，`messageText`（文本段）中的 `[CQ:at,qq=<虚拟用户ID>]` 也会被一并转换，并且**整个 `messageText`**（含 `<qqbot-at-user>` 标签和文本内容）会合并到 Markdown 内容头部。这意味着即使 `[CQ:at]` 写在 Markdown JSON 之外（如作为数组段中的 `{"type":"at","data":{"qq":"..."}}`），也能正确渲染为 @ 标签，且 `[CQ:at]` 前后的文本不会丢失。

### 纯文本消息

在纯文本消息中，`[CQ:at,qq=<虚拟用户ID>]` 会被替换为 `@用户名 `（带空格），用户名来自 `idmap.GetUserName` 缓存（入站时自动缓存，10 分钟 TTL）。缓存过期或不存在时保留原 CQ 码。

```text
纯文本出站: [CQ:at,qq=123213]你好 → @张三 你好
```

### 图文混合消息（msg_type=7）

图文混合消息中的文本段同样会经过 `resolvePlainTextAtMentions` 转换，与纯文本路径行为一致。覆盖范围：

- `send_group_msg` / `send_to_group`（群聊图文混合）
- `send_group_msg_raw`（raw 变体）
- `send_private_msg`（私聊图文混合）
- `send_guild_channel_msg`（频道图文混合，含 base64 与 multipart 两条子路径）

```text
图文混合出站: 图片[CQ:at,qq=123213] → 图片@张三 
```

> 2026-08 修复：此前图文混合路径未调用 `resolvePlainTextAtMentions`，导致 `[CQ:at,qq=数字]` 原文显示。

### 图文混合消息走 Markdown 路径（auto_md，msg_type=2）

当图文混合消息触发 `auto_md`（`transmd=true`，`MsgType=2`）时，`messageText` 会在塞进 Markdown 参数前经过 `ResolveMarkdownAtMentions` 转换，将 `[CQ:at,qq=数字]` 转为 `<qqbot-at-user id="OpenID" />` 标签，与纯 Markdown 消息行为一致。

覆盖范围（共用 `auto_md`）：
- `send_group_msg` / `send_to_group`（群聊图文混合 → Markdown）
- `send_group_msg_raw`（raw 变体图文混合 → Markdown）
- `send_guild_channel_msg`（频道图文混合 → Markdown）

```text
图文混合 Markdown 出站: 图片[CQ:at,qq=123213]你好 → 图片<qqbot-at-user id="OpenID" />你好
```

> 2026-08 修复：此前 `auto_md` 把含 `[CQ:at]` 的 `messageText` 直接塞进 Markdown 参数，从未调用 `ResolveMarkdownAtMentions`，导致 QQ 官方 Markdown 渲染把 `[CQ:at,qq=数字]` 当纯文本显示（变形为 `[CO:at,qq=数字]`）。

## 写法

```text
[CQ:at,qq=<虚拟用户ID>][CQ:markdown,data=base64://<base64-json>]
```

Markdown JSON 中也可以写：

```markdown
你好 [CQ:at,qq=123456]
```

## 入站方向

QQ 平台发送的 `<@OpenID>` 会被自动转换为标准的 `[CQ:at,qq=<虚拟ID>]` 格式，并建立 OpenID 与虚拟 ID 的映射。

## 限制

- `qq` 必须能通过 idmap 反查到 OpenID；失败时保留原 CQ 码。
- 纯文本出站时，用户名从内存缓存获取（10 分钟过期），缓存未命中时保留原 CQ 码。
