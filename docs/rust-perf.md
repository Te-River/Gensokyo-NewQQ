# Rust 侧性能基线

> 适用范围：`crates/` 下的 Rust workspace（与 Go 主链无关）。
> 本文记录基准的方法论、复跑方式、当前数字与后续优化清单；改热路径前后各跑一次做对照。

---

## 1. 目的与边界

- **回答的问题**：「这条路径是不是慢了一个量级」，不是统计显著性，也不是 CI 门禁。
- **覆盖**：能在单进程内同步跑完的三条热路径（CQ 解析、事件反序列化、出站规划）。
- **不覆盖**：tokio 调度、网络 / 磁盘 IO、QQ 上游 RTT、内存峰值与分配次数。
  这些需要集成测量，不能拿本文的数字代替。
- 基准实现在 `xtask/src/bench.rs`，样本数据是该文件顶部的三个常量，改样本等于改基线。

## 2. 方法论

| 决策 | 取值 | 原因 |
|------|------|------|
| 计时 | `std::time::Instant` | 零依赖；只求量级，不追求亚纳秒分辨 |
| 预热 | 1000 次 / 用例 | 摊掉首次分配、惰性初始化、分支预测冷启动 |
| 正式迭代 | 默认 100 000，`--iters` 可改 | 单用例 0.1–2 s，够稳又快 |
| 防优化 | `std::hint::black_box` 包住输入与返回值 | 否则 LTO 会把整个循环消掉，测出 0 ns |
| 哨兵值 | 闭包返回 `len()` / `bool` | 计时区间内不做额外分配，避免把测量开销算进去 |
| 编译口径 | `profile.release`（`xtask bench` 走 `cargo run --release`，被测代码编进释放档二进制） | 基准必须测发布口径，debug 数字没有意义；仓内当前**没有 `cargo bench` 消费者**，`[profile.bench]` 只是同口径保留，见 §5 |
| 不引入 criterion | — | 本机是共享环境（并行构建、后台任务），置信区间只会制造「精确的错觉」；固定迭代 + 多次取中位数足够判断量级 |

**样本坏掉要在计时前炸**：事件 JSON 与 `send_group_msg` 参数在开始计时前各解析/规划一次，
失败直接返回错误，避免出现「很快但什么都没测」的假基线。

## 3. 如何复跑

```powershell
# 默认 100 000 次迭代（务必带 --release；cargo xtask 别名不带 --release）
cargo run --release -p xtask -- bench

# 自定义迭代数（与历史数字对照时必须用同一 --iters）
cargo run --release -p xtask -- bench --iters 20000
```

输出三列关键数据：`iters`、`total_ms`、`ns/op`。
**噪声约 ±10%**：连续跑 3 次取 `ns/op` 中位数，别拿单次数字下结论。

### 本机环境注意（2026-10-25 实测）

该机器没装 MSVC 工具链（`link.exe` 缺失），默认的 msvc host 构建会直接失败在链接阶段。
可用的构建口径是已安装的 GNU host 工具链 + mingw `dlltool`（rustc 的 `raw-dylib` 路径需要它）：

```powershell
$env:RUSTUP_TOOLCHAIN = '1.99.1-x86_64-pc-windows-gnu'
# <mingw-bin> 指含 dlltool.exe 及其依赖 DLL 的目录（MSYS2 mingw-w64 binutils 包解压到临时目录即可，勿放进仓库）
$env:PATH = "<mingw-bin>;$env:PATH"
cargo run --release -p xtask -- bench
```

缺 `dlltool.exe` 时的报错特征：`error calling dlltool 'dlltool.exe': program not found`，
或 `failed to add native library ...kernel32.dll_imports.lib`。

## 4. 当前基线

测量时间 2026-10-25，`windows-x86_64`，rustc 1.99.1（GNU host），release 口径见 §2。
三次独立运行各 100 000 次迭代，取中位数（括号内为三次区间）：

| 用例 | iters | 中位 ns/op | 三次区间（ns/op） | 单次总耗时 |
|------|-------|-----------|------------------|-----------|
| `cqcode::parse`：20 段混合 CQ 文本 → `Vec<Segment>` | 100 000 | **15 771** | 15 711 – 16 991 | ≈ 1.58 s |
| `Event::deserialize`：典型 `GROUP_MESSAGE_CREATE`（数组形态消息） | 100 000 | **4 942** | 4 932 – 5 004 | ≈ 0.49 s |
| `plan_send_group_msg`：5 段消息参数 → `SendRequest` | 100 000 | **6 794** | 6 673 – 7 031 | ≈ 0.68 s |

