# 风险登记表

| ID | 等级 | 状态 | 风险 | 证据/范围 | 处置 |
|---|---|---|---|---|---|
| R-001 | P0 | CONFIRMED | 源码含硬编码 Base64 云凭据 | `imagehosting/nature.go:23-24` | 独立凭据清理/轮换 PR；本轮不轮换 |
| R-002 | P0 | CONFIRMED | msgseq Get/Add 非原子组合 | `echo/echo.go:242-280` 与多处调用方 | 统一 `Next` 并发接口，amd64 race 验证 |
| R-003 | P1 | CONFIRMED | HTTP client/response body 边界分散 | 多处 DefaultClient/http.Get/io.ReadAll | 统一 HTTP adapter、超时、body 上限、状态码 |
| R-004 | P1 | CONFIRMED | server/webhook 缺少 body/header/timeout 限制 | `main.go`、`server/webhook.go` | 先加入口限制和 context |
| R-005 | P1 | CONFIRMED | build.ps1 构建时 tidy 改变依赖 | `build.ps1` | 构建与依赖整理分离，锁定 Go/toolchain |
| R-006 | P1 | CONFIRMED | `foundItems` 隐式 map 协议 | parser/sender 多处读写 | typed ParsedMessage 兼容迁移 |
| R-007 | P1 | CONFIRMED | ID 长度启发式代替身份类型 | send group/private 与 idmap | typed Identity + 兼容解析 |
| R-008 | P1 | LIKELY | echo stack 清理与 Push/Pop 竞争 | `echo/echo.go` 清理路径 | amd64 race + 生命周期测试 |
| R-009 | P1 | LIKELY | WS send/close 竞争及假成功 | `wsclient/ws.go` | 单写协程、关闭协议、发送结果 |
| R-010 | P1 | LIKELY | 配置直接写入导致截断/自重载 | `config/config.go`、watcher | atomic write + debounce |
| R-011 | P1 | CONFIRMED | 前端无真实测试，npm audit 有漏洞 | `frontend/package.json`、npm ci 输出 | 增加测试门槛；单独依赖评估 |
| R-012 | P2 | UNVERIFIED | 真实 QQ 错误码/重复消息行为 | 本轮无线上端到端环境 | 手工矩阵和沙箱账号验证 |
| R-013 | P2 | UNVERIFIED | govulncheck 依赖漏洞结果 | 工具缺失 | 在可联网 amd64 环境运行 |
| R-014 | P2 | UNVERIFIED | 386 目标上的 race/CGO 运行行为 | Go race 不支持 windows/386 | 使用 linux/amd64 或 windows/amd64 |

## 处理原则

P0 风险必须先于结构迁移；P1 风险在影响到对应模块前处理；UNVERIFIED 项不得在发布报告中写成通过。
