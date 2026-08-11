# 构建与测试状态

## 根 Go 模块

| 检查 | 状态 | 证据 |
|---|---|---|
| `go mod download` | PASS | 退出码 0 |
| `go mod verify` | PASS | `all modules verified` |
| 直接 `go build ./...` | BLOCKED | `webui/api.go: pattern dist/*: no matching files found` |
| 准备 ignored `webui/dist` 占位目录后 `go build ./...` | PASS | 退出码 0 |
| 准备占位目录后 `go vet ./...` | PASS | 退出码 0 |
| 准备占位目录后 `go test ./...` | PASS | 根模块及 `Processor`、`handlers` 测试通过 |
| `go test ./... -cover` | FAIL | `handlers/send_group_msg.go: invalid BOM in the middle of the file` |
| `go test -race ./...` | NOT RUN | `-race is not supported on windows/386` |

## 前端

| 检查 | 状态 | 证据 |
|---|---|---|
| `npm ci` | PASS WITH WARNINGS | 安装 1077 个包；审计报告 53 个漏洞，其中 3 个 critical |
| `npm run lint` | PASS WITH WARNINGS | 0 errors，14 warnings |
| `npm test` | NOT_IMPLEMENTED | 脚本仅输出 `No test specified` 并退出 0 |
| `npm run build` | PASS WITH WARNINGS | Quasar 构建成功，并生成 `frontend/dist`、`webui/dist` |

## 嵌套模块

- `botgo` 的 `go test ./...`：FAIL/UNVERIFIED；多个依赖无法从 `proxy.golang.org` 下载，部分包还提示需要更新 `go.mod`。
- `go-silk`：FAIL/UNVERIFIED；依赖下载失败，并提示需要 `go mod tidy`。
- `botgo/examples`：FAIL/UNVERIFIED；同样受依赖下载与只读 go.mod 约束。
- Redis 锁测试未真正运行；其依赖准备阶段失败，且测试需要本机 `localhost:6379`。

## 测试覆盖边界

根仓库存在单元测试，但前端没有真实测试用例；仓库文本范围内未发现 benchmark 或 fuzz 函数。`govulncheck` 未安装，因此漏洞扫描为 NOT RUN。
