//! 构建 / 发布辅助（`build.ps1` 的 Rust 侧等价物）。
//!
//! M0 提供：产物命名规则（只打印，不打包）与 TLS 依赖下限断言。

use std::collections::BTreeMap;
use std::env::consts::{ARCH, OS};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

/// TLS 组件安全下限（design §9.1；RustSec-2026-0382/0383 / CVE-2026-111943）。
const TLS_FLOORS: &[(&str, Version)] = &[
    ("rustls", Version::new(0, 23, 46)),
    ("rustls-webpki", Version::new(0, 103, 16)),
    ("webpki-root-certs", Version::new(1, 0, 10)),
];

/// 三段版本号，仅用于下限比较（预发布后缀忽略）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Version(u64, u64, u64);

impl Version {
    const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self(major, minor, patch)
    }

    /// 解析 `major.minor.patch`；预发布/构建后缀按数字前缀截断。
    fn parse(raw: &str) -> Option<Self> {
        let mut parts = raw.trim().split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch_raw = parts.next().unwrap_or("0");
        let patch_digits: String = patch_raw.chars().take_while(char::is_ascii_digit).collect();
        let patch = if patch_digits.is_empty() {
            0
        } else {
            patch_digits.parse().ok()?
        };
        Some(Self(major, minor, patch))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// 从 `Cargo.lock` 文本里取出 `包名 → 全部版本`。
///
/// 同一包可能同时存在多个版本（间接依赖各锁一个），必须全部保留：下限判定按**最低版本**
/// 走，否则只要有一个新版本进来就会掩盖旧的不安全版本。
fn lock_versions(lock: &str) -> BTreeMap<String, Vec<String>> {
    let mut versions: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut current: Option<String> = None;

    for line in lock.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            current = None;
            continue;
        }
        if let Some(name) = line.strip_prefix("name = \"") {
            current = Some(name.trim_end_matches('"').to_owned());
            continue;
        }
        if let Some(version) = line.strip_prefix("version = \"") {
            if let Some(name) = current.take() {
                versions
                    .entry(name)
                    .or_default()
                    .push(version.trim_end_matches('"').to_owned());
            }
        }
    }

    versions
}

/// 向上查找工作区 `Cargo.lock`（`cargo xtask` 可能在子目录被调用）。
fn find_lockfile() -> Result<PathBuf> {
    let mut dir = std::env::current_dir().context("读取当前目录失败")?;
    for _ in 0..6 {
        let candidate = dir.join("Cargo.lock");
        if candidate.is_file() {
            return Ok(candidate);
        }
        if !dir.pop() {
            break;
        }
    }
    bail!("未找到 Cargo.lock（请在 workspace 或其子目录下运行）")
}

/// 断言 `Cargo.lock` 中 TLS 组件不低于安全下限；任一不满足即非零退出。
///
/// 同名多版本共存时按**最低**版本判定（最保守），并打印实际比较值与版本个数。
fn check_tls_floors(lock_path: &Path) -> Result<()> {
    let text = fs::read_to_string(lock_path)
        .with_context(|| format!("读取 {} 失败", lock_path.display()))?;
    let versions = lock_versions(&text);

    let mut problems = Vec::new();
    for (name, floor) in TLS_FLOORS {
        let Some(raws) = versions.get(*name) else {
            problems.push(format!("{name} 未出现在 {}", lock_path.display()));
            continue;
        };
        let parsed: Vec<(Version, &String)> = raws
            .iter()
            .filter_map(|raw| Version::parse(raw).map(|version| (version, raw)))
            .collect();
        if parsed.len() != raws.len() {
            problems.push(format!("{name} 存在无法解析的版本: {raws:?}"));
            continue;
        }
        let Some((lowest, lowest_raw)) = parsed.iter().min_by_key(|(version, _)| *version) else {
            problems.push(format!("{name} 在 Cargo.lock 中没有版本"));
            continue;
        };
        if *lowest < *floor {
            problems.push(format!(
                "{name} 最低版本 {lowest_raw}（{} 个版本）低于下限 {floor}",
                parsed.len()
            ));
        } else {
            println!(
                "ok {name} {lowest_raw} >= {floor}（共 {} 个版本）",
                parsed.len()
            );
        }
    }

    if problems.is_empty() {
        return Ok(());
    }
    bail!("TLS 下限断言失败:\n  - {}", problems.join("\n  - "));
}

/// Rust 的 OS 常量 → Go 的 `GOOS` 命名（与既有产物名一致）。
fn go_os_name(os: &str) -> &str {
    match os {
        "macos" => "darwin",
        other => other,
    }
}

/// Rust 的 ARCH 常量 → Go 的 `GOARCH` 命名。
fn go_arch_name(arch: &str) -> &str {
    match arch {
        "x86_64" => "amd64",
        "x86" => "386",
        "aarch64" => "arm64",
        other => other,
    }
}

/// 产物路径：`release/gensokyo-{os}-{arch}[-noWebui][.exe]`。
fn artifact_path(os: &str, arch: &str, no_webui: bool, windows: bool) -> String {
    let flavor = if no_webui { "-noWebui" } else { "" };
    let ext = if windows { ".exe" } else { "" };
    format!("release/gensokyo-{os}-{arch}{flavor}{ext}")
}

/// 解析 `--target <os>-<arch>`（Go 命名，例如 `linux-amd64`）。
fn parse_target(target: &str) -> Result<(String, String)> {
    let mut parts = target.split('-');
    let os = parts.next().unwrap_or_default();
    let arch = parts.next().unwrap_or_default();
    if os.is_empty() || arch.is_empty() || parts.next().is_some() {
        bail!("--target 需要 <os>-<arch> 形式（Go 命名），例如 linux-amd64");
    }
    Ok((os.to_owned(), arch.to_owned()))
}

