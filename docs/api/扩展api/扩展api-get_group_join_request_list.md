# `get_group_join_request_list`

> 🧪 **测试插件**：[join_approval](https://github.com/Te-River/Gensokyo-NewQQ-Test-plugins/tree/main/join_approval) — 入群申请转发审批（配合 `join_approval_strategy` / `set_group_add_request`）

范围：`q群 (Group Chat)`

拉取群内入群申请列表（QQ 官方 v2 接口 `GET /v2/groups/{group_openid}/join_request_list`，2026-09 文档）。机器人需拥有群管理员身份。

返回的 `group_id` / `user_id` / `flag` 与入站 `request` 事件完全一致，可直接回传 [`set_group_add_request`](../../../readme.md) 进行审批，无需额外转换。

## 参数

| 参数 | 类型 | 必填 | 说明 |
|------|------|:----:|------|
| `group_id` | string/int | 是 | 群虚拟 ID 或实际 Group OpenID。 |
| `cursor` | string | 否 | 分页游标（官方字段）。首次请求可不传或传空串；后续请求传上次响应中的 `next_cursor`。 |
| `limit` | int | 否 | 单页数量（官方字段）。默认 `20`，最大 `50`，超出按 `50` 截断。 |
| `next_index` | int | 否 | **旧字段（兼容保留）**：`cursor` 为空时回退使用，转成字符串后作为 `cursor` 传给官方。 |

## 返回

| 字段 | 类型 | 说明 |
|------|------|------|
| `list` | array | 申请列表（官方字段），元素结构见下 |
| `next_cursor` | string | 下一页游标（官方字段），空串表示已到末页 |
| `join_requests` | array | **旧字段（兼容保留）**：与 `list` 内容相同的别名 |
| `next_index` | int | **旧字段（兼容保留）**：`next_cursor` 可解析为整数时取其值，否则 `0` |

`list[]` 元素：

| 字段 | 类型 | 说明 |
|------|------|------|
| `group_id` | int | 群虚拟 ID |
| `user_id` | int | 申请者虚拟 ID |
| `flag` | string | 申请 ID（`join_request_id`），审批时回传 |
| `username` | string | 申请者昵称 |
| `apply_at` | string | 申请时间戳（RFC3339，官方字段原样透传；**旧版本此处为秒级时间戳 int**） |
| `apply_source` | string | 申请来源：`self_apply` 主动申请 / `invited` 被邀请 |
| `invited_by` | string | 邀请人 OpenID（`apply_source=invited` 时有效，无邀请人为空） |
| `union_openid` | string | 申请人 union_openid |
| `bot` | bool | 是否为机器人账号 |
| `risk_tips` | string | 安全提示语（可疑消息为 `warning_tips`；命中 `sec_risk_rules` 为 `top_tips`） |
| `verify_info` | string | 验证信息文本（取自官方 `verify_info.verify_message`） |
| `verify_method` | string | 验证方式（官方 `verify_info.method` 原样透传，如 `admin_review_qa`） |
| `review_qa_list` | array | 入群验证问答列表（`verify_info.method=admin_review_qa` 时携带），元素为 `{question, answer}` |
| `auto_approved` | bool | 是否已被自动审批通过（官方列表项无此字段，由 `auto_approved` 对象是否存在推导） |

## 关联：入站 `request` 事件

`GROUP_JOIN_REQUEST` 上报的 OneBot `request` 事件（`request_type=group`、`sub_type=add`）与官方 2026-09 文档对齐，`auto_approved` 为**对象**（官方类型 `AutoAppproved`，仅下行事件携带）：

| 字段 | 类型 | 说明 |
|------|------|------|
| `auto_approved` | object \| null | 官方对象透传：`{"strategy_id": "..."}`；事件未携带时为 `null` |
| `auto_approved_bool` | bool | 布尔伴随字段：`auto_approved != nil`（事件携带自动审批对象时为 `true`），供按布尔语义判断的下游使用 |

> 旧版本 `auto_approved` 声明为 `bool`，与官方对象格式不符会导致整个事件解析失败；现改为对象透传，并额外提供 `auto_approved_bool`。

## 示例

```json
{
  "action": "get_group_join_request_list",
  "params": {
    "group_id": 870389197,
    "cursor": "",
    "limit": 20
  },
  "echo": "req-list-1"
}
```

响应 `data`：

```json
{
  "list": [
    {
      "group_id": 870389197,
      "user_id": 791838020,
      "flag": "AURi8Rr6MfGdUNedupWf2uV5XiayURHaetzwGyOdrj6mHYOsfJFkbe9u8UjCMpLTxUouwr1SJ9IGEbxlbzDi43hPS4rw64G4i2Y4nL4DTH50U15xKPZYRsXPB7WUxZOUdceNSAv_GJtO4ffSrVZIhQxknoPD2SDT",
      "username": "小明",
      "apply_at": "2026-08-05T14:19:09+08:00",
      "apply_source": "self_apply",
      "invited_by": "",
      "union_openid": "",
      "bot": false,
      "risk_tips": "",
      "verify_info": "我是来学习的",
      "verify_method": "",
      "review_qa_list": [],
      "auto_approved": false
    }
  ],
  "next_cursor": "1785767153250497",
  "join_requests": [
    {
      "group_id": 870389197,
      "user_id": 791838020,
      "flag": "AURi8Rr6MfGdUNedupWf2uV5XiayURHaetzwGyOdrj6mHYOsfJFkbe9u8UjCMpLTxUouwr1SJ9IGEbxlbzDi43hPS4rw64G4i2Y4nL4DTH50U15xKPZYRsXPB7WUxZOUdceNSAv_GJtO4ffSrVZIhQxknoPD2SDT",
      "username": "小明",
      "apply_at": "2026-08-05T14:19:09+08:00",
      "apply_source": "self_apply",
      "invited_by": "",
      "union_openid": "",
      "bot": false,
      "risk_tips": "",
      "verify_info": "我是来学习的",
      "verify_method": "",
      "review_qa_list": [],
      "auto_approved": false
    }
  ],
  "next_index": 1785767153250497
}
```

> `join_requests` 与 `list` 内容相同（兼容别名）；`next_index` 仅在 `next_cursor` 可解析为整数时给出，否则为 `0`。

## nonebot2 示例

```python
from nonebot import on_command
from nonebot.adapters.onebot.v11 import Bot, Event

@on_command("approve_all").handle()
async def _(bot: Bot, event: Event):
    # 拉取申请列表（分页：cursor 为空表示首页）
    cursor = ""
    total = 0
    while True:
        data = await bot.call_api("get_group_join_request_list",
                                  group_id=event.group_id, cursor=cursor, limit=50)
        for req in data["list"]:
            await bot.call_api("set_group_add_request",
                group_id=req["group_id"], user_id=req["user_id"],
                flag=req["flag"], approve=True)
            total += 1
        cursor = data["next_cursor"]
        if not cursor:
            break
    await event.finish(f"已通过 {total} 条申请")
```
