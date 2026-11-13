//! Linux 交叉目标（linux-gnu×4）的工具链自动引导：zig + cargo-zigbuild。
//!
//! 探测顺序 PATH（须通过版本比对）→ `<workspace>/.tools/`（本地缓存，gitignore）→ 按宿主平台从官方 URL
//! 下载。下载/解压/校验全部走系统工具（curl + tar + certutil/sha256sum），不新增 cargo
//! 依赖；`.tools/` 齐全时零网络。darwin / MSVC / android 不在引导范围，维持原报错口径。
//! 版本钉死，可用 `GENSOKYO_ZIG_VERSION` / `GENSOKYO_CARGO_ZIGBUILD_VERSION` 覆盖
//! （见 docs/rust-build.md）。

use std::env::consts::{ARCH, OS};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// 钉死版本：cargo-zigbuild README 明确支持 zig 0.15/0.16，0.17 兼容性未证实，勿升级。
pub const ZIG_VERSION: &str = "0.16.0";
pub const CARGO_ZIGBUILD_VERSION: &str = "0.24.1";

/// 本地工具缓存目录名（workspace 根下，.gitignore 忽略）。
pub const TOOLS_DIR: &str = ".tools";

/// 宿主平台（仅引导支持的四类；其余平台报错并给手动安装指引）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostPlatform {
    WindowsX86_64,
    WindowsAarch64,
    LinuxX86_64,
    LinuxAarch64,
}

impl HostPlatform {
    /// 当前宿主；macOS / 32 位等不在官方预编译覆盖内，返回 None。
    pub fn detect() -> Option<Self> {
        Self::from_os_arch(OS, ARCH)
    }

    pub fn from_os_arch(os: &str, arch: &str) -> Option<Self> {
        match (os, arch) {
            ("windows", "x86_64") => Some(Self::WindowsX86_64),
            ("windows", "aarch64") => Some(Self::WindowsAarch64),
            ("linux", "x86_64") => Some(Self::LinuxX86_64),
            ("linux", "aarch64") => Some(Self::LinuxAarch64),
            _ => None,
        }
    }

    /// zig 官方包名里的平台段。
    fn zig_slug(self) -> &'static str {
        match self {
            Self::WindowsX86_64 => "x86_64-windows",
            Self::WindowsAarch64 => "aarch64-windows",
            Self::LinuxX86_64 => "x86_64-linux",
            Self::LinuxAarch64 => "aarch64-linux",
        }
    }

    /// cargo-zigbuild 预编译包的 Rust 三元组。
    fn zigbuild_triple(self) -> &'static str {
        match self {
            Self::WindowsX86_64 => "x86_64-pc-windows-msvc",
            Self::WindowsAarch64 => "aarch64-pc-windows-msvc",
            Self::LinuxX86_64 => "x86_64-unknown-linux-gnu",
            Self::LinuxAarch64 => "aarch64-unknown-linux-gnu",
        }
    }

    fn archive_ext(self) -> &'static str {
        match self {
            Self::WindowsX86_64 | Self::WindowsAarch64 => "zip",
            Self::LinuxX86_64 | Self::LinuxAarch64 => "tar.xz",
        }
    }
}

/// 版本覆盖：环境变量优先，未设置/空白用钉死值；只接受 `数字[.数字]*`。
pub fn resolve_version(env_value: Option<&str>, pinned: &str) -> Result<String> {
    let Some(raw) = env_value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(pinned.to_owned());
    };
    if raw
        .split('.')
        .any(|seg| seg.is_empty() || !seg.bytes().all(|byte| byte.is_ascii_digit()))
    {
        bail!("非法版本号 `{raw}`（每段须为非空纯数字，例如 0.16.0）");
    }
    Ok(raw.to_owned())
}

/// zig 官方下载 URL（逐字使用官方源，不加代理）。
pub fn zig_url(host: HostPlatform, version: &str) -> String {
    format!(
        "https://ziglang.org/download/{version}/zig-{}-{version}.{}",
        host.zig_slug(),
        host.archive_ext()
    )
}