#[derive(Debug, Parser)]
#[command(name = "xtask", about = "Gensokyo-NewQQ 构建辅助")]
struct Cli {
    #[command(subcommand)]
    command: Xtask,
}

#[derive(Debug, Subcommand)]
enum Xtask {
    /// 打印产物命名规则（M0 占位）。
    Build {
        /// 目标，`<os>-<arch>`（Go 命名）；缺省用当前平台。
        #[arg(long)]
        target: Option<String>,
        /// 生成无 WebUI 精简版（等价于 Go 的 `-tags=small`）。
        #[arg(long = "no-webui")]
        no_webui: bool,
    },
    /// 断言 Cargo.lock 中的 TLS 组件不低于安全下限。
    CheckTlsFloors,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Xtask::Build { target, no_webui } => {
            let (os, arch) = match target {
                Some(target) => parse_target(&target)?,
                None => (go_os_name(OS).to_owned(), go_arch_name(ARCH).to_owned()),
            };
            let windows = os == "windows";
            println!("产物命名规则: release/gensokyo-{{os}}-{{arch}}[-noWebui][.exe]");
            println!(
                "当前目标产物: {}",
                artifact_path(&os, &arch, no_webui, windows)
            );
            println!("M0 只打印规则；实际打包与交叉编译在 M1 接入。");
        }
        Xtask::CheckTlsFloors => {
            let lock_path = find_lockfile()?;
            check_tls_floors(&lock_path)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{TLS_FLOORS, Version, check_tls_floors, find_lockfile, lock_versions};

    #[test]
    fn parses_three_part_versions_and_ignores_suffixes() {
        assert_eq!(Version::parse("0.103.16"), Some(Version::new(0, 103, 16)));
        assert_eq!(Version::parse("1.0.10"), Some(Version::new(1, 0, 10)));
        assert_eq!(
            Version::parse("0.23.46-alpha.1"),
            Some(Version::new(0, 23, 46))
        );
        assert_eq!(Version::parse("not-a-version"), None);
    }

    #[test]
    fn version_ordering_matches_semver_for_floors() {
        assert!(Version::parse("0.103.16").unwrap() >= Version::new(0, 103, 16));
        assert!(Version::parse("0.103.15").unwrap() < Version::new(0, 103, 16));
        assert!(Version::parse("0.104.0").unwrap() > Version::new(0, 103, 16));
    }

    #[test]
    fn extracts_versions_from_lock_text() {
        let lock = r#"
version = 4

[[package]]
name = "rustls"
version = "0.23.46"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "webpki-root-certs"
version = "1.0.10"
"#;
        let versions = lock_versions(lock);
        assert_eq!(versions.get("rustls"), Some(&vec!["0.23.46".to_owned()]));
        assert_eq!(
            versions.get("webpki-root-certs"),
            Some(&vec!["1.0.10".to_owned()])
        );
        assert_eq!(versions.len(), 2, "不能把 lock 头部的 version = 4 当包版本");
    }

    #[test]
    fn lowest_of_multiple_versions_decides_the_floor() {
        // 同名两版本共存：新版本不能掩盖旧版本
        let failing = r#"
[[package]]
name = "rustls"
version = "0.23.46"

[[package]]
name = "rustls-webpki"
version = "0.103.15"

[[package]]
name = "rustls-webpki"
version = "0.104.0"

[[package]]
name = "webpki-root-certs"
version = "1.0.10"
"#;
        let tmp = std::env::temp_dir().join("gensokyo-xtask-tls-dupe-fail.lock");
        std::fs::write(&tmp, failing).expect("写入临时 lock 失败");
        let err = check_tls_floors(&tmp).expect_err("最低版本 0.103.15 应触发失败");
        assert!(
            err.to_string().contains("0.103.15"),
            "错误信息应给出最低版本: {err}"
        );
        std::fs::remove_file(&tmp).ok();

        // 两个版本都在下限之上：通过
        let passing = failing.replace("0.103.15", "0.103.16");
        let tmp = std::env::temp_dir().join("gensokyo-xtask-tls-dupe-pass.lock");
        std::fs::write(&tmp, passing).expect("写入临时 lock 失败");
        check_tls_floors(&tmp).expect("两版本都 >= 下限时应通过");
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn fails_when_a_floor_is_missing_or_too_low() {
        let tmp = std::env::temp_dir().join("gensokyo-xtask-tls-floor-test.lock");
        std::fs::write(
            &tmp,
            r#"
[[package]]
name = "rustls"
version = "0.23.46"

[[package]]
name = "rustls-webpki"
version = "0.103.15"

[[package]]
name = "webpki-root-certs"
version = "1.0.10"
"#,
        )
        .expect("写入临时 lock 失败");

        let err = check_tls_floors(&tmp).expect_err("0.103.15 应低于下限");
        assert!(
            err.to_string().contains("rustls-webpki"),
            "错误信息应点名组件"
        );
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn real_workspace_lock_satisfies_floors() {
        let lock = find_lockfile().expect("未找到 Cargo.lock");
        check_tls_floors(&lock).expect("workspace Cargo.lock 未满足 TLS 下限");
        let versions = lock_versions(&std::fs::read_to_string(&lock).unwrap());
        for (name, floor) in TLS_FLOORS {
            let raws = versions.get(*name).expect("组件缺失");
            let found = raws
                .iter()
                .filter_map(|raw| Version::parse(raw))
                .min()
                .expect("版本无法解析");
            assert!(found >= *floor, "{name} {found} < {floor}");
        }
    }
}
