# [CQ:active]

## 当前行为

文本消息中只解析带参数的形式：

```text
[CQ:active,type=<值>,sub_type=<值>]
```

解析结果：

- CQ 码从文本中移除。
- `type` 写入内部 `foundItems["active_type"]`。
- `sub_type` 写入内部 `foundItems["active_sub_type"]`。
- 当前代码没有消费这两个字段，也不会因此改变发送通道、`msg_id` 或 `event_id`。

数组消息段也支持：

```json
{"type":"active","data":{"type":"<值>","sub_type":"<值>"}}
```

## 注意

裸 `[CQ:active]` 当前不会被文本正则匹配。需要 C2C 召回消息时，使用 [`send_private_msg_wakeup`](../../api/扩展api/扩展api-send_private_msg_wakeup.md)。

范围：`-`
