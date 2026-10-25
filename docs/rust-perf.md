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
| 编译口径 | `profile.release` / `profile.bench`（lto=thin、codegen-units=1、opt-level=3） | 基准必须测发布口径，debug 数字没有意义 |
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


## 5. 发布配置

`Cargo.toml` 根部：

```toml
[profile.release]
lto = "thin"
codegen-units = 1
opt-level = 3
strip = "symbols"

[profile.bench]   # 与 release 同档，避免「基准跑的是另一套编译参数」
lto = "thin"
codegen-units = 1
opt-level = 3
strip = "symbols"
```

- **不设 `panic = "abort"`**：runtime 的 handler 隔离依赖 `catch_unwind`，abort 会让单个 handler 的 panic 拖垮整个进程。
- `strip = "symbols"` 只影响产物体积；要 profile 就临时加 `RUSTFLAGS="-C debuginfo=1"`，不要改发布 profile。

## 6. 后续优化清单

| # | 事项 | 落点 | 预期收益 | 验证方式 |
|---|------|------|---------|---------|
| 1 | **CQ 解析零拷贝**：`cqcode::parse` 目前每段都建 `String`（类型名 + 每个参数），改成借用 `&str` 的中间表示（`Cow` / 带生命周期的 `SegmentRef`）再按需转 owned | `crates/gensokyo-protocol/src/cqcode.rs` | 解析路径分配次数从 ~3/段降到 0–1/段，基线 a 有望降 30%+ | 基线 a 前后各 3×100 000 次取中位数 |
| 2 | **`Arc<str>` 替代 `String`**：action 名、`message_type`、昵称等高频字段在 registry / dispatcher / event 之间被反复 clone | `gensokyo-protocol`、`gensokyo-runtime` | clone 退化为原子自增，事件分发路径不必逐字段深拷贝 | 基线 b + 分发路径集成测量（serde 需自定义 `Deserialize`，注意别为此牺牲可读性） |
| 3 | **缓冲复用**：入站 WS 帧解析与出站 JSON 序列化每次新建 `Vec<u8>` / `String` | `gensokyo-runtime` 传输层、`gensokyo-qq` | 减少高频小分配；`bytes::BytesMut` 已在 workspace 依赖中 | 基线 c + 分配计数（后续可加 `#[global_allocator]` 计数版临时 bench） |
| 4 | **连接池**：出站 HTTP 复用 `reqwest::Client` 与 keep-alive，禁止每 action 新建 | `gensokyo-qq` | 省掉每次 TLS 握手，长尾延迟收益远大于热点路径 | 网络路径 bench 覆盖不到，需集成测量（本地 mock server 跑 P50/P99） |
| 5 | **jemalloc 评估**：Windows 上 jemalloc 需要完整 MSVC/编译工具链，落地成本高；先对照 `mimalloc`（更易构建、Windows 支持好） | `gensokyo-bin` 入口 | 分配密集场景（基线 a/c）吞吐与 RSS 的对照数据 | 三档 allocator 各跑 3 次基线 a/b/c + 常驻 RSS；结论写回本文 |

**通用要求**：任何一项落地前先记录「改前」中位数，落地后用同一 `--iters`、同一机器重跑，
数字写回 §4（保留历史行，别覆盖），否则优化无从复核。

---

## 7. 尚未覆盖（待补）

- **action 分发基线**：`Registry` 正在从同步 `Handler` 改为异步（并行重构中），同步 dispatch 基线会被推翻，
  等异步签名定稿后补一条 `tokio` 单线程 runtime 下的 dispatch 用例。
- 内存峰值 / 分配次数：当前 bench 只测时间。
