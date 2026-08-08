# 统一图床/OSS 服务

本包是 `oss_type` 的后端实现，**不再由用户同时启用多个图床**。具体使用哪个后端由配置项 `oss_type` 决定：

| oss_type | 后端 | 说明 |
|----------|------|------|
| 0 | 本机 | 上传到 Gensokyo 本地 HTTP 服务器（默认） |
| 1 | 腾讯云 COS | 旧 `t_COS_*` 字段，走 `oss/tencent.go` |
| 2 | 百度云 BOS | `b_BOS_*` 字段，走 `oss/baidu.go` |
| 3 | 阿里云 OSS | `a_OSS_*` 字段，走 `oss/aliyun.go` |
| 4 | COS 自签 | `cos.*` 字段，走本包 `cos.go` |
| 5 | Bilibili | `bilibili.*` 字段，走本包 `bilibili.go` |
| 6 | QQ频道 | `qq_channel.*` 字段，走本包 `qq_channel.go` |
| 7 | ChatGLM | 免费，开箱即用，走本包 `chatglm.go` |
| 8 | Ukaka | 免费，开箱即用，走本包 `signed.go` |
| 9 | 星野 | 免费，开箱即用，走本包 `signed.go` |
| 10 | Nature | 免费，密钥内置，走本包 `nature.go` |

> **注意：** `oss_type` 仅控制图片上传路径；语音上传不受此选项影响（仍走本机或 1~3 云OSS）。

## 配置 (config.yml)

以下仅展示 oss_type 及图床凭证相关字段在 `config.yml` 中的实际位置。更多完整配置请参考 `readme.md`。

```yaml
# oss_type 选择后端（位于 settings 的"云存储/图床"区域）
oss_type: 0  # 0=本机 1=腾讯云COS 2=百度云BOS ... 10=Nature

# 腾讯云配置 — cos: 紧贴在此
t_COS_BUCKETNAME : ""
t_COS_SECRETID : ""
...
cos:                            # 腾讯云COS自签（oss_type=4）
  secret_id: ""                 # 腾讯云 API SecretId
  secret_key: ""                # 腾讯云 API SecretKey
  region: "ap-guangzhou"        # 存储桶地域
  bucket: ""                    # 存储桶名称
  domain: ""                    # 自定义域名（留空使用COS默认域名）

# 阿里云配置 — bilibili/qq_channel/免费图床紧贴在此
a_OSS_EndPoint : ""
...
bilibili:                       # B站图床（oss_type=5）
  csrf_token: ""                # B站bili_jct
  sessdata: ""                  # B站SESSDATA
  bucket: "openplatform"
qq_channel:                     # QQ频道图床（oss_type=6）
  channel_id: ""
  token: ""                     # Authorization值，如 "QQBot xxx.yyy"
chatglm:                        # 智谱免费图床（oss_type=7，开箱即用）
ukaka:                          # Ukaka免费图床（oss_type=8，开箱即用）
xingye:                         # 星野免费图床（oss_type=9，开箱即用）
nature:                         # Nature腾讯COS直传（oss_type=10，密钥内置）
```

## 集成点

- `images/upload_api.go` 中的 `UploadBase64ImageToServer` 根据 `oss_type` 分发到本包对应后端
- `handlers/message_parser.go` 中的 `ResolveMarkdownImages` 受益于图床获取公开 URL

## 代码结构

```
imagehosting/
├── hosting.go       # 统一接口 + 调度器 + 辅助函数
├── cos.go           # 腾讯云 COS (HMAC 自签)
├── bilibili.go      # B站图床
├── qq_channel.go    # QQ频道图床
├── chatglm.go       # 智谱免费图床
├── signed.go        # Ukaka + 星野 (签名上传)
├── nature.go        # Nature 腾讯 COS 直传 (密钥内置)
├── utils.go         # 辅助函数
└── README.md        # 本文档
```