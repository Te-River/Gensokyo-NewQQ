//! 命令行形状：与 Go `main.go:62-87` 对齐。
//!
//! Go 用标准库 `flag`，长选项是**单横线**（`-faststart`）。M1j 同时接受 Go 写法与 clap 的
//! 双横线写法：`normalize_legacy_flags` 在解析前把已知的 `-name` 改写成 `--name`。
//!
//! 尚未实现的维护开关（`tidy` / `clean_ids` / `del_ids` / `del_cache` / `compaction` /
//! `-m` / `-local-logger`）不静默成功：统一由 [`resolve`] 映射为
//! [`Action::Unimplemented`]，由 `main` 打印「未实现」并以 [`EXIT_UNIMPLEMENTED`] 退出。

use std::ffi::OsString;

use clap::{ArgAction, Parser, Subcommand};

/// 二进制名，与 Go 产物保持一致。
pub const BIN_NAME: &str = "gensokyo";

/// 未实现功能的退出码；与「启动失败」（1）区分，便于脚本判断。
pub const EXIT_UNIMPLEMENTED: u8 = 2;

/// `flag` 接受过长选项名（用于单横线兼容改写）。
const LONG_FLAGS: &[&str] = &[
    "version",
    "faststart",
    "tidy",
    "clean_ids",
    "del_ids",
    "del_cache",
    "compaction",
    "maintenance",
    "local-logger",
    "help",
];

/// 顶层参数。
#[derive(Debug, Parser)]
#[command(
    name = BIN_NAME,
    about = "Gensokyo-NewQQ：OneBot V11 与 QQ 官方 API 的双向桥接",
    disable_version_flag = true
)]
pub struct Cli {
    /// 打印版本并退出。
    #[arg(long = "version", short = 'V', global = true, action = ArgAction::SetTrue)]
    pub version: bool,

    /// 跳过初始化直接启动（Go `-faststart`；M1j 无额外初始化步骤）。
    #[arg(long = "faststart", global = true, action = ArgAction::SetTrue)]
    pub faststart: bool,

    /// 备份并整理 config.yml（Go `-tidy`）。
    #[arg(long = "tidy", global = true, action = ArgAction::SetTrue)]
    pub tidy: bool,

    /// 清理 ids bucket 中的 msg_id（Go `-clean_ids`）。
    #[arg(long = "clean_ids", global = true, action = ArgAction::SetTrue)]
    pub clean_ids: bool,

    /// 删除 ids bucket（Go `-del_ids`，危险）。
    #[arg(long = "del_ids", global = true, action = ArgAction::SetTrue)]
    pub del_ids: bool,

    /// 删除 cache bucket（Go `-del_cache`）。
    #[arg(long = "del_cache", global = true, action = ArgAction::SetTrue)]
    pub del_cache: bool,

    /// 对数据库执行 compaction（Go `-compaction`）。
    #[arg(long = "compaction", global = true, action = ArgAction::SetTrue)]
    pub compaction: bool,

    /// 维护模式（Go `-m`）。
    #[arg(short = 'm', long = "maintenance", global = true, action = ArgAction::SetTrue)]
    pub maintenance: bool,

    /// 开启本地日志文件（Go `-local-logger`；非空即启用）。
    #[arg(long = "local-logger", value_name = "enable", global = true)]
    pub local_logger: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// 与 Go 的 `version` / `run` 及各个维护开关一一对应的子命令形式。
#[derive(Debug, Subcommand)]
pub enum Command {
    /// 启动运行时（默认行为）。
    Run,
    /// 打印版本并退出（等价于 `--version`）。
    Version,
    /// 跳过初始化直接启动（M1j 无额外初始化步骤，等价于 `run`）。
    Faststart,
    /// 备份并整理 config.yml。
    Tidy,
    /// 清理 ids bucket 中的 msg_id。
    #[command(name = "clean_ids")]
    CleanIds,
    /// 删除 ids bucket（危险）。
    #[command(name = "del_ids")]
    DelIds,
    /// 删除 cache bucket。
    #[command(name = "del_cache")]
    DelCache,
    /// 对数据库执行 compaction。
    Compaction,
}

/// 解析后的动作。
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// 打印版本并退出。
    Version,
    /// 启动运行时。
    Run {
        /// Go `-faststart`。
        faststart: bool,
    },
    /// 尚未实现：打印提示并非零退出。
    Unimplemented {
        /// Go 侧的开关名（含单横线），与 CLI 输入一致。
        flag: &'static str,
        /// 该开关在 Go 里的作用，用于提示用户。
        hint: &'static str,
    },
}

const fn unimplemented(flag: &'static str, hint: &'static str) -> Action {
    Action::Unimplemented { flag, hint }
}

