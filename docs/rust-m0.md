# Rust 重写 · M0 脚手架

对应设计：`01-architect-design.md` §1（布局）/ §8（里程碑）/ §9（安全与供应链）。
M0 只新建 Rust 工作区与 CI，**不改动任何 Go 代码**。

## 默认裁决（设计 §10 的 5 个开放问题，M0 按下列默认值执行，lead 可随时推翻）

1. **保留 Go 二进制一版用于 bbolt 导出**：M1 的 idmap 迁移走 Go 侧 `legacy-export` 导出 JSONL，Rust 只消费 JSONL；Go 代码在 M4 前不删。
2. **android 目标保留但标记实验性**：`aarch64-linux-android` 仍在构建清单内，CI 失败不阻塞合入，文档注明实验性。
3. **保留全部 action 别名**：以 `callapi.RegisterHandler` 全集为准，含已废弃别名（如 `send_group_msg` → `send_to_group`），别名指向同一 handler 实现。
4. **gRPC 客户端 + 服务端都保留**：`proto/` 的 17 个 RPC 双向实现，字段号不动。
5. **性能基线**：常驻内存 ≤ Go 版 1.2 倍，吞吐不低于同配置 Go 版；M4 以 72h 压测验收。

- YAML：`serde_norway 0.9.42`（`serde_yaml` 的维护分支，RustSec 推荐）；`serde_yml` 因 RUSTSEC-2025-0068（unsound/unmaintained）弃用。

## M0 交付边界

- 已建：workspace（10 库 crate + bin + xtask）、fmt/clippy/deny/audit 配置、`core`（error/ids/telemetry）、`config`（Settings + loader 骨架）、`protocol`（event/action/cqcode/outbound/registry 骨架）。
- 未建（M1+）：热重载与全量配置字段、idmap/echo、四连接、action 实现、媒体管线、WebUI、MSRV 独立 job、黄金样本回放。
- `gensokyo` 在 M0 只支持 `--version` / `version`；无子命令时打印提示并以退出码 2 结束（不假装启动成功）。
- TLS 下限**接线**：`gensokyo-qq` 依赖 `rustls 0.23.46` / `webpki-root-certs 1.0.10` / `rustls-webpki 0.103.16`（provider 显式留空，M1 再选）；`cargo xtask check-tls-floors` 与 CI `tls-floors` job 断言 `Cargo.lock` 不低于下限；`deny.toml` 同时禁 `openssl`、`openssl-sys`。
- CQ 转义口径**跟随 Go 而非规范**：出站只转 `,` `]` `&`，不转 `[`（OneBot v11 规范要求转 `[`）；入站 `unescape` 仍接受 `&#91;`。M1 勿"修正"为转 `[`，否则黄金样本整串不匹配。
- 待 M1 校准：MSRV job 与 build matrix 按实际依赖重新校准（现有依赖 MSRV 高于 1.85，M0 不放假的 MSRV job）；`dtolnay/rust-toolchain` 无 `1.99.1` tag，工作流用 `@master` + `toolchain: 1.99.1` 输入钉版本。
