# 换机迁移手册（Rust 重构仓整仓搬运）

目标：把本仓（含构建链）搬到另一台机器，照着清单走完即可恢复到可开发状态。

分工：**构建链本身的细节不在本手册**（自动引导流程/钉死版本 URL/目标矩阵/`--all` 双版本/退出码/CI 用法）一律看 [rust-build.md](./rust-build.md)；
本手册只回答三件事：**带什么、新机装什么、按什么顺序验证**。
本手册所有数字均为迁移前在旧机上实测所得；换机后请以给出的命令实际输出为准。

---

## 1. 要带什么、不用带什么

**主路径是「整包拷贝仓库根目录」**：新机不做 `git clone`，直接把整个文件夹搬过去。
`.git/`（实测 **111.9 MB**）一并带走，所以新机**不需要远端仓库权限**，历史、分支、`git blame` 全在，
未提交的工作也会原样跟过去——这正是拷贝优于 clone 的地方（两条路径的取舍见 §7）。

| 对象 | 实测大小 | 是否搬运 | 依据 |
|------|----------|----------|------|
| 工作树（不含 `target/`、`.git/`） | **23.7 MB** | 必带 | 全部源码 + fixture + 文档 |
| `.git/` | **111.9 MB** | 必带（整包拷贝天然带走） | 历史、分支、blame；`git clone` 也能重建，但**不含未提交工作** |
| `target/` | **282.82 MB**（2026-11-14 缓存清扫后实测；清扫前曾达 **3.67 GB**，体积随构建增长） | 排除 | 纯编译缓存，可再生；`.gitignore` 已覆盖 |
| `target-*/`（交叉目标缓存） | 写本手册时点**不存在** | 存在时排除 | `.gitignore` 含 `target/` 与 `target-*/` 两条；名单先枚举再传参，见下 |
| `.tools/`（zig / cargo-zigbuild 缓存） | 写本手册时点**不存在**（首次交叉构建才生成） | 存在时排除 | 已 gitignore，首次构建自动重新引导（见 §2） |
| `release/` | **4.12 MB**（随构建次数增长，以 §1 命令输出为准） | 排除 | 已 gitignore，`cargo xtask build` 可重出 |

排除名单与体积都用这两条命令现取，**别凭记忆写死路径**（没实证的路径写进 `/XD` 只是噪声）：

```powershell
# 1) 枚举仓内「当前真实存在」的待排目录——不存在的（如未交叉构建时的 .tools/）自然不进名单
Get-ChildItem -Directory | Where-Object { $_.Name -like 'target*' -or $_.Name -in '.tools','release' } | Select-Object -Expand FullName

# 2) 复核各目录体积（MB）：本表的大小列以这条的输出为准
Get-ChildItem -Directory | Where-Object { $_.Name -like 'target*' -or $_.Name -in '.tools','release' } | ForEach-Object {
    $s = (Get-ChildItem $_.FullName -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
    '{0}  {1} MB' -f $_.Name, [math]::Round($s / 1MB, 1)
}
```

> **`.gitignore` 不影响 robocopy——这是两个不同的机制，最容易踩。**
> gitignore 只决定 `git add` 收不收；拷贝工具根本不看它。
> `target/`、`target-*/`、`release/`、`.tools/` 四条虽然都在 `.gitignore` 里，
> 整包拷贝时**仍然必须自己排除**，否则编译缓存照样跟过去（本手册写作时点
> `target/` 实测 282.82 MB；清缓存前是 3.67 GB——量级完全取决于构建历史，
> 所以务必用上面的命令现取，别照抄表里的数字）。

拷贝命令（源路径与目标盘按新机实际改）：

```powershell
$skip = Get-ChildItem -Directory | Where-Object { $_.Name -like 'target*' -or $_.Name -in '.tools','release' }
robocopy D:\Github\Gensokyo-NewQQ-Refactor E:\Gensokyo-NewQQ-Refactor /E /XD @($skip.FullName)
```

