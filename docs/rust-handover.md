# Rust 重构仓交接文档

面向「第一次接手本仓」的人：怎么跑起来、哪些能力已落地、哪些差异**不是 bug**、下一步做什么。

配套文档：构建链 [`rust-build.md`](./rust-build.md)｜换机 [`rust-migrate.md`](./rust-migrate.md)｜里程碑流水 [`rust-m0.md`](./rust-m0.md)｜性能 [`rust-perf.md`](./rust-perf.md)｜Go↔Rust 逐项审计 `reports/refactor/17-go-rust-parity.md`｜官方 API 审计 `reports/refactor/16-official-api-audit.md`。

> [!IMPORTANT]
> 本文每条事实都带 `file:line` 或命令。**所有数字有时点**：测试数、产物体积、磁盘水位都会变，判读一律「以命令输出为准」。没写来源的结论视为未核实，不要引用。

## 1. 三仓关系与定位

| 仓 | 路径 | 分支 | 定位 |
|----|------|------|------|
| 主仓 | `D:\Github\Gensokyo-NewQQ` | `main` | Go 生产链，当前线上发布来源；`arch_mode`（`legacy`/`shadow`/`new`）+ 六接缝的并存/回退机制都在这里（主仓 `AGENTS.md`「分层架构」节） |
| **重构仓（本仓）** | `D:\Github\Gensokyo-NewQQ-Refactor` | `Refactor` | Rust 重写，**与主仓并联**：不替换、不接管主仓发布，也不要求主仓先下线 |
| 官网仓 | 文档站（另一线/另有其人） | — | 对外文档站；`docs/rust/actions.md` 只存在于官网仓，本仓 grep 不到属正常（见 §6 ⑨） |

> [!IMPORTANT]
> **本仓只有一条 Rust 链，没有 `arch_mode` 三态切换。** `arch_mode` 在本仓只是配置兼容字段：解析点 `crates/gensokyo-config/src/accessors.rs:707`，唯一消费者是 WebUI 配置投影 `crates/gensokyo-config/src/config_api.rs:61`。「切回 Go」的开关在主仓，不在本仓。

两仓共用同一份**配置语义**（`config.yml` 字段名不变），但**数据文件不共用**：Go 是 bbolt `gensokyo.db`，本仓是 redb（`openid-map.redb` / `short-urls.redb` / `cookie.redb`）。idmap 与 echo 靠 G1 `legacy-export` 的 JSONL 导入，短链没有导入器（`docs/rust-build.md:232-233`、parity `:130`）。

## 2. 当前状态：一句话 + 数据

主干链路（连接/入站/出站/媒体/idmap/短链/重试/URL 转换）已对齐；官方 API 审计的**真缺口（注册集口径）= 0**（`reports/refactor/16-official-api-audit.md:3-4`：原「28 项未接线」是 `d00a5ff` 时点的快照，27 项已接线、余 1 项 `get_qq_profile` 裁决「明确不做」）。`get_friend_list` 的**读写两侧均已落地**（P64a/P64b，写侧含评审收口：`StorageError::DuplicateKey` 独立变体 + 空 `Author.ID` 守卫，见 §6 ⑬⑭），CQ 码表元数据已校准（P69）、lotus stub 用例已加固（P70）；上一版挂着的「入站 `inbound` 位与 Go 实际产出集超集」已由 P71 逐码取证、**P72 裁决并实施**（18 条码位 true→false，`array_value` 上报里用户手打的 `[CQ:…]` 除 `at` 外按原文折回 text，与 Go `ConvertToSegmentedMessage` 逐字节一致）；剩余工作只有两项：**P73 的 `at` 结构改造**（`revert_mentions` 合成 → `inbound` 位回解这条链，设计已到、须先复核两根承重假设再动手）、以及文档欠账，见 §7；上一版列的第三项「`poke` 收位依据复扫」**已于 2026-11-14 完成并确证**（`.go` 零命中 + `REACTION_*` 零消费，证据见 parity B4 该行），惟 Go 侧缺对应的入站段数组用例、已另记测试欠账。

