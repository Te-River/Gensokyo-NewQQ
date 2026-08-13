# Changelog — Release013

> 自 Release012 以来的所有变更。

---

## 🐛 Bug 修复

### SSM 补发链入队路径补齐共享关联标识日志

- `echo.PushGlobalStack` 内部新增 `[SSM][cid] 已入队 group=<群> 队列长度=<n>` 日志，入队事件不再依赖调用方样板日志，任何入队路径（含未来新增调用点）均可通过共享关联标识观测
- `echo.NextSSMCorrelationID` 对短 groupID 做安全截断（长度不足 8 时取全部），避免切片越界 panic
- 入队（`PushGlobalStack`）与补发（`SendStackMessages`）两侧日志沿用同一 `CorrelationID`，跨边界关联方式不变，发送与补发行为零改动