/// 把 CLI 解析结果映射为动作。
///
/// 判定顺序跟随 Go `main.go`：版本 → `tidy` → `clean_ids` → `del_ids` → `del_cache` →
/// `compaction` → `-m` → `-local-logger` → `run`。
#[must_use]
pub fn resolve(cli: Cli) -> Action {
    if cli.version {
        return Action::Version;
    }
    if cli.tidy {
        return unimplemented("-tidy", "备份并整理 config.yml");
    }
    if cli.clean_ids {
        return unimplemented("-clean_ids", "清理 ids bucket 中的 msg_id");
    }
    if cli.del_ids {
        return unimplemented("-del_ids", "删除 ids bucket");
    }
    if cli.del_cache {
        return unimplemented("-del_cache", "删除 cache bucket");
    }
    if cli.compaction {
        return unimplemented("-compaction", "对数据库执行 compaction");
    }
    if cli.maintenance {
        return unimplemented("-m", "维护模式");
    }
    if cli
        .local_logger
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        return unimplemented("-local-logger", "本地日志文件");
    }

    match cli.command {
        Some(Command::Version) => Action::Version,
        Some(Command::Faststart) => Action::Run { faststart: true },
        Some(Command::Tidy) => unimplemented("-tidy", "备份并整理 config.yml"),
        Some(Command::CleanIds) => unimplemented("-clean_ids", "清理 ids bucket 中的 msg_id"),
        Some(Command::DelIds) => unimplemented("-del_ids", "删除 ids bucket"),
        Some(Command::DelCache) => unimplemented("-del_cache", "删除 cache bucket"),
        Some(Command::Compaction) => unimplemented("-compaction", "对数据库执行 compaction"),
        Some(Command::Run) | None => Action::Run {
            faststart: cli.faststart,
        },
    }
}

/// 把 Go 风格的单横线长选项（`-faststart`、`-local-logger=enable`）改写成双横线。
///
/// 只改写 [`LONG_FLAGS`] 里的名字：`-m` / `-V` / `-h` 这类短选项，以及未知参数，原样交给
/// clap 处理（未知参数仍会得到 clap 自己的报错）。
#[must_use]
pub fn normalize_legacy_flags<I>(argv: I) -> Vec<OsString>
where
    I: IntoIterator<Item = OsString>,
{
    argv.into_iter().map(normalize_flag).collect()
}

fn normalize_flag(arg: OsString) -> OsString {
    let Some(raw) = arg.to_str() else {
        return arg;
    };
    // 双横线已是 clap 写法；`-x` 是短选项；无横线是位置参数。
    let Some(name) = raw.strip_prefix('-') else {
        return arg;
    };
    if name.starts_with('-') || name.len() < 2 {
        return arg;
    }
    let flag = name.split_once('=').map_or(name, |(flag, _)| flag);
    if !LONG_FLAGS.contains(&flag) {
        return arg;
    }
    OsString::from(format!("--{name}"))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use clap::Parser as _;

    use super::{Action, Cli, normalize_legacy_flags, resolve};

    fn parse(argv: &[&str]) -> Action {
        let cli = Cli::try_parse_from(argv.iter().copied()).expect("解析失败");
        resolve(cli)
    }

    fn unimplemented_flag(argv: &[&str]) -> &'static str {
        match parse(argv) {
            Action::Unimplemented { flag, .. } => flag,
            other => panic!("{argv:?} 应为未实现，实际 {other:?}"),
        }
    }

    #[test]
    fn version_forms_resolve_to_version() {
        assert_eq!(parse(&["gensokyo", "--version"]), Action::Version);
        assert_eq!(parse(&["gensokyo", "-V"]), Action::Version);
        assert_eq!(parse(&["gensokyo", "version"]), Action::Version);
    }

    #[test]
    fn default_and_run_start_the_runtime() {
        assert_eq!(
            parse(&["gensokyo"]),
            Action::Run { faststart: false },
            "无子命令时默认 run"
        );
        assert_eq!(
            parse(&["gensokyo", "run"]),
            Action::Run { faststart: false }
        );
        assert_eq!(
            parse(&["gensokyo", "run", "--faststart"]),
            Action::Run { faststart: true }
        );
        assert_eq!(
            parse(&["gensokyo", "faststart"]),
            Action::Run { faststart: true }
        );
    }

    #[test]
    fn maintenance_switches_are_unimplemented() {
        assert_eq!(unimplemented_flag(&["gensokyo", "--tidy"]), "-tidy");
        assert_eq!(unimplemented_flag(&["gensokyo", "tidy"]), "-tidy");
        assert_eq!(
            unimplemented_flag(&["gensokyo", "--clean_ids"]),
            "-clean_ids"
        );
        assert_eq!(unimplemented_flag(&["gensokyo", "clean_ids"]), "-clean_ids");
        assert_eq!(unimplemented_flag(&["gensokyo", "del_ids"]), "-del_ids");
        assert_eq!(unimplemented_flag(&["gensokyo", "del_cache"]), "-del_cache");
        assert_eq!(
            unimplemented_flag(&["gensokyo", "compaction"]),
            "-compaction"
        );
        assert_eq!(unimplemented_flag(&["gensokyo", "-m"]), "-m");
        assert_eq!(
            unimplemented_flag(&["gensokyo", "--local-logger=enable"]),
            "-local-logger"
        );
    }

    #[test]
    fn empty_local_logger_keeps_go_semantics() {
        // Go: 只有非空值才启用本地日志文件。
        assert_eq!(
            parse(&["gensokyo", "--local-logger="]),
            Action::Run { faststart: false }
        );
    }

    #[test]
    fn single_dash_long_flags_are_normalized() {
        let argv = [
            "gensokyo",
            "-faststart",
            "-local-logger=enable",
            "--tidy",
            "-m",
            "-V",
            "run",
        ]
        .map(OsString::from);
        let normalized: Vec<String> = normalize_legacy_flags(argv)
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            normalized,
            [
                "gensokyo",
                "--faststart",
                "--local-logger=enable",
                "--tidy",
                "-m",
                "-V",
                "run"
            ]
        );
    }
}
