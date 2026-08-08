# 网络、资源边界与安全审计

## 统计证据

根业务范围内搜索到约 8 个 `http.Client`、7 个 `http.DefaultClient`、25 个 `http.Get`、24 个 `.Do`、30 个 `io.ReadAll`。HTTP server 构造处未发现 `ReadHeaderTimeout`、`ReadTimeout`、`WriteTimeout`、`IdleTimeout` 或 `MaxHeaderBytes`。

## 已确认高风险项

### P0：硬编码云凭据

`imagehosting/nature.go:23-24` 存在硬编码的 Base64 形式云服务凭据常量，Base64 不是加密。报告不复制值；应按已暴露凭据处理、轮换并从仓库历史和运行配置中清除。轮换操作本轮未执行。

### P1：HTTP 客户端和响应体没有统一边界

`idmap/service.go` 多处 `http.Get`，`images/format.go`、`imagehosting/*`、`url/shorturl.go`、`sys/re.go`、`Processor/Processor.go` 等路径使用默认 client 或无超时 client。多个上传、下载、QQ music、媒体和 idmap 路径对响应体直接 `io.ReadAll`，缺少统一大小上限、状态码校验和取消上下文。

### P1：服务端请求资源保护缺失

`main.go:549-598` 创建多个 `http.Server`，未设置超时或 header 限制。`server/webhook.go:75-141` 在签名校验前完整读取请求体并复制，未见 `MaxBytesReader`；随后还存在再次读取/校验路径。恶意大 body、慢连接或大量排队请求可能消耗内存和 goroutine。

### P1：构建脚本改变依赖状态

`build.ps1` 设置 `GOFLAGS=-mod=mod` 并在构建时执行 `go mod tidy`。构建会改变 `go.mod/go.sum`，而且构建机网络和 Go 版本不同可能得到不同依赖结果，破坏可复现性。该项本轮不修。

## 已有正向模式

`handlers/send_group_msg.go` 的共享 HTTP client 具有 30 秒超时并限制重定向，同时复核 SSRF/内网地址；`reply_helpers.go` 也设置 30 秒 client。应将这些能力收敛为统一 HTTP adapter，并补充 body/status/context 策略。

## 工具限制

`govulncheck` 未安装，依赖漏洞扫描 **NOT RUN**；本报告是静态审计，不替代外部网络、真实攻击面和凭据轮换验证。
