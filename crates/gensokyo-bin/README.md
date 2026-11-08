# gensokyo-bin（`gensokyo` 可执行文件）

M1j 起把运行时装配成一个能按 `config.yml` 启动的二进制。**不连 QQ 网关**（WebSocket 入站），
但四种 OneBot 连接（正向 WS / 正向 HTTP API / webhook / 反向 WS）都已装配；配置了 QQ 凭据时
额外装配 OpenAPI 客户端并注册全部出站 action。

## 启动链（`gensokyo` / `gensokyo run`）

1. `ensure_config_file("config.yml")`：文件缺失时按 Go 模板生成（M1c）。
2. `ConfigManager::new`：加载配置并持有快照；随后由 `Runtime` 拉起 config 热重载监听。
3. `install_crypto_provider`：安装进程级 rustls provider（ring）。幂等（已安装时返回
   `false`），安装失败只 `warn`、不 panic。
4. 装配 `AppState`：
   - **有 QQ 凭据**（`app_id != 0` 且 `client_secret` 非空白）：`ReqwestTransport::new()` →
     `TokenManager` → `OpenApiClient`（`sandbox_mode` 决定基址）→ `ActionContext`（idmap 由
     `Runtime::attach_idmap` 在启动时补上）→ `AppState::with_actions`，并挂上 `with_api`。
     注册表 = 内置 3 个 + `gensokyo-actions` 的 `IMPLEMENTED_ACTIONS`（75 个）+ 3 个别名 =
     **81 个 action 名**（随实现增长）。
   - **无凭据，或 TLS 传输构建失败**：`warn` 后走 `AppState::with_builtin_handlers`
     （`get_status` / `get_version_info` / `get_login_info`，共 **3 个**）。
5. `Runtime::start`：按配置装配四种连接与热重载监听
   - 正向 WS：`enable_ws_server=true` 时监听 `0.0.0.0:{port}`，路径 `ws_server_path`
     （`ws`，即 `ws://<addr>/ws`）；`ws_server_token` 非空时握手要 Bearer token。
   - 正向 HTTP API：`http_address` 非空时监听该地址；`POST` 任意路径收 OneBot action JSON，
     `GET /health` 是健康检查（无鉴权；未挂 WebUI 时 `GET /` 同为健康检查）。
   - webhook：`webhook_path` 非空时把 `POST <path>` 并进两个 listener 的路由表（两个 listener
     都没启动时 `warn`）。
   - 反向 WS：`ws_address[]` 非空时主动连接下游框架，入站事件发布到事件总线。
   - WebUI：`disable_webui=false` 时静态页与 `/api` 登录路由挂到 listener；会话清理定时任务
     随句柄停止。
   - config 热重载：`RuntimeOptions::config_watch`（默认开）启动 `ConfigWatcher`。文件变动后
     热字段逐项 `info` 即时生效，重启项逐项 `warn`（快照保持不变、需重启进程）；监听启动失败
     只 `warn`，不阻断启动。
6. 打印 `已注册 action: <N> 个（…）` 与**实际**监听地址（配置 `127.0.0.1:0` 这类写法会打印
   真实端口）。
7. Ctrl-C → `RuntimeHandle::shutdown_with_timeout(5s)`：先停 config 热重载监听与反向 WS 客户端，
   再收摊 listener；超时强制中止残留任务。

两个 listener 都没启用时（`enable_ws_server=false` 且 `http_address` 为空）进程会只等
Ctrl-C，并打一条 `warn`。

## QQ 凭据与降级行为

| 配置情况 | 启动行为 | 对外表现 |
|----------|----------|----------|
| `app_id: 0`，或 `client_secret` 为空/全空白 | `warn 未配置 QQ 凭据…仅启动 WS/HTTP`，只注册内置 3 个 | WS/HTTP 照常；QQ action 未注册 → `1404`；`get_login_info` 只回显配置里的 `user_id` / `nickname` |
| 凭据齐全，但 rustls provider 缺失或 `ReqwestTransport::new()` 失败 | `warn 构建 QQ HTTP 传输失败…`，同样降级为 3 个 | 同上；启动不失败 |
| 凭据齐全且客户端构建成功 | 81 个 action 名 | 启动期**不联网**；access_token 在首次调用上游时才换取；凭据无效应表现为 action 错误 `1500` |
| `token`（机器人令牌）非空但 `client_secret` 为空 | 视为未配置 | OpenAPI v2 用 `app_id` + `client_secret` 换 access_token，`token` 不参与鉴权 |
| idmap 打开失败（路径不可写 / redb 建库失败） | `warn idmap 打开失败…`，启动继续（Go 同样情形会 `Fatalf`） | 入站回退原始 OpenID；虚拟号形式的 `group_id` / `user_id` 反查报 `1500` |

凭据只做**非空**校验，不验证真伪：首启生成的模板带着
`app_id: 12345` / `client_secret: "<YOUR_CLIENT_SECRET>"`，因此新装机器会走「有凭据」路径，
直到真正请求上游才暴露凭据无效。

## 性能参考

同机同负载（`get_login_info`，无 QQ I/O，日志统一 `error`，`oha` 64 并发 × 50k、相同请求体、**10 轮均值 ± 标准差**）：

| 指标 | Go 1.26.4 | Rust（本 crate） | 对比 |
|------|-----------|------------------|------|
| 吞吐 | 49,137 ± 4,085 req/s | **50,581 ± 5,241 req/s** | ≈1.03×，区间重叠 |
| 平均 | 1.295 ms | **1.264 ms** | ≈2.4% 更低 |
| P50 | **0.779 ms** | 1.144 ms | Go 低约 32% |
| P99 | 11.02 ms | **3.43 ms** | ≈3.2× 更低 |
| RSS | 43.4 MB | **10.8 MB** | ≈4.0× 更低 |
| 二进制 | 43.81 MB | **8.12 MB** | ≈5.4× 更小 |

口径与复现见仓库根 [readme.md](../../readme.md#性能) 与 [docs/rust-perf.md](../../docs/rust-perf.md)；这是 loopback 管线对比，不代表公网延迟。注意 Go 的 INFO 每请求日志会把吞吐压到 ~2.9k。

## 尚未接线

- **不连接 QQ 网关**（WebSocket）：不发 `meta_event/lifecycle/connect`；入站事件转换能力在
  `gensokyo-runtime` 内已实现，二进制暂不建立网关会话。
- **idmap / echo 已注入**：`Runtime::attach_idmap` / `attach_echo` 在启动第一步按配置装配
  （工作目录下的 `openid-map.redb` / `msgid-map.redb`），打开失败只 `warn`。`get_login_info`
  仍只回显配置值，不代表 QQ 登录态。
- **维护类 CLI 开关未实现**：`tidy` / `clean_ids` / `del_ids` / `del_cache` / `compaction` /
  `-m` / `-local-logger` 打印「未实现」并以退出码 2 结束（见下表）。

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
