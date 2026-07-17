# 统一图床服务

统一图床模块会按配置顺序尝试已启用后端，并返回第一个成功的公开图片 URL。

## 安全默认值

- Nature 后端已禁用。上游版本曾在公开源码中内置对象存储访问凭据，公开凭据不能继续作为秘密使用。
- ChatGLM、Ukaka、星野属于第三方免配置服务。即使旧配置中仍为 `enabled: true`，默认也不会上传。
- 明确接受图片会离开当前服务器并上传到第三方后，才可设置环境变量：

```text
GENSOKYO_ENABLE_THIRD_PARTY_IMAGE_HOSTS=1
```

推荐优先使用自行控制的 COS 或 QQ 频道后端。

## 配置示例

```yaml
image_hosting:
  cos:
    enabled: false
    secret_id: ""
    secret_key: ""
    region: "ap-guangzhou"
    bucket: ""
    domain: ""
  bilibili:
    enabled: false
    csrf_token: ""
    sessdata: ""
    bucket: "openplatform"
  qq_channel:
    enabled: false
    channel_id: ""
    token: ""
  chatglm:
    enabled: false
  ukaka:
    enabled: false
  xingye:
    enabled: false
  nature:
    enabled: false
```

`nature.enabled` 已不再生效，仅为兼容旧配置保留字段。

## 上传限制

统一入口当前执行以下检查：

- 单张图片最大 10 MiB
- JPEG、PNG、GIF、WebP 文件头检查
- PNG、JPEG、GIF 实际解码检查
- 最大 4000 万像素
- 文件名路径和控制字符清理
- HTTP 请求总超时 15 秒
- 第三方响应体最大读取 1 MiB

无法识别或损坏的数据不会再默认按 JPEG 上传。

## 后端说明

| 优先级 | 后端 | 默认状态 | 说明 |
|---|---|---|---|
| 1 | COS | 关闭 | 需要自行配置访问凭据和存储桶 |
| 2 | Bilibili | 关闭 | 需要用户 Cookie，注意账号安全和平台规则 |
| 3 | QQ频道 | 关闭 | 需要频道 ID 和机器人 Authorization |
| 4 | ChatGLM | 关闭 | 还需要显式启用第三方图床环境变量 |
| 5 | Ukaka | 关闭 | 还需要显式启用第三方图床环境变量 |
| 6 | 星野 | 关闭 | 还需要显式启用第三方图床环境变量 |
| - | Nature | 禁用 | 不再包含或使用上游公开凭据 |

## 集成点

- `images/upload_api.go` 中的 `UploadBase64ImageToServer` 会优先尝试图床链，失败后回退传统模式。
- `handlers/message_parser.go` 中的 `ResolveMarkdownImages` 可使用图床链获取公开 URL。

## 运维提醒

已经出现在 Git 历史或公开页面中的凭据无法通过一次代码提交恢复保密性。对应凭据的所有者应立即在云平台撤销或轮换密钥，并检查访问日志与账单。
