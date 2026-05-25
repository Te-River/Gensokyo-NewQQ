# API: get_avatar

获取用户头像。

## 返回值

```json
{
    "data": {
        "message": "https://q.qlogo.cn/qqapp/102848039/[QQ官方虚拟ID]/640",
        "user_id": 1234567890  #此处的1234567890为Gsk向后端输出的虚拟用户ID
    },
    "message": "",
    "retcode": 0,
    "status": "ok",
    "echo": null
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `data.message` | string | 头像直链 URL |
| `data.user_id` | int64 | 虚拟用户 ID |

## 所需字段

- **group_id**: 群号（当获取群成员头像时需要）
- **user_id**: 用户 QQ 号（当获取私信头像时需要）

## CQcode

CQ头像码格式.支持message segment式传参,将at segment类比修改为avatar即可.
[CQ:avatar,qq=123456]
