# `delete_group_msg`

范围：`q群 (Group Chat)`

撤回群内指定用户或 QQ Bot 自身发送的消息。该功能是独立 OneBot action，不使用 CQ 码；正向／反向 WebSocket action 和正向 HTTP `/delete_group_msg` 均可调用。

## 参数

| 参数 | 类型 | 必填 | 说明 |
|------|------|:----:|------|
| `group_id` | string/int | 是 | 群虚拟 ID 或实际 Group OpenID。 |
| `user_id` | string/int | 条件必填 | 用户虚拟 ID 或实际 OpenID。省略、为 `0` 或负数时，表示撤回 QQ Bot 自己发送的消息。查找普通用户最后一条消息时必须提供。 |
| `message_id` | string/int | 否 | 虚拟消息 ID 或实际 QQ MessageID。省略时自动查找该用户（或 Bot）在群内最后发送的消息。 |

自动查找使用与消息缓存相同的 `msgid_ttl_seconds` 有效期。

## 示例

撤回指定用户最后一条消息：

```json
{
  "action": "delete_group_msg",
  "params": {
    "group_id": 870389197,
    "user_id": 791838020
  },
  "echo": "delete-user-last"
}
```

撤回指定消息：

```json
{
  "action": "delete_group_msg",
  "params": {
    "group_id": "2BE145B55FEEA80DB8D55EF6A1781269",
    "user_id": "FC448F053704EB30042E6EDDBCBD725D",
    "message_id": 1824
  }
}
```

撤回 Bot 自己最後發送的消息：

```json
{
  "action": "delete_group_msg",
  "params": {
    "group_id": 870389197,
    "user_id": 0
  }
}
```
