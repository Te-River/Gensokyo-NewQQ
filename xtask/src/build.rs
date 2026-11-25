//! `xtask build`：release 构建 + 产物复制到 `release/`。
//!
//! 与 Go 版 `build.ps1` 对齐：平台集合（linux×3 + windows×2）与 `-All` 双版本一致，
//! 产物命名加 `rust` 标识：`gensokyo-rust-{os}-{arch}[-noWebui][.exe]`；UPX 压缩同 Go。
//! 差异与取舍见 `docs/rust-build.md`。

use std::env::consts::{ARCH, OS};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;

use crate::bootstrap::{bootstrap_zig_tools, find_on_path, path_with_tools};
use crate::find_lockfile;

/// 发布目标矩阵（`--all` 使用）：与 Go `build.ps1:164-171` 的 5 个平台逐字一致，顺序也照 Go。
///
/// darwin / msvc / android / armv7 不再进矩阵，仍可用 `--target <triple>` 显式构建；
/// 对应的 `[skip]` 判定（[`target_skip_reason`]）保留给那两条路径。
const MATRIX: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "i686-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-pc-windows-gnu",
    "i686-pc-windows-gnu",
];

/// 三元组里可能出现操作系统名的段。位置不固定：`x86_64-unknown-linux-gnu` 的 OS 在中间，
/// 而环境段（`gnu` / `musl` / `msvc`）永远是最后一段，所以不能只取末段。
const OS_SEGMENTS: &[&str] = &[
    "windows", "linux", "darwin", "macos", "android", "freebsd", "netbsd", "openbsd", "ios",
    "illumos", "solaris",
];

/// `webui/dist` 占位文件（该目录被 .gitignore 忽略）。
///
/// `rust-embed` 在编译期读取 `webui/dist`，目录缺失会直接编译失败；这组文件与
/// `build.ps1` 的 `Ensure-WebUIDist`、CI 的占位步骤保持一致。
const WEBUI_PLACEHOLDERS: &[&str] = &[
    "placeholder.html",
    "css/placeholder.css",
    "fonts/placeholder.txt",
    "icons/placeholder.txt",
    "js/placeholder.js",
];

/// `webui/dist/index.html` 占位内容，与 Go `build.ps1` 的 `Ensure-WebUIDist` 逐字节一致。
///
/// 只有空占位文件时没有 `index.html`，`/webui` 与 `/` 都会 404——而横幅照常打印控制台地址，
/// 点开是 404。补这份占位后 clean clone 构建的产物在两个路径上都回 200，与 Go 同形。
const WEBUI_INDEX_PLACEHOLDER: &str = "<!doctype html><title>WebUI placeholder</title><!-- Ensure-WebUIDist 占位：clean clone 可直接构建；真实前端构建产物会覆盖本文件 -->";

#[derive(Debug, Args)]
pub struct BuildArgs {
    /// 目标三元组，可重复（例如 x86_64-unknown-linux-gnu）；缺省构建当前宿主。
    #[arg(long, value_name = "TRIPLE", conflicts_with_all = ["all", "linux_only"])]
    target: Vec<String>,

    /// 构建发布矩阵里的全部目标（未安装的目标只打印跳过原因，不计失败）。
    #[arg(long)]
    all: bool,

    /// 只构建 Linux 目标（隐含 --all）。
    #[arg(long = "linux-only")]
    linux_only: bool,

    /// 生成无 WebUI 精简版（对应 Go 的 `-tags=small`）。
    #[arg(long = "no-webui")]
    no_webui: bool,

    /// 跳过 UPX 压缩。
    #[arg(long = "no-upx")]
    no_upx: bool,

    /// UPX 压缩级别 1~9（与 build.ps1 的默认值 7 一致）。
    #[arg(long = "upx-level", default_value_t = 7, value_parser = clap::value_parser!(u8).range(1..=9))]
    upx_level: u8,

    /// 并行 worker 数 1~4（默认矩阵模式 2、单目标 1；非法值告警后回退默认）。
    #[arg(long, value_name = "N")]
    workers: Option<u8>,
}

/// 并行 worker 数的默认值与上限（每多一个 worker 就多一份构建缓存目录）。
const DEFAULT_WORKERS: usize = 2;
const MAX_WORKERS: usize = 4;

/// 一个待构建目标。宿主构建时 `triple` 为 `None`，交给 cargo 自己选目标。
#[derive(Debug, Clone)]
struct Target {
    triple: Option<String>,
    os: String,
    arch: String,
}

impl Target {
    /// 宿主目标：产物命名用 `std::env::consts` 的 OS/ARCH 换算成 Go 命名。
    fn host() -> Self {
        Self {
            triple: None,
            os: go_os_name(OS).to_owned(),
            arch: go_arch_name(ARCH).to_owned(),
        }
    }

    /// 由三元组推导 os/arch；产物名用 Go 命名（`amd64` / `386` / `arm64`）。
    fn parse(triple: &str) -> Result<Self> {
        let (os, arch) = split_triple(triple)?;
        Ok(Self {
            triple: Some(triple.to_owned()),
            os,
            arch,
        })
    }

    /// 日志里用的目标名。
    fn label(&self) -> &str {
        self.triple.as_deref().unwrap_or("<host>")
    }
}

/// Rust 的 OS 常量 / 三元组段 → Go 的 `GOOS` 命名。
fn go_os_name(os: &str) -> &str {
    match os {
        "macos" => "darwin",
        other => other,
    }
}

/// Rust 的 ARCH 常量 / 三元组首段 → Go 的 `GOARCH` 命名。
fn go_arch_name(arch: &str) -> &str {
    match arch {
        "x86_64" => "amd64",
        "i686" | "i586" | "x86" => "386",
        "aarch64" => "arm64",
        "armv7" | "armv7a" | "arm" => "arm",
        "riscv64gc" => "riscv64",
        other => other,
    }
}

/// 三元组 → (Go 风格 os, Go 风格 arch)。
fn split_triple(triple: &str) -> Result<(String, String)> {
    let segments: Vec<&str> = triple.split('-').filter(|seg| !seg.is_empty()).collect();
    let Some((arch, _)) = segments.split_first() else {
        bail!("目标三元组为空");
    };
    // android 必须先于 linux 判定：`aarch64-linux-android` 的 GOOS 是 android，不是 linux。
    let os = if segments.contains(&"android") {
        "android"
    } else if let Some(segment) = segments.iter().find(|seg| OS_SEGMENTS.contains(seg)) {
        go_os_name(segment)
    } else {
        bail!("无法从三元组 `{triple}` 推导操作系统；期望形如 x86_64-unknown-linux-gnu");
    };
    Ok((os.to_owned(), go_arch_name(arch).to_owned()))
}

/// 产物文件名：`gensokyo-rust-{os}-{arch}[-noWebui][.exe]`。
///
/// `rust` 标识把重构版产物与 Go 主线的 `gensokyo-{os}-{arch}` 区分开，其余部分（Go 命名、
/// 版本顺序、扩展名）与 `build.ps1` 一致。
fn artifact_name(os: &str, arch: &str, no_webui: bool) -> String {
    let flavor = if no_webui { "-noWebui" } else { "" };
    let ext = if os == "windows" { ".exe" } else { "" };
    format!("gensokyo-rust-{os}-{arch}{flavor}{ext}")
}

/// 目标清单：`--all`/`--linux-only` 走矩阵，否则显式三元组或宿主。
fn plan(args: &BuildArgs) -> Result<Vec<Target>> {
    if args.all || args.linux_only {
        let mut targets = MATRIX
            .iter()
            .map(|triple| Target::parse(triple))
            .collect::<Result<Vec<_>>>()?;
        if args.linux_only {
            targets.retain(|target| target.os == "linux");
        }
        return Ok(targets);
    }
    if args.target.is_empty() {
        return Ok(vec![Target::host()]);
    }
    args.target
        .iter()
        .map(|triple| Target::parse(triple))
        .collect()
}

/// 版本清单：`--all` 默认双版本（完整 + noWebui，与 `build.ps1 -All` 一致）；
/// 显式 `--no-webui` 时只出精简版。
fn flavors(args: &BuildArgs) -> &'static [bool] {
    match (args.all || args.linux_only, args.no_webui) {
        (true, false) => &[false, true],
        (true, true) | (false, true) => &[true],
        (false, false) => &[false],
    }
}

/// workspace 根目录：`Cargo.lock` 所在目录（`cargo xtask` 可能在子目录被调用）。
pub(crate) fn workspace_root() -> Result<PathBuf> {
    let lock = find_lockfile()?;
    lock.parent()
        .map(Path::to_path_buf)
        .context("Cargo.lock 没有父目录")
}

