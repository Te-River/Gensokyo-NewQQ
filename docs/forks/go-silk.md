# go-silk Fork Inventory

> 本地 fork 依赖：`go.mod` 中 `replace github.com/wdvxdr1123/go-silk => ./go-silk`

## 基本信息

| 项 | 值 |
|----|----|
| upstream | https://github.com/wdvxdr1123/go-silk |
| 本地 module | `github.com/wdvxdr1123/go-silk`（路径 `./go-silk`） |
| 用途 | Silk 音频编码 SDK（QQ 语音消息格式） |
| 引用方式 | go.mod `replace`（本地目录，非发布版本） |
| 隔离边界 | 只允许 `media/audio adapter` import；其余模块调用 `AudioTranscoder`（P5） |

## 为什么需要 fork

- 需要针对当前构建/平台条件的本地调整。
- `silk/` 目录通过 `go:embed exec/*` 嵌入编译产物，`mp3_real.go`/`mp3_stub.go` 由
  `//go:build !small`/`small` 切换，可能与上游默认构建不同。

## 本地 patch 内容（已知）

- 构建产物嵌入与 platform 相关调整。

> ⚠️ 完整本地 patch 清单需维护者补齐（相对 upstream 的 diff）。

## 未来同步方式

- 定期 `git fetch upstream` 对比；仅合并稳定、无破坏的改动。
- 保持 `AudioTranscoder` 边界，避免业务层直接触碰 go-silk。

## 参考

- `go-silk/README.md`
