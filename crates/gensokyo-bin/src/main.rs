//! gensokyo 可执行入口。
//!
//! M0 只支持版本输出；运行主链（`gensokyo run`）在 M1 接入 `gensokyo-runtime`。

use std::time::{SystemTime, UNIX_EPOCH};

use clap::{ArgAction, Parser, Subcommand};

/// 二进制名，与 Go 产物保持一致。
const BIN_NAME: &str = "gensokyo";

/// 版本串：与 Go `buildinfo.Version()` 语义一致——构建期注入 `<BuildType>-<BuildSpec>`，
/// 未注入时回退为 `dev-<epoch>.<millis>`。
fn version_string() -> String {
    match option_env!("GENSOKYO_BUILD_VERSION") {
        Some(build) if !build.is_empty() => build.to_owned(),
        _ => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            format!("dev-{}.{:03}", now.as_secs(), now.subsec_millis())
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = BIN_NAME,
    about = "Gensokyo-NewQQ：OneBot V11 与 QQ 官方 API 的双向桥接",
    disable_version_flag = true
)]
struct Cli {
    /// 打印版本并退出。
    #[arg(long = "version", short = 'V', global = true, action = ArgAction::SetTrue)]
    version: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 打印版本并退出（等价于 `--version`）。
    Version,
}

fn print_version() {
    println!("{BIN_NAME} {}", version_string());
}

fn main() {
    let cli = Cli::parse();

    if cli.version {
        print_version();
        return;
    }

    match cli.command {
        Some(Command::Version) => print_version(),
        None => {
            // M0 未接入运行主链：显式失败而不是假装启动成功
            eprintln!("M0 脚手架：运行主链尚未接入，见 docs/rust-m0.md");
            std::process::exit(2);
        }
    }
}