单次 `--iters 20000` 对照：14 747 / 4 609 / 6 388 ns/op —— 与 100 000 次相差约 7%，
属共享机器抖动；**跨迭代数比较没有意义**，对照时固定 `--iters` 与机器。

读法：CQ 解析 ≈ 790 ns/段（20 段），是三者中最贵的一条，也是 §6 的第一优化对象。

### 4.1 优化后（2026-10-25）

| 用例 | 优化前 | 优化后 | 提升 | 口径 |
|------|--------|--------|------|------|
| `cqcode::parse` | 14.7 µs | ≈ 7.3 µs | ≈ 2.0× | 同进程 A/B，交替 20 轮取 min |
| `Event::deserialize` | 6.92 µs | 4.15 µs | −40% | 同进程 A/B，10 次中位 |
| `dispatch_json`（get_status） | 68.8 µs | 20.9 µs | −69.6% | debug/current_thread，20k 次 |

说明：CQ 剩余成本是 `Segment` 形状固定的 169 次分配（≈10 µs 地板），进一步优化需把参数改成借用/Cow 形态；`Event` 仍有 `Segment::Scalar` 的 untagged 缓冲热点。两者都属 API 级改动，留待后续评估。


### 4.2 同负载端到端对比（Rust vs Go，2026-10-25）

> **历史口径**（`lto=thin`、无 mimalloc、`oha` 压测）：最新一次同机复测见 **§4.4**（2026-11-10，最终 release + 最新 Go）。

**环境**：AMD Ryzen 5 5600U（6C/12T）、13.8 GB、Windows 11 26300、Go 1.26.4、rustc 1.99.1（release，**LTO=thin**——历史数字，当前口径已改为 `lto=fat`，见 §5）、oha 1.16.0。

**场景**：本地 `get_login_info`（无 QQ 网络 I/O）；Go 走 HTTP API 路径 `/get_login_info`，Rust 走统一 action 入口 `POST /action`；两边均 `disable_webui: true`、`log_level: 3`（error）、无反向 WS，请求体相同。64 并发、50,000 请求/轮，Go/Rust 交替跑 **10 轮**，取均值 ± 标准差（括号内为 10 轮区间），预热 3,000。

| 指标 | Go | Rust | 对比 |
|------|----|------|------|
| 吞吐 | 49,137 ± 4,085 req/s（41,954–56,059） | **50,581 ± 5,241 req/s（40,157–58,006）** | ≈1.03×，区间重叠 |
| 平均 | 1.295 ms | **1.264 ms** | ≈2.4% 更低 |
| P50 | **0.779 ms** | 1.144 ms | Go 低约 32% |
| P99 | 11.02 ms | **3.43 ms** | ≈3.2× 更低 |
| RSS | 43.4 MB | **10.8 MB** | ≈4.0× 更低 |
| 二进制 | 43.81 MB | **8.12 MB** | ≈5.4× 更小 |
| 成功率 | 100% | 100% | — |

**读法**：吞吐分布高度重叠，不能宣称 Rust 吞吐显著领先；稳定差异在尾部延迟（P99）、内存与体积，而 Go 的 P50 更低。高并发下单次均值受调度/后台进程影响，**必须多轮取均值并看标准差**。

**关键方法学**：Go 保持默认 `INFO` 且每请求打日志时，吞吐会掉到约 2.9k req/s（约 15× 差距），P99 升到百毫秒级；关掉后两端恢复同一量级。压测必须统一日志级别，否则结论完全失真。

**复现要点**：

- Go 侧 `config.yml` 用完整模板（`template/config_template.go`），`app_id: 12345`（Go 的免 token 启动分支）、`http_address: 127.0.0.1:5701`、`disable_webui: true`、`log_level : 3`。
- Rust 侧同模板，`app_id: 0`、`http_address: 127.0.0.1:5702`、`log_level : 3`。
- Go：`oha -c 64 -n 50000 -m POST -H "Content-Type: application/json" -d '{"action":"get_login_info","params":{},"echo":1}' http://127.0.0.1:5701/get_login_info`
- Rust：同 body，URL 换 `http://127.0.0.1:5702/action`。

