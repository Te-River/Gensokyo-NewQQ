//! `xtask pgo`：Profile-Guided Optimization 流水线。
//!
//! 顺序：基线 bench → `-Cprofile-generate` 插桩构建 → 跑负载收集 `.profraw` →
//! `llvm-profdata merge` → `-Cprofile-use` 重建 → 再跑一次 bench 做前后对比。
//! 流程、测量口径与 BOLT 的边界见 `docs/rust-perf.md`。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::Args;

use crate::bench;
use crate::build::{cargo, ensure_webui_dist, target_directory, workspace_root};

/// `-Cprofile-use` 的附加参数：profile 里缺函数时显式告警，别把"没生效"当成"已优化"。
const PGO_USE_LLVM_ARGS: &str = "-Cllvm-args=-pgo-warn-missing-function";

#[derive(Debug, Args)]
pub struct PgoArgs {
    /// bench 负载的迭代次数（仅默认负载使用；试跑建议 --iters 5000）。
    #[arg(long, default_value_t = bench::DEFAULT_ITERS, conflicts_with = "workload")]
    iters: u64,

    /// 自定义负载命令（在 workspace 根目录执行，环境里的 RUSTFLAGS 与插桩构建一致）。
    #[arg(long, value_name = "CMD")]
    workload: Option<String>,

    /// 插桩数据与报告目录（默认 `<target-dir>/pgo`）。
    #[arg(long = "profile-dir", value_name = "DIR")]
    profile_dir: Option<PathBuf>,
}

/// 跑完整流水线，并把前后对比写到 `--profile-dir` 下的 `pgo-report.md`。
///
/// # Errors
/// 缺 `llvm-profdata`、负载没跑出 `.profraw`、任一构建/merge 失败时不继续：
/// PGO 是"构建三次"的长流程，任何一步失败后的后续数字都不可比。
pub fn run(args: PgoArgs) -> Result<()> {
    let root = workspace_root()?;
    // rust-embed 在编译期读 `webui/dist`，缺目录时 `cargo build` 直接失败（与 `xtask build` 同一处理）。
    ensure_webui_dist(&root)?;

    // 拆字段：`run_workload` 要借用 workload，整体借用 `args` 会和 `profile_dir` 的所有权移动打架。
    let PgoArgs {
        iters,
        workload,
        profile_dir,
    } = args;
    let profile_dir = match profile_dir {
        Some(dir) => dir,
        None => target_directory(&root)?.join("pgo"),
    };
    let merged = profile_dir.join("merged.profdata");
    fs::create_dir_all(&profile_dir)
        .with_context(|| format!("创建 {} 失败", profile_dir.display()))?;

    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let (release, host) = rustc_identity(&rustc);
    let toolchain = format!(
        "{} ({})",
        release.as_deref().unwrap_or("?"),
        host.as_deref().unwrap_or("?")
    );

    // 本机实测（2026-11，rustc 1.99.1）：x86_64-pc-windows-gnu 的发行版不带 `profiler_builtins`，
    // 插桩构建在编译第一个 crate 时就 E0463。提前说明，别让人白等一次 release 构建。
    if host
        .as_deref()
        .is_some_and(|host| host.ends_with("windows-gnu"))
    {
        println!(
            "[pgo] 注意：{toolchain} 的 rustc 不带 profiler_builtins，`-Cprofile-generate` 会报 E0463；\
             插桩需要在 Linux 或 msvc 宿主上跑（docs/rust-perf.md §7）"
        );
    }

    let gen_flags = format!("-Cprofile-generate={}", slashes(&profile_dir));
    let use_flags = format!("-Cprofile-use={} {PGO_USE_LLVM_ARGS}", slashes(&merged));
    let workload_label = match &workload {
        Some(command) => command.clone(),
        None => format!("xtask bench --iters {iters}"),
    };

    println!("[pgo] workspace: {}", root.display());
    println!("[pgo] profile dir: {}", profile_dir.display());
    println!("[pgo] 负载: {workload_label}");

    // 0) 基线：普通 release 口径，先产出对比数字（这一步不依赖 llvm-profdata）。
    //    自定义负载没有基准数字，跳过。
    let baseline = if workload.is_none() {
        Some(step("基线 bench", || run_bench(&root, iters, None))?)
    } else {
        println!("[pgo] 自定义负载：跳过基线 bench 与前后对比");
        None
    };

    // 探测 llvm-profdata：只在 merge 时用得上，但在插桩构建 + 负载之前，缺工具时立刻
    // 给出可操作提示。放在基线之后：探测失败时这次运行已经留下基线数字，不会完全白跑。
    let profdata = find_llvm_profdata(&rustc, host.as_deref())?;
    println!("[pgo] llvm-profdata: {}", profdata.display());

    // 1) 插桩构建，2) 跑负载（同一 RUSTFLAGS，负载才会执行插桩产物）
    step("插桩构建", || build_release(&root, &gen_flags))?;
    clean_profraw(&profile_dir)?;
    step("运行负载", || {
        run_workload(&root, workload.as_deref(), iters, &gen_flags)
    })?;

    let profraw = count_profraw(&profile_dir)?;
    if profraw == 0 {
        bail!(
            "负载跑完但 {} 里没有 .profraw：负载必须执行插桩产物（自定义 --workload 若绕过 cargo，\
             需自己用同一条 RUSTFLAGS 构建，或把 RUSTFLAGS 传给内层 cargo）",
            profile_dir.display()
        );
    }

    // 3) merge
    step("llvm-profdata merge", || {
        merge(&profdata, &profile_dir, &merged)
    })?;
    let merged_bytes = fs::metadata(&merged)
        .with_context(|| format!("读取 {} 失败", merged.display()))?
        .len();
    if merged_bytes == 0 {
        bail!("{} 为空，profile 不可用", merged.display());
    }

    // 4) profile-use 重建，5) 复测
    step("profile-use 重建", || build_release(&root, &use_flags))?;
    let after = match &baseline {
        Some(_) => Some(step("PGO bench", || {
            run_bench(&root, iters, Some(&use_flags))
        })?),
        None => None,
    };

    let rows = match (&baseline, &after) {
        (Some(before), Some(after)) => compare(before, after),
        _ => Vec::new(),
    };
    print_rows(&rows);

    let report = render_report(
        &toolchain,
        &profile_dir,
        &profdata,
        &workload_label,
        &gen_flags,
        &use_flags,
        profraw,
        merged_bytes,
        &rows,
    );
    let path = profile_dir.join("pgo-report.md");
    fs::write(&path, report).with_context(|| format!("写入 {} 失败", path.display()))?;
    println!("[pgo] 报告: {}", path.display());
    Ok(())
}

