//! `xtask build`：release 构建 + 产物复制到 `release/`。
//!
//! 与 Go 版 `build.ps1` 对齐：产物命名 `gensokyo-{os}-{arch}[-noWebui][.exe]`、
//! `-All` 双版本、UPX 压缩。差异与取舍见 `docs/rust-build.md`。

use std::env::consts::{ARCH, OS};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::Args;

use crate::find_lockfile;

/// 发布目标矩阵（`--all` 使用），对齐 `.github/workflows/cross_compile.yml` 的 Go 矩阵。
const MATRIX: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "i686-unknown-linux-gnu",
    "armv7-unknown-linux-gnueabihf",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "i686-pc-windows-msvc",
    "aarch64-linux-android",
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
}

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

/// 产物文件名：与 Go 版 `build.ps1` / CI 完全一致。
fn artifact_name(os: &str, arch: &str, no_webui: bool) -> String {
    let flavor = if no_webui { "-noWebui" } else { "" };
    let ext = if os == "windows" { ".exe" } else { "" };
    format!("gensokyo-{os}-{arch}{flavor}{ext}")
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
fn workspace_root() -> Result<PathBuf> {
    let lock = find_lockfile()?;
    lock.parent()
        .map(Path::to_path_buf)
        .context("Cargo.lock 没有父目录")
}

/// cargo 可执行文件：`cargo run` 会注入 `$CARGO`，否则用 PATH 里的 cargo。
fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned())
}