| 指标 | 现值 | 来源 / 取数命令 |
|------|------|-----------------|
| parity 覆盖面 | A/B/C 三表逐项标注状态，主干/功能/边角项均为「已对齐 / 关闭」；B6 是**历史 severity 汇总**，个别行只写估量、未逐条回写状态 | `reports/refactor/17-go-rust-parity.md`：**按节定位**（A 表 / C 表 / B1-3 / B4 CQ 码进出站 / B5 / B6 缺口汇总 / D / E），该文件行号随每次 parity 回写浮动，不再钉死数字区间 |
| 测试基线 | **43 suites / 1379 passed / 0 failed**（2026-11-14 P73 后实测；`ignored` 计数以门禁输出为准） | `cargo test --workspace --no-fail-fast`（计数随新增用例变化，取数见 §9） |
| clippy | `-D warnings` **零告警** | `cargo clippy --workspace --all-targets -- -D warnings` |
| 格式 | `cargo fmt --all -- --check` **退出码 0** | 同左 |
| release 产物 | **≈10.3 MiB**（同表记录 Go `44.82 MiB`，≈4.3× 更小） | `docs/rust-perf.md:167`（2026-11-10 口径）；当前体积以 `cargo xtask build` 打印为准 |

> 关于测试数字：suite/passed 随新增用例变化，且各里程碑文档记的是各自时点值
> （如 `docs/rust-m0.md` 的 R4m 记 1356、R4n 记 1359，均对当时成立）。
> 当前实测以 2026-11-14（P73 收口后）为准：**43 suites / 1379 passed / 0 failed**；
> `ignored` 是 `#[ignore]` 用例（要 `--ignored` 才跑），计数随用例增删浮动。
> 引用时以**你刚跑出的
> `test result:` 行**为准，以最终门禁输出为最终口径。

报表本身带**时效声明**：`reports/refactor/16-official-api-audit.md:3` 明说旧计数是 `d00a5ff` 时点的快照，「现状以『R3 接线回写』节 + 当前代码为准」——引用任何历史数字前先看这一行。

## 3. 已落地的能力（按域）

| 域 | 能力 | 证据 |
|----|------|------|
| 连接与入站 | 四连接模式：反向 WS / 正向 WS / HTTP API / webhook POST | parity `:11-16`；`ws_client.rs`、`ws_server.rs`、`http_api.rs`、`webhook.rs` |
| | 15 类 QQ 事件转换、`array_value` 段数组、`remove_at`/`convert_other_at`、`add_at_group`、`op_user_id_type` | parity `:17-27`；分派臂 `crates/gensokyo-runtime/src/inbound.rs:504` 起 |
| 出站 action | Go 53 个唯一注册名 **53/53 覆盖**；`KNOWN_ACTIONS` = **56**（53 + 素材 3），机器断言 | `crates/gensokyo-actions/src/register.rs:386`（守卫 `:385-392`，缺项必须恰等于 runtime 内置 3 项）；口径 parity `:60` |
| | `voice`≡`record` 别名归一、裸媒体码回原文、`array_value` 出站形态 | parity `:73`（P57-② / P59） |
| CQ 码 | 进出站编解码、`encode_value` 四字符转义、段 JSON 键序（`data` 先于 `type`）；`array_value` 上报里**用户手打的 `[CQ:…]` 除 `at` 外按原文折回 text**（P72 收 18 码位，`at` 例外见 §6 ⑯ / §7 P73） | parity `:17`、`:18`、`:66-73`、B4 末三行 |
| 限速 / 队列 | `queue.rs` 令牌桶（5qps / burst10）+ 3 次线性退避 + SSM 队列；**全局发送队列按 Go 活路径不做**（Go 的 `messagequeue` 队列是死代码） | parity `:82-86` |
| WebUI / 观测 | 9 条余路由（config 写 / accounts / status / logs WS）+ `cookie.redb`；botstats redb + `/metrics` 6 指标 | parity `:43`、`:40` |
| unioncache | 入站写 + markdown 反查 + `union_id` 身份切换 | parity `:48`、`:128` |
| 媒体链 | 图床 `oss_type` 4~10 全量后端 + 分派接线 | parity `:44` |
| | 图片压缩 / silk 转码 / >5MB 分片 / card 本地图 / markdown 内嵌图替换 | parity `:64` |
| | SSRF 守卫（通用下载 + 出站 URL 入口） | parity `:51`（`core/net` + `media/redirect`） |
| idmap | redb 单库（替代 bbolt）+ gRPC 服务端 | parity `:33`、`:36` |
| | 17 个同步 RPC；lotus 客户端（`RemoteBridge` 专职 OS 线程） | `docs/rust-m0.md:36`（R4l / P55-P1） |
| | HTTP 回退 type 全表 | parity `:36`（`5ca58f3`）；模块注释 `crates/gensokyo-storage/src/idmap/http_remote.rs:12` |
| | `/getid` type 1~19（5/12 走 idmap 重绑定；14 在现代 schema 恒空 `{"keys":null}`） | parity `:38` |
| 出站 URL / 二维码 | `transfer_url` 门控，模板默认 **true**（升级须知：默认就开，发的是外链替换后的内容） | 默认 `crates/gensokyo-config/templates/config.yml:186`；门控 `crates/gensokyo-runtime/src/url_pipeline.rs:275`、`crates/gensokyo-runtime/src/deferred.rs:386` |
| | `url_to_qrimage` **opt-in**（模板默认 `false`） | `crates/gensokyo-config/templates/config.yml:184` |
| | QR 图条**先发**、无 reply、`msg_type=7`；`depth` 防递归重入 | parity `:50`；`crates/gensokyo-runtime/src/deferred.rs:356` |
| 重试分类 | `22009`→SSM 入队；`40034025`→清空 `event_id` 重发一次；超时重试 3 次、`attempt×1s`；`40034026` 是 Go 死码**不移植** | parity `:84` |
| 短链 | 本地 / lotus 双 mint；`POST /url` + `GET /url/:shortURL`（301/404） | parity `:126`；`boot.rs::attach_short_url` |

