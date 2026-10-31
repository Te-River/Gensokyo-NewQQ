# API: 素材管理（media-images）

> 官方文档：素材管理页（页脚 2026-10-21）。Gensokyo 扩展 API，三个 action 共用同一条上传链路：`prepare` → 预签名 PUT → `complete`。

素材管理用于把图片上传到 QQ 官方素材库，换取 `qqbot-media://image/v1/<token>` 引用。该引用可直接写进 Markdown 图片语法与 keyboard 按钮图片字段，Gensokyo **原样透传**（不下载、不转存）。

| Action | 说明 |
|--------|------|
| [`upload_media_image`](#upload_media_image) | 上传单张素材图片，返回 `image_id` 与引用地址。 |
| [`get_media_image_list`](#get_media_image_list) | 分页查询当前机器人的素材图片。 |
| [`delete_media_image`](#delete_media_image) | 删除素材图片，释放配额。 |

## 通用限制

| 项目 | 限制 |
|------|------|
| 单图大小 | ≤ 10MB（按十进制 **10,000,000 字节**） |
| 图片格式 | `png` / `jpeg`（`jpg`）/ `gif` / `webp`，扩展名与 `content_type` 必须一致 |
| 配额 | 单机器人最多 **100 张** |
| 有效期 | 自最后一次被消息引用起 **90 天**滑动过期（`ttl=7776000` 秒） |
| 预签名上传任务 | `upload_id` 与预签名 URL 均为 4 小时有效 |

`file` 入参支持以下形式，**不支持 `http(s)://` 链接**（上传前需在本地计算 md5 与 sha1，远端内容无法在不下载的前提下满足）：

- 本地路径（如 `D:/images/cover.png`）或 `file://` 前缀；
- `base64://<裸 base64>`；
- `data:image/png;base64,<...>`（也支持百分号编码的 data URI）。

## upload_media_image

### 请求

```json
{
    "action": "upload_media_image",
    "params": {
        "file": "D:/images/cover.png",
        "file_name": "cover.png",
        "content_type": "image/png"
    }
}
```

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `file` | string | **是** | 图片内容：本地路径 / `file://` / `base64://` / `data:`，不支持 `http(s)://`。 |
| `file_name` | string | 否 | 文件名（含扩展名）。留空时取本地路径的文件名；仍为空则生成 `media_image_<Unix 秒>`。 |
| `content_type` | string | 否 | `image/png` / `image/jpeg` / `image/gif` / `image/webp`。留空时按扩展名推导；显式传入时扩展名必须与其同族（`jpg` 与 `jpeg` 视为同族）。 |

### 返回值

```json
{
    "data": {
        "image_id": "img_xxxxxxxx",
        "url": "qqbot-media://image/v1/<token>",
        "ttl": 7776000,
        "preview_url": "https://cos.example/preview?sig=...",
        "file_name": "cover.png",
        "expires_in": 14400,
        "expires_at": 1798761600
    },
    "message": "",
    "retcode": 0,
    "status": "ok",
    "echo": null
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `data.image_id` | string | 素材图片 ID，删除时使用。 |
| `data.url` | string | 引用地址，格式 `qqbot-media://image/v1/<token>`，可直接写进 Markdown / keyboard。 |
| `data.ttl` | uint32 | 滑动有效期（秒），官方值 `7776000`（90 天）。 |
| `data.preview_url` | string | COS 预签名短链，**仅供即时预览，禁止持久化**。 |
| `data.file_name` | string | 实际提交的文件名。 |
| `data.expires_in` | uint32 | 预签名上传 URL 有效期（秒，官方 4 小时）。 |
| `data.expires_at` | int64 | 失效时间（Unix 秒）；上游未返回时省略。 |

## get_media_image_list

### 请求

```json
{
    "action": "get_media_image_list",
    "params": {
        "cursor": ""
    }
}
```

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `cursor` | string | 否 | 分页游标，首页留空；后续取上一页响应中的 `next_cursor`。 |

### 返回值

```json
{
    "data": {
        "images": [
            {
                "image_id": "img_xxxxxxxx",
                "url": "qqbot-media://image/v1/<token>",
                "file_name": "cover.png",
                "preview_url": "https://cos.example/preview?sig=...",
                "created_at": 1796169600,
                "expires_at": 1798761600
            }
        ],
        "next_cursor": "",
        "total": 1,
        "next": false
    },
    "message": "",
    "retcode": 0,
    "status": "ok",
    "echo": null
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `data.images[].image_id` | string | 素材图片 ID。 |
| `data.images[].url` | string | `qqbot-media://` 引用地址。 |
| `data.images[].file_name` | string | 文件名。 |
| `data.images[].preview_url` | string | 预签名预览短链，禁止持久化。 |
| `data.images[].created_at` | int64 | 创建时间（Unix 秒）。 |
| `data.images[].expires_at` | int64 | 失效时间（Unix 秒，每次被消息引用后顺延 90 天）。 |
| `data.next_cursor` | string | 下一页游标，空串表示没有更多。 |
| `data.total` | int | 当前机器人素材总数（与 100 张上限同口径）。 |
| `data.next` | bool | `next_cursor` 非空时为 `true`。 |

## delete_media_image

### 请求

```json
{
    "action": "delete_media_image",
    "params": {
        "image_id": "img_xxxxxxxx"
    }
}
```

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `image_id` | string | **是** | 要删除的素材图片 ID。 |

### 返回值

```json
{
    "data": {},
    "message": "",
    "retcode": 0,
    "status": "ok",
    "echo": null
}
```

删除成功后引用链接立即失效，**已发送的消息不受影响**；配额同步释放，可继续上传。

## 错误码

| retcode | 场景 |
|---------|------|
| `0` | 成功。 |
| `1400` | 参数或本地校验失败：`file` 为空、`http(s)` 链接、base64/data URI 解码失败、文件名无法推导 `content_type`、扩展名与 `content_type` 不一致、超过 10MB、`image_id` 为空。 |
| `1404` | 本地文件不可读（不存在、是目录、读取失败）。 |
| `1401` | 上游或网络失败（prepare / PUT / complete / list / delete 出错、OpenAPI v2 未初始化）。 |

上游错误会以「中文说明(官方码)」的形式出现在 `message` 中，官方码原值保留，便于调用方自行判定：

| 官方码 | 说明 |
|--------|------|
| `40035001` | 图片格式不支持（扩展名与 `Content-Type` 须为 png/jpeg/gif/webp 且一致）。 |
| `40035002` | 图片超过大小上限（单图不超过 10MB）。 |
| `40035003` | 上传任务不存在、已过期（超过 4 小时）或不属于当前机器人。 |
| `40035006` | 服务端内部错误（COS 处理或链接生成失败），可稍后重试。 |
| `40035007` | 图片数量超过上限（单机器人 100 张），请删除后重试。 |
| `40035008` | 图片不存在或已被删除。 |
| `40035009` | 图片已过期或已被删除，引用链接不再可用。 |
| `40035010` | 引用链接无效，或不属于当前机器人。 |

## 引用方式

Markdown 图片（尺寸语法可保留）：

```markdown
![说明 #100px #100px](qqbot-media://image/v1/<token>)
```

keyboard 按钮图片字段同样接受该引用。两种场景都由 `ResolveMarkdownImages` / `keyboardMediaResolver` 识别为 `qqbot-media://` 并原样透传——令牌与机器人绑定，本地无从校验，下载改写还会破坏官方尺寸语法。

## nonebot2 示例

```python
import base64

from nonebot import on_command
from nonebot.adapters.onebot.v11 import Bot, Event

upload = on_command("上传素材")

@upload.handle()
async def _(bot: Bot, event: Event):
    result = await bot.call_api(
        "upload_media_image",
        file="D:/images/cover.png",
        file_name="cover.png",
    )
    data = result["data"]
    # data["url"] 形如 qqbot-media://image/v1/<token>
    md = f'![封面 #200px #120px]({data["url"]})\n素材已上传（image_id={data["image_id"]}）'
    payload = base64.b64encode(md.encode("utf-8")).decode()
    await bot.call_api(
        "send_group_msg",
        group_id=event.group_id,
        message=f"[CQ:markdown,data=base64://{payload}]",
    )
```

## 注意

- `file` 不接受 `http(s)://`，需要远端图片时请先下载到本地或转成 base64。
- `preview_url` 是带签名的临时链接，不要落库或转发给用户长期使用；长期引用请用 `url`。
- 素材引用只能用于图片类字段（Markdown 图片、keyboard 按钮图片）；卡片 `pic` 字段仍要求 `https://`。
- Go 链与 Rust 新链（`arch_mode=new`）均注册了这三个 action。
