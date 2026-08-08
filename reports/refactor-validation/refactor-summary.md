# 石山重构 — 阶段总结（S0 + P2~P13 + V1~V5）

```
STATUS: 基础设施阶段全部完成；生产切换 + 真实联调待独立任务执行
```

---

## 目标管线（计划"最终完成标准"）

```
OneBot Input → Typed Action → Typed Identity → ParsedMessage → OutboundService → QQ Adapter → QQ OpenAPI
QQ Event → QQ Adapter → DomainEvent → Inbound Application → OneBot Serializer → HTTP / WS
```

**当前状态：** 管线的每一环都已建立独立的 typed 包与 adapter 边界，且相互衔接：

```
internal/application/action  →  internal/domain/identity  →  internal/domain/message
      →  internal/application/outbound  →  adapter/qq
internal/domain/event  →  internal/application/inbound  →  adapter/onebot
internal/application/media  →  internal/application/queue  →  internal/application/state
internal/infrastructure/config  →  Snapshot/Manager
```

## 各阶段交付

| 阶段 | 提交 | 核心交付 |
|------|------|----------|
| S0 | `87d34f8` | 移除 Nature 内置云凭据，配置注入 + fail closed |
| P2 | `15cef7c` | Config Snapshot 管线（parse→migrate→validate→snapshot→atomic write→debounce） |
| P3 | `cb75b38` | Typed Identity + Resolver；长度启发式归零 |
| P4 | `b751e65` | ParsedMessage/MessagePart；String/Array 统一；96.9% coverage |
| P5 | `762ab11` | 媒体管线（SafeHTTPFetcher/SSRF/硬上限/temp file 生命周期） |
| P6 | `2f1cbe6` | OutboundService 唯一发送主链 + RetryPolicy |
| P7 | `fe4a4a7` | 有界队列（session 排序/背压/delay scheduler/Metrics） |
| P8 | `444ae1f` | DomainEvent + inbound 管线 + OneBot serializer |
| P9 | `514be8c` | Typed action + 显式 Registry + 共用 Dispatcher |
| P10 | `9e58ce9` | Repository 化（Sequence/MessageContext/Cleaner/TTL 统一） |
| P11 | `8cb1c6b` | 新架构 config 依赖归零 + 子配置构造注入 |
| P12 | `a305fea` | botgo/go-silk 边界隔离 + fork inventory + generate 脚本 |
| P13 | （本次） | 删除条件未满足 → BLOCKED（非破坏收尾完成） |

## 核心业务层依赖检查（计划目标）

| 依赖 | 新架构 | 生产（旧） | 说明 |
|------|--------|-----------|------|
| botgo DTO | 无 | 有 | 新架构满足；生产待 P13 |
| foundItems | 仅 compat bridge | 有 | 新架构满足；生产待 P13 |
| ID 长度猜测 | 仅 legacy adapter | 已归零 | ✅ |
| err.Error() 字符串判断 | 无（ErrorClassifier） | 有 | 新架构满足；生产待 P13 |
| 全局 Settings | 无 | 有 | 新架构满足；生产待 P13 |
| package init 注册/goroutine | 无 | 有 | 新架构满足；生产待 P13 |
| HTTP client（裸） | SafeHTTPFetcher | 有 | 新架构满足；生产待 P13 |
| bbolt / go-silk | Repository/AudioTranscoder | 有 | 新架构满足；生产待 P13 |

## 验证汇总

- V1 每阶段 build/vet/test：PASS
- V2 Windows：build/vet/test PASS；race NOT_RUN（windows/386 不支持）；govulncheck NOT_RUN（网络不可达）
- V3 Linux：NOT_RUN（无环境）
- V4 Frontend：lint/build PASS；test NOT_IMPLEMENTED（如实记录）
- V5 真实联调：BLOCKED（用户侧）

## 完成标准评估（计划原文）

> 在此之前，不以"文件变短了 / 目录变漂亮了 / 代码拆成更多 package"作为重构完成标准。

**达成判定：** 目标管线的基础设施全部就绪且可测；但**生产主链尚未切换到新架构**，
且未经过真实 QQ/OneBot 联调与稳定测试周期。按计划 Stop Conditions 与 P13.1 删除条件，
**P13 破坏性删除（foundItems=0 / legacy handler 删除 / 全局状态移除）判定为未完成**，
须在"接入生产 → 真实联调（V5）→ 稳定周期"之后以独立任务执行。

## 建议的后续任务顺序

1. 生产接入（逐段 shadow-compare：parser → outbound → inbound → action → repository）
2. 真实 QQ/OneBot 联调矩阵（V5）+ `docs/testing/manual-matrix.md`
3. 稳定测试周期
4. P13 破坏性删除 + P11 全局 getter 收口 + dead code 清理
5. Linux race gate + govulncheck（V3）