## 4. 构建与验证（接手第一件事）

> [!WARNING]
> 三个必设项，缺一个就白跑一轮。

1. **工具链**：`rust-toolchain.toml` 钉 `1.99.1`；GNU 宿主必须显式 `rustup override set 1.99.1-x86_64-pc-windows-gnu`（`docs/rust-build.md:13`、`docs/rust-migrate.md:132`）。
2. **PATH 必须含 `C:\msys64\mingw64\bin`**：旧机实测 `gcc.exe` **只在** `mingw64\bin`，`usr\bin` 下**没有**（`docs/rust-migrate.md:89`）。
3. **`CARGO_TARGET_DIR` 必须显式设、且指向仓外非系统盘**：`.cargo/config.toml` 只有 `[alias]`，不设就落仓内 `target/`（历史上留下过 68 GB，`docs/rust-build.md:85-88`、`docs/rust-migrate.md:131`）。

| # | 命令 | 通过标准 | 别被什么骗 |
|---|------|----------|------------|
| 1 | `cargo test --workspace --no-fail-fast` | 日志出现 `test result: ok.`，0 failed | PowerShell 的 `Exit code: 1` 常是 stderr 伪影，**只看 `test result:`**（`docs/rust-migrate.md:179`） |
| 2 | `cargo clippy --workspace --all-targets -- -D warnings` | 末尾 `Finished`，零告警 | 同上，不看退出码字面 |
| 3 | `cargo build --release -p gensokyo-bin` | `Finished` 且产出二进制 | 体积数字与 §2 不同属正常，看实际打印 |
| 4 | `cargo fmt --all -- --check` | **退出码 0**（无 diff 输出） | 会改写文件的只有 `cargo fmt --all`（无 `--check`） |

**本机约束（团队约定）**

- 只测 **win64 宿主**，不跑交叉矩阵（9 目标矩阵归 CI，`docs/rust-build.md:212`）。
- 编译缓存用完即删：`Remove-Item -Recurse -Force $env:CARGO_TARGET_DIR`。
- 磁盘水位（写本文时点：C ≈ 55 GB、D ≈ 131 GB 可用，**以命令为准**）：
  `Get-PSDrive C,D | Select-Object Name,@{n='FreeGB';e={[math]::Round($_.Free/1GB,1)}}`

## 5. 运维必读（上线前）

