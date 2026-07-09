# Changelog — Release007 (since Release006)

> 自 Release006 (`7ec67db`) 以来的所有变更。

---

## 🐛 Bug 修复

### 语音/图片本地图床上传失败（内网 server_dir 场景）

**问题：** 当 `server_dir` 设为内网地址（`127.0.0.1` / `192.168.x.x`）时，语音和图片上传到本地图床后，QQ CDN 拉取不到文件，导致"local server uses a private address"错误，媒体消息发送失败。

**根因：** `send_group_msg.go` 中所有语音和图片上传都走了一条统一的路径：base64 → 上传到 Gensokyo 本地 HTTP 服务器 → 拿到 URL → QQ CDN 拉取。该路径要求 `server_dir` 是公网可访问的地址。

**修复：** 语音和图片（非 Markdown）统一改为直接 base64 上传到 QQ CDN，跳过本地图床中转。

```
修复前: base64 → 本地图床 → 公网URL → QQ CDN 拉取 ✅(公网) ❌(内网)
修复后: base64 → 直接提交 QQ CDN API 上传 ✅(任何网络)
```

仅 Markdown 消息中的图片/视频保留本地图床路径（Markdown 内容需要嵌入公开 URL）。

**涉及修改：**
- `send_group_msg.go` — 移除全部 `GetUploadPicV2Base64()` 条件分支和 `UploadBase64ImageToServer`/`UploadBase64RecordToServer` 调用
- 群聊 `base64_image`、`url_image`、`url_images` → `CreateAndUploadMediaMessage`
- 群聊 `base64_record`、`local_record`、`url_record` → `CreateAndUploadMediaMessage`
- C2C 私聊同路径 → `CreateAndUploadMediaMessagePrivate`
- 共约 -350 行冗余代码

### MP3 ID3 标签头未识别

- `detectAudioFormat` 未识别以 `ID3` 标签头开头的 MP3 文件，导致语音转码失败
- 修复：新增 ID3v2 头检测，正确识别为 MP3 格式，走 `go-mp3` 纯 Go 解码器

---

## 🎨 WebUI 视觉重构 (PR #9)

- **颜色体系更新** — 去掉紫色渐变，改用更克制、适合运维面板的蓝/青/紫色系
- **Health strip** — 仪表盘顶部新增状态条，关键指标一目了然
- **布局打磨** — 卡片 hover 效果、响应式布局、阴影去除
- **Login 页面精简** — 减少装饰性元素，更聚焦
- **LogsConsole 重连修复** — 修复 reconnect 事件未正确绑定的 bug
- **clean checkout 兼容** — 新增 `webui/dist` 占位文件确保 `go:embed` 在未构建前端时也能通过编译
- **`go vet` 兼容** — 修复 `fmt.Errorf` 非恒定格式字符串，通过 Go 1.26+ 校验

---
## 🔧 工程化改进

- `.gitignore` 修复编码错误，添加 `webui/dist` 占位文件忽略规则
- 停止跟踪 `webui/dist` 占位文件，由 `build.ps1 Ensure-WebUIDist` 自动创建
- 删除已失效的 `GetUploadPicV2Base64()` 死代码
- 清理 `base64_record` 冗余双层检查
- 统一 `url_record`/`url_records` 变量命名（`imageURLs` → `recordURLs`）

---

## 📝 文档 & TypeScript

- 补全 `SystemMemoryDetail` 接口的 `used` 字段类型声明

---
## 🔧 工程化改进

- `.gitignore` 修复编码错误，添加 `webui/dist` 占位文件忽略规则
- 停止跟踪 `webui/dist` 占位文件，由 `build.ps1 Ensure-WebUIDist` 自动创建

---

## ✅ 提交记录

```
6a59179  语音统一直接base64上传QQ CDN
b68957c  图片统一直接base64上传QQ CDN，仅Markdown用图床
7866c0b  detectAudioFormat 识别ID3标签头的MP3文件
00d325b  PR #9: fix tests and refine webui
576bc9f  修复.gitignore编码错误
2983b4b  停止跟踪webui/dist占位文件
6a56b73  最终清理webui/dist跟踪状态
9e15f6b  清理冗余代码，统一变量命名
```