/// cargo 可执行文件：`cargo run` 会注入 `$CARGO`，否则用 PATH 里的 cargo。
pub(crate) fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned())
}

/// `cargo metadata` 的 `target_directory`：尊重 `CARGO_TARGET_DIR` 与 `.cargo/config.toml`，
/// 不能假定产物在 `<root>/target`。
pub(crate) fn target_directory(root: &Path) -> Result<PathBuf> {
    let output = Command::new(cargo())
        .args(["metadata", "--no-deps", "--format-version", "1", "--locked"])
        .current_dir(root)
        .output()
        .context("执行 cargo metadata 失败")?;
    if !output.status.success() {
        bail!(
            "cargo metadata 失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let meta: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("解析 cargo metadata 输出失败")?;
    let dir = meta
        .get("target_directory")
        .and_then(|value| value.as_str())
        .context("cargo metadata 输出缺少 target_directory")?;
    Ok(PathBuf::from(dir))
}

/// 保证 `webui/dist` 存在：`rust-embed` 编译期读目录，缺目录直接编译失败。
///
/// 只要 `css/style.css` 在就认为已有真实前端产物（`frontend: npm run build`），不再占位。
pub(crate) fn ensure_webui_dist(root: &Path) -> Result<()> {
    let dist = root.join("webui").join("dist");
    if dist.join("css/style.css").is_file() {
        return Ok(());
    }
    for rel in WEBUI_PLACEHOLDERS {
        let path = dist.join(rel);
        if path.is_file() {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("创建 {} 失败", parent.display()))?;
        }
        fs::write(&path, "").with_context(|| format!("写入占位文件 {} 失败", path.display()))?;
    }
    let index = dist.join("index.html");
    if !index.is_file() {
        fs::write(&index, WEBUI_INDEX_PLACEHOLDER)
            .with_context(|| format!("写入占位文件 {} 失败", index.display()))?;
    }
    println!("webui/dist 无前端产物，已写入占位文件（rust-embed 需要目录存在）");
    Ok(())
}

/// `rustup target list --installed`（带与 add 相同的 `--toolchain`，避免查到默认工具链的清单）；
/// rustup 不可用时返回 `None`（退化为直接尝试构建）。
fn installed_targets(toolchain: Option<&str>) -> Option<Vec<String>> {
    let mut command = Command::new("rustup");
    command.args(["target", "list", "--installed"]);
    if let Some(toolchain) = toolchain {
        command.args(["--toolchain", toolchain]);
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

/// 目标缺失时自动 `rustup target add`（幂等）；rustup 不可用或安装失败返回 false。
/// `--toolchain` 必须带当前实际工具链，否则会装进 rust-toolchain.toml 的默认工具链。
fn ensure_target_installed(
    triple: &str,
    installed: Option<&Vec<String>>,
    toolchain: Option<&str>,
) -> bool {
    if installed.is_some_and(|list| list.iter().any(|value| value == triple)) {
        return true;
    }
    let mut command = Command::new("rustup");
    command.args(["target", "add"]);
    if let Some(toolchain) = toolchain {
        command.args(["--toolchain", toolchain]);
    }
    command.arg(triple);
    println!("  [bootstrap] rustup target add {triple}（缺失自动补装）");
    match command.status() {
        Ok(status) if status.success() => true,
        Ok(status) => {
            eprintln!("  rustup target add {triple} 失败（退出码 {status}）");
            false
        }
        Err(err) => {
            eprintln!("  无法执行 rustup（{err}），跳过 target 预装");
            false
        }
    }
}

/// 当前实际使用的 rustup 工具链（`<channel>-<host>`，如 `1.99.1-x86_64-pc-windows-gnu`）。
///
/// target 补装必须带上它：`rust-toolchain.toml` 把默认工具链钉在 msvc 宿主上，`cargo +<gnu>`
/// 只是临时覆盖，不带 `--toolchain` 的 `rustup target add` 会把 target 装进 msvc 工具链，
/// 重新构建时依旧缺失。来源依次为 `RUSTUP_TOOLCHAIN`（`cargo +<tc>` / `rustup run` 注入）、
/// `rustc -vV`、`rust-toolchain.toml` 的 channel + 本机推断。
fn active_toolchain(root: &Path) -> Option<String> {
    if let Ok(value) = std::env::var("RUSTUP_TOOLCHAIN") {
        let value = value.trim();
        if !value.is_empty() {
            return Some(value.to_owned());
        }
    }
    rustc_toolchain().or_else(|| {
        let channel = toolchain_channel(root)?;
        Some(format!("{channel}-{}", host_triple()?))
    })
}

/// `rustc -vV` 的 `release:` + `host:`：既反映 `+<tc>` 覆盖，也给出真实 ABI（gnu / msvc）。
fn rustc_toolchain() -> Option<String> {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let output = Command::new(rustc).arg("-vV").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut release = None;
    let mut host = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("release: ") {
            release = Some(value.trim());
        } else if let Some(value) = line.strip_prefix("host: ") {
            host = Some(value.trim());
        }
    }
    Some(format!("{}-{}", release?, host?))
}

/// `rust-toolchain.toml` 的 `channel`（本仓为 `1.99.1`）；rustc 都跑不起来时的兜底。
fn toolchain_channel(root: &Path) -> Option<String> {
    let text = fs::read_to_string(root.join("rust-toolchain.toml")).ok()?;
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("channel"))?;
    let value = line.split_once('=')?.1.trim().trim_matches('"');
    (!value.is_empty()).then(|| value.to_owned())
}

/// 本机三元组的粗略推断（ABI 取各平台默认：Windows → msvc、Linux → gnu）。
fn host_triple() -> Option<String> {
    let arch = match ARCH {
        "x86_64" | "aarch64" => ARCH,
        "x86" => "i686",
        _ => return None,
    };
    let rest = match OS {
        "windows" => "pc-windows-msvc",
        "linux" => "unknown-linux-gnu",
        "macos" => "apple-darwin",
        _ => return None,
    };
    Some(format!("{arch}-{rest}"))
}

/// 交叉目标走 `cargo zigbuild`（zig cc 当交叉 C 编译器，ring 等 C 依赖不再需要各平台
/// 原生工具链）；其余目标（darwin/MSVC/android/musl/宿主）维持 `cargo build`。
///
/// windows-gnu 也走 zig：rustc 对 `*-pc-windows-gnu` 会调 mingw 的 `dlltool` 生成
/// `windows-sys` 等 crate 的导入库，而 MSYS2 的 dlltool 只认 `-m i386`、不认 rustc 传的
/// `-m i386pe`，于是报
/// `dlltool.exe: Can't create .lib file: …: invalid bfd target`（32 位目标稳定复现，
/// `.tools\zig-0.16.0` 前置到 PATH 时两种位宽都命中）。zig 自带 PE 导入库与 Windows 头文件，
/// `cargo zigbuild` 不经过 dlltool，与 linux-gnu 同一条路径，也不再需要 MSVC / mingw。
fn uses_zigbuild(target: &Target) -> bool {
    target.triple.as_deref().is_some_and(|triple| {
        triple.ends_with("-linux-gnu")
            || triple.ends_with("-linux-gnueabihf")
            || triple.ends_with("-pc-windows-gnu")
    })
}

/// MSYS2 常见安装根：`<root>\mingw64\bin\gcc.exe`（64 位）/ `<root>\mingw32\bin\gcc.exe`（32 位）。
const MSYS2_ROOTS: &[&str] = &[r"C:\msys64", r"C:\msys2"];

/// windows-gnu 三元组 → (MSYS2 mingw 子目录, PATH 上探测的 gcc 名)。
///
/// i686 不回落裸 `gcc`：64 位宿主上的裸 gcc 位宽不对，链接 32 位目标会失败。
fn windows_gnu_gcc_names(triple: &str) -> Option<(&'static str, &'static [&'static str])> {
    match triple.split('-').next()? {
        "x86_64" => Some(("mingw64", &["x86_64-w64-mingw32-gcc", "gcc"])),
        "i686" => Some(("mingw32", &["i686-w64-mingw32-gcc"])),
        _ => None,
    }
}

/// `*-pc-windows-gnu` 目标的 mingw-w64 gcc 绝对路径：它同时充当 linker 与 C 编译器
/// （`ring` 等 C 依赖）。探测顺序：PATH 上带位宽前缀的 gcc → MSYS2 常见安装目录 →
/// PATH 上的裸 `gcc`（仅 x86_64）。找不到返回 `None`，矩阵模式下该目标 `[skip]`。
fn windows_gnu_toolchain(triple: &str) -> Option<PathBuf> {
    if !triple.ends_with("-pc-windows-gnu") {
        return None;
    }
    let (mingw_dir, names) = windows_gnu_gcc_names(triple)?;
    for name in names {
        let exe = format!("{name}.exe");
        if let Some(candidate) = find_on_path(&exe).map(|dir| dir.join(&exe))
            && candidate.is_file()
        {
            return Some(candidate);
        }
    }
    for root in MSYS2_ROOTS {
        let candidate = Path::new(root).join(mingw_dir).join("bin").join("gcc.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Windows GNU 目标的环境变量对（纯映射，便于单测）：`CARGO_TARGET_<TRIPLE>_LINKER` 加上
/// cc crate 认的 `CC_<target>`——连字符与下划线两种拼写都设（不同版本/不同 crate 各认一种）；
/// 再加一条 `PATH`：gcc 靠 PATH 找自己的 `cc1` / `as` / `ld`，MSYS2 两个环境并存时
/// `mingw64\bin` 常排在前面，32 位 gcc 会调到 64 位工具链而静默失败（`ring` 的编译器家族
/// 探测就是这么挂的），所以 gcc 自己的目录必须领先。`base_path` 是拼 PATH 的底（`None` 取
/// 进程 PATH），便于单测不依赖本机安装。
fn windows_gnu_env(
    gcc: &Path,
    triple: &str,
    base_path: Option<&std::ffi::OsStr>,
) -> Vec<(String, String)> {
    let value = gcc.display().to_string();
    let upper = triple.to_uppercase().replace('-', "_");
    let mut env = vec![
        (format!("CARGO_TARGET_{upper}_LINKER"), value.clone()),
        (format!("CC_{triple}"), value.clone()),
        (format!("CC_{}", triple.replace('-', "_")), value),
    ];
    if let Some(bin) = gcc.parent() {
        let base = base_path
            .map(std::ffi::OsStr::to_os_string)
            .or_else(|| std::env::var_os("PATH"))
            .unwrap_or_default();
        let mut path = bin.display().to_string();
        for entry in std::env::split_paths(&base).filter(|entry| !entry.as_os_str().is_empty()) {
            path.push(if cfg!(windows) { ';' } else { ':' });
            path.push_str(&entry.to_string_lossy());
        }
        env.push(("PATH".to_owned(), path));
    }
    env
}

/// 该三元组要让 cargo 子进程多带的环境变量；非 windows-gnu 或缺 mingw 时为空。
fn target_env_overrides(triple: &str) -> Vec<(String, String)> {
    windows_gnu_toolchain(triple)
        .map(|gcc| windows_gnu_env(&gcc, triple, None))
        .unwrap_or_default()
}

/// zigbuild 目标额外的构建环境：目前只有 windows-gnu 的 `CFLAGS_*`。
///
/// zig cc 把 `-Wdate-time` 当错误（`-Werror`），而 mimalloc v3 的 `options.c:236` 用了
/// `__DATE__` / `__TIME__`（`libmimalloc-sys` 的构建脚本编译它），windows-gnu 的 zigbuild
/// 因此以 `expansion of date or time macro is not reproducible [-Werror,-Wdate-time]` 失败。
/// cc crate 认 `CFLAGS_<target>`，连字符与下划线两种拼写各设一份（不同版本各认一种）；
/// 只放宽这一条，且只在 windows-gnu 上注入——linux 目标与其余目标不带，保持原有告警口径。
fn zigbuild_env(target: &Target) -> Vec<(String, String)> {
    let Some(triple) = target
        .triple
        .as_deref()
        .filter(|triple| triple.ends_with("-pc-windows-gnu"))
    else {
        return Vec::new();
    };
    if !uses_zigbuild(target) {
        return Vec::new();
    }
    let value = "-Wno-error=date-time".to_owned();
    vec![
        (format!("CFLAGS_{triple}"), value.clone()),
        (format!("CFLAGS_{}", triple.replace('-', "_")), value),
    ]
}

/// 工具链可用性闸门（纯函数，便于单测）：`--all` 只尝试宿主真正能构建的目标，
/// 其余打印 `[skip]` 理由、计入跳过而非失败（见 `BuildArgs::all` 的注释）。
///
/// linux-gnu / windows-gnu 不在这里拦：它们由 zig + cargo-zigbuild 自动引导（见
/// [`uses_zigbuild`]），引导失败时走 bootstrap 的 `[skip]` 路径；
/// 宿主自身（`triple` 为 `None`）交给 cargo 选目标，永远不跳过。
///
/// MSVC 的判据是 `cl.exe` 而不是 `link.exe`：MSYS2 coreutils 在 `usr/bin/link.exe`
/// 提供一个 Unix `link`（硬链接工具），按 `link.exe` 探测会在无 MSVC 的机器上误判可用。
fn target_skip_reason(
    host_os: &str,
    triple: Option<&str>,
    has_msvc_cl: bool,
) -> Option<&'static str> {
    let triple = triple?;
    let (os, _) = split_triple(triple).ok()?;
    if os == "darwin" && !matches!(host_os, "macos" | "darwin") {
        return Some("macOS 目标需要 macOS 宿主或专用交叉工具链");
    }
    if os == "windows" && triple.ends_with("-msvc") && !has_msvc_cl {
        return Some("需要 MSVC 工具链（cl.exe 不在 PATH）");
    }
    if os == "android" {
        return Some("需要 Android NDK");
    }
    None
}

/// cargo 调用构造：zigbuild 目标换子命令，其余参数（--release --locked -p …）不变。
fn cargo_invocation(target: &Target, no_webui: bool) -> (&'static str, Vec<String>) {
    let subcommand = if uses_zigbuild(target) {
        "zigbuild"
    } else {
        "build"
    };
    let mut args: Vec<String> = [
        "--release",
        "--locked",
        "-p",
        "gensokyo-bin",
        "--bin",
        "gensokyo",
    ]
    .map(str::to_owned)
    .to_vec();
    if let Some(triple) = &target.triple {
        args.push("--target".to_owned());
        args.push(triple.clone());
    }
    if no_webui {
        // gensokyo-bin 尚无 small/webui 特性（Go 侧对应 `-tags=small`），先按
        // `--no-default-features` 占位；特性落地后改这里即可，见 docs/rust-build.md。
        args.push("--no-default-features".to_owned());
    }
    (subcommand, args)
}

/// 版本串推导（纯函数，便于单测）：显式覆盖 > `git-<短 hash>` > `dev-<epoch>.<毫秒>`。
///
/// 形态与 Go `build.ps1` 的 `Get-BuildLdflags` 一致：有 git 时 `<type>=git`、
/// `<spec>=<短 hash>`，否则 `dev` + UTC 秒/毫秒；空串等同于未设置。
fn build_version(override_value: Option<&str>, commit: Option<&str>, now: SystemTime) -> String {
    if let Some(value) = override_value.filter(|value| !value.is_empty()) {
        return value.to_owned();
    }
    match commit.filter(|commit| !commit.is_empty()) {
        Some(commit) => format!("git-{commit}"),
        None => {
            let elapsed = now.duration_since(UNIX_EPOCH).unwrap_or_default();
            format!("dev-{}.{:03}", elapsed.as_secs(), elapsed.subsec_millis())
        }
    }
}

/// `git rev-parse --short=7 HEAD`；git 不在 PATH、不在仓库内或仓库无提交时返回 `None`。
fn git_short_commit(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let commit = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!commit.is_empty()).then_some(commit)
}

/// 本次构建注入的版本串：已有的 `GENSOKYO_BUILD_VERSION` 优先（发布流水线可覆盖），
/// 否则按 [`build_version`] 推导。
fn resolve_build_version(root: &Path) -> String {
    let override_value = std::env::var("GENSOKYO_BUILD_VERSION").ok();
    let commit = git_short_commit(root);
    build_version(
        override_value.as_deref(),
        commit.as_deref(),
        SystemTime::now(),
    )
}

/// 构建阶段各 worker 共享的上下文（`jobs` 是分摊给单个 worker 的 cargo job 数）。
struct BuildContext<'a> {
    root: &'a Path,
    release_dir: &'a Path,
    tool_path: Option<&'a std::ffi::OsStr>,
    jobs: usize,
}

/// 一个 (target, flavor) 的构建产出：`log` 是捕获的 cargo 输出（串行模式实时继承终端，为空串）。
struct JobOutput {
    path: PathBuf,
    log: String,
}

/// `cargo build|zigbuild --release --locked`：交叉时加 `--target`，精简版加 `--no-default-features`。
///
/// 只构建 bin 包——工作区还有 10 个库 crate 与 xtask 自身，全量构建没有意义。
/// `tool_path` 非空时注入引导好的 PATH（zig / cargo-zigbuild 所在目录前置）。
/// 每次调用都把版本串经 `GENSOKYO_BUILD_VERSION` 传给 cargo 子进程，供
/// `gensokyo-bin` 的 `option_env!` 收进 `--version`（Go 侧 ldflags 的等价物）。
/// `CARGO_TARGET_DIR` 显式下发：并行时每个 worker 用自己的缓存目录（cargo 对整个 target
/// 目录加独占文件锁，共用目录会退化成 `Blocking waiting for file lock` 的串行）；
/// `CARGO_BUILD_JOBS` 按 worker 摊开，避免两个 cargo 各自吃满 CPU。
/// `capture` 为真时捕获输出并返回（并行模式下由主线程按矩阵顺序回放），否则直接继承终端。
fn cargo_build(
    ctx: &BuildContext<'_>,
    target_dir: &Path,
    target: &Target,
    no_webui: bool,
    capture: bool,
) -> Result<String> {
    let (subcommand, args) = cargo_invocation(target, no_webui);
    let version = resolve_build_version(ctx.root);
    let header = format!(
        "  $ cargo {subcommand} {} (GENSOKYO_BUILD_VERSION={version})",
        args.join(" ")
    );
    let mut command = Command::new(cargo());
    command
        .current_dir(ctx.root)
        .arg(subcommand)
        .args(&args)
        .env("GENSOKYO_BUILD_VERSION", &version)
        .env("CARGO_TARGET_DIR", target_dir)
        .env("CARGO_BUILD_JOBS", ctx.jobs.to_string());
    if let Some(path) = ctx.tool_path {
        command.env("PATH", path);
    }
    if let Some(triple) = &target.triple
        && !uses_zigbuild(target)
    {
        // zigbuild 目标由 cargo-zigbuild 自己设 linker/CC，这里再注入 mingw gcc 只会让同一批
        // 键上的两套工具链互相覆盖（env 同键后写者生效），所以只在非 zigbuild 目标上兜底。
        command.envs(target_env_overrides(triple));
    }
    command.envs(zigbuild_env(target));
    if !capture {
        println!("{header}");
        let status = command
            .status()
            .with_context(|| format!("执行 cargo {subcommand} 失败"))?;
        if !status.success() {
            bail!(
                "cargo {subcommand} 失败（退出码 {status}）: {}",
                target.label()
            );
        }
        return Ok(String::new());
    }
    let output = command
        .output()
        .with_context(|| format!("执行 cargo {subcommand} 失败"))?;
    let mut log = header;
    log.push('\n');
    log.push_str(&String::from_utf8_lossy(&output.stdout));
    log.push_str(&String::from_utf8_lossy(&output.stderr));
    if !output.status.success() {
        // 并行模式下没人实时看 cargo 的输出，失败原因必须随错误一起抛出去。
        bail!(
            "cargo {subcommand} 失败（退出码 {}）: {}\n{log}",
            output.status,
            target.label()
        );
    }
    Ok(log)
}

/// cargo 的产物位置：`<target_dir>/<triple>/release/gensokyo[.exe]`，宿主构建无 triple 层。
fn built_binary(target_dir: &Path, target: &Target) -> PathBuf {
    let mut path = target_dir.to_path_buf();
    if let Some(triple) = &target.triple {
        path.push(triple);
    }
    path.push("release");
    path.push(if target.os == "windows" {
        "gensokyo.exe"
    } else {
        "gensokyo"
    });
    path
}

/// 构建一个目标的某一个版本，复制到 `release/` 并返回产物路径与捕获的日志。
fn build_one(
    ctx: &BuildContext<'_>,
    target_dir: &Path,
    target: &Target,
    no_webui: bool,
    capture: bool,
) -> Result<JobOutput> {
    let name = artifact_name(&target.os, &target.arch, no_webui);
    let log = cargo_build(ctx, target_dir, target, no_webui, capture)?;

    let built = built_binary(target_dir, target);
    if !built.is_file() {
        bail!("cargo 报告成功，但未找到产物 {}", built.display());
    }
    let dest = ctx.release_dir.join(&name);
    fs::copy(&built, &dest)
        .with_context(|| format!("复制 {} → {} 失败", built.display(), dest.display()))?;
    Ok(JobOutput { path: dest, log })
}

/// 文件大小（MB）；读不到时返回 0（刚复制完的文件不该读不到，这里不额外报错）。
fn megabytes(path: &Path) -> f64 {
    fs::metadata(path).map_or(0, |meta| meta.len()) as f64 / (1024.0 * 1024.0)
}

/// UPX 压缩；PATH 里没有 upx 只提示并跳过（不是构建失败），返回是否真的压缩了。
fn compress(path: &Path, level: u8) -> Result<bool> {
    match Command::new("upx")
        .arg(format!("-{level}"))
        .arg(path)
        .status()
    {
        Ok(status) if status.success() => Ok(true),
        Ok(status) => bail!("UPX 压缩失败（退出码 {status}）: {}", path.display()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).with_context(|| format!("执行 upx 失败: {}", path.display())),
    }
}

/// 把 `len` 个 job 连续切给 `workers` 个 worker（选连续切分而非轮转：同一 worker 的 job 相邻，
/// 打印与实际执行的顺序一致，也便于把同目标的两个版本放在同一个缓存目录里连续构建）。
/// 空 plan 返回空 vec；`workers > len` 时不生成空 shard。
fn shard(len: usize, workers: usize) -> Vec<Vec<usize>> {
    if len == 0 {
        return Vec::new();
    }
    let workers = workers.max(1).min(len);
    let base = len / workers;
    let extra = len % workers;
    let mut shards = Vec::with_capacity(workers);
    let mut start = 0;
    for worker in 0..workers {
        let size = base + usize::from(worker < extra);
        shards.push((start..start + size).collect());
        start += size;
    }
    shards
}

/// worker 的缓存目录：worker 1 直接用 `<base>` 本体（单目标/热缓存可复用），worker i≥2
/// 用 `<base>-w<i>`。cargo 对整个 target 目录加独占文件锁，共用同一目录的两个 cargo 进程
/// 会排队成串行；这样默认 2 worker = base + `-w2`，正好 2 份缓存（预算）。
fn worker_target_dir(base: &Path, index: usize) -> PathBuf {
    if index <= 1 {
        return base.to_path_buf();
    }
    let mut name = base.as_os_str().to_os_string();
    name.push(format!("-w{index}"));
    PathBuf::from(name)
}

/// worker 数：`--workers` > `GENSOKYO_BUILD_WORKERS` > 默认（矩阵 2 / 单目标 1）；
/// 非法值告警后回退默认，上限 [`MAX_WORKERS`]，且不超过待构建的 (target, flavor) 数量。
fn resolve_workers(cli: Option<u8>, default: usize, job_count: usize) -> usize {
    let env_value = std::env::var("GENSOKYO_BUILD_WORKERS").ok();
    let requested = match (cli, env_value.as_deref()) {
        (Some(value), _) => Some((value.to_string(), "--workers")),
        (None, Some(text)) => Some((text.trim().to_owned(), "GENSOKYO_BUILD_WORKERS")),
        (None, None) => None,
    };
    let mut workers = default;
    if let Some((text, source)) = requested {
        match text.parse::<usize>() {
            Ok(value) if (1..=MAX_WORKERS).contains(&value) => workers = value,
            _ => eprintln!(
                "[warn] {source}=`{text}` 非法（期望 1..={MAX_WORKERS}），回退默认 {default} 个 worker"
            ),
        }
    }
    workers.clamp(1, MAX_WORKERS).min(job_count.max(1))
}

/// 总 job 数：`CARGO_BUILD_JOBS` 优先（与 cargo 同一开关），否则按可用 CPU 数，下限 1。
fn total_jobs() -> usize {
    std::env::var("CARGO_BUILD_JOBS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|&jobs| jobs > 0)
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |cpus| cpus.get()))
}

/// 每个 worker 的 cargo job 数：总 job 数按 worker 数摊开，下限 1。
fn jobs_per_worker(total: usize, workers: usize) -> usize {
    (total / workers.max(1)).max(1)
}

/// 并发执行 `jobs` 个作业：按 `shards` 切分，结果按 job 下标写回原位（`None` = 该 job panic）。
/// xtask 是 dev profile（`panic=unwind`），单个 job panic 只记失败，不影响同 worker 的后续 job。
fn run_sharded<T, F>(jobs: usize, shards: &[Vec<usize>], f: F) -> Vec<Option<T>>
where
    T: Send,
    F: Fn(usize, usize) -> T + Sync,
{
    let slots: Mutex<Vec<Option<T>>> = Mutex::new((0..jobs).map(|_| None).collect());
    std::thread::scope(|scope| {
        for (worker, indices) in shards.iter().enumerate() {
            let slots = &slots;
            let f = &f;
            let _ = scope.spawn(move || {
                for &job in indices {
                    if let Ok(value) =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(worker, job)))
                    {
                        slots.lock().unwrap_or_else(PoisonError::into_inner)[job] = Some(value);
                    }
                }
            });
        }
    });
    slots.into_inner().unwrap_or_else(PoisonError::into_inner)
}