| 风险 | 后果 | 处置 | 证据 |
|------|------|------|------|
| `panic = "abort"` | 任何 panic（含 tokio 任务内）**直接退出进程**；SSM 待补发队列、echo 缓存、延迟队列全丢 | **必须有 supervisor**：systemd `Restart=always` / Windows 服务恢复策略 / 容器 `restartPolicy` | `Cargo.toml:144`；`docs/rust-build.md:176-179` |
| 同上（隔离失效） | `catch_unwind` 的 handler 隔离**只在 dev/test（unwind）有效** | 不要把「单测能覆盖 panic→1500」当成生产兜底 | `crates/gensokyo-runtime/src/dispatcher.rs:28-30`、`docs/rust-build.md:181-182` |
| 短链不迁移 | Go `gensokyo.db` 的 `shortURLs` 不导入，**旧短码 404** | 上线后重新生成；没有导入器 | `docs/rust-build.md:232-234`；parity `:130` |
| `/getid` type 5/12 | 会改 ID 映射且**不可回滚** | 操作前备份 `openid-map.redb` | `docs/rust-build.md:235-238` |
| gRPC 两个 Update RPC | 错误码由 `Unimplemented` 变为 `NotFound` / `AlreadyExists` | 调用方按新码判分支 | `docs/rust-build.md:237-238` |
| lotus 远程模式 | 启动建连失败只 warn 降级本地（Go 是 panic）；成功后远端不可达**无自动切回** | 人工介入或重启；`lotus=true` 时主 listener 用 `backup_port` | `docs/rust-build.md:239-244` |
| 配置写回 | 加载期清洗 / 补键 / 首启模板三处共用**原子写**（`config.yml.<pid>.tmp` → rename），不会写一半留半截；写回前先自检可解析 | 备份策略按普通配置文件处理即可；不要手工制造「两个 `settings:`」验证 | parity `:132` |

## 6. 已知缺口与有意差异（「不要当成 bug」清单）

下面 16 条都是**已裁决 / 已登记**的差异：看到时先查证据列，不要直接按「和 Go 不一样」开修。真正待办的只在 §7。

