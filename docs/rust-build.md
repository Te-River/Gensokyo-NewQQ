# Rust 构建链（`cargo xtask build`）

构建入口只有一个：`cargo xtask build`（等价于 `cargo run -p xtask -- build`，别名来自 `.cargo/config.toml`）。
`build.ps1` 已退化成薄包装，只把 Go 命名的参数翻译成 Rust 三元组后转发给 xtask，**不再依赖 Go 工具链**。

对应实现：`xtask/src/build.rs`；Go 版口径参考仓库根 `build.ps1`（改造前版本）与 `.github/workflows/cross_compile.yml`。

## 前置条件

| 项 | 说明 |
|----|------|
| Rust 工具链 | `rust-toolchain.toml` 钉 `1.99.1`；MSRV 见根 `Cargo.toml`（1.90） |
| 链接器 | 宿主 abi 必须有可用链接器：MSVC 需 VS Build Tools（`link.exe`），GNU 需 mingw。本仓库当前开发机走 GNU：`rustup override set 1.99.1-x86_64-pc-windows-gnu` |
| Linux 交叉工具链 | **自动引导，无需预装**：linux-gnu×4 目标构建前 xtask 自动探测/下载 zig + cargo-zigbuild（见下节）；rustup target 缺失时自动 `rustup target add` |
| 系统工具（仅首次下载需要） | `curl`、`tar`（Windows 10+ 自带两者；Linux 需发行版自带，zip 解压另可回退 `unzip`）、sha256 校验用 `certutil`（Windows）/ `sha256sum`（Linux）。全部系统自带，**不新增任何 cargo 依赖** |
| `webui/dist` | **`rust-embed` 在编译期读取该目录**，目录缺失会直接编译失败。它以 `.gitignore` 忽略，xtask 会在缺失时自动写入 5 个占位文件（与旧 `build.ps1` 的 `Ensure-WebUIDist`、CI 占位步骤同一组文件） |
| 前端产物（可选） | `cd frontend && npm run build` 会产出真实的 `webui/dist`；存在 `webui/dist/css/style.css` 时 xtask 不再写占位文件 |
| UPX（可选） | 默认尝试压缩；PATH 里没有 `upx` 只提示并跳过，不是构建失败 |

## Linux 交叉工具链自动引导（zig + cargo-zigbuild）

linux-gnu×4 目标（`x86_64` / `i686` / `armv7-gnueabihf` / `aarch64`）改用 `cargo zigbuild`：
zig cc 充当交叉 C 编译器，ring 等含 C 代码的依赖不再需要各平台原生工具链。
引导逻辑在 `xtask/src/bootstrap.rs`，**跨平台（Windows / Linux 宿主均可）**，流程：

1. **探测**：先 PATH（`where.exe` / `which`，命中后跑 `zig version` / `cargo-zigbuild --version`
   与钉死版本比对，不符则 warn 并忽略 PATH 命中），再 `<workspace>/.tools/` 缓存
   （`.tools/zig-<版本>/`、`.tools/cargo-zigbuild-<版本>/`，已 gitignore）；命中即用，不重复下载。
2. **缺失则下载**：按宿主平台（x86_64/aarch64 × Windows/Linux）选官方 URL，
   `curl --fail --location --retry 3 --connect-timeout 30 --max-time 900` 下载到 `.tools/`。
3. **校验**：cargo-zigbuild 下载官方同名 `.sha256` 并用 `certutil`/`sha256sum` 逐字比对
   （不符时清理归档与 `.sha256` 残留后报错）；
   zig 官方下载索引不提供逐文件 sha256，退化为非空检查 + tar 解压完整性校验（损坏即报错）。
4. **解压**：`tar -xf`（Windows 自带 bsdtar 同时支持 zip 与 tar.xz；Linux 解 zip 失败回退 `unzip`）。
   二进制在一级子目录时把该子目录**全部条目**提升到根部（zig 包的 `lib/` 必须与可执行文件同目录，
   zig 按相对路径找它）；根部布局（cargo-zigbuild）不动。失败时清掉半成品目录，重试即重新下载。
