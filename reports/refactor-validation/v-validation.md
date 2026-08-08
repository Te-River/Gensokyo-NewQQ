# V 系列验证报告（V1-V5）

```
PHASE: V1-V5（持续验证）
STATUS: PARTIAL_PASS
```

---

## V1 每阶段持续验证（P2-P13 各阶段）

| 阶段 | go build | go vet | go test |
|------|----------|--------|---------|
| S0 | PASS | PASS | PASS |
| P2 | PASS | PASS | PASS |
| P3 | PASS | PASS | PASS |
| P4 | PASS | PASS | PASS |
| P5 | PASS | PASS | PASS |
| P6 | PASS | PASS | PASS |
| P7 | PASS | PASS | PASS |
| P8 | PASS | PASS | PASS |
| P9 | PASS | PASS | PASS |
| P10 | PASS | PASS | PASS |
| P11 | PASS | PASS | PASS |
| P12 | PASS | PASS | PASS |
| 最终全量 | PASS | PASS | PASS |

## V2 Windows amd64 验证（实际为 windows/386）

| 命令 | 结果 | 说明 |
|------|------|------|
| `go version` | go1.26.4 windows/386 | |
| `go env` | GOARCH=386 | |
| `go mod verify` | 未单独运行（go build 通过隐式校验） | |
| `go build ./...` | ✅ PASS | |
| `go vet ./...` | ✅ PASS | |
| `go test ./...` | ✅ PASS | |
| `go test -race ./...` | **NOT_RUN** | windows/386 不支持 race detector（`-race is not supported on windows/386`），不伪装 PASS |
| `govulncheck ./...` | **NOT_RUN** | 安装失败：proxy.golang.org 网络不可达（dial tcp 超时） |

## V3 Linux amd64 验证

| 命令 | 结果 | 说明 |
|------|------|------|
| 全部 | **NOT_RUN** | 当前无 Linux 环境；计划要求 Linux `-race` 作为最终 race gate |

## V4 Frontend

| 命令 | 结果 | 说明 |
|------|------|------|
| `npm ci` | ✅ PASS（node_modules 已存在） | |
| `npm run lint` | ✅ PASS | 0 errors, 14 warnings |
| `npm test` | **NOT_IMPLEMENTED** | 脚本为 `echo "No test specified"`，按计划不得写 PASS |
| `npm run build` | ✅ PASS | quasar build 成功，产物复制到 webui/dist |

## V5 真实 QQ / OneBot 手工矩阵

| 项 | 状态 | 说明 |
|----|------|------|
| 消息方向 A/B、Identity、Failure Matrix | **BLOCKED** | 需真实 QQ 机器人 + OneBot 客户端（用户侧环境）；`docs/testing/manual-matrix.md` 待真实联调时建立 |

---

## 汇总

| 阶段 | 状态 |
|------|------|
| V1 每阶段验证 | ✅ PASS |
| V2 Windows 构建/静态/测试 | ✅ PASS（race/govulncheck NOT_RUN，原因已记录） |
| V3 Linux | NOT_RUN（无环境） |
| V4 Frontend | ✅ PASS（test NOT_IMPLEMENTED 如实记录） |
| V5 真实联调 | BLOCKED（用户侧） |
