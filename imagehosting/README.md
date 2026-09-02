# 统一图床/OSS 服务

图片图床由 `settings.oss_type` 单选。`0~3` 使用原有本机或云 OSS，`4~10` 使用本包的后端；不会同时轮询多个图床。

| oss_type | 后端 | 说明 |
|---:|---|---|
| 0 | 本机 | Gensokyo 本地 HTTP 上传（默认） |
| 1 | 腾讯云 COS | 原有 `t_COS_*` 配置 |
| 2 | 百度云 BOS | 原有 `b_BOS_*` 配置 |
| 3 | 阿里云 OSS | 原有 `a_OSS_*` 配置 |
| 4 | COS 自签 | `cos.*`，需要自行配置凭据 |
| 5 | Bilibili | `bilibili.*`，需要 Cookie |
| 6 | QQ频道 | `qq_channel.*`，需要频道 ID 和 Authorization |
| 7 | ChatGLM | 第三方免配置服务，需显式允许 |
| 8 | Ukaka | 第三方签名服务，需显式允许 |
| 9 | 星野 | 第三方签名服务，需显式允许 |
| 10 | Nature | 已禁用，不再使用公开源码中的内置凭据 |

## 配置示例

```yaml
oss_type: 0
cos:
  secret_id: ""
  secret_key: ""
  region: "ap-guangzhou"
  bucket: ""
  domain: ""
bilibili:
  csrf_token: ""
  sessdata: ""
  bucket: "openplatform"
qq_channel:
  channel_id: ""
  token: ""
```

ChatGLM、Ukaka、星野需要管理员明确接受图片上传到第三方服务后设置：

```text
GENSOKYO_ENABLE_THIRD_PARTY_IMAGE_HOSTS=1
```

## 安全限制

- 单张图片最大 10 MiB，最多 4000 万像素。
- 只接受 PNG、JPEG、GIF、WebP，并检查可解析性。
- 文件名会移除路径穿越和控制字符。
- 图床 HTTP 请求总超时 15 秒，响应体最多读取 1 MiB。
- 只允许 HTTPS 外部 URL，拒绝本机、私有网段和不安全重定向。
- 已出现在 Git 历史或公开页面中的凭据无法通过一次提交恢复保密性，应立即撤销或轮换。

## 集成点

- `images/upload_api.go` 根据 `oss_type` 调用 `UploadProvider`。
- `handlers/message_parser.go` 可使用图床获取公开 URL。

## 目录

```text
imagehosting/
├── hosting.go       # provider 调度、校验和 HTTP 辅助函数
├── cos.go           # 腾讯云 COS 自签
├── bilibili.go      # B站图床
├── qq_channel.go    # QQ频道图床
├── chatglm.go       # ChatGLM
├── signed.go        # Ukaka + 星野
├── nature.go        # 已禁用的 Nature 后端
└── README.md
```

