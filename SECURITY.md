# 安全说明

本仓库是 `Gensokyo-NewQQ` 的维护分支。网关会接触机器人访问令牌、用户消息、图片、WebUI 凭据和网络监听端口，应按公网服务的标准部署。

## 凭据处理

禁止向仓库提交以下内容：

- QQ 机器人 Token、Client Secret
- WebSocket / HTTP API 访问令牌
- WebUI 真实密码
- 云存储 SecretId、SecretKey、AccessKey
- Cookie、SESSDATA、CSRF Token
- 私有服务地址和运行时数据库

曾经出现在公开提交中的凭据已经失去保密性。删除源码中的字符串不能恢复安全性，凭据所有者必须在对应平台撤销或轮换，并检查访问日志和账单。

Nature 图床后端已因公开内置凭据问题永久禁用。请使用自行配置最小权限凭据的 COS 后端。

## 启动安全审计

程序启动时会读取 `config.yml` 并输出带代码的安全提示。可通过以下环境变量指定其他配置路径：

```text
GENSOKYO_CONFIG_FILE=/path/to/config.yml
```

启用严格模式后，发现高风险配置会在启动网络服务前终止程序：

```text
GENSOKYO_STRICT_SECURITY=1
```

严格模式当前会阻止的典型配置包括：

- 正向 WebSocket 已启用但 `ws_server_token` 为空
- HTTP API 可能对外监听但 `http_access_token` 为空
- WebUI 用户名或密码为空
- WebUI 使用模板默认账号密码

普通模式只记录警告，不自动修改用户配置。

## 第三方图床

ChatGLM、Ukaka 和星野会将图片发送到第三方服务器。即使旧版 YAML 中仍写有 `enabled: true`，默认也不会调用这些服务。

只有明确接受数据外传风险时才设置：

```text
GENSOKYO_ENABLE_THIRD_PARTY_IMAGE_HOSTS=1
```

建议优先使用：

1. 自行管理、采用最小权限凭据的 COS 存储桶
2. 专门用于机器人图片上传的 QQ 频道
3. 完全关闭统一图床并使用原有本地上传路径

## 网络部署

- HTTP API 优先监听 `127.0.0.1`，通过可信反向代理或专用网络访问。
- HTTP API 令牌使用 `Authorization: Bearer <token>` 传递，不要放在 URL 查询参数中。
- 正向 WebSocket 必须配置随机长令牌。
- WebUI 不应使用模板默认凭据，密码建议至少 12 个字符。
- 公网访问应使用 HTTPS，并在防火墙中只开放必要端口。
- `/metrics`、上传接口和管理接口不应直接暴露给不可信网络。

## 图片上传限制

统一图床入口执行以下限制：

- 单张图片最大 10 MiB
- 仅接受 JPEG、PNG、GIF、WebP 文件头
- PNG、JPEG、GIF 必须能够实际解析
- 最大 4000 万像素
- 清理路径、控制字符和不匹配的文件扩展名
- 图床 HTTP 请求总超时 15 秒
- 最多读取 1 MiB 第三方响应体
- 第三方返回地址必须使用 HTTPS，并拒绝明显的本机或私网目标

## 验证

仓库中的安全验证工作流执行：

```bash
gofmt
go test -race ./imagehosting ./securityaudit
go test ./...
go vet ./...
go build ./...
```

提交安全相关修改时，不应以“没有 CI 结果”代替测试通过。

## 报告安全问题

不要在公开 Issue 中粘贴真实凭据、用户消息、数据库内容或完整生产配置。报告时应先脱敏，并仅提供复现所需的最小信息。若仓库启用了 GitHub Private Vulnerability Reporting，应优先通过私密安全报告提交。
