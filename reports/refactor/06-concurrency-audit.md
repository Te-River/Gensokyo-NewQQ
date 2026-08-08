# 并发、生命周期与消息序号审计

## 静态规模

根仓库及本地业务依赖范围内统计到：约 31 处 `go func(`、11 处 `sync.Mutex`、7 处 `sync.RWMutex`、38 处 `sync.Map`、7 处 `WaitGroup`、6 处 `time.NewTicker`、约 41 处 `time.Sleep`。这是搜索统计，不等同于活跃 goroutine 数。

## 消息序号

`echo/echo.go:242-280` 同时存在：

- `GetMappingSeq`：读取或生成值；
- `AddMappingSeq`：单独写入；
- `IncrementMappingSeq`：在 mutex 内完成读改写。

旧调用方仍在 `Processor/Processor.go:886-983`、`message_parser.go:2084-2099`、`send_group_msg_raw.go`、`send_group_msg.go`、`send_private_msg.go` 中使用 `GetMappingSeq` 后再 `AddMappingSeq`。因此存在确认的非原子读改写窗口；在并发重试时可能重复或跳号。实际 race 输出因 windows/386 不支持 `-race` 而未验证。

## 其他热点

- `echo` 的 stack 清理路径直接替换底层 slice，未持有该 stack 的 mutex；与 Push/Pop 并发时存在数据竞争风险，运行时结果 **UNVERIFIED**。
- `Processor.BroadcastMessageToAllFAF` 为客户端发送启动 goroutine 并立即返回，错误被忽略；调用方无法获得完成/失败语义。
- `messagequeue.StartWorker` 每次调用创建 worker，但现有 `WaitGroup` 计数与 worker 生命周期不对称，`Wait()` 的覆盖范围需要重构确认。
- `wsclient.SendMessage` 写入 `writeCh` 后立即返回，`Close` 同时关闭 channel；并发发送/关闭有 send-on-closed-channel 和“返回成功但尚未写出”的风险。
- 多处采用 `time.Sleep` 重试和无上限 goroutine，缺少统一上下文、容量和关闭协议。

## 目标模型

统一 `MsgSeqStore.Next(ctx, key)` 原子接口；所有重试复用同一序号策略。为 WS、reverse post、message queue 定义有界队列、关闭状态、worker 计数和错误回传。所有后台任务由应用生命周期管理器注册并等待退出。

## 本轮结论

P0 候选为序号读改写；本轮只报告，不改动旧调用方。必须在单独 PR 加并发回归测试，并使用支持 race 的 amd64 环境验证。
