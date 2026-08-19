# 媒体链路审计

## 当前链路

```text
OneBot segment/CQ
 → foundItems 字符串值
 → local/url/base64 归一化
 → 下载或 Base64 decode 得到 []byte
 → 图片压缩 / 语音 Silk / 分片上传 / OSS 或 imagehosting
 → QQ API DTO
```

`images/upload_api.go` 的公开 helper 存在 10 参数函数；`imagehosting/hosting.go` 同时负责 provider 选择、Base64 解码和上传；`handlers/upload_helper.go` 负责分片上传；`silk/silk.go` 负责编码。群聊、私聊、raw、reply、wakeup 多条路径重复处理相同媒体语义。

## 统计与热点

- 根业务范围内约 53 处 Base64 编解码、521 处 `[]byte`、30 处 `io.ReadAll`、124 处 Upload 相关符号、61 处 Silk 相关符号。
- `handlers/reply_helpers.go:112-139` 下载图片后整包读取并重新 Base64 编码。
- `send_group_msg.go` 在多个 local/url/base64 分支中整包读取、解码、重新编码，存在重复内存峰值。
- 多个上传 provider helper 直接接收 `[]byte`，没有统一的大小、MIME、清理和取消策略。

## 已确认问题

1. 没有统一的 `MediaRef`/`MediaObject`/`UploadResult` 类型，媒体来源和目标依赖 key 名称及调用约定。
2. 缺少流式或 spool-to-disk 抽象，URL/文件/Base64 都容易先进入整块内存。
3. 下载路径中存在无上限 `io.ReadAll` 和不一致的 HTTP status/Content-Length 校验。
4. group/private 两套生成器重复处理，修复安全边界需要同步多个位置。

## 目标模型

定义 `MediaRef{Source, Locator, DeclaredType, SizeHint}`、`MediaReader`、`MediaTransformer`、`MediaUploader` 和容量策略；来源校验、下载、转码、上传分别有接口和上下文。对大文件默认 spool/stream，对 Base64 设置 decode 上限，输出只返回 typed media handle。

## 迁移顺序

先把现有 `foundItems` 转换为 typed media plan，再让 group/private 共享 planner；然后集中 HTTP 下载和限额；最后替换 provider helper。保持旧 CQ/segment 输入兼容。
