# Markdown 中的 [CQ:at]

## 行为

普通文本中的 `[CQ:at,qq=<虚拟用户ID>]` 会先转成 QQ 官方 @ 标签：

```text
<qqbot-at-user id="OpenID" />
```

当消息包含 Markdown 时，Gensokyo 会把该标签合并到 Markdown 内容中。Markdown 内容自身也可以直接写 `[CQ:at,qq=<虚拟用户ID>]`，发送前会转换为同样的 QQ 标签。

## 写法

```text
[CQ:at,qq=<虚拟用户ID>][CQ:markdown,data=base64://<base64-json>]
```

Markdown JSON 中也可以写：

```markdown
你好 [CQ:at,qq=123456]
```

## 限制

- `qq` 必须能通过 idmap 反查到 OpenID；失败时保留原 CQ 码或原数字。
- 该转换在 `send_group_msg` 和 `send_guild_channel_msg` 路径中执行。

范围：`q群 (Group Chat)` / `q頻 (QQ Guild)`
