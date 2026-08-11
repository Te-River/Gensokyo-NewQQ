# `get_group_join_request_list`

范围：`q群 (Group Chat)`

拉取群内入群申请列表（QQ 官方 v2 接口 `GET /v2/groups/{group_openid}/join_request_list`）。机器人需拥有群管理员身份。

返回的 `group_id` / `user_id` / `flag` 与入站 `request` 事件完全一致，可直接回传 [`set_group_add_request`](../../readme.md) 进行审批，无需额外转换。

## 参数

| 参数 | 类型 | 必填 | 说明 |
|------|------|:----:|------|
| `group_id` | string/int | 是 | 群虚拟 ID 或实际 Group OpenID。 |
| `next_index` | int | 否 | 分页游标。首次请求可不传；后续请求传上次响应中的 `next_index`。 |

## 返回

| 字段 | 类型 | 说明 |
|------|------|------|
| `join_requests` | array | 申请列表，元素结构见下 |
| `next_index` | int | 下一页游标，`0` 表示已到末页 |

`join_requests[]` 元素：

| 字段 | 类型 | 说明 |
|------|------|------|
| `group_id` | int | 群虚拟 ID |
| `user_id` | int | 申请者虚拟 ID |
| `flag` | string | 申请 ID（`join_request_id`），审批时回传 |
| `username` | string | 申请者昵称 |
| `apply_at` | int | 申请时间（秒级时间戳） |
| `apply_source` | string | 申请来源 |
| `invited_by` | string | 邀请人 OpenID（无邀请人为空） |
| `verify_info` | string | 验证信息 |
| `auto_approved` | bool | 是否已被自动审批通过 |

## 示例

```json
{
  "action": "get_group_join_request_list",
  "params": {
    "group_id": 870389197
  },
  "echo": "req-list-1"
}
```

响应 `data`：

```json
{
  "join_requests": [
    {
      "group_id": 870389197,
      "user_id": 791838020,
      "flag": "AURi8Rr6MfGdUNedupWf2uV5XiayURHaetzwGyOdrj6mHYOsfJFkbe9u8UjCMpLTxUouwr1SJ9IGEbxlbzDi43hPS4rw64G4i2Y4nL4DTH50U15xKPZYRsXPB7WUxZOUdceNSAv_GJtO4ffSrVZIhQxknoPD2SDT",
      "username": "小明",
      "apply_at": 1723360000,
      "apply_source": "",
      "invited_by": "",
      "verify_info": "我是来学习的",
      "auto_approved": false
    }
  ],
  "next_index": 0
}
```

## nonebot2 示例

```python
from nonebot import on_command
from nonebot.adapters.onebot.v11 import Bot, Event

@on_command("approve_all").handle()
async def _(bot: Bot, event: Event):
    # 拉取申请列表
    data = await bot.call_api("get_group_join_request_list", group_id=event.group_id)
    # 批量通过
    for req in data["join_requests"]:
        await bot.call_api("set_group_add_request",
            group_id=req["group_id"], user_id=req["user_id"],
            flag=req["flag"], approve=True)
    await event.finish(f"已通过 {len(data['join_requests'])} 条申请")
```