5. **生效**：工具目录前置注入构建子进程的 PATH（不改 xtask 自身进程环境），
   随后 `cargo zigbuild --release --locked -p gensokyo-bin --bin gensokyo --target <triple>`。

`.tools/` 齐全时**零网络**；下载/校验失败直接报错（给出 URL 与原因），
**不静默回退到必败的 plain `cargo build`**。`--all` 矩阵模式下引导失败只把 linux 目标记入
失败清单，darwin/windows/android 目标照常继续。

### 钉死版本与官方 URL（勿换源/勿加代理）

zig **0.16.0**（cargo-zigbuild README 明确支持 0.15/0.16；0.17 兼容性未证实，勿升级）、
cargo-zigbuild **v0.24.1** 预编译二进制（免 `cargo install` 编译）：

| 宿主 | zig | cargo-zigbuild |
|------|-----|----------------|
| Windows x86_64 | `https://ziglang.org/download/0.16.0/zig-x86_64-windows-0.16.0.zip` | `https://github.com/rust-cross/cargo-zigbuild/releases/download/v0.24.1/cargo-zigbuild-x86_64-pc-windows-msvc.zip` |
| Windows aarch64 | `https://ziglang.org/download/0.16.0/zig-aarch64-windows-0.16.0.zip` | `https://github.com/rust-cross/cargo-zigbuild/releases/download/v0.24.1/cargo-zigbuild-aarch64-pc-windows-msvc.zip` |
| Linux x86_64 | `https://ziglang.org/download/0.16.0/zig-x86_64-linux-0.16.0.tar.xz` | `https://github.com/rust-cross/cargo-zigbuild/releases/download/v0.24.1/cargo-zigbuild-x86_64-unknown-linux-gnu.tar.xz` |
| Linux aarch64 | `https://ziglang.org/download/0.16.0/zig-aarch64-linux-0.16.0.tar.xz` | `https://github.com/rust-cross/cargo-zigbuild/releases/download/v0.24.1/cargo-zigbuild-aarch64-unknown-linux-gnu.tar.xz` |

cargo-zigbuild 每个包另有同名 `.sha256` 校验文件（自动下载比对）。
macOS / 32 位宿主没有官方预编译源：引导会报错并提示手动安装 zig 与 cargo-zigbuild 加入 PATH。

### 版本覆盖开关（环境变量）

| 变量 | 默认 | 说明 |
|------|------|------|
| `GENSOKYO_ZIG_VERSION` | `0.16.0` | 覆盖 zig 版本（只影响下载 URL 与缓存目录名） |
| `GENSOKYO_CARGO_ZIGBUILD_VERSION` | `0.24.1` | 覆盖 cargo-zigbuild 版本 |

只接受 `数字[.数字]*` 形式的版本号（按 `.` 分段、每段非空纯数字，拒绝 `.`/`1..2`/`.5` 等）；非法值直接报错。例：
`$env:GENSOKYO_ZIG_VERSION="0.15.2"; .\build.ps1 -LinuxOnly`。

### 不自动引导的目标（维持现状）

darwin×2 仍需 macOS + Xcode CLT；windows-msvc×2 仍需 VS Build Tools；
android 仍需 NDK（实验性）。这些目标缺工具链时按原口径报错/跳过，不自动下载。
linux-musl 也不在引导范围（cargo-zigbuild 只路由 linux-gnu；musl 需自备工具链）。

## 迁移到新机器（整仓搬运）

> 完整的换机清单（搬运体积实测数字、新机前提校验命令、门禁模板、验收基线、已知坑、自检脚本）见 [rust-migrate.md](./rust-migrate.md)；本节只留构建链视角的要点。