**边界**：loopback 单机、入口形态不同（Go 路径分发 vs Rust 统一 action）、Rust 尚未覆盖全部 action/媒体；数字用于管线对比与回归基线，不是公网 SLA，也不代表 QQ 端到端延迟。

### 4.3 全局分配器（P33，mimalloc，2026-11-09）

**决策**：`mimalloc 0.1.52`，`default-features = false`（该 crate 默认特性本为空；`override` 会顶掉 C 侧
`malloc` 符号、`secure` 加元数据校验，都不是本仓要的语义）。依赖声明在根 `Cargo.toml` 的
`[workspace.dependencies]`，由 `crates/gensokyo-bin` 引用，`#[global_allocator] static GLOBAL: MiMalloc`
落在 `crates/gensokyo-bin/src/main.rs` —— **只有最终二进制会装**，库 crate 与 `xtask` 不受影响
（所以 `xtask bench` 的默认口径仍是系统分配器，与 §4 同口径；但两次测量的机器状态不同，
绝对值仍以本节复测为准）。

**工具链细节**：本机 rustc 1.99.1 里 `global_allocator` **不是** unsafe 属性，写成
`#[unsafe(global_allocator)]` 会报 `error: 'global_allocator' is not an unsafe attribute`；普通写法也不会
触发 workspace 的 `unsafe_code = "forbid"`。构建需要 C 工具链（本机 mingw gcc 16.1.0）。

**度量口径**：生产分配器在 `gensokyo-bin` 里，`xtask` 是独立二进制、量不到它，因此 A/B 时把同一
`#[global_allocator]` **临时**挂到 `xtask/src/main.rs`（取完数已移除，最终树里没有这段）。同一机器、
同一 `--iters 100000`、独立 `CARGO_TARGET_DIR`，系统档 n=8（首轮 3 + 复测 5）、mimalloc 档 n=3。

| 用例 | 系统分配器 中位 ns/op（区间） | mimalloc 中位 ns/op（区间） | 变化 |
|------|------------------------------|----------------------------|------|
| `cqcode::parse` 20 段混合 | **10 485**（10 164 – 12 086） | **5 169**（4 900 – 5 185） | **−51%** |
| `Event::deserialize` 群消息 | **3 276**（3 186 – 4 150） | **2 578**（2 194 – 3 146） | **−21%** |
| `plan_send_group_msg` 5 段 | **7 026**（6 883 – 10 463） | **3 757**（3 046 – 4 294） | **−47%** |

**读法**：三条都远超本机 ±10% 噪声（`cqcode` 两档区间完全不重叠，两档最小值之间也差近 2×）。三条用例
都是分配密集路径（`cqcode` 一轮约 169 次分配），Windows CRT 堆的跨线程争用被 mimalloc 的每线程堆吃掉。
注意本次系统档实测 10.5 µs 与 §4 记录的 15.8 µs 不同，说明 §4 的数字还叠加了当时的机器状态差异；
**跨会话比较仍然只能自己重测**，别把 §4 的历史值直接当对照。§6 第 1 条那类"把分配降到 0"的改动，
收益上限会随本节落地而收窄，复测必须带同一分配器口径。

**度量陷阱（本轮实测踩到）**：bench 期间并行跑 `cargo clippy` / `cargo test`，系统档立刻从 10.5 µs
飙到 23.9–33.7 µs —— **同一档位能差 3×**。任何 A/B 都必须独占机器，否则会把编译争用读成分配器收益。

**产物与构建耗时**：`cargo build --release -p gensokyo-bin` 产出 `gensokyo.exe` = 10 791 936 B
（10.29 MiB / 10.79 MB）；依赖已缓存时本轮"编译本 crate + fat-LTO 链接"耗时 5 m 50 s。相对 P31 记录的
10.06 基线：若该数是 MiB 则 +2.3%，若是十进制 MB 则 +7.3%（mimalloc 静态库带来的一次性体积，无运行期开销）。
RSS 收益本节未测（§8 已列"内存峰值 / 分配次数未覆盖"）。

### 4.4 同负载端到端复测（Rust 最终 release vs 最新 Go，2026-11-10）

**环境**：同一台本机（AMD Ryzen 5 5600U 6C/12T、Windows 11）、Go 1.26.10（`main` 仓 `0c151b6`，`go build` 产物 44.82 MiB）、Rust `Refactor` 仓 `2805da2`（release：`lto=fat` + `panic=abort` + mimalloc，产物 10.31 MiB）。

