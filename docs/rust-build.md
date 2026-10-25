# Rust 构建链（`cargo xtask build`）

构建入口只有一个：`cargo xtask build`（等价于 `cargo run -p xtask -- build`，别名来自 `.cargo/config.toml`）。
`build.ps1` 已退化成薄包装，只把 Go 命名的参数翻译成 Rust 三元组后转发给 xtask，**不再依赖 Go 工具链**。

对应实现：`xtask/src/build.rs`；Go 版口径参考仓库根 `build.ps1`（改造前版本）与 `.github/workflows/cross_compile.yml`。

## 前置条件

| 项 | 说明 |
|----|------|
| Rust 工具链 | `rust-toolchain.toml` 钉 `1.99.1`；MSRV 见根 `Cargo.toml`（1.90） |
| 链接器 | 宿主 abi 必须有可用链接器：MSVC 需 VS Build Tools（`link.exe`），GNU 需 mingw。本仓库当前开发机走 GNU：`rustup override set 1.99.1-x86_64-pc-windows-gnu` |
| `webui/dist` | **`rust-embed` 在编译期读取该目录**，目录缺失会直接编译失败。它以 `.gitignore` 忽略，xtask 会在缺失时自动写入 5 个占位文件（与旧 `build.ps1` 的 `Ensure-WebUIDist`、CI 占位步骤同一组文件） |
| 前端产物（可选） | `cd frontend && npm run build` 会产出真实的 `webui/dist`；存在 `webui/dist/css/style.css` 时 xtask 不再写占位文件 |
| UPX（可选） | 默认尝试压缩；PATH 里没有 `upx` 只提示并跳过，不是构建失败 |

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
| `x86_64-unknown-linux-gnu` | linux | amd64 | `gensokyo-linux-amd64` | |
| `i686-unknown-linux-gnu` | linux | 386 | `gensokyo-linux-386` | |
| `armv7-unknown-linux-gnueabihf` | linux | arm | `gensokyo-linux-arm` | |
| `aarch64-unknown-linux-gnu` | linux | arm64 | `gensokyo-linux-arm64` | |
| `x86_64-apple-darwin` | darwin | amd64 | `gensokyo-darwin-amd64` | 需 macOS + Xcode CLT |
| `aarch64-apple-darwin` | darwin | arm64 | `gensokyo-darwin-arm64` | 同上 |
| `x86_64-pc-windows-msvc` | windows | amd64 | `gensokyo-windows-amd64.exe` | |
| `i686-pc-windows-msvc` | windows | 386 | `gensokyo-windows-386.exe` | |
| `aarch64-linux-android` | android | arm64 | `gensokyo-android-arm64` | **实验性**（M0 裁决 2：失败不阻塞合入） |

`--all` 与 `--linux-only` 的语义：

- 逐个目标执行；`rustup target list --installed` 里**没有**的目标打印
  `[skip] <triple> 未安装 → rustup target add <triple>` 后继续，**不计失败**。
- 已安装但编译失败的目标记入失败清单并继续跑其余目标，结束时以非零退出码汇总
  （真编译错误不会被当成"环境缺失"吞掉）。
- 宿主不在矩阵里时不会自动补进来；单独构建宿主用不带 `--all` 的命令。
- 机器上没有 `rustup`（系统包管理器安装的 cargo）时无法预检，`--all` 会真实尝试每个目标，
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

## CI 用法

```yaml
- run: cargo run --locked --package xtask -- build --target x86_64-unknown-linux-gnu --no-upx
```

`--locked` 落在 `cargo run` 上（同 `check-tls-floors` job 的理由：防止 cargo 在校验前改写 `Cargo.lock`）。
交叉编译前记得 `rustup target add <triple>` 并准备对应链接器 / `cross` 容器。

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
| 交叉编译 | Go 自带 | 需 `rustup target add` + 链接器/`cross` |

## 验证命令

```powershell
cargo +1.99.1-x86_64-pc-windows-gnu test -p xtask
cargo +1.99.1-x86_64-pc-windows-gnu clippy -p xtask --all-targets -- -D warnings
cargo +1.99.1-x86_64-pc-windows-gnu run -p xtask -- build --help
cargo +1.99.1-x86_64-pc-windows-gnu run -p xtask -- build --no-upx   # 宿主 release 构建
```

单测只覆盖纯函数（三元组解析、产物命名、版本矩阵、占位目录），不启动 cargo，因此不需要交叉工具链。