- **只需拷贝/克隆仓库本身**：`target/`、`target-*/`、`.tools/` 都在 `.gitignore` 里，不必带
  （`target/` 可达数 GB）。走 git 克隆更省：`git clone -b Refactor <repo> && cd <repo>`。
- 新机前提：装好 rustup（`rust-toolchain.toml` 钉 1.99.1，首次 `cargo` 会自动装工具链）、
  `curl` + `tar`（Windows 10+ 自带）。**不需要**预装 zig / cargo-zigbuild——首次
  `.\build.ps1 -LinuxOnly` / `cargo xtask build --linux-only` 会自动下载到 `.tools/`
  （见上文「自动引导」），之后缓存命中零网络。
- **构建缓存已移出仓库**：本项目的 `CARGO_TARGET_DIR` 统一指向 `D:\rust-target\gensokyo`（不在仓内、
  不在系统盘）。它是纯缓存，可随时整目录删除换取空间，代价是下次冷编译一次。新机自定义位置时
  记住两件事：`protoc` 构建脚本对跨盘路径敏感（同一分区最稳），且别在 Go 主仓目录里跑 cargo
  （历史上曾在此留下 68 GB 的 `target/`）。
- 宿主构建直接可用；Linux×4 交叉由自动引导补齐；darwin / MSVC / android 仍需各自
  工具链或 CI（见目标矩阵备注）。
- 前端产物可选：`cd frontend && npm run build`；不构建时 xtask 自动写 `webui/dist` 占位文件。

## 本地构建

```powershell
# 宿主（不传 --target，交给 cargo 选当前 abi）
cargo xtask build

# 单个交叉目标（可重复 --target）
cargo xtask build --target aarch64-unknown-linux-gnu

# 精简版 + 跳过 UPX
cargo xtask build --no-webui --no-upx

# 全矩阵（双版本）/ 只 Linux
cargo xtask build --all
cargo xtask build --linux-only
```

薄包装的等价用法：

```powershell
.\build.ps1                      # 宿主完整版
.\build.ps1 linux amd64          # 映射到 --target x86_64-unknown-linux-gnu
.\build.ps1 -NoWebUI -NoUPX
.\build.ps1 -All
.\build.ps1 -LinuxOnly
```

`.\build.ps1` 的 `-UPXLevel`（默认 7）对应 `cargo xtask build --upx-level <1..9>`。

## 目标矩阵（`--all`）

对齐 CI 的 Go 矩阵；`os`/`arch` 是 Go 命名，用于产物文件名。

| 三元组 | os | arch | 产物 | 备注 |
|--------|----|------|------|------|
| `x86_64-unknown-linux-gnu` | linux | amd64 | `gensokyo-linux-amd64` | `cargo zigbuild`，工具链自动引导 |
| `i686-unknown-linux-gnu` | linux | 386 | `gensokyo-linux-386` | 同上 |
| `armv7-unknown-linux-gnueabihf` | linux | arm | `gensokyo-linux-arm` | 同上 |
| `aarch64-unknown-linux-gnu` | linux | arm64 | `gensokyo-linux-arm64` | 同上 |
| `x86_64-apple-darwin` | darwin | amd64 | `gensokyo-darwin-amd64` | 需 macOS + Xcode CLT（不自动引导） |
| `aarch64-apple-darwin` | darwin | arm64 | `gensokyo-darwin-arm64` | 同上 |
| `x86_64-pc-windows-msvc` | windows | amd64 | `gensokyo-windows-amd64.exe` | 需 VS Build Tools（不自动引导） |
| `i686-pc-windows-msvc` | windows | 386 | `gensokyo-windows-386.exe` | 同上 |
| `aarch64-linux-android` | android | arm64 | `gensokyo-android-arm64` | **实验性**（M0 裁决 2：失败不阻塞合入）；需 NDK（不自动引导） |

`--all` 与 `--linux-only` 的语义：

- 逐个目标执行；rustup target 缺失时**自动 `rustup target add --toolchain <当前工具链>`**
  （幂等）；补装失败才打印 `[skip] <triple> …` 后继续，**不计失败**。