/// `[build]` 行：串行路径在构建前实时打印，并行路径在回放阶段打印（保证矩阵顺序）。
fn print_job_header(target: &Target, no_webui: bool) {
    println!(
        "[build] {} → release/{}",
        target.label(),
        artifact_name(&target.os, &target.arch, no_webui)
    );
}

/// 打印一个 job 的日志并记账：矩阵模式记失败后继续，单目标模式直接上抛（保持原退出语义）。
fn report_job(
    target: &Target,
    no_webui: bool,
    result: Result<JobOutput>,
    matrix_mode: bool,
    outputs: &mut Vec<PathBuf>,
    failures: &mut Vec<String>,
) -> Result<()> {
    match result {
        Ok(output) => {
            print!("{}", output.log);
            println!(
                "  OK: {} ({:.1} MB)",
                output.path.display(),
                megabytes(&output.path)
            );
            outputs.push(output.path);
        }
        Err(err) if matrix_mode => {
            eprintln!("[fail] {err:#}");
            failures.push(format!("{}/{no_webui}", target.label()));
        }
        Err(err) => return Err(err),
    }
    Ok(())
}

/// `xtask build` 入口：解析目标 → 引导交叉工具链 → 构建 → 复制 → 可选 UPX → 汇总。
///
/// linux-gnu 目标先自动引导 zig + cargo-zigbuild（见 bootstrap.rs），缺失的 rustup
/// target 自动补装；补装失败的目标在矩阵模式下打印跳过原因并继续，引导失败的 linux
/// 目标同样跳过（`[warn]` + `[skip]`），真正编译失败的目标记入失败清单，其余目标继续
/// 构建，最终以非零退出码汇总。
///
/// 矩阵模式默认起 2 个 worker 并行构建（`--workers` / `GENSOKYO_BUILD_WORKERS` 覆盖）：
/// rustup target 补装仍串行前置，缓存目录用 `<base>`（worker 1）与 `<base>-w<i>`（i≥2），
/// 日志按矩阵顺序回放，对外的产物命名、`[skip]` 语义、汇总行与退出码不变。
pub fn run(args: BuildArgs) -> Result<()> {
    let root = workspace_root()?;
    let targets = plan(&args)?;
    let flavors = flavors(&args);
    let matrix_mode = args.all || args.linux_only;

    ensure_webui_dist(&root)?;
    let base_target_dir = target_directory(&root)?;
    let release_dir = root.join("release");
    fs::create_dir_all(&release_dir)
        .with_context(|| format!("创建 {} 失败", release_dir.display()))?;

    println!("=== Gensokyo Build (cargo xtask) ===");
    println!("workspace : {}", root.display());
    println!(
        "targets   : {} 个目标 × {} 个版本 ({})",
        targets.len(),
        flavors.len(),
        if flavors.len() > 1 {
            "完整 + noWebui"
        } else if flavors[0] {
            "仅 noWebui"
        } else {
            "仅完整版"
        }
    );

    // 引导 zig 交叉工具链（linux-gnu / windows-gnu，zig + cargo-zigbuild）；全部命中 PATH/.tools
    // 缓存时零网络。
    let needs_zigbuild = targets.iter().any(uses_zigbuild);
    let mut tool_path: Option<std::ffi::OsString> = None;
    // 引导失败只意味着「zig 目标跳过」，不是构建失败：既可能是网络（镜像不可达），
    // 也可能是校验/解包缺陷。环境缺失不该让宿主目标连带失败；某个 zig 目标真正编译
    // 失败时仍走下面的 [fail] 分支，不会被吞。
    let mut bootstrap_failed = false;
    if needs_zigbuild {
        match bootstrap_zig_tools(&root) {
            Ok(dirs) => tool_path = Some(path_with_tools(&dirs)?),
            Err(err) if matrix_mode => {
                eprintln!(
                    "[warn] zig 交叉工具链不可用（zig/cargo-zigbuild 引导失败），linux / windows-gnu 目标将跳过：{err:#}\n\
                     零网络兜底：手工把工具解压到 <repo>\\.tools\\<tool>-<version>\\ 后重试\
                     （如 .tools\\zig-0.16.0\\ 与 .tools\\cargo-zigbuild-0.24.1\\，与自动引导同布局即可复用）"
                );
                bootstrap_failed = true;
            }
            Err(err) => return Err(err),
        }
    }

    // target 查询与补装都要指明工具链，否则会落在 rust-toolchain.toml 的默认工具链上。
    let toolchain = active_toolchain(&root);
    let installed = installed_targets(toolchain.as_deref());
    // MSVC 目标需要 cl.exe（不是 link.exe：MSYS2 coreutils 有个同名 link.exe 会误判）；
    // 只在矩阵模式下探测（单目标模式不拦，交给 cargo 真实尝试）。
    let has_msvc_cl = matrix_mode && find_on_path("cl.exe").is_some();

    // 串行前置：按矩阵顺序定下每个目标的跳过原因，并把缺失的 rustup target 全部装好。
    // 补装必须串行——并发的 `rustup target add` 会撞 `could not rename … .partial (os error 2)`。
    let mut verdicts: Vec<Option<String>> = Vec::with_capacity(targets.len());
    for target in &targets {
        if bootstrap_failed && uses_zigbuild(target) {
            verdicts.push(Some(format!(
                "{}: zig 交叉工具链不可用（zig/cargo-zigbuild 引导失败）",
                target.label()
            )));
            continue;
        }
        // 工具链闸门先于 target 补装：darwin / MSVC / android 缺工具链时没必要下载 rust-std
        // 再在 C 代码里撞 gcc 的 `-arch` 报错。
        if matrix_mode
            && let Some(reason) = target_skip_reason(OS, target.triple.as_deref(), has_msvc_cl)
        {
            verdicts.push(Some(format!("{}: {reason}", target.label())));
            continue;
        }
        // windows-gnu 不再需要 mingw 探测：它和 linux-gnu 一样走 zigbuild，工具链缺失统一由
        // 上面的 zig 引导失败路径（bootstrap_failed → `[skip]`）处理。
        if let Some(triple) = &target.triple
            && !ensure_target_installed(triple, installed.as_ref(), toolchain.as_deref())
            && matrix_mode
        {
            verdicts.push(Some(match toolchain.as_deref() {
                Some(toolchain) => format!(
                    "{triple} 自动补装失败 → 手动执行 rustup target add --toolchain {toolchain} {triple}"
                ),
                None => format!(
                    "{triple} 自动补装失败 → 手动执行 rustup target add {triple}（请确认加到当前使用的工具链）"
                ),
            }));
            continue;
        }
        // 单目标模式补装失败不跳过：交给 cargo 真实尝试（rustup 缺失时 target 可能已就位）。
        verdicts.push(None);
    }

    // 待构建的 (target 下标, noWebui)，按矩阵顺序展开；并行阶段只填结果，打印留到回放。
    let jobs: Vec<(usize, bool)> = targets
        .iter()
        .enumerate()
        .filter(|(index, _)| verdicts[*index].is_none())
        .flat_map(|(index, _)| flavors.iter().map(move |&no_webui| (index, no_webui)))
        .collect();
    let workers = resolve_workers(
        args.workers,
        if matrix_mode { DEFAULT_WORKERS } else { 1 },
        jobs.len(),
    );
    let cargo_jobs = jobs_per_worker(total_jobs(), workers);
    let ctx = BuildContext {
        root: &root,
        release_dir: &release_dir,
        tool_path: tool_path.as_deref(),
        jobs: cargo_jobs,
    };
    if workers > 1 {
        let dirs = (1..=workers)
            .map(|worker| {
                worker_target_dir(&base_target_dir, worker)
                    .display()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "workers   : {workers} 个并行 worker（每个 cargo --jobs {cargo_jobs}；缓存目录 {dirs}）"
        );
    }

    let mut outputs = Vec::new();
    let mut skipped = Vec::new();
    let mut failures = Vec::new();

    if workers == 1 {
        // 单 worker：沿用旧行为——用 `<base>` 本体缓存，cargo 输出实时继承终端。
        for (index, target) in targets.iter().enumerate() {
            if let Some(reason) = &verdicts[index] {
                println!("[skip] {reason}");
                skipped.push(reason.clone());
                continue;
            }
            for &no_webui in flavors {
                print_job_header(target, no_webui);
                let output = build_one(&ctx, &base_target_dir, target, no_webui, false);
                report_job(
                    target,
                    no_webui,
                    output,
                    matrix_mode,
                    &mut outputs,
                    &mut failures,
                )?;
            }
        }
    } else {
        let shards = shard(jobs.len(), workers);
        let worker_dirs: Vec<PathBuf> = (1..=shards.len())
            .map(|worker| worker_target_dir(&base_target_dir, worker))
            .collect();
        let mut records = run_sharded(jobs.len(), &shards, |worker, job| {
            let (index, no_webui) = jobs[job];
            build_one(&ctx, &worker_dirs[worker], &targets[index], no_webui, true)
        });
        let mut next = 0;
        for (index, target) in targets.iter().enumerate() {
            if let Some(reason) = &verdicts[index] {
                println!("[skip] {reason}");
                skipped.push(reason.clone());
                continue;
            }
            for &no_webui in flavors {
                let record = records[next].take().unwrap_or_else(|| {
                    Err(anyhow!("构建线程 panic，未回报结果: {}", target.label()))
                });
                next += 1;
                print_job_header(target, no_webui);
                report_job(
                    target,
                    no_webui,
                    record,
                    matrix_mode,
                    &mut outputs,
                    &mut failures,
                )?;
            }
        }
    }

    if args.no_upx {
        println!("UPX 已禁用（--no-upx）");
    } else {
        // UPX 一样按 worker 数并行；日志按产物顺序回放，避免多线程交错。
        let level = args.upx_level;
        let shards = shard(outputs.len(), workers);
        let mut compressed = run_sharded(outputs.len(), &shards, |_, index| {
            compress(&outputs[index], level)
        })
        .into_iter();
        for path in &outputs {
            println!("[upx] 压缩 {} ...", path.display());
            match compressed.next().flatten() {
                Some(Ok(true)) => {}
                Some(Ok(false)) => println!(
                    "  UPX 未在 PATH 中找到，跳过压缩（安装：winget install upx；或加 --no-upx 静默）"
                ),
                Some(Err(err)) => return Err(err),
                None => bail!("UPX 压缩未回报结果（线程 panic）: {}", path.display()),
            }
        }
    }

    println!(
        "\n=== 完成：{} 个产物 / {} 个跳过 / {} 个失败 ===",
        outputs.len(),
        skipped.len(),
        failures.len()
    );
    for path in &outputs {
        println!("  {} ({:.1} MB)", path.display(), megabytes(path));
    }

    if !failures.is_empty() {
        bail!("{} 个构建失败: {}", failures.len(), failures.join(", "));
    }
    if outputs.is_empty() {
        // 全部目标都被跳过 = 环境缺失（缺 MSVC / NDK / zig 引导失败），不是构建缺陷；
        // 但 release 入口不能以 0 退出码交付空产物，这里以非零 + 指引收尾。
        bail!(
            "没有产出任何产物（跳过 {} 个目标）：这是环境缺失而非构建缺陷\n\
             本平台构建：.\\build.ps1（即 cargo xtask build），不需要安装任何 target\n\
             linux-gnu / windows-gnu 目标会自动引导 zig/cargo-zigbuild 并补装 rustup target；\n\
             darwin / MSVC / android 仍需各自的工具链（见 docs/rust-build.md）",
            skipped.len()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        BuildArgs, Target, artifact_name, build_version, built_binary, cargo_invocation,
        ensure_webui_dist, flavors, git_short_commit, plan, shard, split_triple,
        target_env_overrides, target_skip_reason, uses_zigbuild, windows_gnu_env,
        windows_gnu_gcc_names, windows_gnu_toolchain, worker_target_dir, zigbuild_env,
    };
    use std::time::{Duration, SystemTime};

    /// 测试用参数；未设置的字段一律取“不生效”的值。
    fn args(target: &[&str], all: bool, linux_only: bool, no_webui: bool) -> BuildArgs {
        BuildArgs {
            target: target.iter().map(|value| (*value).to_owned()).collect(),
            all,
            linux_only,
            no_webui,
            no_upx: true,
            upx_level: 7,
            workers: None,
        }
    }

    #[test]
    fn maps_known_triples_to_go_naming() {
        let cases = [
            ("x86_64-unknown-linux-gnu", ("linux", "amd64")),
            ("i686-unknown-linux-gnu", ("linux", "386")),
            // 环境段（musl）不是 OS：OS 在中间
            ("x86_64-unknown-linux-musl", ("linux", "amd64")),
            ("x86_64-pc-windows-msvc", ("windows", "amd64")),
            ("i686-pc-windows-gnu", ("windows", "386")),
            ("aarch64-apple-darwin", ("darwin", "arm64")),
            ("x86_64-apple-darwin", ("darwin", "amd64")),
            ("armv7-unknown-linux-gnueabihf", ("linux", "arm")),
            // android 优先于 linux
            ("aarch64-linux-android", ("android", "arm64")),
        ];
        for (triple, (os, arch)) in cases {
            let parsed = split_triple(triple).unwrap_or_else(|err| panic!("{triple}: {err}"));
            assert_eq!(parsed, (os.to_owned(), arch.to_owned()), "三元组 {triple}");
        }
    }

    #[test]
    fn rejects_triples_without_a_known_os() {
        assert!(split_triple("wasm32-unknown-unknown").is_err());
        assert!(split_triple("").is_err());
    }

    /// 平台集合对齐 Go，文件名带 `rust` 标识：`gensokyo-rust-{os}-{arch}{flavor}{ext}`。
    #[test]
    fn artifact_names_carry_the_rust_marker() {
        assert_eq!(
            artifact_name("windows", "amd64", false),
            "gensokyo-rust-windows-amd64.exe"
        );
        assert_eq!(
            artifact_name("windows", "386", true),
            "gensokyo-rust-windows-386-noWebui.exe"
        );
        assert_eq!(
            artifact_name("linux", "arm64", false),
            "gensokyo-rust-linux-arm64"
        );
        assert_eq!(
            artifact_name("linux", "amd64", true),
            "gensokyo-rust-linux-amd64-noWebui"
        );
    }

    #[test]
    fn flavors_follow_all_and_no_webui() {
        assert_eq!(flavors(&args(&[], false, false, false)), &[false]);
        assert_eq!(flavors(&args(&[], false, false, true)), &[true]);
        // -All 双版本；显式 --no-webui 时只出精简版
        assert_eq!(flavors(&args(&[], true, false, false)), &[false, true]);
        assert_eq!(flavors(&args(&[], true, false, true)), &[true]);
    }

    #[test]
    fn shard_splits_jobs_into_contiguous_ranges() {
        // 空 plan：不起 worker
        assert!(shard(0, 1).is_empty());
        assert!(shard(0, 4).is_empty());
        // 1 个 job：无论多少 worker 都只切出一段
        assert_eq!(shard(1, 1), vec![vec![0]]);
        assert_eq!(shard(1, 4), vec![vec![0]]);
        // 9 个 job：连续切分，余数落在前几段
        assert_eq!(shard(9, 1), vec![(0..9).collect::<Vec<_>>()]);
        assert_eq!(shard(9, 2), vec![vec![0, 1, 2, 3, 4], vec![5, 6, 7, 8]]);
        assert_eq!(
            shard(9, 3),
            vec![vec![0, 1, 2], vec![3, 4, 5], vec![6, 7, 8]]
        );
        assert_eq!(
            shard(9, 4),
            vec![vec![0, 1, 2], vec![3, 4], vec![5, 6], vec![7, 8]]
        );
        // 不变式：每个 job 恰好出现一次、顺序不变，且不产生空 shard
        for (len, workers) in [(1, 2), (2, 4), (7, 5), (8, 2), (9, 4)] {
            let shards = shard(len, workers);
            assert!(shards.len() <= workers, "len={len} workers={workers}");
            assert!(
                shards.iter().all(|indices| !indices.is_empty()),
                "len={len} workers={workers}"
            );
            assert_eq!(
                shards.concat(),
                (0..len).collect::<Vec<_>>(),
                "len={len} workers={workers}"
            );
        }
    }

    /// `--all` 的 5 个平台与 Go `build.ps1` 的矩阵逐字一致，顺序也照 Go。
    #[test]
    fn all_plans_the_go_matrix_in_order() {
        let targets = plan(&args(&[], true, false, false)).expect("矩阵解析失败");
        let triples: Vec<Option<&str>> = targets
            .iter()
            .map(|target| target.triple.as_deref())
            .collect();
        assert_eq!(
            triples,
            [
                Some("x86_64-unknown-linux-gnu"),
                Some("i686-unknown-linux-gnu"),
                Some("aarch64-unknown-linux-gnu"),
                Some("x86_64-pc-windows-gnu"),
                Some("i686-pc-windows-gnu"),
            ]
        );
    }

    /// 缓存预算 2 份：worker 1 复用 `<base>`，worker i≥2 才派生 `-w<i>`。
    #[test]
    fn first_worker_reuses_the_base_cache() {
        let base = std::path::Path::new("target");
        assert_eq!(worker_target_dir(base, 1), std::path::Path::new("target"));
        assert_eq!(
            worker_target_dir(base, 2),
            std::path::Path::new("target-w2")
        );
        assert_eq!(
            worker_target_dir(base, 3),
            std::path::Path::new("target-w3")
        );
    }

    /// windows-gnu 的 gcc 名按位宽区分；i686 不回落裸 `gcc`（位宽不对会链接失败）。
    #[test]
    fn windows_gnu_gcc_names_follow_the_architecture() {
        let (mingw_dir, names) =
            windows_gnu_gcc_names("x86_64-pc-windows-gnu").expect("x86_64 映射");
        assert_eq!(mingw_dir, "mingw64");
        assert_eq!(names, &["x86_64-w64-mingw32-gcc", "gcc"][..]);
        let (mingw_dir, names) = windows_gnu_gcc_names("i686-pc-windows-gnu").expect("i686 映射");
        assert_eq!(mingw_dir, "mingw32");
        assert_eq!(names, &["i686-w64-mingw32-gcc"][..]);
        assert!(windows_gnu_gcc_names("aarch64-pc-windows-gnu").is_none());
    }

    /// 注入变量：`CARGO_TARGET_<TRIPLE>_LINKER` + cc crate 认的两种 `CC_` 拼写 + 前置 PATH。
    #[test]
    fn windows_gnu_env_sets_linker_cc_and_path() {
        let gcc = std::path::Path::new(r"C:\msys64\mingw64\bin\gcc.exe");
        let env = windows_gnu_env(gcc, "x86_64-pc-windows-gnu", Some(std::ffi::OsStr::new("")));
        let pairs: Vec<(&str, &str)> = env
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                (
                    "CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER",
                    r"C:\msys64\mingw64\bin\gcc.exe"
                ),
                ("CC_x86_64-pc-windows-gnu", r"C:\msys64\mingw64\bin\gcc.exe"),
                ("CC_x86_64_pc_windows_gnu", r"C:\msys64\mingw64\bin\gcc.exe"),
                ("PATH", r"C:\msys64\mingw64\bin"),
            ]
        );
    }

    /// gcc 自己的 bin 必须领先 PATH：32 位 gcc 混在 `mingw64\bin` 后面的 PATH 里会静默失败
    /// （`gcc -E` 退出码 1、无输出，`ring` 的编译器家族探测就是这么挂的）。
    #[test]
    fn windows_gnu_path_leads_with_the_toolchain_bin() {
        let base = std::ffi::OsStr::new(r"C:\Windows\system32");
        let sep = if cfg!(windows) { ';' } else { ':' };
        for (triple, gcc, bin) in [
            (
                "x86_64-pc-windows-gnu",
                r"C:\msys64\mingw64\bin\gcc.exe",
                r"C:\msys64\mingw64\bin",
            ),
            (
                "i686-pc-windows-gnu",
                r"C:\msys64\mingw32\bin\gcc.exe",
                r"C:\msys64\mingw32\bin",
            ),
        ] {
            let env = windows_gnu_env(std::path::Path::new(gcc), triple, Some(base));
            let path = env
                .iter()
                .find(|(key, _)| key == "PATH")
                .map(|(_, value)| value.as_str())
                .expect("缺少 PATH");
            assert_eq!(path, format!("{bin}{sep}C:\\Windows\\system32"));
        }
        // 非 windows-gnu 目标不注入 PATH
        assert!(
            !target_env_overrides("x86_64-unknown-linux-gnu")
                .iter()
                .any(|(key, _)| key == "PATH")
        );
    }

    /// 本机装了 mingw 时，PATH 指向对应位宽的 mingw bin（没装则不做断言，环境缺失不算失败）。
    #[test]
    fn target_env_overrides_follow_the_installed_mingw() {
        for (triple, mingw_dir) in [
            ("x86_64-pc-windows-gnu", "mingw64"),
            ("i686-pc-windows-gnu", "mingw32"),
        ] {
            let Some(gcc) = windows_gnu_toolchain(triple) else {
                continue;
            };
            let dir = gcc.parent().expect("gcc 无父目录").display().to_string();
            assert!(
                dir.to_lowercase().contains(mingw_dir),
                "{triple} 应命中 {mingw_dir}: {dir}"
            );
            let env = target_env_overrides(triple);
            let path = env
                .iter()
                .find(|(key, _)| key == "PATH")
                .map(|(_, value)| value.as_str())
                .expect("缺少 PATH");
            assert!(path.starts_with(&dir), "{triple}: {path}");
        }
    }

    /// 非 windows-gnu 目标不注入任何环境变量（linux 走 zig，宿主交给 cargo）。
    #[test]
    fn target_env_overrides_only_covers_windows_gnu() {
        assert!(target_env_overrides("x86_64-unknown-linux-gnu").is_empty());
        assert!(target_env_overrides("x86_64-pc-windows-msvc").is_empty());
    }

    /// zig cc 把 `-Wdate-time` 当错误：windows-gnu 的 zigbuild 带 `CFLAGS_*` 放宽它，
    /// linux 目标与其余目标不带（不污染其它目标的口径）。
    #[test]
    fn zigbuild_env_relaxes_date_time_only_for_windows_gnu() {
        for triple in ["x86_64-pc-windows-gnu", "i686-pc-windows-gnu"] {
            let target = Target::parse(triple).expect("三元组解析失败");
            let expected: Vec<(String, String)> = [
                format!("CFLAGS_{triple}"),
                format!("CFLAGS_{}", triple.replace('-', "_")),
            ]
            .into_iter()
            .map(|key| (key, "-Wno-error=date-time".to_owned()))
            .collect();
            assert_eq!(
                zigbuild_env(&target),
                expected,
                "{triple} 应带两套拼写的 CFLAGS_*"
            );
        }
        for triple in [
            "x86_64-unknown-linux-gnu",
            "i686-unknown-linux-gnu",
            "x86_64-pc-windows-msvc",
        ] {
            let target = Target::parse(triple).expect("三元组解析失败");
            assert!(
                zigbuild_env(&target).is_empty(),
                "{triple} 不该带 CFLAGS_*: {:?}",
                zigbuild_env(&target)
            );
        }
        assert!(
            zigbuild_env(&Target::host()).is_empty(),
            "宿主构建不带 CFLAGS_*"
        );
    }

    #[test]
    fn linux_only_filters_the_matrix() {
        let targets = plan(&args(&[], false, true, false)).expect("矩阵解析失败");
        let triples: Vec<Option<&str>> = targets
            .iter()
            .map(|target| target.triple.as_deref())
            .collect();
        assert_eq!(
            triples,
            [
                Some("x86_64-unknown-linux-gnu"),
                Some("i686-unknown-linux-gnu"),
                Some("aarch64-unknown-linux-gnu"),
            ]
        );
    }

    #[test]
    fn explicit_targets_keep_their_order_and_host_is_the_fallback() {
        let targets = plan(&args(
            &["aarch64-unknown-linux-gnu", "x86_64-pc-windows-msvc"],
            false,
            false,
            false,
        ))
        .expect("显式目标解析失败");
        assert_eq!(targets.len(), 2);
        assert_eq!(
            targets[0].triple.as_deref(),
            Some("aarch64-unknown-linux-gnu")
        );
        assert_eq!(targets[1].arch, "amd64");

        let host = plan(&args(&[], false, false, false)).expect("宿主目标解析失败");
        assert_eq!(host.len(), 1);
        assert!(host[0].triple.is_none(), "宿主构建不该带 --target");
        assert!(!host[0].os.is_empty() && !host[0].arch.is_empty());
    }

    #[test]
    fn built_binary_path_includes_the_triple_directory() {
        let target_dir = std::path::Path::new("target");
        let cross = Target::parse("x86_64-unknown-linux-gnu").expect("三元组解析失败");
        assert_eq!(
            built_binary(target_dir, &cross),
            std::path::Path::new("target/x86_64-unknown-linux-gnu/release/gensokyo")
        );

        let windows = Target::parse("x86_64-pc-windows-msvc").expect("三元组解析失败");
        assert_eq!(
            built_binary(target_dir, &windows),
            std::path::Path::new("target/x86_64-pc-windows-msvc/release/gensokyo.exe")
        );

        let host = Target::host();
        let host_path = built_binary(target_dir, &host);
        assert_eq!(
            host_path.parent(),
            Some(std::path::Path::new("target/release")),
            "宿主构建不应有 triple 层: {}",
            host_path.display()
        );
    }

    #[test]
    fn zig_gnu_targets_route_to_zigbuild() {
        let zigbuild = [
            "x86_64-unknown-linux-gnu",
            "i686-unknown-linux-gnu",
            "armv7-unknown-linux-gnueabihf",
            "aarch64-unknown-linux-gnu",
            "x86_64-pc-windows-gnu",
            "i686-pc-windows-gnu",
        ];
        for triple in zigbuild {
            let target = Target::parse(triple).expect("三元组解析失败");
            assert!(uses_zigbuild(&target), "{triple} 应走 zigbuild");
        }
        let plain = [
            "x86_64-unknown-linux-musl",
            "x86_64-apple-darwin",
            "aarch64-apple-darwin",
            "x86_64-pc-windows-msvc",
            "i686-pc-windows-msvc",
            "aarch64-linux-android",
        ];
        for triple in plain {
            let target = Target::parse(triple).expect("三元组解析失败");
            assert!(!uses_zigbuild(&target), "{triple} 不应走 zigbuild");
        }
        assert!(!uses_zigbuild(&Target::host()), "宿主构建不走 zigbuild");
    }

    #[test]
    fn skips_matrix_targets_whose_toolchain_the_host_lacks() {
        // windows-gnu 宿主：darwin 需 macOS，MSVC 需 cl.exe，android 需 NDK。
        assert_eq!(
            target_skip_reason("windows", Some("x86_64-apple-darwin"), false),
            Some("macOS 目标需要 macOS 宿主或专用交叉工具链")
        );
        assert_eq!(
            target_skip_reason("windows", Some("aarch64-apple-darwin"), false),
            Some("macOS 目标需要 macOS 宿主或专用交叉工具链")
        );
        assert_eq!(
            target_skip_reason("windows", Some("x86_64-pc-windows-msvc"), false),
            Some("需要 MSVC 工具链（cl.exe 不在 PATH）")
        );
        assert_eq!(
            target_skip_reason("windows", Some("i686-pc-windows-msvc"), false),
            Some("需要 MSVC 工具链（cl.exe 不在 PATH）")
        );
        assert_eq!(
            target_skip_reason("windows", Some("aarch64-linux-android"), false),
            Some("需要 Android NDK")
        );
    }

    #[test]
    fn keeps_targets_the_host_can_build() {
        // linux-gnu 由 zigbuild 自动引导，不拦；宿主自身（无 triple）永远构建。
        for triple in [
            "x86_64-unknown-linux-gnu",
            "i686-unknown-linux-gnu",
            "armv7-unknown-linux-gnueabihf",
            "aarch64-unknown-linux-gnu",
        ] {
            assert_eq!(target_skip_reason("windows", Some(triple), false), None);
        }
        assert_eq!(target_skip_reason("windows", None, false), None);
        assert_eq!(target_skip_reason("linux", None, false), None);
        // macOS 宿主可直接构建 darwin 目标；宿主上有 cl.exe（真 MSVC 工具链）时 MSVC 也不拦。
        assert_eq!(
            target_skip_reason("macos", Some("aarch64-apple-darwin"), false),
            None
        );
        assert_eq!(
            target_skip_reason("windows", Some("x86_64-pc-windows-msvc"), true),
            None
        );
        // windows-gnu 也走 zigbuild（zig 自带 PE 导入库，绕开 mingw dlltool 的 `-m i386pe`）。
        assert_eq!(
            target_skip_reason("windows", Some("x86_64-pc-windows-gnu"), false),
            None
        );
    }

    #[test]
    fn zigbuild_keeps_the_same_cargo_arguments() {
        let target = Target::parse("aarch64-unknown-linux-gnu").expect("三元组解析失败");
        let (subcommand, args) = cargo_invocation(&target, false);
        assert_eq!(subcommand, "zigbuild");
        assert_eq!(
            args,
            [
                "--release",
                "--locked",
                "-p",
                "gensokyo-bin",
                "--bin",
                "gensokyo",
                "--target",
                "aarch64-unknown-linux-gnu",
            ]
        );
        // 精简版只多一个 --no-default-features
        let (_, slim) = cargo_invocation(&target, true);
        assert_eq!(
            slim.last().map(String::as_str),
            Some("--no-default-features")
        );
        assert_eq!(slim.len(), args.len() + 1);

        // 非 linux-gnu 目标子命令保持 build
        let windows = Target::parse("x86_64-pc-windows-msvc").expect("三元组解析失败");
        let (subcommand, _) = cargo_invocation(&windows, false);
        assert_eq!(subcommand, "build");
        let (subcommand, args) = cargo_invocation(&Target::host(), false);
        assert_eq!(subcommand, "build");
        assert!(
            !args.iter().any(|arg| arg == "--target"),
            "宿主不带 --target"
        );
    }

    #[test]
    fn creates_webui_placeholders_when_dist_is_missing() {
        let root =
            std::env::temp_dir().join(format!("gensokyo-xtask-webui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("创建临时根目录失败");

        ensure_webui_dist(&root).expect("写入占位文件失败");
        for rel in super::WEBUI_PLACEHOLDERS {
            assert!(
                root.join("webui").join("dist").join(rel).is_file(),
                "缺少占位文件 {rel}"
            );
        }
        // `index.html` 是横幅控制台地址能打开的前提（缺它 `/` 与 `/webui` 都 404），
        // 内容与 Go `Ensure-WebUIDist` 一致
        let index = root.join("webui/dist/index.html");
        assert_eq!(
            std::fs::read_to_string(&index).expect("缺少 index.html 占位"),
            super::WEBUI_INDEX_PLACEHOLDER,
            "index.html 占位内容应与 Go 的 Ensure-WebUIDist 一致"
        );

        // 已有真实前端产物时不再介入
        let style = root.join("webui/dist/css/style.css");
        std::fs::write(&style, "body{}").expect("写入真实产物失败");
        let placeholder = root.join("webui/dist/placeholder.html");
        std::fs::remove_file(&placeholder).ok();
        ensure_webui_dist(&root).expect("已有产物时不应失败");
        assert!(
            !placeholder.exists(),
            "存在 css/style.css 时不该再写占位文件"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn build_version_prefers_override_then_git_then_dev() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_millis(1_700_000_000_007);
        // 流水线显式覆盖优先于自动推导
        assert_eq!(
            build_version(Some("git-release016"), Some("deadbee"), now),
            "git-release016"
        );
        // 有 git：git-<短 hash>，对应 Go 的 BuildType=git / BuildSpec=<short>
        assert_eq!(build_version(None, Some("deadbee"), now), "git-deadbee");
        // 无 git：dev-<epoch 秒>.<毫秒 3 位>，与 Go 的 BuildType=dev 同口径
        assert_eq!(build_version(None, None, now), "dev-1700000000.007");
        // 空串等同未设置，不能产出空版本串
        assert_eq!(build_version(Some(""), Some("deadbee"), now), "git-deadbee");
        assert_eq!(build_version(Some(""), Some(""), now), "dev-1700000000.007");
    }

    #[test]
    fn git_short_commit_is_seven_hex_chars_or_absent() {
        // cargo test 的工作目录是本 crate 目录（仓库内），git 可用时必能取到短 hash
        let root = std::env::current_dir().expect("取当前目录失败");
        let Some(commit) = git_short_commit(&root) else {
            return; // 无 git / 非仓库（打包源码树）：走 dev 回退，不做断言
        };
        assert_eq!(commit.len(), 7, "短 hash 长度: {commit}");
        assert!(
            commit.chars().all(|c| c.is_ascii_hexdigit()),
            "短 hash 应全为十六进制: {commit}"
        );
    }
}