| # | 项 | 说明 | 证据 |
|---|----|------|------|
| ① | `panic=abort` 语义 | 见 §5；这是刻意的性能取舍，不是 parity 缺口 | `Cargo.toml:144`、`docs/rust-perf.md:202-206` |
| ② | PGO / BOLT / UPX | PGO 在 Windows GNU **不可用**（`error[E0463] can't find crate for 'profiler_builtins'`）；BOLT 仅 Linux；发布链仍是 plain release + UPX（PGO 未接 `xtask build`） | `docs/rust-perf.md:290`、`:315-323`、`:310-311` |
| ③ | 二维码 / 压缩图 parity 粒度 | QR 是 **矩阵级**（`qrcode.High`、无静区、Go 尺寸语义），PNG **不可能逐字节一致**；图片压缩类用例按像素级断言比对，不追字节 | parity `:50`、`:64` |
| ④ | 无 DNS pinning | SSRF 校验在 **URL 字符串层**（`core/net` + `media/redirect`），解析后重绑定不在防护范围；**与 Go 同口径**，不按缺口排期 | parity `:51` |
| ⑤ | lotus HTTP 回退**不带 token** | 照抄 Go 死代码 `buildGetIDURL`（全仓零调用者，`idmap/service.go:46`）；Rust 侧模块注释自述 | `crates/gensokyo-storage/src/idmap/http_remote.rs:12`；Go `idmap/service.go:46` |
| ⑥ | `/getid` 按**实际状态**隐藏 | Go 是 `lotus_grpc=true` 一律不挂；Rust 是「服务端真起或远程真挂」才隐藏，降级保留挂载 | parity `:36` 末句 |
| ⑦ | 三类路径**不挂** URL 变换 | `execute_sse`、SSM 补发、WS 掉线兜底——**与 Go 同**，已登记 | parity `:50` 末句 |
| ⑧ | `get_qq_profile` 明确不做 | 即 `GET /users/@me`；真缺口表裁决「不做」 | `reports/refactor/16-official-api-audit.md:62`、`:64` |
| ⑨ | 约 25 个标准 OneBot action 中 **15 个是 1500 占位** | 实现集中在 `crates/gensokyo-actions/src/unsupported.rs`（模块头表逐项列明原因，`9` 个专用 handler + 各模块 `checked_unsupported` / `unsupported()` 调用点）；`docs/rust/actions.md` 在官网仓，本仓以代码为准 | parity `:137`；`unsupported.rs:1-40` |
| ⑩ | `/getrealvalues` 两侧都无服务端注册 | Go 只拼客户端 URL（`idmap/service.go:1956`），Rust 照抄；不要「补一个服务端」当修 bug | Go `idmap/service.go:1956`；`http_remote.rs:887` |
| ⑪ | 短链 token 自指校验 | Go `isValidToken` 比的是 `hex(HMAC(pwd, token)) == token`，客户端送 `HMAC(pwd, 长链)`，两端口径从来不一致 ⇒ `lotus_password` 非空时**创建一律 400**；修要两端同时改 | parity `:131` |
| ⑫ | `/url` 的挂载位置与 301 差异 | Rust 绑 WS listener（`enable_ws_server=true` 才有），否则落 HTTP API 端口；301 **无响应体**，多段路径 `/url/a/b` 落 WebUI SPA（Go 是 404） | parity `:126` |
| ⑬ | `user_info` 重复键用独立错误变体承载 | Go 的 `StoreUserInfo` 遇重复键回 `fmt.Errorf("duplicate key: %s", key)` 字符串错误；Rust 是 `StorageError::DuplicateKey(String)`（载荷为**完整 key**，`Display` 与 Go 文案**逐字节一致**）——**不复用 `Corrupt`**（重复键是稳态、不代表库损坏，借 `Corrupt` 会逼调用方把真损坏一起吞掉），也不借只装 vUIN 的 `AlreadyExists`；入站因此能只把重复键压 debug、真损坏保持 warn；gRPC `status()` 归 `already_exists`。**看到 `duplicate key` 日志是稳态，不是 bug，不要当损坏告警去修** | `crates/gensokyo-storage/src/error.rs`（`DuplicateKey`）、`idmap/mod.rs:799-824`、`idmap/grpc.rs`；Go `idmap/service.go:2272`；登记 `docs/rust-m0.md` R4p |
| ⑭ | C2C 空 `Author.ID`：Rust 只「不写库」，不整条早退 | Go `Processor/ProcessC2CMessage.go:30-33` 对 `data.Author == nil \|\| data.Author.ID == ""` 是 `return nil`——整条事件**既不上报也不入库**；Rust 只取「不写库」这一半（`record_user_info` 进门判空串直接返回，**只判空、不 trim** ⇒ 与 Go 两个子条件等价），事件上报字节不变。取舍：改上报形态会动外部 JSON，收益不抵风险；不补守卫则会写出键 `":"` 的垃圾身份行（Go 库绝无此行） | `crates/gensokyo-runtime/src/inbound.rs:756-760`；Go `Processor/ProcessC2CMessage.go:30-33`；登记 `docs/rust-m0.md` R4p |
| ⑮ | 入站不做 alias 替换与 visual 前缀剥离 | Rust 入站两条路径都不做 Go `message_parser.go:2003-2024` 的 alias 替换与 visual 前缀剥离，框架指令/黑白名单前缀场景仍与 Go 不同——**既有范围限定，未随 P72 改变**，不要当成 P72 的回归 | Go `handlers/message_parser.go:2003-2024`；Rust 入站无对应实现（`gensokyo-runtime` 的 `revert_mentions` 只做 @，`reported_message` 只做落段/折回判定） |
| ⑯ | `CodeSpec::inbound` 已无生产读取点 | **P73 之后**：数组路径不再解析 CQ（`revert_mentions` 写 NUL 哨兵 + 槽位表、`expand_at_placeholders` 只按哨兵切片直出 text/at 段），`reported_message` 的 `cqcode::parse` 调用已删 ⇒ `CodeSpec::inbound` 与 `params`/`outbound` 同属**描述性元数据**（仅测试模块读）。`at` 也已置 `false`，手打 `[CQ:at,qq=…]` 与 Go 一样按原文留 text；真实 @ 仍由 `<@!hex>` 占位符产 at 段 | `crates/gensokyo-protocol/src/cqcode.rs` 的 `CodeSpec` 文档块与 `name: "at"` 条目（按符号定位）；`crates/gensokyo-runtime/src/inbound.rs` 的 `revert_mentions`/`expand_at_placeholders`；登记 `docs/rust-m0.md` R4s、parity B4 |

## 7. 未完成项与下一步