- linux-gnu×4 目标走自动引导的 `cargo zigbuild`（见上节）：**裸机上 `-All` / `-LinuxOnly`
  的 linux 部分开箱即跑**，无需预装任何交叉工具链；首次运行会下载 zig + cargo-zigbuild
  到 `.tools/`（之后零网络）。
- 已安装但编译失败的目标记入失败清单并继续跑其余目标，结束时以非零退出码汇总
  （真编译错误不会被当成"环境缺失"吞掉）。
- 宿主不在矩阵里时不会自动补进来；单独构建宿主用不带 `--all` 的命令。
- 机器上没有 `rustup`（系统包管理器安装的 cargo）时无法预检/补装，`--all` 会真实尝试每个目标，
  缺失的目标表现为构建失败。

## `--no-webui` 的当前状态（重要）

Go 版用 `-tags=small` 裁剪 WebUI/OSS/gRPC/二维码。**Rust 侧目前没有任何 crate 定义 feature**，
`gensokyo-runtime` 无条件依赖 `gensokyo-webui`，因此 `--no-webui` 目前翻译为：

```
--no-default-features
```

在 `gensokyo-bin` 上它**当前是 no-op**：产物名带 `-noWebui`，但内容与完整版一致（WebUI 仍被嵌入）。
等 `small`/`webui` 特性落地后，只需改 `xtask/src/build.rs::cargo_build` 里的这一行，产物命名与参数
已经就位。

## 产物与目录

- 产物目录：`<workspace>/release/`（`.gitignore` 忽略），文件名
  `gensokyo-{os}-{arch}[-noWebui][.exe]`，与 Go 版逐字一致。
- cargo 原始产物位置从 `cargo metadata` 的 `target_directory` 推导，尊重 `CARGO_TARGET_DIR` 与
  `.cargo/config.toml`；交叉目标多一层 `<triple>/`。
- 只构建 bin 包：`cargo build --release --locked -p gensokyo-bin --bin gensokyo`
  （不用 `--workspace`，避免把 10 个库 crate 与 xtask 自身一起打包）。
- 日志打印每个产物的**实际路径与大小（MB）**；UPX 压缩在复制之后进行，最终汇总里的体积是压缩后的。

## release 的 panic 语义（运维必读）

`[profile.release]` 是 `panic = "abort"`（P31）：任何 panic（含 tokio 任务内）**直接退出进程**，
不会只丢当前请求。进程内的易失状态——SSM 待补发队列、echo 缓存、延迟队列 / 未完成计划——随
进程一起消失；生产部署必须有 supervisor / 服务自启（systemd `Restart=always`、Windows 服务恢复
策略或等价物），否则一次 panic 就是停机。

dev/test 仍是 unwind：`dispatcher.rs` 的 `catch_unwind` handler 隔离只在 unwind 口径有效，单测
能覆盖 panic→1500；release 下这层隔离退化为进程退出。

## CI 用法

```yaml
- run: cargo run --locked --package xtask -- build --target x86_64-unknown-linux-gnu --no-upx
```

`--locked` 落在 `cargo run` 上（同 `check-tls-floors` job 的理由：防止 cargo 在校验前改写 `Cargo.lock`）。
linux-gnu 交叉目标无需预装链接器 / `cross` 容器：xtask 自动引导 zig + cargo-zigbuild 并补装
rustup target（runner 需能访问 ziglang.org 与 github.com；缓存 `.tools/` 可跨 job 复用）。
darwin / MSVC / android 目标仍需各自工具链。

## 退出码

| 情形 | 退出码 |
|------|--------|
| 单目标构建成功、产物已复制 | 0 |
| 单目标构建失败（cargo 非零 / 产物缺失） | 非 0，`anyhow` 打印错误链 |
| `--all` 下有目标被跳过、其余成功且无失败 | 0 |
| `--all` 下有目标真编译失败 | 非 0，末尾列出失败目标 |
| 所有目标都被跳过（无任何产物） | 非 0 |
| UPX 存在但压缩失败 | 非 0 |
| UPX 不存在 | 0（打印安装提示） |