> `/XD` 接的就是上面**枚举出来的**目录；**不需要 `/XF`**——要排的全是整目录，
> 仓内没有值得按文件名排除的东西。**`.git/` 必须留在拷贝里**，漏掉它新机就没有历史。
> 拷前请先走完 §7.1 的「有没有并发写入」检查。

构建缓存目录 `D:\rust-target\gensokyo`（旧机实测 **40 GB+**）在**仓外**，整包拷贝不会、也不需要把它带过去。
新机首次构建前重新选一个仓外、非系统盘的目录并设 `CARGO_TARGET_DIR`（位置约定见 §3）。

**结论：整包拷贝「工作树 23.7 MB + `.git/` 111.9 MB」≈ 136 MB 即可带走全部有效内容，且不依赖远端权限、包含未提交工作。**

只有当你决定走 `git clone` 备选路径（只想干净重来，见 §7）时，才需要先复核「有没有游离文件会漏掉」——两条命令输出均为空 ⇒ clone 不会漏东西：

```powershell
git status --porcelain --untracked-files=all
git ls-files --others --exclude-standard
```

> 迁移前实测结果：未跟踪且未被 ignore 的文件 **0 个**。换机当天请重跑这两条确认；
> 若有输出说明存在未入库内容——走整包拷贝（§7）即可原样带走，走 clone 则必须先入库。

---

## 2. 新机前提（最小集）

| 项 | 是否必需 | 校验命令 | 缺失时的处置 |
|----|----------|----------|--------------|
| `git` | 必需 | `git --version` | 装 Git for Windows / 发行版 git |
| `rustup` | 必需 | `rustup --version` | 官方 `rustup-init`；`rust-toolchain.toml` 钉 **1.99.1**，首次跑 `cargo` 会自动装该工具链（含 `rustfmt`/`clippy`，profile=minimal） |
| mingw 链接器（Windows GNU 宿主） | 必需 | `gcc --version` | PATH 前置 mingw 目录，见下 |
| `curl` + `tar` | 必需 | `curl --version` / `tar --version` | Windows 10+ 自带，供交叉工具链自动引导 |
| zig / cargo-zigbuild / protoc | **不需要预装** | — | xtask 首次构建 Linux×4 时自动下载钉死版本：zig **0.16.0**、cargo-zigbuild **v0.24.1**，官方源 + sha256 校验，落到 `.tools/`，二跑零网络。流程与 URL 见 [rust-build.md](./rust-build.md)「Linux 交叉工具链自动引导」 |
| Go 工具链 | **只有重新生成金样才需要** | — | `crates/gensokyo-protocol/tests/xurls_golden.json`、`crates/gensokyo-media/tests/qr_golden.json` 已作为 fixture 入库，日常跑测试不需要 Go |
| Node / 前端 | 可选 | — | `cd frontend && npm run build`；不构建也行，xtask 自动写 `webui/dist` 占位文件 |
| UPX | 可选 | `upx --version` | PATH 里没有只提示并跳过，不是构建失败 |

### 2.1 Windows 侧链接器（最容易踩的一条）

