# P5 — 媒体管线统一 验证报告

```
PHASE: P5（媒体管线统一）
STATUS: PASS
```

---

## 目标

```
Local / URL / Base64 / Bytes → MediaSource → MediaService → PreparedMedia
```

Handler 不再直接实现 HTTP 下载 / 图片校验 / FFmpeg / go-silk；媒体大小有硬上限，
临时文件生命周期可预测。

---

## 实现

### 新增 `internal/application/media/`

| 文件 | 职责 |
|------|------|
| `policy.go` | `MediaPolicy` 硬上限（MaxEncodedBytes / MaxDecodedBytes / MaxBytes / 图片尺寸像素 / AllowedDirs / AllowedExtensions） |
| `prepared.go` | `PreparedMedia`（内存或临时文件）+ `Close()` 资源清理（幂等） |
| `service.go` | `MediaService.Prepare(ctx, source, policy)` 统一入口，按来源分发 |
| `fetcher.go` | `SafeHTTPFetcher`：timeout / max bytes / 重定向限制 / **SSRF 检查** / 状态码 / Content-Type / 文件签名；大媒体流式落临时文件 |
| `base64.go` | decode 前限制编码长度（防 OOM），decode 后限制解码大小 |
| `image.go` | `ValidateImage`：decode → 尺寸/像素校验（防解压炸弹） |
| `uploader.go` | `MediaUploader` 接口（图床/COS/OSS 统一）+ `AudioTranscoder` 边界（FFmpeg/go-silk 封装点） |

### 关键安全点

- **SSRF**：默认拒绝 loopback/私网/链路本地地址（`AllowPrivate` 仅测试/受控内网开启）。
- **Base64 OOM**：`MaxEncodedBytes` 检查在 decode 之前。
- **解压炸弹**：图片 `MaxWidth/MaxHeight/MaxPixels` 校验。
- **任意文件读取**：本地文件 `AllowedDirs` 前缀 + 扩展名白名单 + regular file + 大小校验。
- **临时文件生命周期**：`PreparedMedia.Close()` 删除临时文件，幂等；测试覆盖。
- **大媒体内存**：超过 1MB 的 URL 内容流式写入临时文件（避免 `ReadAll → Base64` 重复大缓冲）。

---

## 验收清单

| 验收项 | 状态 |
|--------|------|
| Handler 不再直接实现 HTTP 下载 | ✅ 基础设施已建（fetcher），接入属 P13 |
| Handler 不再直接做图片压缩 | ✅ `ValidateImage` 已建，接入属 P13 |
| Handler 不再直接调用 FFmpeg/go-silk | ✅ `AudioTranscoder` 边界已建 |
| 所有媒体大小存在硬上限 | ✅ PASS（MediaPolicy） |
| 临时文件生命周期有测试 | ✅ PASS |

---

## 变更文件

- 新增：`internal/application/media/{policy,prepared,service,fetcher,base64,image,uploader}.go`
- 新增：`internal/application/media/{fetcher,media}_test.go`

---

## 验证结果

| 命令 | 结果 |
|------|------|
| `go build ./...` | ✅ PASS |
| `go vet ./...` | ✅ PASS |
| `go test ./internal/application/media/` | ✅ PASS（69.6% coverage） |
| `go test ./...` | ✅ PASS |
| `go test -race ./...` | NOT_RUN（计划 V2/V3） |
| `govulncheck ./...` | NOT_RUN（计划 V2/V3） |
| frontend | NOT_RUN（不涉及） |

---

## KNOWN_LIMITATIONS

- `AudioTranscoder`/`MediaUploader` 为接口边界，具体云 SDK / FFmpeg / go-silk 实现未在本阶段接入。
- `ValidateImage` 只做校验不压缩；压缩策略由调用方（P6/P13）决定。
- SSRF 检查基于 DNS 解析结果，存在 DNS rebinding 的残余窗口（受控内网用 `AllowPrivate` 显式开启）。

---

## LEGACY_REMAINING

- `handlers/send_group_msg.go` / `send_private_msg.go` 等仍直接做 HTTP 下载 / 图片处理 / FFmpeg（P13 收敛）。

---

## NEXT_PHASE

- P6 出站消息模型（`OutboundService`，消费 `ParsedMessage` + `ResolvedTarget`）。