/// 给一个阶段计时（全流程十几分钟，没有分段耗时就看不出哪一步慢）。
fn step<T>(label: &str, body: impl FnOnce() -> Result<T>) -> Result<T> {
    let start = Instant::now();
    println!("[pgo] == {label} ==");
    let value = body()?;
    println!("[pgo]    {:.1}s", start.elapsed().as_secs_f64());
    Ok(value)
}

/// 定位 `llvm-profdata`：优先 rustup 的 `llvm-tools-preview` 组件（sysroot 内），其次 PATH。
///
/// # Errors
/// 两者都没有时返回错误并给出安装命令——这一步失败后整条流水线都无从谈起。
fn find_llvm_profdata(rustc: &str, host: Option<&str>) -> Result<PathBuf> {
    let exe = format!("llvm-profdata{}", std::env::consts::EXE_SUFFIX);

    if let (Some(sysroot), Some(host)) = (command_stdout(rustc, &["--print", "sysroot"]), host) {
        let candidate = PathBuf::from(sysroot)
            .join("lib")
            .join("rustlib")
            .join(host)
            .join("bin")
            .join(&exe);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    if Command::new("llvm-profdata")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        return Ok(PathBuf::from("llvm-profdata"));
    }

    bail!(
        "未找到 llvm-profdata：先执行 `rustup component add llvm-tools-preview`（用 `+<toolchain>` 时\
         要带上同一个工具链，例如 `rustup component add llvm-tools-preview --toolchain <toolchain>`），\
         或把系统 LLVM 的 llvm-profdata 放进 PATH，然后重跑 `cargo xtask pgo`。\n\
         期望路径：<sysroot>/lib/rustlib/<host>/bin/{exe}"
    )
}

/// 跑一条命令并取 stdout（非零退出或无法执行时返回 `None`）。
fn command_stdout(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// `rustc -vV` 的 `(release, host)`：host 用来拼 llvm-tools 路径，两者都写进报告。
fn rustc_identity(rustc: &str) -> (Option<String>, Option<String>) {
    let Some(text) = command_stdout(rustc, &["-vV"]) else {
        return (None, None);
    };
    let mut release = None;
    let mut host = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("release: ") {
            release = Some(value.trim().to_owned());
        } else if let Some(value) = line.strip_prefix("host: ") {
            host = Some(value.trim().to_owned());
        }
    }
    (release, host)
}

/// RUSTFLAGS 以空格分隔参数，Windows 反斜杠路径在这里容易被当成转义，统一换正斜杠。
fn slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// workspace 根目录下的 cargo 命令，`RUSTFLAGS` 由本模块独占。
///
/// 不继承外部 `RUSTFLAGS`：插桩与 profile-use 两次构建必须是同一口径，混入外部 flag 会让
/// 两次构建的指纹与产物都不一致，也会让"为什么数字变了"无从解释。
fn cargo_command(root: &Path, rustflags: Option<&str>) -> Command {
    let mut command = Command::new(cargo());
    command.current_dir(root);
    match rustflags {
        Some(flags) => {
            command.env("RUSTFLAGS", flags);
        }
        None => {
            command.env_remove("RUSTFLAGS");
        }
    }
    command
}