## 与 Go 版 `build.ps1` 的差异

| 项 | Go 版 | Rust 版 |
|----|-------|---------|
| 工具链依赖 | `go`、`GOPROXY` | 仅 `cargo`（+ 可选 `upx`） |
| `-All` 目标数 | 5（linux×3 + windows×2） | 9（对齐 CI 矩阵：linux×4、darwin×2、windows×2、android×1） |
| 版本信息 | `-ldflags -X buildinfo.BuildType/BuildSpec` 注入 git commit | **暂未注入**，版本取自 crate `version`（0.1.0）；等 `buildinfo` 落地再接线 |
| WebUI 占位 | 脚本内 `Ensure-WebUIDist` | xtask 内 `ensure_webui_dist`（同一组占位文件） |
| `-NoWebUI` | `-tags=small`，真实裁剪 | `--no-default-features`，当前 no-op（见上节） |
| 交叉编译 | Go 自带 | linux-gnu×4 自动引导 zig + cargo-zigbuild；其余目标需 `rustup target add` + 各自工具链 |

## 验证命令

```powershell
cargo +1.99.1-x86_64-pc-windows-gnu test -p xtask
cargo +1.99.1-x86_64-pc-windows-gnu clippy -p xtask --all-targets -- -D warnings
cargo +1.99.1-x86_64-pc-windows-gnu run -p xtask -- build --help
cargo +1.99.1-x86_64-pc-windows-gnu run -p xtask -- build --no-upx   # 宿主 release 构建
```

单测只覆盖纯函数（三元组解析、产物命名、版本矩阵、占位目录、引导的 URL/版本选择、
工具探测优先级、zigbuild 路由与命令构造），不启动 cargo、不发网络请求，因此不需要交叉工具链。

## 从 Go 版升级时的数据注意

- **短链不迁移**：Go 存在 `gensokyo.db` bucket `shortURLs` 里的短码不会导入 Rust 的
  `short-urls.redb`，旧短链在 Rust 版上直接 404，需要重新生成（idmap / echo 走 JSONL 导入，
  短链没有对应导入器）。
- **`/getid` type 5/12 会改 ID 映射，且不可回滚**：Rust 版这两个分支已是真实实现（重绑定/解绑，
  与 Go 相同的三段反查与"目标已存在"拒绝），操作会直接改写 `openid-map.redb`——操作前先备份该文件。
  同时 gRPC `UpdateVirtualValueV2` / `UpdateVirtualValueV2Pro` 由 `Unimplemented` 变为真实实现，
  错误码从 `Unknown` 变为 `NotFound` / `AlreadyExists`（对应 Go 的 `不存在:<old>` / `<new> :已存在`）。
- **lotus 远程模式运维注意（P55）**：`lotus=true` 时 idmap 17 操作走远端（gRPC 客户端或 HTTP 回退）。
  启动期建连失败只 **warn 降级本地**（Go 是 panic），`/getid` 保留挂载；启动成功后远端不可达按 Go
  语义返回错误，**无运行时自动切回本地**（避免双写分裂），需人工介入或重启。桥线程 panic 在 release
  `panic = "abort"` 下**直接退出进程**（见上文「release 的 panic 语义」），生产部署必须有 supervisor。
  `lotus=true` 时主 listener（正向 WS，缺席则 HTTP API 端口）改用 `backup_port`——Go 的 `backup_port`
  作用于主 HTTP 服务器 port，端口语义映射与完整差异清单见 `docs/rust-m0.md` R4l 条目。

接手本仓的全局视图（已落地能力 / 运维必读 / 已知差异 / 未完成项）：见 [`rust-handover.md`](./rust-handover.md)。