/// `cargo metadata` 的 `target_directory`：尊重 `CARGO_TARGET_DIR` 与 `.cargo/config.toml`，
/// 不能假定产物在 `<root>/target`。
fn target_directory(root: &Path) -> Result<PathBuf> {
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
fn ensure_webui_dist(root: &Path) -> Result<()> {
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
    println!("webui/dist 无前端产物，已写入占位文件（rust-embed 需要目录存在）");
    Ok(())
}

/// `rustup target list --installed`；rustup 不可用时返回 `None`（退化为直接尝试构建）。
fn installed_targets() -> Option<Vec<String>> {
    let output = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .ok()?;
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

/// `cargo build --release --locked`：交叉时加 `--target`，精简版加 `--no-default-features`。
///
/// 只构建 bin 包——工作区还有 10 个库 crate 与 xtask 自身，全量构建没有意义。
fn cargo_build(root: &Path, target: &Target, no_webui: bool) -> Result<()> {
    let mut args: Vec<String> = [
        "build",
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

    println!("  $ cargo {}", args.join(" "));
    let status = Command::new(cargo())
        .current_dir(root)
        .args(&args)
        .status()
        .context("执行 cargo build 失败")?;
    if !status.success() {
        bail!("cargo build 失败（退出码 {status}）: {}", target.label());
    }
    Ok(())
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

/// 构建一个目标的某一个版本，复制到 `release/` 并返回产物路径。
fn build_one(
    root: &Path,
    target_dir: &Path,
    release_dir: &Path,
    target: &Target,
    no_webui: bool,
) -> Result<PathBuf> {
    let name = artifact_name(&target.os, &target.arch, no_webui);
    println!("[build] {} → release/{name}", target.label());
    cargo_build(root, target, no_webui)?;

    let built = built_binary(target_dir, target);
    if !built.is_file() {
        bail!("cargo 报告成功，但未找到产物 {}", built.display());
    }
    let dest = release_dir.join(&name);
    fs::copy(&built, &dest)
        .with_context(|| format!("复制 {} → {} 失败", built.display(), dest.display()))?;
    println!("  OK: {} ({:.1} MB)", dest.display(), megabytes(&dest));
    Ok(dest)
}

/// 文件大小（MB）；读不到时返回 0（刚复制完的文件不该读不到，这里不额外报错）。
fn megabytes(path: &Path) -> f64 {
    fs::metadata(path).map_or(0, |meta| meta.len()) as f64 / (1024.0 * 1024.0)
}

/// UPX 压缩；PATH 里没有 upx 只提示并跳过（不是构建失败）。
fn compress(path: &Path, level: u8) -> Result<()> {
    println!("[upx] 压缩 {} ...", path.display());
    match Command::new("upx")
        .arg(format!("-{level}"))
        .arg(path)
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => bail!("UPX 压缩失败（退出码 {status}）: {}", path.display()),
        Err(err) if err.kind() == ErrorKind::NotFound => {
            println!(
                "  UPX 未在 PATH 中找到，跳过压缩（安装：winget install upx；或加 --no-upx 静默）"
            );
            Ok(())
        }
        Err(err) => Err(err).with_context(|| format!("执行 upx 失败: {}", path.display())),
    }
}

/// `xtask build` 入口：解析目标 → 构建 → 复制 → 可选 UPX → 汇总。
///
/// `--all` 下未安装的目标打印跳过原因并继续；真正编译失败的目标记入失败清单，
/// 其余目标继续构建，最终以非零退出码汇总。
pub fn run(args: BuildArgs) -> Result<()> {
    let root = workspace_root()?;
    let targets = plan(&args)?;
    let flavors = flavors(&args);
    let matrix_mode = args.all || args.linux_only;

    ensure_webui_dist(&root)?;
    let target_dir = target_directory(&root)?;
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

    let installed = if matrix_mode {
        installed_targets()
    } else {
        None
    };
    let mut outputs = Vec::new();
    let mut skipped = Vec::new();
    let mut failures = Vec::new();

    for target in &targets {
        if let (Some(installed), Some(triple)) = (&installed, &target.triple)
            && !installed.iter().any(|value| value == triple)
        {
            let reason = format!("{triple} 未安装 → rustup target add {triple}");
            println!("[skip] {reason}");
            skipped.push(reason);
            continue;
        }
        for &no_webui in flavors {
            match build_one(&root, &target_dir, &release_dir, target, no_webui) {
                Ok(path) => outputs.push(path),
                Err(err) if matrix_mode => {
                    eprintln!("[fail] {err:#}");
                    failures.push(format!("{}/{no_webui}", target.label()));
                }
                Err(err) => return Err(err),
            }
        }
    }

    if args.no_upx {
        println!("UPX 已禁用（--no-upx）");
    } else {
        for path in &outputs {
            compress(path, args.upx_level)?;
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
        bail!("没有产出任何产物（目标全部跳过？本地 --all 请先 rustup target add）");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        BuildArgs, Target, artifact_name, built_binary, ensure_webui_dist, flavors, plan,
        split_triple,
    };

    /// 测试用参数；未设置的字段一律取“不生效”的值。
    fn args(target: &[&str], all: bool, linux_only: bool, no_webui: bool) -> BuildArgs {
        BuildArgs {
            target: target.iter().map(|value| (*value).to_owned()).collect(),
            all,
            linux_only,
            no_webui,
            no_upx: true,
            upx_level: 7,
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

    #[test]
    fn artifact_names_match_go_build_ps1() {
        assert_eq!(
            artifact_name("windows", "amd64", false),
            "gensokyo-windows-amd64.exe"
        );
        assert_eq!(
            artifact_name("windows", "386", true),
            "gensokyo-windows-386-noWebui.exe"
        );
        assert_eq!(
            artifact_name("linux", "arm64", false),
            "gensokyo-linux-arm64"
        );
        assert_eq!(
            artifact_name("darwin", "amd64", true),
            "gensokyo-darwin-amd64-noWebui"
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
    fn linux_only_filters_the_matrix() {
        let targets = plan(&args(&[], false, true, false)).expect("矩阵解析失败");
        assert!(targets.len() >= 3, "Linux 目标至少 3 个: {targets:?}");
        assert!(
            targets.iter().all(|target| target.os == "linux"),
            "linux-only 不能带出非 Linux 目标: {targets:?}"
        );
        assert!(
            targets
                .iter()
                .any(|target| target.arch == "arm64" && target.os == "linux"),
            "缺少 linux/arm64: {targets:?}"
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
}
