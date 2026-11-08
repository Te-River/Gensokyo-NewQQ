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
- 未建（M1+）：黄金样本回放、M4 性能基线（72h 压测）。（MSRV 独立 job 已建：`.github/workflows/rust.yml` 的 `msrv` job 钉 `toolchain: "1.90"`、只跑 `cargo build --workspace --locked` 验可编译，测试矩阵仍归 `test` job。热重载与全量配置字段、idmap/echo、四连接、媒体管线、WebUI 均已建：`ConfigWatcher`+`ConfigManager::reload` 带重启项 diff、`echo` 存储 M1e、`boot.rs` 四连接装配、`gensokyo-media`/`gensokyo-webui` crate 就位；action 实现明细见下文「里程碑状态」，接线明细以 `reports/refactor/16-official-api-audit.md` 为准。）

## 里程碑状态（截至 2026-11-08，HEAD `da3fe3c`）

- **M3 `a60c4ed`（Rust 端点补齐与出站段类型放开）**：22 个 action 由 1500 占位改真实端点（面板 6 / 自定义菜单 2 / 入群自动审批策略 6 / 群黑名单 2 / 踢人 / 撤回×2 / `set_group_add_request` / `put_interaction` / `get_robot_share_link`；提交标题与 `qqapi` trait 计数口径为 20，按模块表逐项核为 22）；`HttpMethod` 补 PUT/PATCH/DELETE；`gensokyo-qq` 新增 22 个端点方法与 panel DTO；出站段放开 keyboard / input_notify / stream（music 降级文本）。验证：`cargo test --workspace` 全 suite ok、`cargo clippy --workspace --all-targets -- -D warnings` 零告警，评审 approve 无 Critical/Major。
- **R3a `bbee6d6`（`get_group_list` 真实化）**：`IdMapStore` 新增 `find_config_sections` 只读枚举、入站 4 群事件注册 `<上报形态ID>:type=group`、handler 逐群聚合（反查 / 信息 / 正查任一失败 = 整群跳过 + warn）。刻意差异：Go 留空字段行、不回 uin 前缀形态、无逐群节流。验证口径同 M3（全 suite ok + clippy 零告警），评审 approve 无 Critical/Major。
- **R3b `c6b44a2`（出站发送链接线）**：raw（按 Go 实际=普通链，非原生直传）/ wakeup（`is_wakeup` 与 `msg_id`/`event_id` 三字段互斥；空 message 收紧 1400；`c2c_wakeup_resp` notice 回推已由 R4a 接线）/ sse（`msg_type=2`、`msg_seq=index+10`，index/id 调用方显式传）/ 合并转发×2（逐条重发、保留 Go `limit+1` 边界、间隔 500ms 经 `DeferredDeps` 可注入、`failed[].index` 用原 node 序号、node 解析失败入清单不静默丢）；dispatcher deferred 载荷三分支，握手新键全部 `serde(default)` 向后兼容。验证：`cargo test --workspace` 33 suites 全绿 / 770 passed（R3b 时点口径）、clippy 零告警，评审修复轮后 approve 无 Critical/Major。
- **修复 `ab79a50`（素材 DTO 宽容解析）**：官方素材接口把数值字段以字符串下发（如 `"total":"36"`，Go 主仓线上日志实证），`gensokyo-qq` 的 `total/created_at/expires_at/ttl/expires_in` 五字段改 number/数字串双收（非法形态仍显式报错、缺省语义不变）；与 Go 侧 `51cc2f9`/`ea693c6` 同源同修，修复"配额清理拉不到列表→降级外链→图不显示"链路。验证：33 suites / 776 passed、clippy 零告警。
- **R4a `da3fe3c`（wakeup 事件回推）**：wakeup 发送终局（成功 / 上游失败 / 媒体上传失败）向既有 `EventHub` 推一条 `c2c_wakeup_resp` notice，JSON 逐键对齐 Go `sendWakeupNotice`（`user_id` 恒数字 0、`real_user_id/status/error_msg` 经 extra 顶层展开、`message_id` 仅成功且非空时下发否则整键消失对齐 omitempty）；注入链 `AppState.dispatcher() → Dispatcher::with_hub → DeferredDeps.hub`，未装配静默跳过、非 wakeup 零影响；一计划一 notice（Go 逐条口径由计划切分承担）；`error_msg` 为 Rust `Error::Display` 文本（刻意差异备案）。验证：33 suites / 782 passed、clippy 零告警，评审 approve 无 Critical/Major。
- `gensokyo` 在 M0 只支持 `--version` / `version`；无子命令时打印提示并以退出码 2 结束（不假装启动成功）。
- TLS 下限**接线**：`gensokyo-qq` 依赖 `rustls 0.23.46` / `webpki-root-certs 1.0.10` / `rustls-webpki 0.103.16`（provider 显式留空，M1 再选）；`cargo xtask check-tls-floors` 与 CI `tls-floors` job 断言 `Cargo.lock` 不低于下限；`deny.toml` 同时禁 `openssl`、`openssl-sys`。
- CQ 转义口径**跟随 Go 而非规范**：出站只转 `,` `]` `&`，不转 `[`（OneBot v11 规范要求转 `[`）；入站 `unescape` 仍接受 `&#91;`。M1 勿"修正"为转 `[`，否则黄金样本整串不匹配。
- MSRV 定档 **1.90**（按实际依赖校准：`redb 4.4.0` 需 1.90、`time 0.3.55` 需 1.88），CI `msrv` job 钉 `toolchain: "1.90"` 只跑 `cargo build --workspace --locked`，本地 1.90.0 全量实编已通过（测试矩阵归 `test` job）；前置条件是 `reqwest` 的 `default-features = false` 必须留在根 `[workspace.dependencies]`——成员继承时覆写该字段会让 Cargo ≤1.98 拒绝解析清单，MSRV 直接失效；build matrix 其余目标仍待按实际依赖复核；`dtolnay/rust-toolchain` 无 `1.99.1` tag，工作流用 `@master` + `toolchain: 1.99.1` 输入钉版本。
