# 配置、重载与持久化审计

## 现状

`config/config.go` 以全局 `instance` 加 `RWMutex` 提供配置 singleton，`structs.Settings` 承载网络、认证、媒体、OneBot、重试、WebUI 等大量字段。`LoadConfig` 同时负责读取、去重、YAML 解析、补全、重启字段比较和替换全局实例。

## 已确认问题

1. `isValidConfig` 主要检查非空和 `Version != 0`，没有独立的 schema/语义校验阶段。
2. `UpdateConfig`、`WriteYAMLToFile` 等路径直接 `os.WriteFile`；写入错误处理不完整，缺少统一临时文件、fsync、原子 rename 和备份策略。
3. `cleanupDuplicateSettings` 通过行级 YAML 操作重建内容，配置格式变化时容易产生隐性破坏。
4. 配置版本字段存在，但未发现明确的版本迁移 dispatcher；当前更像模板补全而非 schema migration。
5. `main.go` 的 fsnotify watcher 在配置写入时直接 reload，缺少稳定窗口/去抖；配置自写可能触发自重载。
6. 全局 getter 数量多，业务用例容易在任意层读取全局状态，难以测试和保证单次请求的一致快照。

## 风险

- `P1`：直接覆盖写在进程中断或磁盘异常时可能留下截断配置。
- `P1`：重载期间不同 goroutine 可能观察不同配置组合。
- `P1`：认证和服务地址等字段与普通设置混合，权限审计边界不清晰。
- 上述数据损坏和并发重载是否已在生产触发：**UNVERIFIED**。

## 目标模型

配置分为不可变启动配置、可热加载运行配置和秘密引用；加载阶段执行 parse → schema validate → semantic validate → immutable snapshot。持久化采用 temp + fsync + atomic rename + 备份，watcher 采用 debounce，并把变更事件发布给应用层。

## 迁移约束

先增加配置快照和 schema/语义校验测试，再替换写入路径；不在第一 PR 修改全部字段或删除全局 getter。
