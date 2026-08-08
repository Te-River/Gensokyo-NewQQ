# 目标架构

## 模块化单体目标

```text
cmd/gensokyo
 └─ composition root

internal/domain
 ├─ identity
 ├─ message
 ├─ media
 ├─ delivery
 ├─ echo
 └─ error

internal/application
 ├─ inbound_event
 ├─ send_message
 ├─ resolve_identity
 ├─ upload_media
 └─ retry_delivery

internal/ports
 ├─ QQGateway
 ├─ OneBotPublisher
 ├─ IdentityRepository
 ├─ MediaStore
 ├─ RetryClock
 └─ ConfigSnapshot

internal/adapters
 ├─ botgo
 ├─ onebot_http
 ├─ onebot_ws
 ├─ idmap_bbolt
 ├─ media_http
 └─ provider_oss

internal/infrastructure
 ├─ config
 ├─ logging
 ├─ lifecycle
 └─ observability
```

## 依赖规则

- `domain` 只依赖标准库和领域类型，不依赖 botgo、gin、gorilla、bbolt、HTTP client 或全局 config。
- `application` 只依赖 domain 与 ports，负责用例编排、幂等、重试策略和生命周期语义。
- `adapters` 把外部协议和 SDK 转成 ports；不把外部 DTO 泄漏到 domain。
- `infrastructure` 负责组装、配置快照、日志、存储和运行时资源。
- 兼容层可以暂时保留 `foundItems`、旧 ID map 和旧 handler，但只能位于 adapters/compatibility 边界。

## 核心接口草案

```go
type QQGateway interface {
    Send(ctx context.Context, target Target, message OutboundMessage) (Receipt, error)
}

type IdentityResolver interface {
    Resolve(ctx context.Context, id Identity) (ResolvedIdentity, error)
}

type MediaPipeline interface {
    Prepare(ctx context.Context, ref MediaRef, policy MediaPolicy) (PreparedMedia, error)
}

type RetryPolicy interface {
    Decide(attempt int, err error, operation Operation) Decision
}
```

完整类型命名和字段需在第一 PR 的测试夹具中落定；本报告只定义方向，不代表接口已实现。