**口径**：本地 `get_login_info`（无 QQ 网络 I/O）；Go 走 `/get_login_info`，Rust 走 `POST /action`；两边 `app_id: 12345`、`disable_webui: true`、`log_level: 3`、无 ws/webhook，请求体相同。64 并发、每轮 49,984 请求（`50000/64` 向下取整 × 64）、keep-alive、预热 3,000（c=8）、各跑 10 轮；**先 Go 全程单独跑完并关停，再 Rust 单独跑**（期间无并行编译/子代理，满足独占口径）。压测器为 Go stdlib 自写（`loadgen`，`oha` 本机不可用），两侧用同一二进制。

| 指标 | Go（main@0c151b6） | Rust（@2805da2） | 对比 |
|---|---|---|---|
| 吞吐（10 轮均值） | 44,939 req/s（42,700–46,717） | **45,584 req/s（43,396–46,436）** | ≈1.014×，区间重叠 |
| 平均 | 1.40 ms | **1.38 ms** | ≈1.4% 更低 |
| P50（各轮均值） | **1.12 ms**（1.07–1.19） | 1.14 ms（1.12–1.17） | Go 低约 1.8% |
| P99（各轮均值） | 5.43 ms（5.26–5.82） | **4.93 ms**（4.60–5.35） | ≈9.2% 更低 |
| RSS | 39.8 MB | **17.3 MB** | ≈2.3× 更低 |
| 二进制 | 44.82 MiB | **10.31 MiB** | ≈4.3× 更小 |
| 成功率 | 100%（499,840） | 100%（499,840） | — |

**读法**：吞吐与均值基本打平（区间高度重叠，不能宣称 Rust 显著领先）；稳定差异在**尾部延迟（P99 ≈ −9%）、内存（≈2.3× 更低）与体积（≈4.3× 更小）**，Go 的 P50 略低（≈2%）。与 §4.2（2026-10-25）相比绝对值整体下移（该轮 Rust 50.6k / Go 49.1k）——**不同会话的压测器与机器状态不同，跨会话绝对值不可比**，只用于本次同机同轮次的横向对照。

**与 §4.2 的口径差异**：①压测器由 `oha` 换成本仓自写 Go 工具（stdlib，同 body、同并发、同 keep-alive）；②Rust 编译口径由 `lto=thin` 改为 `lto=fat` + `panic=abort` + mimalloc（§4.3）；③Go 用官方 `-faststart`（跳过 `InitBase` 的安全启动弹窗与控制台标题，不影响 HTTP 链路）；④每轮请求数 49,984（按并发数整除）。

**复现要点**：

```powershell
# Go（必须先单独跑完；无控制台环境下 safelaunch 会弹窗，-faststart 跳过 InitBase）
.\run-perf.ps1 -Exe <go>\gensokyo-go.exe -Port 5701 -WorkDir <go-dir> -Label go -ActionPath /get_login_info -ServerArgs '-faststart'
# Rust（关停 Go 并静置后再跑）
.\run-perf.ps1 -Exe <rust>\gensokyo.exe -Port 5702 -WorkDir <rust-dir> -Label rust -ActionPath /action
```

## 5. 发布配置

`Cargo.toml` 根部：

```toml
[profile.release]
lto = "fat"
codegen-units = 1
opt-level = 3
strip = "symbols"
panic = "abort"

[profile.bench]   # 与 release 同档，避免「基准跑的是另一套编译参数」
lto = "fat"
codegen-units = 1
opt-level = 3
strip = "symbols"
```

- **`panic = "abort"`（P31 用户裁定）**：省掉展开表、异常路径更短。代价与原先的取舍相反——
  runtime 的 handler 隔离靠 `dispatcher.rs` 的 `catch_unwind`，abort 下它不再生效：任何 panic
  （含 tokio 任务内）**直接终止进程**，不再是「只丢这一条请求」。要恢复隔离就删掉这一行。
  Go 侧在 action 汇聚点 `callapi.go:271-272` 用 `defer mylog.RecoverPanic(...)` 兜住 handler
  panic，只丢当前请求；Rust release 在 P31 后没有这一层兜底——这是刻意的性能取舍，不是 parity 缺口。
- `[profile.bench]` 当前**没有 `cargo bench` 消费者**：本仓基准统一由 `xtask bench` 以
  `--release` 跑，测的是 `profile.release`；该段只保留 lto/cgu/opt 同口径，将来直接启用
  `cargo bench` 时不用另调。
