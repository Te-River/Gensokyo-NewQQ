# Gensokyo-NewQQ（Rust 重构分支）

> Gensokyo-NewQQ 的 **Rust 重写**。目标是 parity：**对外输入输出与 Go 版一致**——同一份 `config.yml`、同一套 OneBot V11 action 与事件、同一套扩展 CQ 码与扩展 API 行为，内部实现全部用 Rust。

`Refactor` 分支**已是纯 Rust 工程**：Go 源码树于 2026-11-18 移除（提交 `63cd9b0`）。Go 主线在 `main` 分支（同远端仓）与兄弟目录 `../Gensokyo-NewQQ`，仅作 parity 参照。

## 项目定位

- OneBot V11 服务端：QQ 官方 API 与 WebSocket 事件 → OneBot V11 协议。
- 四种连接方式可组合：反向 WS、正向 WS、HTTP API、HTTP POST（webhook）。
- 覆盖范围：入站事件转换、出站 action 与 CQ 码、媒体（下载/上传/压缩/silk/QR）、idmap 与 echo、短链、idmap gRPC、WebUI 管理 API。
- 数据文件与 Go 不同：redb（`openid-map.redb` / `short-urls.redb` / `cookie.redb`）vs Go bbolt `gensokyo.db`；idmap 与 echo 靠 JSONL 导入迁移。
- parity 口径：主干/功能/边角项逐项标注在 [`reports/refactor/17-go-rust-parity.md`](./reports/refactor/17-go-rust-parity.md)，刻意差异登记在该文件 §F 与 [`docs/rust-handover.md`](./docs/rust-handover.md) §6。

## 快速开始

前置：`rustup`（`rust-toolchain.toml` 钉 **1.99.1**，workspace MSRV **1.90**）；Windows GNU 宿主需 mingw 链接器在 PATH；构建缓存目录务必设在**仓外**（`CARGO_TARGET_DIR`）。

| 目的 | 命令 |
|------|------|
| 构建（宿主） | `cargo xtask build`（Windows 便捷入口：`.\build.ps1`） |
| 构建全矩阵 | `cargo xtask build --all`（`.\build.ps1 -All`） |
| 只构建 Linux×3 | `cargo xtask build --linux-only` |
| 精简版（不嵌 WebUI） | `cargo xtask build --no-webui` |
| UPX 压缩档 | `cargo xtask build --upx-level 7`（1..9，默认 7） |
| 全量测试 | `cargo test --workspace` |
| 静态检查 | `cargo clippy --workspace --all-targets -- -D warnings` |
| 格式检查 | `cargo fmt --all -- --check` |
| TLS 依赖下限断言 | `cargo xtask check-tls-floors` |

产物落 `release/`，命名带 `rust` 标识（如 `gensokyo-rust-windows-amd64.exe`）。`--all` = Go `build.ps1` 的 5 个平台（linux×3 + windows×2）× 双版本 = 10 个产物；linux×3 与 windows-gnu×2 全部走 zig 自动引导（缓存落 `.tools/`，二跑零网络；windows 目标由 zig 交叉，不需要 MSVC / mingw），缺工具链的目标输出 `[skip]` 不算失败。

门禁基线（2026-11-18，HEAD `59510ca`）：`cargo test --workspace` = **45 suites / 1478 passed / 1 ignored / 0 failed**；clippy `-D warnings` 零告警；fmt 退出码 0。

## crate 布局

依赖方向只能向下：`bin → runtime → actions → 能力 crate → config → core`。

| crate | 职责 |
|-------|------|
| `gensokyo-core` | 统一错误类型、OneBot 虚拟 ID、日志与指标门面（最底层，不依赖其它 gensokyo crate） |
| `gensokyo-config` | `config.yml` 映射、Go `Get*()` 等价访问器、快照热重载、首启模板生成 |
| `gensokyo-protocol` | OneBot V11 模型、CQ 码、`OutboundPlan`、handler 注册表（不依赖 qq/storage/media） |
| `gensokyo-qq` | QQ 官方 API 适配层：token 刷新、OpenAPI v2、群管理、DTO |
| `gensokyo-storage` | idmap、echo、cookie、JSONL 导入、IDMapService gRPC |
| `gensokyo-media` | 下载上传、压缩、图床、silk 编解码、QR |
| `gensokyo-actions` | 全部出站 action 实现（Go `handlers/` 的出站部分） |
| `gensokyo-runtime` | 入站转换、四种连接、dispatch、队列与限速、优雅关闭 |
| `gensokyo-webui` | 内嵌前端资源（`webui/dist`）与管理 API |
| `gensokyo-bin` | 可执行入口与装配 |
| `xtask` | 构建/发布辅助：`build`、`check-tls-floors`、`bench`、`pgo` |

## 文档索引

| 文档 | 内容 |
|------|------|
| [`docs/rust-handover.md`](./docs/rust-handover.md) | 接手第一份：现状与数据、已落地能力、**「不可当 bug 的差异」清单**、下一步 |
| [`docs/rust-build.md`](./docs/rust-build.md) | 构建链：`--all` 矩阵、zig 自动引导、产物命名、版本串注入、CI 用法 |
| [`docs/rust-migrate.md`](./docs/rust-migrate.md) | 换机迁移：带什么、新机装什么、按什么顺序验证 |
| [`reports/refactor/17-go-rust-parity.md`](./reports/refactor/17-go-rust-parity.md) | Go↔Rust 逐项 parity 审计（§F 为刻意差异登记） |
| [`release_log/CHANGELOG_v016.md`](./release_log/CHANGELOG_v016.md) | 当前版本线 `Release 016 Rust` 的完整变更 |

产品行为文档（[`docs/api/`](./docs/api/)、[`docs/cq码/`](./docs/cq码/)、[`docs/本版新增功能.md`](./docs/本版新增功能.md)）沿用 Go 版口径；其中出现的 `handlers/*.go`、`Processor/*.go` 等路径属 Go 主仓，Rust 侧对应实现见 `crates/`。

## 与 Go 主线的关系

- **Go 主线**：`main` 分支（远端同仓）+ 兄弟目录 `../Gensokyo-NewQQ`。本分支不含任何 Go 源码，**Go 构建请用 `main` 分支**。
- 本分支与主仓**并联**：不替换、不接管主仓发布，也不要求主仓先下线。
- 共用配置语义（`config.yml` 字段名不变），但数据文件不共用（见上）。
- 改行为前先核对 Go 行为；与 Go 的有意差异必须登记到 parity §F 与 handover §6，未登记的差异一律按 bug 处理。

## Release 命名

当前版本线为 **`Release 016 Rust`**（见 [`release_log/CHANGELOG_v016.md`](./release_log/CHANGELOG_v016.md)，**尚未发布**）。GitHub Release 标题用 `Release 016 Rust`，与 Go 主线的 `Release016` 区分。

## 许可

MIT，见 [LICENSE](./LICENSE)。
