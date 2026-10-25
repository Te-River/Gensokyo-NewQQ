//! CLI 形状的黑盒测试：直接跑 `gensokyo` 二进制，断言退出码与输出。
//!
//! 启动链（临时 config.yml + `GET /` 健康检查）在 `src/startup.rs` 的单元测试里，
//! 那里可以直接用 `RuntimeOptions` 把端口压到 `127.0.0.1:0`，不占固定端口、也不会等 Ctrl-C。

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_gensokyo");

fn run(args: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("运行 gensokyo 失败")
}

#[test]
fn version_flag_exits_zero_and_prints_name() {
    for args in [["--version"], ["-V"], ["version"]] {
        let output = run(&args);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{args:?} 应退出 0，stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.trim().starts_with("gensokyo "),
            "{args:?} 应输出 `gensokyo <ver>`，实际: {stdout:?}"
        );
        assert!(
            stdout.split_whitespace().count() >= 2,
            "{args:?} 应带版本号，实际: {stdout:?}"
        );
    }
}

#[test]
fn unimplemented_switches_exit_non_zero_and_say_so() {
    // Go 写法（单横线）与子命令写法都要覆盖。
    let cases = [
        "tidy",
        "clean_ids",
        "del_ids",
        "del_cache",
        "compaction",
        "-tidy",
        "-clean_ids",
        "-m",
        "-local-logger=enable",
        "--local-logger=enable",
    ];
    for arg in cases {
        let output = run(&[arg]);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{arg} 应以退出码 2 结束，实际: {:?}",
            output.status.code()
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("未实现"),
            "{arg} 的 stderr 应说明未实现，实际: {stderr:?}"
        );
    }
}