/// cargo-zigbuild 官方 release 下载 URL。
pub fn zigbuild_url(host: HostPlatform, version: &str) -> String {
    format!(
        "https://github.com/rust-cross/cargo-zigbuild/releases/download/v{version}/cargo-zigbuild-{}.{}",
        host.zigbuild_triple(),
        host.archive_ext()
    )
}

/// 归档文件名（URL 的最后一段）。
pub fn archive_name(url: &str) -> String {
    url.rsplit('/').next().unwrap_or(url).to_owned()
}

/// 可执行文件名（Windows 带 .exe）。
pub fn exe_name(base: &str) -> String {
    if OS == "windows" {
        format!("{base}.exe")
    } else {
        base.to_owned()
    }
}

/// `.tools/` 下的版本化安装目录：`zig-<版本>` / `cargo-zigbuild-<版本>`。
pub fn tool_dir(root: &Path, tool: &str, version: &str) -> PathBuf {
    root.join(TOOLS_DIR).join(format!("{tool}-{version}"))
}

/// PATH 探测：命中返回可执行文件所在目录。Windows 用 where.exe（避开 PowerShell 的
/// curl/where 别名），其余用 which；命令不存在按“未找到”处理。
fn find_on_path(name: &str) -> Option<PathBuf> {
    let (program, arg) = if OS == "windows" {
        ("where.exe", name)
    } else {
        ("which", name)
    };
    let output = Command::new(program).arg(arg).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let first = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?
        .to_owned();
    Path::new(&first).parent().map(Path::to_path_buf)
}

/// 缓存探测：`.tools/<tool>-<version>/<可执行文件>` 存在即命中。
pub fn locate_cached(root: &Path, tool: &str, version: &str) -> Option<PathBuf> {
    let dir = tool_dir(root, tool, version);
    dir.join(exe_name(tool)).is_file().then_some(dir)
}

/// 版本输出是否与钉死版本一致：输出里须有与钉死值相等的独立词段
/// （`zig version` → `0.16.0`；`cargo-zigbuild --version` → `cargo-zigbuild 0.24.1`）。
fn version_output_matches(output: &str, expected: &str) -> bool {
    output.split_whitespace().any(|token| token == expected)
}