/// `cargo build --release -p gensokyo-bin`；插桩还是 profile-use 由 `rustflags` 决定。
fn build_release(root: &Path, rustflags: &str) -> Result<()> {
    println!("[pgo] RUSTFLAGS={rustflags}");
    let status = cargo_command(root, Some(rustflags))
        .args(["build", "--release", "--locked", "-p", "gensokyo-bin"])
        .status()
        .context("执行 cargo build 失败")?;
    if !status.success() {
        bail!("cargo build --release -p gensokyo-bin 失败（RUSTFLAGS={rustflags}）");
    }
    Ok(())
}

/// 跑 `xtask bench` 并解析 `ns/op`；`rustflags` 为 `None` 即普通 release 基线。
fn run_bench(root: &Path, iters: u64, rustflags: Option<&str>) -> Result<Vec<(String, f64)>> {
    let output = cargo_command(root, rustflags)
        .args([
            "run",
            "--release",
            "--locked",
            "-p",
            "xtask",
            "--",
            "bench",
            "--iters",
        ])
        .arg(iters.to_string())
        .output()
        .context("执行 cargo run -p xtask -- bench 失败")?;

    // bench 输出是 ASCII，可以原样转写；cargo 的构建进度在 stderr。
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    if !output.status.success() {
        bail!("xtask bench 失败（退出码 {:?}）", output.status.code());
    }

    let rows = parse_bench_output(&String::from_utf8_lossy(&output.stdout));
    if rows.is_empty() {
        println!("[pgo] 警告：bench 输出里没有解析出 ns/op，跳过前后对比");
    }
    Ok(rows)
}

/// 从 `xtask bench` 的表格里取 `(用例名, ns/op)`。
///
/// 表格列是 `name / iters / total_ms / ns/op`，用例名自带空格（`cqcode::parse 20-segment mixed`），
/// 所以从右往左吃三列数字，剩下的拼回名称。
fn parse_bench_output(stdout: &str) -> Vec<(String, f64)> {
    let mut rows = Vec::new();
    for line in stdout.lines() {
        let mut columns: Vec<&str> = line.split_whitespace().collect();
        if columns.len() < 4 {
            continue;
        }
        let ns = columns.pop().and_then(|value| value.parse::<f64>().ok());
        let total_ms = columns.pop().and_then(|value| value.parse::<f64>().ok());
        let iters = columns.pop().and_then(|value| value.parse::<u64>().ok());
        let (Some(ns), Some(_), Some(_)) = (ns, total_ms, iters) else {
            continue;
        };
        if columns.is_empty() {
            continue;
        }
        rows.push((columns.join(" "), ns));
    }
    rows
}

/// 按用例名配对两轮 bench：`(名称, 基线 ns/op, PGO ns/op, 变化百分比)`。
fn compare(before: &[(String, f64)], after: &[(String, f64)]) -> Vec<(String, f64, f64, f64)> {
    after
        .iter()
        .filter_map(|(name, after_ns)| {
            let before_ns = before
                .iter()
                .find(|(candidate, _)| candidate == name)
                .map(|(_, ns)| *ns)?;
            let delta = (after_ns - before_ns) / before_ns * 100.0;
            Some((name.clone(), before_ns, *after_ns, delta))
        })
        .collect()
}

/// 把前后对比打到控制台（先粗看，细读报告文件）。
fn print_rows(rows: &[(String, f64, f64, f64)]) {
    if rows.is_empty() {
        return;
    }
    println!();
    println!(
        "{:<34} {:>12} {:>12} {:>10}",
        "name", "baseline ns", "pgo ns", "delta"
    );
    for (name, before, after, delta) in rows {
        println!("{name:<34} {before:>12.1} {after:>12.1} {delta:>9.1}%");
    }
}

