# Gensokyo-NewQQ 首轮重构基线

## 范围

- 仓库：`H:\Gensokyo-NewQQ`
- 分支：`refactor/modular-core`
- 基线提交：`4bbaf17332236f556baf4d6aa8beb04a1c82a0f7`
- 本轮只做基线、审计、现状架构和第一 PR 设计，不做大规模代码迁移。

## 工具链

- Go：`go1.26.4 windows/386`，环境中 `CGO_ENABLED=0`。
- Node：`v24.18.0`，npm：`11.16.0`。
- `ffmpeg` 可用；`gcc`、`govulncheck`、`upx` 未找到。
- 根模块声明 Go `1.25.0`，并包含本地 `botgo`、`go-silk` replace。

## 基线事实

- 根模块 `go list ./...`：27 个包。
- `go list -m all`：134 个模块条目。
- Go 文件：226 个，85,477 行，约 3.35 MB。
- 前端依赖安装成功；前端构建产物用于 `webui` 的 `go:embed`。
- 当前工作区没有已跟踪的用户修改；测试产生的数据库、词表、`dist`、`node_modules` 等均为 ignored 文件，保留不清理。

## 已知前置条件

首次直接执行 Go 构建/测试时，`webui/dist` 不存在，会触发 `webui/api.go` 的 `go:embed dist/*` 编译错误；按现有 `build.ps1` 的占位目录逻辑准备静态目录后，根模块构建、vet、普通测试均可继续。

## 结论

当前代码可以在准备 `webui/dist` 后完成根模块静态构建，但前端测试脚本没有实际测试，386 环境无法运行 Go race，安全扫描工具缺失，且已有凭据、HTTP 资源边界、消息序号非原子更新等高优先级审计项需要在独立 PR 处理。