旧机实测：`gcc.exe` **只存在于** `C:\msys64\mingw64\bin\`，`C:\msys64\usr\bin` 下**没有** gcc。
所以每次跑门禁前都要 PATH 前置（**新机请换成新机自己的 msys64/工具链路径**）：

```powershell
$env:PATH = "C:\msys64\usr\bin;C:\msys64\mingw64\bin;" + $env:PATH
gcc --version   # 必须能打印版本，否则链接阶段必失败
```

Linux 宿主同理：先确认 `gcc --version` 可用（发行版自带 build-essential 一类），路径由系统 PATH 决定，无需手工前置。

### 2.2 `webui/dist` 与嵌入资源（务必确认）

`rust-embed` 在**编译期**读取 `webui/dist`，目录缺失会直接编译失败（Go 主仓侧同理：`go:embed` 缺该目录也失败）。
`build.ps1` / xtask 会在缺失时自动写入占位文件，CI 也有对应占位步骤，因此：
clean clone 后**不要**手工删除 `webui/dist`，也**不要**把它拷进搬运清单（它按 `.gitignore` 忽略，入库的是例外占位文件）。

---

## 3. 构建缓存的位置约定

`CARGO_TARGET_DIR` 统一指向**仓外、非系统盘**的目录；旧机为 `D:\rust-target\gensokyo`。
新机自选路径，但三条硬要求：

1. **空间充足**：该目录历史峰值实测 **40 GB+**。
2. **与仓库同分区或至少可用**：实测教训——曾经借用实现者临时副本里建的 target 目录，
   `protoc` 构建脚本报 `Could not make proto path relative` 而失败。跨盘/跨机器拼出来的路径组合不要赌。
3. **不要在 Go 主仓目录里跑 cargo**：旧机曾在 `D:\Github\Gensokyo-NewQQ\target` 留下 **68.52 GB** 纯缓存。

仓库的 `.cargo/config.toml` 只定义 `xtask` 别名，**没有** `[build] target-dir`，
所以缓存位置完全由环境变量决定——每条门禁命令前都必须设置它。

该目录是纯缓存，可随时整目录删除换空间，代价只是下次冷编译一次。

---

## 4. 环境变量与门禁命令模板（可直接粘贴）

Windows（PowerShell，先设 PATH 与缓存目录）：

```powershell
$env:PATH = "C:\msys64\usr\bin;C:\msys64\mingw64\bin;" + $env:PATH   # 新机换成新机路径
$env:CARGO_INCREMENTAL = "0"
$env:CARGO_TARGET_DIR  = "D:\rust-target\gensokyo"                    # 新机换成自选的仓外非系统盘目录
rustup override set 1.99.1-x86_64-pc-windows-gnu                      # GNU 宿主需显式钉 abi
```

Linux（bash，同一组语义；缓存目录同样要求仓外、非系统盘）：

```bash
export CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR=/path/outside/repo/rust-target/gensokyo   # 与仓库同分区最稳
gcc --version                                                     # 确认链接器在位
```

四条门禁命令（在工作树根目录依次执行）：

| # | 命令 | 通过标准 |
|---|------|----------|
| 1 | `cargo test --workspace --no-fail-fast` | 见 §5 基线；判定看日志 `test result:` |
| 2 | `cargo clippy --workspace --all-targets -- -D warnings` | 零告警，日志 `Finished` |
| 3 | `cargo build --release -p gensokyo-bin` | 成功产出 release 二进制 |
| 4 | `cargo fmt --all -- --check` | 退出码 **0** |

> 与 CI 对齐：`rust.yml` 的 clippy/test 带 `--locked`。本地迁移验收建议同样加 `--locked`
> （`cargo test --workspace --locked --no-fail-fast`），确保依赖解析没有偏移 `Cargo.lock`。

---

## 5. 验收基线（迁机后应当看到的数字）

| 门禁 | 基线（旧机 HEAD `69afb8d` 时点实测） |
|------|----------------------------------------|
| `cargo test --workspace --no-fail-fast` | **43 suites / 1379 passed / 0 failed**（2026-11-14 P73 收口后实测；数字随新增用例增长，以命令输出为准） |
| `cargo clippy --workspace --all-targets -- -D warnings` | **零告警** |
| `cargo fmt --all -- --check` | **退出码 0** |

CI 侧 `rust.yml` 在 push 到 `Refactor` / `main` 时跑：**fmt + clippy + msrv(1.90) + test(ubuntu/windows/macos) + audit + deny + tls-floors**，
本地四条门禁通过后再 push，CI 应当全绿。

**判定口径**：若新机的数字**低于**该基线（suite 数变少、出现 failed、clippy 有告警），先按**环境或迁移问题**处理，
而不是改代码——第一顺位查 `rustc --version` 是否 **1.99.1**，第二顺位查 `gcc --version` / 链接器是否在 PATH，
第三顺位查 `webui/dist` 与 `crates/*/tests/*_golden.json` 是否齐全（用 §8 的命令）。
若迁移后 HEAD 已前移（新提交），以新 HEAD 重跑基线为准，不强行对齐旧数字。

---

## 6. 已知坑清单（全部为旧机实测）

| 现象 | 处置 |
|------|------|
| PowerShell 报「Exit code: 1」但测试其实通过——stderr 进度行被包成 `NativeCommandError` 的假象 | 判定一律以日志里的 `test result:` / `Finished` 为准，不信 PowerShell 的退出码叙述 |
| 用 PowerShell `Set-Content` 改源码后中文/UTF-8 乱码 | **禁止** `Set-Content` 改源码；中文提交信息写成 UTF-8 临时文件再 `git commit -F <file>`，临时文件放 OS temp、不放仓库 |
| `core.autocrlf=true` + `rustfmt.toml` 的 `newline_style="Unix"`：`cargo fmt --all` 把工作树刷成 LF，`git status` 瞬间爆出一大片 | 提交内容仍按 LF 归一化、历史不受污染。旧机实测：一次刷出 741 条假象，刷新 stat 后真差异 57。处置：`git add --renormalize .` 或 `git status` 重扫后再判断真差异 |
| 只想格式化自己的文件，结果整个 crate 被刷 | `rustfmt` 会**顺 `mod` 递归**格式化整个 crate 的子模块。对具体文件跑 `rustfmt --edition 2024 <file>`，不要随手 `cargo fmt --all` |
| PGO 在 Windows GNU 不可用（缺 `profiler_builtins`，报 E0463）；BOLT 仅 Linux | 按平台选择优化档：Windows 只保留 UPX 压缩，PGO/BOLT 留给 Linux 与 CI |
| 环回连接被间歇中止：`os error 10053` / `error sending request`（火绒等安全软件） | 测试侧已有「仅连接层重试 ≤3」口径；新机出现同类失败先按**环境**定性（安全软件/代理），再复跑，不要当回归改代码 |
| `webui/dist` 缺失 ⇒ `rust-embed` 编译失败 | 别手删该目录；`build.ps1` / xtask 会自动补占位文件，CI 亦有占位步骤 |

---

## 7. 整包拷贝的流程：拷前必查、两条路径取舍、拷后必验

### 7.1 拷前必查：仓库有没有在被并发修改（最关键的一条）

未提交改动清单每次现取，本手册不列文件名、也不描述其完成状态：

```powershell
# 留档：拷贝前的工作树快照（写本手册时点实测输出为 6 项，属另一包的进行中工作）
git status --porcelain | Out-File -Encoding utf8 $env:TEMP\pre-copy-status.txt
Get-Content $env:TEMP\pre-copy-status.txt
```

**风险**：如果拷贝期间仓库还在被改（多代理/多人同时在建、在跑测试、在写文件），拷到的是**半写入状态**——
某个文件写了一半，或 `Cargo.lock` 与源码不是同一个时间点的快照。
这比丢改动更难排查：任何命令都不会报错，症状要等新机编译或跑测试时才浮出来。

建议动作顺序（整包拷贝前跑完这四步）：

1. **停止一切构建与测试**：`cargo build` / `cargo test` / `xtask` 还在跑的时候不要拷。
2. `git status --porcelain` **留档**未提交改动清单（上面那条命令，存到 OS temp，不放仓库）。
3. **关闭会占用或写文件的进程**：IDE（`rust-analyzer` 会写索引）、`cargo`/`rustc`、本地 dev server。
   robocopy 遇到被占用的文件会重试后跳过，跳过的文件不报错，只在汇总里体现为少了文件。
4. 再执行 §1 的 robocopy。

> 未提交改动**会随文件夹一起过去**（这正是拷贝优于 clone 之处）；代价就是拷贝期间**不要有人改文件**。

### 7.2 两条路径的取舍

| 路径 | 做法 | 带走什么 | 适用 |
|------|------|----------|------|
| ① 整包拷贝（**主路径**） | §1 的 robocopy：仓库根目录整体，排除枚举出来的 `target*`、`release`、`.tools` | 工作树 **23.7 MB** + `.git/` **111.9 MB**；未提交改动、`git blame`、全部分支与历史都跟着走；**不需要远端仓库权限** | 现状迁移：工作树有未提交内容，或新机拿不到远端权限 |
| ② `git clone`（备选，保留） | 旧机先 `git add <确切文件>` → `git commit -F <UTF-8 临时文件>` → `git push`，新机 `git clone -b Refactor` | 约 **136 MB**，只含**已入库**内容，**不含未提交工作** | 只想干净重来：确认工作树没有需要保留的游离改动，或已全部入库 |

> 路径 ② 的前置条件就是 §1 末尾那两条游离文件检查命令输出为空；一旦有输出，走 ①，别指望 clone 带上未提交内容。

### 7.3 拷后必验（新机侧，逐条给命令）

在新机工作树根目录依次执行（第 5 条之前先按 §4 设好 `PATH` 前置与 `CARGO_TARGET_DIR`）：

```powershell
# 1) 提交点位：应与旧机拷贝前 `git rev-parse --short HEAD` 的输出一致
git rev-parse --short HEAD

# 2) 换行策略：记下值，第 3 条判读要用
git config core.autocrlf

# 3) 未提交改动：应与 §7.1 留档的 pre-copy-status.txt 一致
git status --porcelain

# 4) 工具链与链接器
rustc --version      # 应为 rust-toolchain.toml 钉的 1.99.1
gcc --version        # Windows 需 mingw；旧机实测 gcc.exe 只在 C:\msys64\mingw64\bin，usr\bin 下没有（PATH 前置见 §2.1）

# 5) 最快证明「webui/dist 占位 + 两份金样 fixture 都在位」的一条
cargo test -p gensokyo-protocol
```

判读口径：

1. 第 3 条与留档清单**逐行一致**（顺序可能不同），拷贝即视为完整。
2. 第 3 条若冒出**大面积 modified**，先怀疑 `core.autocrlf` 与换行风格差异（`rustfmt.toml` 用的是 `newline_style="Unix"`，见 §6 那条坑），
   处置按 §6：`git add --renormalize .` 或 `git status` 重扫后再判断真差异，**不要**先动代码。
3. 第 5 条通过即说明 `webui/dist` 占位文件、`crates/gensokyo-protocol/tests/xurls_golden.json`、
   `crates/gensokyo-media/tests/qr_golden.json` 三者都在位——金样 fixture 缺失时相关测试会直接失败。
4. 之后再走 §8 的完整自检，数字对照 §5 的验收基线。

---

## 8. 迁移后自检（一次性命令清单）

在新机工作树根目录，依次执行（前三条是环境，中间两条是位置，最后三条是门禁）：

```powershell
# 1) 提交点位：与旧机计划搬运的 HEAD 一致
git rev-parse --short HEAD

# 2) 工具链：必须是 rust-toolchain.toml 钉的 1.99.1
rustc --version

# 3) 链接器（Windows 需先做 §2.1 的 PATH 前置）
gcc --version

# 4) 缓存实际落点：确认没有落进仓库或系统盘
cargo metadata --format-version 1 --no-deps | Select-String target_directory   # Linux: ... | grep target_directory

# 5) 最快证明「嵌入资源 + 金样 fixture 都在位」的一条
cargo test -p gensokyo-protocol

# 6) 格式门禁（退出码 0）
cargo fmt --all -- --check

# 7) 全量门禁：test + clippy + release 构建
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release -p gensokyo-bin
```

判据：第 4 条输出的目录必须在仓外；第 5 条通过即说明 `webui/dist` 占位与 `xurls_golden.json` 都在；
第 7 条的数字对照 §5 基线。

---

## 9. 相关文档

- [rust-build.md](./rust-build.md) — 构建链、自动引导、目标矩阵、CI 用法、退出码
- [rust-m0.md](./rust-m0.md) — 分层架构与里程碑
- [rust-perf.md](./rust-perf.md) — 性能与优化档（PGO/BOLT/UPX 的平台口径）
