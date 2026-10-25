# gensokyo-bin（`gensokyo` 可执行文件）

M1j 的交付：把 M1h-a 的运行时装配成一个能按 `config.yml` 启动的二进制。**只做正向连接**，
不连 QQ、不启动反向 WS。

## 启动链（`gensokyo` / `gensokyo run`）

1. `ensure_config_file("config.yml")`：文件缺失时按 Go 模板生成（M1c）。
2. `ConfigManager::new`：加载配置并持有快照（config watcher 尚未接入）。
3. `AppState::with_builtin_handlers`：注册 `get_status` / `get_version_info` / `get_login_info`。
4. `Runtime::start`：按配置启动 listener
   - 正向 WS：`enable_ws_server=true` 时监听 `0.0.0.0:{port}`，路径 `ws_server_path`
     （`ws`，即 `ws://<addr>/ws`）；`ws_server_token` 非空时握手要 Bearer token。
   - 正向 HTTP API：`http_address` 非空时监听该地址；`GET /` 是健康检查（无鉴权），
     `POST` 任意路径收 OneBot action JSON。
5. 打印**实际**监听地址（配置 `127.0.0.1:0` 这类写法会打印真实端口）。
6. Ctrl-C → `RuntimeHandle::shutdown_with_timeout(5s)`，超时强制中止残留任务。

两个 listener 都没启用时（`enable_ws_server=false` 且 `http_address` 为空）进程会只等
Ctrl-C，并打一条 `warn`。

## M1j 明确不做

- 不连接 QQ 官方 API：`app_id` / `token` / `client_secret` 只进配置快照，不发任何上游请求。
- 不启动反向 WS 客户端、不注册 webhook（M1h-b）。
- 不装配 idmap / echo / OpenAPI 句柄（`AppState::with_idmap` / `with_echo` / `with_api` 留给
  M1h-b、M1i）——`get_login_info` 因此只回显配置里的 `user_id` / `nickname`，不代表 QQ 登录态。
- 不启动 config watcher、WebUI 与 Go 的主服务器路由；正向 WS 在 M1j 是独立监听端口
  （Go 把 WS 挂在 `serverPort` 主服务器上）。
- 不发 `meta_event/lifecycle/connect`（入站事件转换属 M1h-b）。

## CLI

| 形式 | 行为 |
|------|------|
| `gensokyo` / `gensokyo run` | 启动运行时（默认） |
| `gensokyo --version` / `-V` / `gensokyo version` | 打印 `gensokyo <ver>` 并退出 0 |
| `gensokyo faststart` / `--faststart` / `-faststart` | 同 `run`；M1j 没有需要跳过的初始化步骤 |
| `tidy` / `clean_ids` / `del_ids` / `del_cache` / `compaction` / `-m` / `-local-logger <v>` | **未实现**：stderr 打印「未实现…」，退出码 2 |

Go 用标准库 `flag`，长选项是单横线（`-faststart`、`-local-logger=enable`）；本二进制在解析前
把已知的单横线长选项改写成 `--name`，两种写法都可用（`-m` / `-V` 短选项不变）。

### 退出码

| 码 | 含义 |
|----|------|
| 0 | 正常退出（`--version`、Ctrl-C 优雅关闭） |
| 1 | 配置生成/解析失败、端口绑定失败、Ctrl-C 处理器注册失败 |
| 2 | 未实现的子命令/开关；以及 clap 的参数错误 |

## 环境变量

- `GENSOKYO_BUILD_VERSION`：**构建期**注入的版本串（`ldflags` 等价），`--version` 输出
  `gensokyo <该值>`；未注入时回退 `dev-<epoch>.<millis>`（与 Go `buildinfo.Version()` 一致）。
- `RUST_LOG`：覆盖 `config.yml` 的 `log_level`。支持 `info`、`warn,gensokyo_runtime=debug`
  这类简化语法；未知片段忽略。

`log_level` 映射与 Go `mylog.GetLogLevelFromConfig` 一致：`0=debug 1=info 2=warn 3=error`，
其它值回退 `info`。日志写 stderr，格式 `<RFC3339> <LEVEL> <target>: <message> key=value`，
**不写日志文件**（`-local-logger` 未实现，`save_logs` 目前不生效）。