- `panic` 对 `[profile.bench]` 被 cargo 忽略（`warning: panic setting is ignored for bench profile`），
  因此 bench 段不写该字段；将来若启用 `cargo bench`，其展开语义由 cargo 固定为 unwind。
- `strip = "symbols"` 只影响产物体积；要 profile 就临时加 `RUSTFLAGS="-C debuginfo=1"`，不要改发布 profile。

## 6. 后续优化清单

| # | 事项 | 落点 | 预期收益 | 验证方式 |
|---|------|------|---------|---------|
| 1 | **CQ 解析零拷贝**：`cqcode::parse` 目前每段都建 `String`（类型名 + 每个参数），改成借用 `&str` 的中间表示（`Cow` / 带生命周期的 `SegmentRef`）再按需转 owned | `crates/gensokyo-protocol/src/cqcode.rs` | 解析路径分配次数从 ~3/段降到 0–1/段，基线 a 有望降 30%+ | 基线 a 前后各 3×100 000 次取中位数 |
| 2 | **`Arc<str>` 替代 `String`**：action 名、`message_type`、昵称等高频字段在 registry / dispatcher / event 之间被反复 clone | `gensokyo-protocol`、`gensokyo-runtime` | clone 退化为原子自增，事件分发路径不必逐字段深拷贝 | 基线 b + 分发路径集成测量（serde 需自定义 `Deserialize`，注意别为此牺牲可读性） |
| 3 | **缓冲复用**：入站 WS 帧解析与出站 JSON 序列化每次新建 `Vec<u8>` / `String` | `gensokyo-runtime` 传输层、`gensokyo-qq` | 减少高频小分配；`bytes::BytesMut` 已在 workspace 依赖中 | 基线 c + 分配计数（后续可加 `#[global_allocator]` 计数版临时 bench） |
| 4 | **连接池**：出站 HTTP 复用 `reqwest::Client` 与 keep-alive，禁止每 action 新建 | `gensokyo-qq` | 省掉每次 TLS 握手，长尾延迟收益远大于热点路径 | 网络路径 bench 覆盖不到，需集成测量（本地 mock server 跑 P50/P99） |
| 5 | **分配器对照**：`mimalloc` 已落地（§4.3：a −51% / b −21% / c −47%）；jemalloc 在 Windows 需要完整 MSVC 工具链，本机不可评估 | `gensokyo-bin` 入口 | mimalloc 的时间收益已兑现；剩余缺口是 RSS 与分配次数的对照 | 系统档 / mimalloc 档各跑 3 次基线 a/b/c（数字见 §4.3）+ 常驻 RSS；**必须独占机器**，并行编译会让同一档位差 3× |
| 6 | **共享 HTTP 传输（P41 已落地）**：`gensokyo_runtime::boot::shared_transport()` 用 `OnceLock` 缓存首个**成功**构建的 `Arc<ReqwestTransport>`，QQ 网关 / action 客户端 / post_url 上报 / union_webhook 扇出四处共用同一个 `reqwest::Client`；构建失败不入缓存（下次重试，各调用点降级语义不变）。边界：四条链路共用**同一个**连接池上限，一条链路占满空闲连接时其他链路要排队；URL / 超时 / 头 / 重试 / 代理 / 证书口径均未改 | `crates/gensokyo-runtime/src/boot.rs`（+ `reporter` / `webhook` / `gensokyo-bin/src/startup.rs` 三个调用点） | 启动期 Client 构建 4 次 → 1 次；运行期 webhook 每条回调不再新建 Client，keep-alive 才真正复用（省 TLS 握手，长尾收益大于热点路径） | §4 微基准覆盖不到（走网络）；实例等价性用 `Arc::ptr_eq`（`gensokyo-bin` 的 `shared_transport_returns_same_instance` 用例）证明 |

**通用要求**：任何一项落地前先记录「改前」中位数，落地后用同一 `--iters`、同一机器重跑，
数字写回 §4（保留历史行，别覆盖），否则优化无从复核。

---

## 7. PGO / BOLT（提效，可选）

> 两者都是「不改代码、只改编译产物」的优化：PGO 让编译器知道哪些分支/函数真的热，BOLT 按采样重排二进制的代码布局。
> 本仓把 PGO 做成可复现的子命令；BOLT 只在 Linux 上可用，不进流水线（§7.4）。

### 7.1 PGO 流水线（`cargo xtask pgo`）