/// 查询工具版本输出；无法执行或退出码非零返回 None。
fn tool_version_output(dir: &Path, tool: &str) -> Option<String> {
    let args: &[&str] = if tool == "zig" {
        &["version"]
    } else {
        &["--version"]
    };
    let output = Command::new(dir.join(exe_name(tool)))
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// 探测顺序 PATH（须版本比对通过）→ `.tools/` 缓存；都未命中返回 None（调用方决定是否下载）。
/// PATH 命中但版本不符（或查不出版本）时不用它，warn 后落回 `.tools/` 钉版，
/// 防止宿主自装的其他版本击穿钉版。
pub fn locate_tool(root: &Path, tool: &str, version: &str) -> Option<PathBuf> {
    if let Some(dir) = find_on_path(&exe_name(tool)) {
        if tool_version_output(&dir, tool).is_some_and(|out| version_output_matches(&out, version)) {
            return Some(dir);
        }
        eprintln!(
            "  [bootstrap] 警告: PATH 上的 {tool} 与钉死版本 {version} 不符，忽略 PATH 命中，改用 {TOOLS_DIR}/ 钉版（缺失则下载）"
        );
    }
    locate_cached(root, tool, version)
}

/// 从校验文本里提取 64 位十六进制 sha256（兼容官方 `.sha256` 的
/// `<hash>  <文件名>` 与 Windows certutil 的多行输出）。
pub fn parse_sha256(text: &str) -> Option<String> {
    text.split(|ch: char| ch.is_whitespace() || ch == ':')
        .find(|token| token.len() == 64 && token.chars().all(|ch| ch.is_ascii_hexdigit()))
        .map(str::to_lowercase)
}

/// 计算文件 sha256：Windows 用 certutil，其余用 sha256sum（都是系统自带）。
fn file_sha256(path: &Path) -> Result<String> {
    let output = if OS == "windows" {
        Command::new("certutil")
            .args(["-hashfile"])
            .arg(path)
            .arg("SHA256")
            .output()
            .context("执行 certutil 失败")?
    } else {
        Command::new("sha256sum")
            .arg(path)
            .output()
            .context("执行 sha256sum 失败")?
    };
    if !output.status.success() {
        bail!(
            "计算 sha256 失败（{}）: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    parse_sha256(&String::from_utf8_lossy(&output.stdout))
        .with_context(|| format!("无法从校验工具输出解析 sha256: {}", path.display()))
}

/// curl 下载（系统自带；Windows 显式 curl.exe，避开 PowerShell 的 Invoke-WebRequest 别名）。
fn download(url: &str, dest: &Path) -> Result<()> {
    let program = if OS == "windows" { "curl.exe" } else { "curl" };
    println!("  [bootstrap] 下载 {url}");
    let status = Command::new(program)
        .args([
            "--fail",
            "--location",
            "--retry",
            "3",
            "--connect-timeout",
            "30",
            "--max-time",
            "900",
            "--output",
        ])
        .arg(dest)
        .arg(url)
        .status()
        .with_context(|| format!("执行 {program} 失败（系统需自带 curl）"))?;
    if !status.success() {
        bail!("下载失败（curl 退出码 {status}）: {url}");
    }
    let size = fs::metadata(dest).map(|meta| meta.len()).unwrap_or(0);
    if size == 0 {
        bail!("下载结果为空文件: {url}");
    }
    Ok(())
}

/// 校验归档：cargo-zigbuild 用官方 `.sha256` 逐字比对；zig 官方下载索引不提供
/// 逐文件 sha256，退化为非空检查 + 解压时 tar 的归档完整性校验（失败即报错）。
fn verify_archive(tool: &str, url: &str, archive: &Path) -> Result<()> {
    if tool == "cargo-zigbuild" {
        let sha_url = format!("{url}.sha256");
        let sha_path = archive.with_extension("sha256");
        download(&sha_url, &sha_path)?;
        let expected = parse_sha256(
            &fs::read_to_string(&sha_path)
                .with_context(|| format!("读取 {} 失败", sha_path.display()))?,
        )
        .with_context(|| format!("官方 .sha256 内容无法解析: {sha_url}"))?;
        let actual = file_sha256(archive)?;
        if actual != expected {
            // 中止前清掉损坏归档与校验残留文件
            let _ = fs::remove_file(archive);
            let _ = fs::remove_file(&sha_path);
            bail!(
                "sha256 校验失败: {url}\n  期望 {expected}\n  实际 {actual}\n归档可能损坏或被篡改，已中止"
            );
        }
        println!("  [bootstrap] sha256 校验通过: {}", archive_name(url));
        let _ = fs::remove_file(&sha_path);
    } else {
        println!(
            "  [bootstrap] zig 官方索引无逐文件 sha256，按非空 + tar 解压完整性校验: {}",
            archive_name(url)
        );
    }
    Ok(())
}

/// 解压到安装目录：tar 同时支持 zip（Windows 自带 bsdtar）与 tar.xz；
/// Linux 上 tar 解 zip 失败时回退 unzip。
fn extract(archive: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest).with_context(|| format!("创建 {} 失败", dest.display()))?;
    let is_zip = archive.extension().is_some_and(|ext| ext == "zip");
    let status = Command::new("tar")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .status()
        .context("执行 tar 失败（系统需自带 tar）")?;
    if status.success() {
        return Ok(());
    }
    if is_zip && OS != "windows" {
        let unzip = Command::new("unzip")
            .arg("-o")
            .arg(archive)
            .arg("-d")
            .arg(dest)
            .status();
        if matches!(&unzip, Ok(status) if status.success()) {
            return Ok(());
        }
    }
    bail!(
        "解压失败（tar 退出码 {status}）: {} → {}",
        archive.display(),
        dest.display()
    );
}

/// 解压后二进制不在目录根部时，找到含它的一级子目录，把该子目录的**全部条目**
/// 提升到根部：zig 官方包布局是 `zig-<平台>-<版本>/zig + lib/`，zig 按可执行文件
/// 相对路径找 lib/，只提 exe 不提 lib/ 必挂；cargo-zigbuild 包是根部布局
/// （无内层目录），此时直接返回、不误伤。
fn hoist_binary(dir: &Path, exe: &str) -> Result<()> {
    if dir.join(exe).is_file() {
        return Ok(());
    }
    let entries = fs::read_dir(dir).with_context(|| format!("读取 {} 失败", dir.display()))?;
    for entry in entries.flatten() {
        let inner = entry.path();
        if !inner.is_dir() || !inner.join(exe).is_file() {
            continue;
        }
        let items =
            fs::read_dir(&inner).with_context(|| format!("读取 {} 失败", inner.display()))?;
        for item in items.flatten() {
            let from = item.path();
            let to = dir.join(item.file_name());
            fs::rename(&from, &to)
                .with_context(|| format!("移动 {} → {} 失败", from.display(), to.display()))?;
        }
        let _ = fs::remove_dir(&inner);
        return Ok(());
    }
    bail!(
        "解压完成但未找到 {exe}（目录 {}）；官方包布局可能已变化",
        dir.display()
    );
}

/// 确保单个工具可用：已探测到直接返回；否则下载 → 校验 → 解压 → 确认二进制。
fn ensure_tool(root: &Path, host: HostPlatform, tool: &str, version: &str) -> Result<PathBuf> {
    if let Some(dir) = locate_tool(root, tool, version) {
        println!("  [bootstrap] {tool} {version} 已就绪: {}", dir.display());
        return Ok(dir);
    }
    let url = if tool == "zig" {
        zig_url(host, version)
    } else {
        zigbuild_url(host, version)
    };
    let dir = tool_dir(root, tool, version);
    let archive = root.join(TOOLS_DIR).join(archive_name(&url));
    if let Some(parent) = archive.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建 {} 失败", parent.display()))?;
    }
    let result = (|| -> Result<()> {
        download(&url, &archive)?;
        verify_archive(tool, &url, &archive)?;
        extract(&archive, &dir)?;
        hoist_binary(&dir, &exe_name(tool))?;
        Ok(())
    })();
    // 失败时清掉半成品目录与归档，避免下次误判“已缓存”；重试即重新下载。
    if result.is_err() {
        let _ = fs::remove_dir_all(&dir);
    }
    let _ = fs::remove_file(&archive);
    result.with_context(|| format!("引导 {tool} {version} 失败（URL: {url}）"))?;
    println!("  [bootstrap] {tool} {version} 已安装到 {}", dir.display());
    Ok(dir)
}

/// 工具目录前置后的 PATH 值（供 `Command::env("PATH", …)` 注入子进程）。
///
/// workspace `forbid(unsafe_code)`，edition 2024 的 `env::set_var` 是 unsafe，
/// 所以不改本进程环境，只把算好的 PATH 交给 cargo 子进程（cargo-zigbuild 再继承它找 zig）。
pub fn path_with_tools(dirs: &[PathBuf]) -> Result<std::ffi::OsString> {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut parts: Vec<PathBuf> = std::env::split_paths(&current).collect();
    for dir in dirs.iter().rev() {
        if !parts.iter().any(|part| part == dir) {
            parts.insert(0, dir.clone());
        }
    }
    std::env::join_paths(&parts).context("拼接 PATH 失败")
}

/// linux-gnu 交叉构建前的工具引导入口：zig + cargo-zigbuild 就绪，返回目录清单
/// （调用方用 `path_with_tools` 注入构建子进程的 PATH）。
/// 全部命中缓存/PATH 时零网络；任一下载失败直接报错（不静默回退 plain cargo build）。
pub fn bootstrap_linux_tools(root: &Path) -> Result<Vec<PathBuf>> {
    let Some(host) = HostPlatform::detect() else {
        bail!(
            "当前宿主（{OS}/{ARCH}）没有官方预编译的 zig/cargo-zigbuild 下载源；\
             请手动安装 zig {ZIG_VERSION} 与 cargo-zigbuild {CARGO_ZIGBUILD_VERSION} 并加入 PATH 后重试"
        );
    };
    let zig_version = resolve_version(
        std::env::var("GENSOKYO_ZIG_VERSION").ok().as_deref(),
        ZIG_VERSION,
    )?;
    let zigbuild_version = resolve_version(
        std::env::var("GENSOKYO_CARGO_ZIGBUILD_VERSION").ok().as_deref(),
        CARGO_ZIGBUILD_VERSION,
    )?;
    println!(
        "[bootstrap] Linux 交叉工具链: zig {zig_version} + cargo-zigbuild {zigbuild_version}（探测 PATH → {TOOLS_DIR}/ → 官方下载）"
    );
    let zig_dir = ensure_tool(root, host, "zig", &zig_version)?;
    let zigbuild_dir = ensure_tool(root, host, "cargo-zigbuild", &zigbuild_version)?;
    Ok(vec![zig_dir, zigbuild_dir])
}

#[cfg(test)]
mod tests {
    use super::{
        CARGO_ZIGBUILD_VERSION, HostPlatform, ZIG_VERSION, archive_name, locate_cached,
        locate_tool, parse_sha256, resolve_version, tool_dir, zig_url, zigbuild_url,
    };

    #[test]
    fn host_detection_covers_the_four_official_platforms() {
        assert_eq!(
            HostPlatform::from_os_arch("windows", "x86_64"),
            Some(HostPlatform::WindowsX86_64)
        );
        assert_eq!(
            HostPlatform::from_os_arch("windows", "aarch64"),
            Some(HostPlatform::WindowsAarch64)
        );
        assert_eq!(
            HostPlatform::from_os_arch("linux", "x86_64"),
            Some(HostPlatform::LinuxX86_64)
        );
        assert_eq!(
            HostPlatform::from_os_arch("linux", "aarch64"),
            Some(HostPlatform::LinuxAarch64)
        );
        // macOS / 32 位没有官方预编译源
        assert_eq!(HostPlatform::from_os_arch("macos", "aarch64"), None);
        assert_eq!(HostPlatform::from_os_arch("windows", "x86"), None);
    }

    #[test]
    fn urls_match_the_official_sources_verbatim() {
        assert_eq!(
            zig_url(HostPlatform::WindowsX86_64, ZIG_VERSION),
            "https://ziglang.org/download/0.16.0/zig-x86_64-windows-0.16.0.zip"
        );
        assert_eq!(
            zig_url(HostPlatform::LinuxX86_64, ZIG_VERSION),
            "https://ziglang.org/download/0.16.0/zig-x86_64-linux-0.16.0.tar.xz"
        );
        assert_eq!(
            zig_url(HostPlatform::WindowsAarch64, ZIG_VERSION),
            "https://ziglang.org/download/0.16.0/zig-aarch64-windows-0.16.0.zip"
        );
        assert_eq!(
            zig_url(HostPlatform::LinuxAarch64, ZIG_VERSION),
            "https://ziglang.org/download/0.16.0/zig-aarch64-linux-0.16.0.tar.xz"
        );
        assert_eq!(
            zigbuild_url(HostPlatform::WindowsX86_64, CARGO_ZIGBUILD_VERSION),
            "https://github.com/rust-cross/cargo-zigbuild/releases/download/v0.24.1/cargo-zigbuild-x86_64-pc-windows-msvc.zip"
        );
        assert_eq!(
            zigbuild_url(HostPlatform::WindowsAarch64, CARGO_ZIGBUILD_VERSION),
            "https://github.com/rust-cross/cargo-zigbuild/releases/download/v0.24.1/cargo-zigbuild-aarch64-pc-windows-msvc.zip"
        );
        assert_eq!(
            zigbuild_url(HostPlatform::LinuxX86_64, CARGO_ZIGBUILD_VERSION),
            "https://github.com/rust-cross/cargo-zigbuild/releases/download/v0.24.1/cargo-zigbuild-x86_64-unknown-linux-gnu.tar.xz"
        );
        assert_eq!(
            zigbuild_url(HostPlatform::LinuxAarch64, CARGO_ZIGBUILD_VERSION),
            "https://github.com/rust-cross/cargo-zigbuild/releases/download/v0.24.1/cargo-zigbuild-aarch64-unknown-linux-gnu.tar.xz"
        );
    }

    #[test]
    fn versions_are_pinned_and_overridable() {
        assert_eq!(ZIG_VERSION, "0.16.0");
        assert_eq!(CARGO_ZIGBUILD_VERSION, "0.24.1");
        assert_eq!(resolve_version(None, ZIG_VERSION).unwrap(), "0.16.0");
        assert_eq!(resolve_version(Some("  "), ZIG_VERSION).unwrap(), "0.16.0");
        assert_eq!(
            resolve_version(Some("0.15.2"), ZIG_VERSION).unwrap(),
            "0.15.2"
        );
        assert!(resolve_version(Some("0.16.0; rm -rf"), ZIG_VERSION).is_err());
        assert!(resolve_version(Some("latest"), ZIG_VERSION).is_err());
        // 严格分段：每段非空纯数字
        for bad in [".", "..", "1..2", ".5", "5.", "0.16.0.", "1.2.x"] {
            assert!(
                resolve_version(Some(bad), ZIG_VERSION).is_err(),
                "应拒绝非法版本号 `{bad}`"
            );
        }
        // 覆盖只影响 URL 里的版本段
        assert_eq!(
            zig_url(HostPlatform::LinuxX86_64, "0.15.2"),
            "https://ziglang.org/download/0.15.2/zig-x86_64-linux-0.15.2.tar.xz"
        );
    }

    #[test]
    fn archive_name_is_the_url_tail() {
        assert_eq!(
            archive_name(&zig_url(HostPlatform::WindowsX86_64, ZIG_VERSION)),
            "zig-x86_64-windows-0.16.0.zip"
        );
    }

    #[test]
    fn cached_tool_is_found_under_dot_tools() {
        let root = std::env::temp_dir().join(format!("gensokyo-bootstrap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = tool_dir(&root, "zig", ZIG_VERSION);
        assert!(locate_cached(&root, "zig", ZIG_VERSION).is_none());
        std::fs::create_dir_all(&dir).expect("创建缓存目录失败");
        let exe = dir.join(super::exe_name("zig"));
        std::fs::write(&exe, b"stub").expect("写入假二进制失败");
        assert_eq!(locate_cached(&root, "zig", ZIG_VERSION), Some(dir.clone()));
        // 版本不匹配不算命中
        assert!(locate_cached(&root, "zig", "0.15.2").is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn version_output_matching_compares_standalone_tokens() {
        assert!(super::version_output_matches("0.16.0", "0.16.0"));
        assert!(super::version_output_matches("0.16.0\n", "0.16.0"));
        assert!(super::version_output_matches(
            "cargo-zigbuild 0.24.1",
            "0.24.1"
        ));
        assert!(!super::version_output_matches("0.17.0", "0.16.0"));
        assert!(!super::version_output_matches("0.16.0-dev.42", "0.16.0"));
        assert!(!super::version_output_matches("", "0.16.0"));
    }

    #[test]
    fn path_hit_requires_matching_version() {
        // `cargo` 一定在 PATH 里（测试由 cargo 启动）；取其真实版本做比对。
        let output = std::process::Command::new("cargo")
            .arg("--version")
            .output()
            .expect("应能执行 cargo --version");
        let text = String::from_utf8_lossy(&output.stdout);
        let version = text
            .split_whitespace()
            .nth(1)
            .expect("cargo --version 应含版本段")
            .to_owned();
        let root =
            std::env::temp_dir().join(format!("gensokyo-bootstrap-prio-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = tool_dir(&root, "cargo", &version);
        std::fs::create_dir_all(&dir).expect("创建缓存目录失败");
        std::fs::write(dir.join(super::exe_name("cargo")), b"stub").expect("写入假二进制失败");
        let hit = locate_tool(&root, "cargo", &version).expect("PATH 里应有 cargo");
        assert_ne!(hit, dir, "版本相符的 PATH 命中必须优先于 .tools 缓存");
        // 版本不符：不用 PATH 命中，落回 .tools 钉版目录
        let mismatch = tool_dir(&root, "cargo", "0.0.0");
        std::fs::create_dir_all(&mismatch).expect("创建缓存目录失败");
        std::fs::write(mismatch.join(super::exe_name("cargo")), b"stub")
            .expect("写入假二进制失败");
        assert_eq!(
            locate_tool(&root, "cargo", "0.0.0"),
            Some(mismatch),
            "PATH 版本不符应落回 .tools 钉版"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn hoist_moves_the_whole_inner_directory_including_lib() {
        // zig 官方包布局：内层目录 = exe + lib/ + 其他文件，必须整体提升
        let root =
            std::env::temp_dir().join(format!("gensokyo-bootstrap-hoist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let inner = root.join("zig-x86_64-windows-0.16.0");
        std::fs::create_dir_all(inner.join("lib")).expect("创建内层目录失败");
        let exe = super::exe_name("zig");
        std::fs::write(inner.join(&exe), b"stub").expect("写入假二进制失败");
        std::fs::write(inner.join("lib").join("std.zig"), b"stub").expect("写入 lib 失败");
        std::fs::write(inner.join("LICENSE"), b"stub").expect("写入 LICENSE 失败");
        super::hoist_binary(&root, &exe).expect("整体提升应成功");
        assert!(root.join(&exe).is_file(), "exe 应提升到根部");
        assert!(
            root.join("lib").join("std.zig").is_file(),
            "lib/ 必须随 exe 一起提升"
        );
        assert!(root.join("LICENSE").is_file(), "其余条目也应提升");
        assert!(!inner.exists(), "清空后的内层目录应移除");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn hoist_leaves_root_layout_untouched() {
        // cargo-zigbuild 包布局：二进制在根部、无内层目录，不得误伤
        let root = std::env::temp_dir()
            .join(format!("gensokyo-bootstrap-hoist-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("docs")).expect("创建目录失败");
        let exe = super::exe_name("cargo-zigbuild");
        std::fs::write(root.join(&exe), b"stub").expect("写入假二进制失败");
        std::fs::write(root.join("docs").join(&exe), b"decoy").expect("写入诱饵失败");
        super::hoist_binary(&root, &exe).expect("根部布局应直接通过");
        assert_eq!(
            std::fs::read(root.join(&exe)).unwrap(),
            b"stub",
            "根部二进制不得被替换"
        );
        assert!(root.join("docs").join(&exe).is_file(), "子目录不得被动");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn hoist_fails_when_the_exe_is_missing() {
        let root = std::env::temp_dir()
            .join(format!("gensokyo-bootstrap-hoist-miss-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("创建目录失败");
        std::fs::write(root.join("README"), b"stub").expect("写入失败");
        assert!(super::hoist_binary(&root, &super::exe_name("zig")).is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn path_with_tools_prepends_and_dedups_without_touching_the_process_env() {
        let before = std::env::var_os("PATH").expect("测试进程应有 PATH");
        let dir = std::path::PathBuf::from("gensokyo-bootstrap-fake-tools");
        let joined = super::path_with_tools(std::slice::from_ref(&dir)).expect("拼接 PATH 失败");
        let parts: Vec<_> = std::env::split_paths(&joined).collect();
        assert_eq!(parts.first(), Some(&dir), "工具目录必须前置");
        // 重复注入不再叠加
        let twice = super::path_with_tools(&[dir.clone(), dir.clone()]).expect("拼接 PATH 失败");
        let count = std::env::split_paths(&twice)
            .filter(|part| part == &dir)
            .count();
        assert_eq!(count, 1, "同一目录只应出现一次");
        assert_eq!(std::env::var_os("PATH"), Some(before), "不得修改进程环境");
    }

    #[test]
    fn sha256_parsing_handles_both_official_and_certutil_formats() {
        let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(
            parse_sha256(&format!("{hash}  cargo-zigbuild-x86_64-pc-windows-msvc.zip\n")).as_deref(),
            Some(hash)
        );
        let certutil = format!("SHA256 hash of file:\n{hash}\nCertUtil: -hashfile command completed successfully.\n");
        assert_eq!(parse_sha256(&certutil).as_deref(), Some(hash));
        assert_eq!(parse_sha256("no hash here"), None);
    }
}