# gensokyo-webui（WebUI 静态资源 + 管理 API）

M3 的交付：把前端构建产物嵌进二进制，并提供管理台的登录 / 会话 / 配置读取接口。
**只交付路由**，监听端口仍由 runtime 决定（`WebUi::router` 返回未绑定的 `axum::Router`）。

## 路由

| 方法 | 路径 | 说明 |
|------|------|------|
| `GET` | `/`、非 API 的 SPA 路径 | 嵌入的 `index.html`（`webui/dist` 未构建时用内置兜底页） |
| `GET` | 任意命中嵌入文件的路径 | 按扩展名给 `Content-Type`；index 带 `Cache-Control: no-cache` |
| `*` | 未命中且形如文件的路径（末段含 `.`） | 404 |
| `*` | 未注册的 `/api`、`/api/*` | 404（**不回退** `index.html`） |
| `POST` | `/api/login` | body `{username,password}`；成功 `200 {"code":0}` + 24h 会话 cookie；失败 `401 {"code":1}` |
| `GET` | `/api/checkLoginStatus` | cookie 有效 `200 {"code":0}`；否则 `401 {"code":1}` |
| `POST` | `/api/logout` | 删除会话、清除 cookie，回 `200 {"code":0}`（未登录也回 0） |
| `GET` | `/api/config` | 登录后返回脱敏字段；未登录 `401` |

### `/api/config` 返回字段（白名单）

`app_id`、`port`、`enable_ws_server`、`ws_server_path`、`arch_mode`、`disable_webui`、`oss_type`。

`token` / `client_secret` / `server_user_password` 等敏感项**永不进响应体**：`PublicConfig`
是白名单结构体，新增字段必须显式加进去，不做 `Settings` 整体序列化。

## 登录与会话

- 凭据取自 `config.yml` 的 `server_user_name` / `server_user_password`；任一项为空即视为
  未启用登录，**空凭据一律 401**（Go 的 `==` 比较会让空/空通过，这里刻意收紧）。
- cookie 名 `login_cookie`（与 Go `webui/api.go` 相同，前端无需改动），属性
  `Path=/; HttpOnly; SameSite=Lax; Max-Age=86400`。
- 会话存储是进程内的 `RwLock<HashMap<token, Instant>>`（`SessionStore`），24h TTL。
  进程重启后所有浏览器都要重新登录。
- 过期条目在校验时即失效，但需要调用方定期 `SessionStore::cleanup()` 释放内存；
  M3 没有后台定时任务。
- token 为 32 位 hex：不引入 `rand`/`uuid`，用进程内 OS 熵播种的 `RandomState` 两轮 SipHash
  折叠「纳秒时间 + 原子计数」。

## 构建前提

`#[derive(RustEmbed)] #[folder = "../../webui/dist"]` 在编译期读取仓库根的 `webui/dist`，
该目录必须存在——与 Go `//go:embed webui/dist` 是同一前提：

- 已有前端构建产物时：`cd frontend && npm run build`（输出到 `webui/dist`）；
- 只想编译时：`.\build.ps1` 的 `Ensure-WebUIDist` 会生成占位文件；
- 占位目录里没有 `index.html`，此时 `/` 由内置兜底页返回（提示前端未构建），不是 404。

## 性能参考

WebUI 静态资源不参与 action 热路径；同机同负载下 Rust 主链与 Go 的对比（`get_login_info`、日志 `error`、64 并发 × 50k、相同请求体、3 轮中位）：

| 指标 | Go 1.26.4 | Rust | 对比 |
|------|-----------|------|------|
| 吞吐 | 43,512 req/s | **53,870 req/s** | ≈1.24× |
| 平均 | 1.45 ms | **1.17 ms** | ≈1.24× 更低 |
| P50 | **0.84 ms** | 1.08 ms | Go 低约 29% |
| P99 | 13.20 ms | **2.83 ms** | ≈4.7× 更低 |
| RSS | 44.3 MB | **10.6 MB** | ≈4.2× 更低 |

完整口径见仓库根 [readme.md](../../readme.md#性能) 与 [docs/rust-perf.md](../../docs/rust-perf.md)。

## M3 明确不做

- **会话持久化**：Go 把 cookie 写进 bbolt `cookie.db`，这里只在内存；落盘、跨重启恢复、
  持久化后的清理任务留 **M3b**。
- **SSE 日志推送**（Go `/api/logs` 的流式日志）。
- **监控图表**（CPU/内存/消息速率等历史曲线）。
- **写配置**：`PATCH /api/config`（Go 写回 `config.yml` 并触发热重载）、`DELETE /api/config`。
- 用户管理、多账号、权限分级：仍只有单个管理凭据。

## 测试

```bash
cargo test -p gensokyo-webui
```

测试用 `tower::ServiceExt::oneshot` 直接喂 `Router`，不监听端口、不访问外网；临时
`config.yml` 写在系统临时目录，`Drop` 时删除。