| 优先级 | 项 | 现状 | 下一步 | 证据 |
|--------|----|------|--------|------|
| ✅ | P64b（`get_friend_list` **写侧**，含评审收口） | **已完成**：storage `store_user_info`（键 `"<rawID>:<user_id>"` 与 value 三字段紧凑 JSON 逐字对齐 Go `service.go:2270-2276`、复用读侧同一个 `UserInfoRow`、单写事务且不得在 `with_write_batch` 闭包内调用）+ 入站 C2C 臂接线（`record_user_info`，写失败不阻断上报）；评审收口另改两处——① 重复键改由**独立变体** `StorageError::DuplicateKey(String)` 承载（`Display` 与 Go 文案逐字节一致，不再借 `Corrupt`；日志分级随之修正：重复键 debug、真损坏 warn；gRPC `status()` 补 `already_exists` 臂），② 空 `Author.ID` 补守卫（原先会写出键 `":"` 的垃圾身份行）。⇒ 纯 Rust 部署该表**在运行期自然积累**，`data:null` 只剩「没导入也没收过私信」的初始态 | —（写侧无遗留；差异见 §6 ⑬⑭） | `crates/gensokyo-storage/src/idmap/mod.rs:811`、`src/error.rs`、`src/idmap/grpc.rs`、`crates/gensokyo-runtime/src/inbound.rs:756`（调用点 `:1106`）；Go `Processor/ProcessC2CMessage.go:231-241` + `idmap/service.go:2267-2287`；登记 `docs/rust-m0.md` R4p、parity B6 好友行 |
| ✅ | P69（CQ 码表元数据校准） | **已完成**（只动 `cqcode.rs`）：`ark.params` 由 `["data"]` 改为**入站三键** `["ark_type","ark_name","prompt"]`（Go 入站 CQ 文本只有三键 `message_parser.go:2496-2498`；`fields` 只进段 JSON 的 `data`；出站对 ark 整段剥离不读参数 ⇒ 旧值两个方向都不成立）；`CodeSpec` 三字段消费者写实（`params`/`outbound` **描述性、无生产消费者**，`inbound` **是行为开关**）；新增两条行为断言 `ark_inbound_cq_form_carries_three_keys`、`face_cq_text_is_preserved_verbatim`。**注意**：本行当时保留的「`face` 维持 `inbound: true`」结论**已被 P72 推翻并收假**（见下一行），而 `face_cq_text_is_preserved_verbatim` 钉的是 parse/`to_cq` 的字节保真、与落段落段无关，故不受收位影响 | —（元数据无遗留；由它牵出的产出集口径已由 P71/P72 收口） | `crates/gensokyo-protocol/src/cqcode.rs` 码表 `name: "ark"` 条目（`params` 三键）、`CodeSpec` 结构体文档块、同文件测试模块的 `CODES` 往返用例与 `code_spec(…).outbound` 断言（该文件行号随码表增删浮动，按符号定位）；登记 `docs/rust-m0.md` R4q、parity B4 |
| ✅ | P71 → P72（入站 `inbound` 位口径：手打 CQ 折回 text） | **已完成，口径已裁决并实施**（不再是「未裁决」）：判定依据是 Go `ConvertToSegmentedMessage`（`handlers/message_parser.go:2253-2334` + `:2340` 的段类型集合）对正文**只做整段切片 + `<@OpenID>` 正则、完全不解析用户手打的 CQ** ⇒ `cqcode.rs` 收 **18 条码位 true→false**（reply/image/record/video/file/face/markdown/card/keyboard/stream/input_notify/music/active/wakeup/node/poke/ark/json，每条行内注明依据），`CodeSpec::inbound` 文档改写为「只回答手打的码要不要落段」；`crates/gensokyo-runtime/src/lib.rs` 的模块文档按代码事实改写（image/record/ark/reply 已按 `attachments`/`msg_elements` 产段，只有**视频附件**仍不产段），旧表述（称 image / record / video / ark 四类段尚未产出）作废。影响面唯一：`code_spec(…).inbound` 生产读取点只有 `inbound.rs` 的 `reported_message` 一处，`image_segments`/`message_event_segments` 在其之后 `extend` ⇒ 结构化附件与卡片上报不受收位影响（混排用例实证）。测试：`array_report` 三视图助手 + `array_value_folds_every_inbound_false_code_verbatim`（20 例，断言 `message` 文本 == `raw_message` 逐字节）+ `array_value_keeps_structured_image_segment_while_folding_typed_image_code`，并纠正旧用例（原 `array_value_parses_at_and_multiple_segments` → 现名 `array_value_keeps_at_segment_and_folds_typed_face_code`，5 段→3 段）；`cargo test --workspace` **43 suites / 1373 passed / 0 failed**（P72 时点实测，lead 独立复核通过；当前基线见 §2）、clippy 零告警、`build --release` Finished、`fmt --all --check` 退出码 0 | —（无遗留：`at` 已由 P73 收口；`poke` 已于 2026-11-14 复扫确证） | parity B4 末三行（`已对齐` 一行 + `at` 的 `部分` 一行 + `CodeSpec` 一行）；登记 `docs/rust-m0.md` R4r、`release_log/CHANGELOG_v016.md` P72 子节；`cqcode.rs` 与 `tests/inbound.rs` 一律按符号定位 |
| ✅ | P73：`at` 位的结构改造 | **已完成（2026-11-14）**：数组形态改「Go 同款 NUL 哨兵 `"\x00ATPH%d\x00"` 正文 + at 槽位表」，`reported_message` 数组臂删除 `cqcode::parse`、由 `expand_at_placeholders` 在「条件 trim → 去首个 `/`」之后按哨兵切片直出 text/at 段（顺序与「找不到哨兵就跳过、不中断」照 Go）；at 段用 `Segment::new("at").param("qq", …)` 构造（不用 `Segment::at`，避免 id 规范化风险）；`push_text_segment` 随最后一个 parse 调用点删除（全仓 grep 零调用）；码表 `at.inbound` 置 false，`CodeSpec::inbound` 降级为描述性元数据。测试：Go `TestInboundAtOrderPreserved` 的 **8 组 wantStr/wantArr 逐字移植** + 交叉不变式 `rejoin_at_text` + 非 hex `<@all>`/前导零/超 i64/哨兵丢失/连续多 @ 五类边界。门禁：`cargo test --workspace` **43 suites / 1379 passed / 0 failed**（lead 独立复核）、clippy 零告警、release Finished、fmt 退出码 0 | —（无遗留；`text` 码表条目仍 `inbound: true` 属描述性、无行为影响） | `crates/gensokyo-runtime/src/inbound.rs`（`revert_mentions`/`at_placeholder`/`emit_at`/`expand_at_placeholders`/`reported_message`）、`crates/gensokyo-protocol/src/cqcode.rs` 码表 at 条目、`crates/gensokyo-runtime/tests/inbound.rs`；登记 `docs/rust-m0.md` R4s、parity B4「`at` 段（P73 已对齐）」行 |
| ✅ | `poke` 收位依据复扫 | **已完成并确证（2026-11-14，非乐观推断）**：`rg -i "poke"` 全 Go 仓仅 3 处命中（`readme.md:153` 的 CQ 码表宣称 + `docs/api/扩展api/扩展api-get_robot_share_link.md:45,48` 的 nonebot 用法示例），**`.go` 文件零命中**；`Processor/` 对 `REACTION\|Reaction\|GROUP_POKED\|message_poke\|qq_poke` **零命中**，botgo 事件常量表内无 `POKED`/`GROUP_POKED`（只有频道表态 `EventMessageReactionAdd/Remove`，非戳一戳）⇒「常量存在但零消费」；Rust 侧 19 处命中全为码表条目/`Segment::poke` builder（零调用）/`NoticeKind::Poke` 事件分类（与 CQ 段无关）/passthrough 注释，生产路径既不产也不消费 poke 段。判定核心：`inbound` 只影响「`array_value=true` 时用户手打码要不要落段」，Go 对同输入整段留 text ⇒ 收位后一致。**须如实记录的覆盖不对称**：此结论靠源码级事实成立，但 **Go 侧没有对应入站段数组用例**（最接近的 `handlers/cq_legacy_semantics_test.go:171-175` 是出站文本保留断言，不能替代）；Rust 侧用例 `array_value_folds_every_inbound_false_code_verbatim` 的 poke 断言点为「确被 parse 成 `kind=="poke"` 码段 → 整段折回单条 text → `raw_message`/`message` 逐字节等于原文」 | 遗留「Go 缺测试钉住」**已于 2026-11-14 补齐**（主仓 `handlers/inbound_handtyped_cq_test.go`，提交 `39a4d5b`） | 登记 parity B4「`inbound` 位」行与 `docs/rust-m0.md` R4r 遗留①（已收敛） |
| ✅ | P70（测试加固：`url_pipeline` lotus stub） | **已完成**（同批提交）：`lotus_stub_replaces_with_remote_link_inplace` 按仓库既有「仅连接层中止才重试、上限 3 次」口径加固——每轮清空 stub 请求记录与 api 发送记录；判定指纹从「错误文本」下移到「stub 侧未收到完整 `POST /url`」（`LotusMint::mint` 按 Go 口径吞掉发送错误），并要求先有记录才算被掐断（封掉 `all()` 对空集合恒真的白跑路径）；业务失败不参与重试 | —（其余用例加固另需再立项，本行只覆盖 lotus stub） | `crates/gensokyo-runtime/tests/url_pipeline.rs:478`（用例）、`:506`（重试判定）；同口径先例 `tests/lotus.rs`、storage `http_remote.rs` 的 10053 处理 |
| P2 | 文档欠账 | `release_log/CHANGELOG_v016.md` 的「✅ 提交记录」段自 `eca1843` 起未维护 | 回填提交记录（`git log --oneline eca1843..HEAD`，排除 merge/仓库卫生类） | `release_log/CHANGELOG_v016.md` |
| ✅ | 入站用例（P75①，原 P2） | **已完成（2026-11-14，主仓提交 `39a4d5b`）**：新建 `handlers/inbound_handtyped_cq_test.go`——poke/json/node/face 断言「段数=1、类型=text、载荷与原文逐字节一致」，并断言手打 `[CQ:image]` 留 text 而同条真实 `attachments` 仍产 image 段；复用既有 `atAssertConsistency` 校验 string/array 两路径同序 | —（已闭环） | 主仓 `handlers/inbound_handtyped_cq_test.go`；`go test ./handlers` ok、`go build ./...` 退出码 0 |
| ✅ | 主仓文档与代码不符（P75②） | **已完成（2026-11-14，主仓提交 `39a4d5b`）**：`readme.md` 三处「戳一戳」已标注「未实现，仅码表占位 / 仅列表占位」（CQ 码表 `[CQ:poke]`、通知事件群内戳一戳、拓展 Event 好友/群内戳一戳），不删行、不虚报；本仓 `readme.md` 同名表述同步标注 | —（已闭环） | 主仓 `readme.md:153/297/310-311`；依据：全仓 `*.go` 产品代码 `poke` 零命中、botgo 无 `POKED` 类常量 |
| — | 迁移/交接 | 换机清单只在 `docs/rust-migrate.md` | 本文**不重复**，按那份执行 | `docs/rust-migrate.md` |
| 不排期 | 视频抽帧 / 转码 / 压缩 | Go 侧是死代码且函数自身破损（全仓零调用者），**不可移植**；视频发送链本身已覆盖 | 保持关闭，不要当缺口重开 | parity `:49` |