/// 生成 `pgo-report.md`：口径、产物路径、前后数字与已知边界一起落盘。
#[allow(clippy::too_many_arguments)]
fn render_report(
    toolchain: &str,
    profile_dir: &Path,
    profdata: &Path,
    workload: &str,
    gen_flags: &str,
    use_flags: &str,
    profraw: usize,
    merged_bytes: u64,
    rows: &[(String, f64, f64, f64)],
) -> String {
    let mut report = String::new();
    report.push_str("# PGO 报告\n\n");
    report.push_str("> 由 `cargo xtask pgo` 生成；方法论见 `docs/rust-perf.md` §7。\n\n");
    report.push_str(&format!(
        "- 平台: {}-{}\n",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    report.push_str(&format!("- 工具链: {toolchain}\n"));
    report.push_str(&format!("- profile dir: `{}`\n", profile_dir.display()));
    report.push_str(&format!("- llvm-profdata: `{}`\n", profdata.display()));
    report.push_str(&format!("- 负载: `{workload}`\n"));
    report.push_str(&format!(
        "- profraw: {profraw} 个；merged.profdata: {} KiB\n",
        merged_bytes / 1024
    ));
    report.push_str(&format!("- RUSTFLAGS(插桩): `{gen_flags}`\n"));
    report.push_str(&format!("- RUSTFLAGS(重建): `{use_flags}`\n\n"));

    if rows.is_empty() {
        report.push_str("自定义负载不产出基准数字，报告里没有前后对比。\n");
        return report;
    }

    report.push_str("| 用例 | 基线 ns/op | PGO ns/op | 变化 |\n|---|---:|---:|---:|\n");
    for (name, before, after, delta) in rows {
        report.push_str(&format!(
            "| `{name}` | {before:.1} | {after:.1} | {delta:+.1}% |\n"
        ));
    }
    report.push_str(
        "\n**口径**：同一次运行内的两个独立进程（普通 release vs profile-use），同一 `--iters`、\
         同一机器；本机噪声约 ±10%，单轮差异落在噪声内不算收益，要看多轮中位数。\n\n\
         **覆盖边界**：默认负载是 `xtask bench`，只覆盖 `gensokyo-protocol` / `gensokyo-actions` \
         的同步热路径；服务端主循环、线程池与网络栈不在 profile 里，那部分的 PGO 收益为 0。\n\n\
         **BOLT**：不在本命令范围内——BOLT 只支持 Linux（需要 perf 采样与 llvm-bolt），\
         Windows 上没有可用实现，详见 `docs/rust-perf.md` §7。\n",
    );
    report
}

/// 清掉上一轮遗留的 `.profraw`：旧数据混进来会让 merge 失败，或把上一轮负载算进 profile。
fn clean_profraw(dir: &Path) -> Result<()> {
    let mut removed = 0;
    for entry in fs::read_dir(dir).with_context(|| format!("读取 {} 失败", dir.display()))? {
        let path = entry?.path();
        if is_profraw(&path) {
            fs::remove_file(&path).with_context(|| format!("删除 {} 失败", path.display()))?;
            removed += 1;
        }
    }
    if removed > 0 {
        println!("[pgo] 清理上一轮 profraw: {removed} 个");
    }
    Ok(())
}

/// 统计本轮产生的 `.profraw` 个数。
fn count_profraw(dir: &Path) -> Result<usize> {
    let mut count = 0;
    for entry in fs::read_dir(dir).with_context(|| format!("读取 {} 失败", dir.display()))? {
        if is_profraw(&entry?.path()) {
            count += 1;
        }
    }
    Ok(count)
}

/// 扩展名判定：LLVM 的原始 profile 文件是 `.profraw`。
fn is_profraw(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "profraw")
}

/// `llvm-profdata merge -o <merged> <dir>`：目录下全部 `.profraw` 合成一个 profile。
fn merge(profdata: &Path, dir: &Path, merged: &Path) -> Result<()> {
    println!(
        "[pgo] {} merge -o {} {}",
        profdata.display(),
        merged.display(),
        dir.display()
    );
    let status = Command::new(profdata)
        .arg("merge")
        .arg("-o")
        .arg(merged)
        .arg(dir)
        .status()
        .with_context(|| format!("执行 {} 失败", profdata.display()))?;
    if !status.success() {
        bail!("llvm-profdata merge 失败（见上方输出）");
    }
    Ok(())
}

/// 跑负载：默认 `xtask bench`，`--workload` 给的自定义命令交给 shell。
///
/// 两种情况都带上插桩时的 RUSTFLAGS：负载必须执行插桩产物，否则不会有 `.profraw`。
fn run_workload(root: &Path, workload: Option<&str>, iters: u64, rustflags: &str) -> Result<()> {
    let Some(command) = workload else {
        run_bench(root, iters, Some(rustflags))?;
        return Ok(());
    };

    println!("[pgo] RUSTFLAGS={rustflags}");
    #[cfg(windows)]
    let mut child = {
        let mut child = Command::new("cmd");
        child.args(["/C", command]);
        child
    };
    #[cfg(not(windows))]
    let mut child = {
        let mut child = Command::new("sh");
        child.args(["-c", command]);
        child
    };

    let status = child
        .current_dir(root)
        .env("RUSTFLAGS", rustflags)
        .status()
        .with_context(|| format!("执行自定义负载 `{command}` 失败"))?;
    if !status.success() {
        bail!("自定义负载 `{command}` 退出码 {:?}", status.code());
    }
    Ok(())
}