实现见 `xtask/src/pgo.rs`，五步：

| 步 | 命令 | 作用 |
|----|------|------|
| 0 基线 | `cargo run --release -p xtask -- bench --iters N` | 未优化口径的 ns/op，作为对比起点 |
| 1 插桩构建 | `RUSTFLAGS="-Cprofile-generate=<dir>" cargo build --release -p gensokyo-bin` | 每个函数入口插计数 |
| 2 跑负载 | 默认同上 bench（带同一条 `RUSTFLAGS`），或 `--workload <cmd>` | 产出 `<dir>/*.profraw` |
| 3 合并 | `llvm-profdata merge -o <dir>/merged.profdata <dir>` | 多份 profraw → 一份 profile |
| 4 profile-use 重建 | `RUSTFLAGS="-Cprofile-use=<merged> -Cllvm-args=-pgo-warn-missing-function" cargo build --release -p gensokyo-bin` | 按热度重排分支/内联；profile 缺函数时告警 |
| 5 复测 | 同第 0 步 | 前后对比写入 `<dir>/pgo-report.md` |

```powershell
# 默认负载：100 000 次迭代的 bench，profile 与报告落在 <target-dir>/pgo
cargo run --release -p xtask -- pgo

# 快速试跑（只想确认流水线通不通）
cargo run --release -p xtask -- pgo --iters 5000

# 自定义负载：命令在 workspace 根目录、带插桩 RUSTFLAGS 执行（跑压测框架时用这个）
cargo run --release -p xtask -- pgo --workload "<压测命令，例如 §4.2 的 oha 调用>"

# 换 profile 目录（默认 target/pgo）
cargo run --release -p xtask -- pgo --profile-dir D:\pgo
```

- **`llvm-profdata` 探测顺序**：`rustc --print sysroot` + `rustc -vV` 的 `host:` →
  `<sysroot>/lib/rustlib/<host>/bin/llvm-profdata[.exe]`；找不到再退回 PATH 里的同名工具。
  都没有时报错并提示 `rustup component add llvm-tools-preview`（用 `+<toolchain>` 时要带 `--toolchain`）。
- 探测放在**基线之后、插桩构建之前**：基线 bench 不依赖 `llvm-profdata`，先跑它保证每次运行
  都留下对比数字；探测失败时只损失一次基线，不会白跑十几分钟的插桩构建 + 负载。
- 自定义 `--workload` 会**跳过基线 bench 与前后对比**（没有基准数字），报告里只有插桩 /
  merge / profile-use 重建的结果；`--iters` 与 `--workload` 互斥，此路径下不生效。
- 每轮负载前清空插桩目录里的 `.profraw`：混入上一轮的 profile 会让 merge 失败，或把旧负载算进来。
- 负载跑完没有 `.profraw` 直接报错：说明负载执行的不是插桩产物（自定义 `--workload` 绕过 cargo 时最常见）。

### 7.2 测量口径与边界

- 对比必须**同一 `--iters`、同一机器、同一次运行内的两个进程**；本机噪声约 ±10%（共享环境冷启动时可达 ~30%），
  单轮差异落在噪声内不算收益，看 3 轮中位数。报告文件里的数字是单轮的，只用于判断量级。
- 默认负载是 `xtask bench`，只覆盖 `gensokyo-protocol` / `gensokyo-actions` 的**同步热路径**：
  服务端主循环、tokio 调度、网络栈不在 profile 里，那部分 PGO 收益为 0。要吃满收益就用 `--workload` 跑真实压测。
- profile 与源码/rustc 版本绑定：升级工具链后必须重新采集，别复用旧 `merged.profdata`（函数指纹变了会退化成无优化，`-pgo-warn-missing-function` 只负责告警）。
- 与 §5 的 `lto = "fat"` 叠加时，插桩与重建两轮都要走全量 LTO，构建时间明显上升；CI 上建议把 `target/` 与 `pgo/` 一起缓存。

### 7.3 本机实测（2026-11-09，Windows / GNU 宿主：不可用）

环境：HEAD `328189f` + P37 待提交工作树、`rustc 1.99.1 (58cf8f9c3 2026-10-13)`、
host `x86_64-pc-windows-gnu`、`cargo run --release -p xtask -- pgo --iters 200`。