## 8. 接手流程（团队约定）

1. **实现 → 独立测试 → 评审 → 门禁 → 提交**：实现者不给自己写验收结论，测试与评审由另一角色独立出证据。
2. **中文提交信息必须走文件**：`git commit -F <UTF-8 文件>`；**禁用** PowerShell `Set-Content` 改源码/临时拼提交信息（编码会坏，`AGENTS.md:91`）。
3. **一次波次一笔 commit**：同一波次的多个小修合并成一笔；不同请求不塞进同一笔。
4. **评审发现 Critical / Major 必修才能合**，Minor 可登记留痕（parity 报表的「已登记差异」就是这种留痕）。
5. 改动前先跑一次 §4 的门禁建立基线；改动后只在**相关**范围复跑，再用全量门禁收尾。
6. **破坏性操作要用户授权**：`git push --force`、改写已推送历史的 rebase、`git reset --hard`、`git checkout --` / `restore` 丢改动，一律先问（`AGENTS.md`「禁止的破坏性操作」节）。

## 9. 常用命令速查

| 目的 | 命令 | 看什么判成功 |
|------|------|--------------|
| 全量测试 | `cargo test --workspace --no-fail-fast` | `test result:` 行；同时记下 suites / passed / ignored 三个数（§2 的基线取数就靠它） |
| 单 crate / 单用例 | `cargo test -p gensokyo-runtime url_pipeline -- --nocapture` | `test result: ok. N passed` |
| 跑被忽略的用例 | `cargo test --workspace -- --ignored` | 只有显式加了 `#[ignore]` 的用例会跑，别混进日常基线 |
| 单 crate clippy | `cargo clippy -p gensokyo-runtime --all-targets -- -D warnings` | 改动面小时用它，比全量快 |
| 静态检查 | `cargo clippy --workspace --all-targets -- -D warnings` | 末尾 `Finished`，无 warning |
| 格式门禁 | `cargo fmt --all -- --check` | 退出码 0 且无 diff |
| release 构建 | `cargo build --release -p gensokyo-bin` 或 `cargo xtask build --no-upx` | `Finished`；xtask 会打印产物路径与 MB |
| 交叉矩阵（别在本机跑） | `cargo xtask build --all` | CI 的 9 目标矩阵，本地只测 win64 宿主 |
| 追某能力现状 | `cargo xtask build --help` / 直接读 parity 表 | 先查 `reports/refactor/17-go-rust-parity.md`，再回代码 |
| 核对未回填提交 | `git log --oneline eca1843..HEAD` | 与 `CHANGELOG_v016.md` 的「✅ 提交记录」段比对 |
| 看缓存与水位 | `echo $env:CARGO_TARGET_DIR`；`Get-PSDrive C,D` | 缓存不在仓内、不在系统盘；水位够一次全量构建 |
