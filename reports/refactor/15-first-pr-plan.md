# 第一 PR 计划

## 建议标题

`security: establish bounded HTTP and atomic message sequencing`

## 目标

只处理两个可独立验证的基础风险：统一关键 HTTP 入口的资源边界，以及把消息序号推进收敛到原子接口。凭据清理另开安全 PR，避免把秘密轮换、代码改造和行为变更混在一个提交中。

## 变更范围

### A. HTTP 边界

- 引入带 context、timeout、重定向策略、最大响应体和状态码检查的内部 HTTP helper/adapter。
- 先覆盖 webhook body、媒体下载、QQ music、idmap 外部查询和现有无 timeout client。
- 为 HTTP server 设置 header/read/write/idle timeout 和 header/body 限制。
- 保留现有 SSRF/内网地址防护，并将其放入 adapter policy，不降低现有规则。

### B. 消息序号

- 保留旧 API 作为临时兼容层，但让旧调用转发到 `Next`/原子更新实现。
- 统一 retry、markdown、card、raw、private 等路径的序号获取。
- 增加同 key 并发推进、跨 key 隔离、首次值、重试值和清理后的行为测试。

## 不在本 PR

- 不迁移 `foundItems`、`Processor`、`config` 或媒体 pipeline。
- 不改变 OneBot JSON 兼容格式。
- 不执行凭据轮换、不 push、不部署、不修改数据库和运行时 ignored 数据。

## 验收命令

建议执行：`go test ./echo ./handlers ./Processor`、`go vet ./...`、支持 race 的 amd64 环境执行 `go test -race ./...`、`go test ./... -cover` 和 `git diff --check`。

## 手工验证

1. 同一群/私聊目标并发发送多条消息，确认序号无重复且 retry 不覆盖新值。
2. 用超大 webhook body、慢响应、非 2xx、重定向到内网地址验证拒绝和超时。
3. 验证正常图片、语音、文件和 QQ music 路径的返回结果未改变。
4. 验证关闭服务时后台 HTTP、queue、WS goroutine 能退出。

## 提交策略

第一 PR 完成后只做本地独立 commit；push、合并和部署需要用户另行授权。