| 步 | 结果 |
|----|------|
| 0 基线 bench | `cqcode::parse` **10 023.5** / `Event::deserialize` **3 410.0** / `plan_send_group_msg` **6 623.5** ns/op（`--iters 200`，与 §4 的 100 000 次口径不可直接比；同机复跑波动可达 ~10–28%，只看量级） |
| 探测 llvm-profdata | **失败**（预期）：`未找到 llvm-profdata：先执行 rustup component add llvm-tools-preview …`；命令在此退出，**基线数字已经产出** |
| 1 插桩构建（手工分步） | **失败**：`error[E0463]: can't find crate for 'profiler_builtins'` / `the compiler may have been built without the profiler runtime`（连 `cfg-if` 这类依赖都编译不过） |

说明：前两行来自**同一次 `xtask pgo` 运行**——当前顺序是「基线 → 探测 → 插桩」，缺
`llvm-profdata` 时基线数字照样输出，不会白跑。第 3 行是**手工分步执行**（前一次实测；跳过
探测，直接 `RUSTFLAGS="-Cprofile-generate=<dir>" cargo build --release -p gensokyo-bin`）
复现的；在本机当前顺序下命令走不到插桩这一步。

结论：**`x86_64-pc-windows-gnu` 的 rustc 发行版不带 profiler runtime**，`-Cprofile-generate` 在本机无法插桩，
因此拿不到 PGO 前后数字；本机同时也没装 `llvm-tools-preview`（merge 步骤缺工具）。
`xtask pgo` 在这两种情况下都给可操作提示，不会静默跳过或伪造数字。

可行路径：

```bash
# Linux CI（x86_64-unknown-linux-gnu）：完整流水线；PGO 产物尚未接入 build.ps1/xtask build
rustup component add llvm-tools-preview
cargo xtask pgo --iters 20000          # 等价 cargo run --release -p xtask -- pgo
# 报告：target/pgo/pgo-report.md；要留档就把它作为 CI artifact 上传
```

PGO 产物**尚未接入 `build.ps1` / `xtask build`**：发布链仍是 plain release + UPX。要在发布物里用
PGO，需手工以 profile-use 口径构建，或把接线作为独立事项排期。

Windows 侧只能换 msvc 宿主（本机缺 `link.exe`，见 §3），或等 rustc 给 gnu 宿主补上 profiler runtime。

### 7.4 BOLT（Linux-only，可选，不进流水线）

- **做法**：`llvm-bolt` 用 perf 采样得到的 `perf.data`（`perf2bolt` 转 `perf.fdata`）重排函数布局、
  拆分 hot/cold 段、做 `-reorder-blocks=ext-tsp -reorder-functions=hfsort`。
- **为什么只在 Linux**：需要 `perf record` 与 `llvm-bolt`，还要求 `perf_event_paranoid` 允许采样；
  Windows 没有可用实现（也没有等价的采样后端），所以本仓不做 Windows BOLT。
- **前置条件**：二进制要能解析符号——`RUSTFLAGS="-C debuginfo=1 -C force-frame-pointers=yes"` 构建，
  且**不要** `strip = "symbols"`（§5 默认 strip，BOLT 前临时关掉）。
- **预估收益**：服务端这种循环密集负载，QPS +5–15%、P99 略降；体积 +5–10%。与 PGO 互补（PGO 改代码生成，BOLT 改布局）。
- **何时值得**：发布前最后一步、构造稳定、已有可复现压测；**不值得**：功能还在变、负载不具代表性、容器里拿不到 perf 权限。

### 7.5 该做 / 不该做

| 该做 | 不该做 |
|------|--------|
| 用稳定、覆盖真实热点的负载（§4.2 的 oha 场景）采集 | 拿 bench 的 3 条微基准代表整个服务（只覆盖库层同步路径） |
| 收益用 ≥3 轮中位数确认，`--iters` 固定 | 拿单轮 ±3% 的差异当结论 |
| 把 PGO 放进发布流水线（可重复、与源码/rustc 版本绑定） | 把 PGO 产物当通用二进制分发到别的机器 |
| 同时验证 §4 基线没有退化（PGO 只改布局，不该变慢） | 用 PGO 掩盖算法/分配问题（§6 那些仍要做） |

---

## 8. 尚未覆盖（待补）

- **action 分发基线**：`Registry` 正在从同步 `Handler` 改为异步（并行重构中），同步 dispatch 基线会被推翻，
  等异步签名定稿后补一条 `tokio` 单线程 runtime 下的 dispatch 用例。
- 内存峰值 / 分配次数：当前 bench 只测时间。
