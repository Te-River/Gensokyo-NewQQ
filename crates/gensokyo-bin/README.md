# gensokyo-bin（`gensokyo` 可执行文件）

M1j 的交付：把运行时装配成一个能按 `config.yml` 启动的二进制。**只做正向连接**：不连 QQ
网关、不启动反向 WS，但会在配置了 QQ 凭据时装配 OpenAPI 客户端并注册全部出站 action。

## 启动链（`gensokyo` / `gensokyo run`）

1. `ensure_config_file("config.yml")`：文件缺失时按 Go 模板生成（M1c）。
2. `ConfigManager::new`：加载配置并持有快照（config watcher 尚未接入）。
3. `install_crypto_provider`：安装进程级 rustls provider（ring）。幂等（已安装时返回
   `false`），安装失败只 `warn`、不 panic。
4. 装配 `AppState`：
   - **有 QQ 凭据**（`app_id != 0` 且 `client_secret` 非空白）：`ReqwestTransport::new()` →
     `TokenManager` → `OpenApiClient`（`sandbox_mode` 决定基址）→ `ActionContext`（**不注入
     idmap**）→ `AppState::with_actions`，并挂上 `with_api`。注册表 = 内置 3 个 +
     `gensokyo-actions` 的 41 个注册名 + 3 个别名 = **47 个 action 名**。
   - **无凭据，或 TLS 传输构建失败**：`warn` 后走 `AppState::with_builtin_handlers`
     （`get_status` / `get_version_info` / `get_login_info`，共 **3 个**）。
5. `Runtime::start`：按配置启动 listener
   - 正向 WS：`enable_ws_server=true` 时监听 `0.0.0.0:{port}`，路径 `ws_server_path`
     （`ws`，即 `ws://<addr>/ws`）；`ws_server_token` 非空时握手要 Bearer token。
   - 正向 HTTP API：`http_address` 非空时监听该地址；`GET /` 是健康检查（无鉴权），
     `POST` 任意路径收 OneBot action JSON。
6. 打印 `已注册 action: <N> 个（…）` 与**实际**监听地址（配置 `127.0.0.1:0` 这类写法会打印
   真实端口）。
7. Ctrl-C → `RuntimeHandle::shutdown_with_timeout(5s)`，超时强制中止残留任务。

两个 listener 都没启用时（`enable_ws_server=false` 且 `http_address` 为空）进程会只等
Ctrl-C，并打一条 `warn`。

## QQ 凭据与降级行为

| 配置情况 | 启动行为 | 对外表现 |
|----------|----------|----------|
| `app_id: 0`，或 `client_secret` 为空/全空白 | `warn 未配置 QQ 凭据…仅启动 WS/HTTP`，只注册内置 3 个 | WS/HTTP 照常；QQ action 未注册 → `1404`；`get_login_info` 只回显配置里的 `user_id` / `nickname` |
| 凭据齐全，但 rustls provider 缺失或 `ReqwestTransport::new()` 失败 | `warn 构建 QQ HTTP 传输失败…`，同样降级为 3 个 | 同上；启动不失败 |
| 凭据齐全且客户端构建成功 | 47 个 action 名 | 启动期**不联网**；access_token 在首次调用上游时才换取；凭据无效应表现为 action 错误 `1500` |
| `token`（机器人令牌）非空但 `client_secret` 为空 | 视为未配置 | OpenAPI v2 用 `app_id` + `client_secret` 换 access_token，`token` 不参与鉴权 |
| 未装配 idmap（M1j 现状） | `ActionContext` 无 idmap | 只接受 32 位 OpenID；虚拟号形式的 `group_id` / `user_id` 会被拒（`1400`） |

凭据只做**非空**校验，不验证真伪：首启生成的模板带着
`app_id: 12345` / `client_secret: "<YOUR_CLIENT_SECRET>"`，因此新装机器会走「有凭据」路径，
直到真正请求上游才暴露凭据无效。

## M1j 明确不做

- 不连接 QQ 网关（WebSocket）：不发 `meta_event/lifecycle/connect`（入站事件转换属 M1h-b）。
- 不启动反向 WS 客户端、不注册 webhook（M1h-b）。
- 不装配 idmap / echo：`AppState::with_idmap` / `with_echo` 留给 M1h-b、M1i；虚拟号互转
  因此不可用，`get_login_info` 只回显配置值，不代表 QQ 登录态。
- 不启动 config watcher、WebUI 与 Go 的主服务器路由；正向 WS 在 M1j 是独立监听端口
  （Go 把 WS 挂在 `serverPort` 主服务器上）。
- 媒体上传未接线：deferred 发送只接受已经是上游 `file_info` 的图片，其余形态回 `1500`
  （见 `gensokyo-actions` 的 README）。

## 构建注意

本 crate 给 `rustls` 打开 `ring` feature（`gensokyo-qq` / `gensokyo-media` 只带
`rustls-no-provider` 骨架，provider 由二进制注入），因此需要 C 工具链：CI ubuntu 的 gcc、
本机 mingw gcc 均可。Windows GNU 工具链示例：

```powershell
$env:PATH="C:\msys64\mingw64\bin;C:\msys64\usr\bin;"+$env:PATH
$env:RUSTFLAGS="-C linker=rust-lld -C link-self-contained=yes"
rustup run 1.99.1-x86_64-pc-windows-gnu cargo test -p gensokyo-bin
```

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
