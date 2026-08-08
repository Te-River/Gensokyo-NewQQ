# 错误模型与重试审计

## 当前实现

`handlers/qq_error_codes.go` 已有错误码映射和格式化函数，但业务调用方仍大量使用 `strings.Contains(err.Error(), ...)`。根业务范围内约有 140 处 `err.Error()`、52 处 `strings.Contains`、23 处 `time.Sleep`，并重复匹配 `22009`、`40034025`、`40034025` 相邻错误和超时中文文本。

主要重试函数包括：

- `send_group_msg.go:3005-3070` 的群普通消息和群富媒体重试；
- `send_private_msg.go` 的 C2C 普通/富媒体重试路径；
- `messagequeue` 的最大重试/退避；
- `wsclient` 的重连循环。

## 已确认问题

1. QQ 错误码已集中定义，但没有统一的 typed `QQError` 传播模型。
2. 重试条件、次数、等待时间、是否刷新 msgseq、是否入队在不同路径不一致。
3. 字符串匹配依赖错误文本/JSON 格式，错误返回格式变化时可能失效。
4. 多处 `time.Sleep` 不接受请求 context，关闭服务时可能延迟退出。
5. 缺少统一的 `RetryPolicy`，不可重试错误和幂等性边界没有集中声明。

## 语义风险

- `22009` 在不同路径可能入队、延迟、记录或丢弃，用户可见行为不一致。
- 事件过期类错误会清理 event id 并重试，但普通发送、媒体上传和 C2C 路径的处理不同。
- msgseq 在部分重试路径仍走旧的 Get/Add 组合，和并发审计互相放大。

以上是代码级确认的结构问题；实际 QQ 返回矩阵和重复消息结果 **UNVERIFIED**。

## 目标模型

定义 `QQError{Code, HTTPStatus, Operation, Retryable, Cause}`，由 adapter 统一解析；定义 `RetryPolicy{MaxAttempts, Backoff, Jitter, RetryOn, Idempotency}`，应用用例只选择策略，不自行匹配字符串。每次重试记录 attempt、reason、msgseq 和最终结果，但不记录 token/完整消息体。
