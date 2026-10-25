//! gensokyo 可执行入口（M1j：二进制装配）。
//!
//! # M1j 范围
//! - 保留 Go 的 CLI 形状（`main.go:62-87`）：`version` / `--version`、`run`（默认）、
//!   `faststart`、`tidy`、`clean_ids`、`del_ids`、`del_cache`、`compaction`、`-m`、
//!   `-local-logger`。未实现的维护开关打印「未实现」并以退出码 [`cli::EXIT_UNIMPLEMENTED`]
//!   结束，不静默成功。
//! - 启动链见 [`startup`]：`config.yml` → [`gensokyo_config::ConfigManager`] →
//!   [`gensokyo_runtime::AppState`] → [`gensokyo_runtime::Runtime::start`]，即**正向 WS +
//!   正向 HTTP API**。有 QQ 凭据时装配 `ReqwestTransport` + `ActionContext`（47 个 action
//!   名：内置 3 + `gensokyo-actions` 41 + 别名 3），无凭据或传输构建失败时降级为内置 3 个；
//!   启动后打印 action 数量与实际监听地址，Ctrl-C 触发带超时的优雅关闭。
//! - 启动期**不联网**（access_token 在首次调用上游时才惰性换取），不启动反向 WS / webhook
//!   （M1h-b）、不装配 idmap / echo（M1i），也不启动 config watcher 与 WebUI。
//!
//! 退出码与 CLI 细节见 `crates/gensokyo-bin/README.md`。

mod cli;
mod logging;
mod startup;
mod version;

use std::process::ExitCode;

use clap::Parser as _;

use crate::cli::{Action, BIN_NAME, Cli, EXIT_UNIMPLEMENTED};

fn main() -> ExitCode {
    let argv = cli::normalize_legacy_flags(std::env::args_os());
    let cli = Cli::parse_from(argv);

    match cli::resolve(cli) {
        Action::Version => {
            println!("{BIN_NAME} {}", version::version_string());
            ExitCode::SUCCESS
        }
        Action::Unimplemented { flag, hint } => {
            // M1j 只装配正向 WS/HTTP：维护类开关一律显式失败，避免"看起来执行了"。
            eprintln!(
                "未实现：{flag}（{hint}）；M1j 只装配正向 WS/HTTP，见 crates/gensokyo-bin/README.md"
            );
            ExitCode::from(EXIT_UNIMPLEMENTED)
        }
        Action::Run { faststart } => run(faststart),
    }
}

/// 起异步运行时并跑启动链。
fn run(faststart: bool) -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("构建异步运行时失败：{err}");
            return ExitCode::FAILURE;
        }
    };

    match runtime.block_on(startup::serve(faststart)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("启动失败：{err}");
            ExitCode::FAILURE
        }
    }
}
