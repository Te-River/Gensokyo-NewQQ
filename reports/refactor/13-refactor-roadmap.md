# 重构路线图

## 第一轮已完成

- 固定分支和基线提交。
- 完成 Go、前端、嵌套模块构建/测试状态。
- 完成文件规模、依赖、foundItems、identity、concurrency、network/security、config、error/retry、media 审计。
- 输出当前架构、目标架构、风险登记和第一 PR 计划。

## 后续 PR 顺序

| PR | 主题 | 关键验收 |
|---|---|---|
| P0 | 凭据清理、HTTP 资源边界、消息序号原子化 | secret 不在源码；body/status/timeout 限制；并发 msgseq 测试 |
| P1 | 错误模型与 RetryPolicy | typed QQ error；错误码/幂等策略表；无字符串散落匹配 |
| P2 | 配置快照与原子持久化 | schema/semantic validation；atomic write；reload debounce |
| P3 | Identity typed model | OpenID/Virtual/UIN/Group 类型；兼容矩阵 |
| P4 | foundItems → ParsedMessage | parser 单一语义；旧输入回归测试 |
| P5 | Media pipeline | source/transform/upload 分层；大小限制；共享 group/private planner |
| P6 | OutboundMessage 与发送用例 | botgo DTO 只在 adapter；群/私共享编排 |
| P7 | messagequeue/wsclient lifecycle | 有界队列；close/Wait 语义；backpressure |
| P8 | inbound event application service | Processor 变薄；publisher port；广播结果可观测 |
| P9 | callapi/HTTP/WS adapters | typed action decode；注册表显式组装 |
| P10 | idmap/echo repositories | 存储、清理、过期、owner 语义集中 |
| P11 | 配置和运行时依赖收口 | 删除业务层全局 getter 依赖 |
| P12 | 生成代码和第三方 SDK 隔离 | 目录与构建边界稳定，不重写生成文件 |
| P13 | 删除兼容层与全量回归 | `foundItems`、旧 handler、旧 ID heuristic 清理 |

## 每个 PR 的门槛

- 保持旧 OneBot 输入/输出兼容，除非单独声明行为变更。
- 增加针对性测试和失败路径测试。
- `go test`、`go vet`、必要时 amd64 `go test -race`。
- `git diff --check`，不修改无关用户文件，不提交数据库、凭据、dist、node_modules。
- 先独立 commit，push 需要另行授权。
