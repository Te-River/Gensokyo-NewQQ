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
