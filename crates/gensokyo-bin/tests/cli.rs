//! CLI 形状的黑盒测试：直接跑 `gensokyo` 二进制，断言退出码与输出。
//!
//! 启动链（临时 config.yml + `GET /` 健康检查）在 `src/startup.rs` 的单元测试里，
//! 那里可以直接用 `RuntimeOptions` 把端口压到 `127.0.0.1:0`，不占固定端口、也不会等 Ctrl-C。

use std::path::Path;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_gensokyo");

fn run(args: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("运行 gensokyo 失败")
}

/// 在给定工作目录跑一次（`import-idmap` 的库路径与 `config.yml` 都相对工作目录）。
fn run_in(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .current_dir(dir)
        .args(args)
        .output()
        .expect("运行 gensokyo 失败")
}

/// 最小的 `config.yml`：导入只需要 AppID 与（默认）库路径。
const MINIMAL_CONFIG: &str = r#"version: 1
settings:
  enable_ws_server: false
  http_address: ""
"#;

/// 一份合法的最小导出文件（`end.sha256` 由 Windows CNG 独立算出）。
const MINIMAL_EXPORT: &str = concat!(
    r#"{"kind":"meta","format":"gensokyo-idmap","version":1,"allocator":"counter"}"#,
    "\n",
    r#"{"kind":"identity","key":"raw:alice","raw":"alice","vuin":1}"#,
    "\n",
    r#"{"kind":"end","lines":2,"sha256":"3d1c541eb60219664bd294b5a1086a452b414b550373e354b6be877869a1b0f4"}"#,
    "\n",
);

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

fn import_fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("临时目录");
    std::fs::write(dir.path().join("config.yml"), MINIMAL_CONFIG).expect("写 config.yml");
    std::fs::write(dir.path().join("idmap.jsonl"), MINIMAL_EXPORT).expect("写 idmap.jsonl");
    dir
}

#[test]
fn import_idmap_imports_and_reports() {
    let dir = import_fixture();
    let output = run_in(dir.path(), &["import-idmap", "--file", "idmap.jsonl"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "导入应成功，stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("记录 2 条"),
        "应打印导入摘要，实际: {stdout:?}"
    );
    assert!(
        stdout.contains("身份 新 1"),
        "应报出新建身份数，实际: {stdout:?}"
    );
    assert!(
        dir.path().join("openid-map.redb").exists(),
        "应在工作目录建库"
    );

    // 默认要求库为空：第二次不带 --merge 直接拒绝（退出码 1）
    let again = run_in(dir.path(), &["import-idmap", "--file", "idmap.jsonl"]);
    assert_eq!(again.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&again.stderr).contains("目标库非空"),
        "stderr: {}",
        String::from_utf8_lossy(&again.stderr)
    );

    // --dry-run 只校验
    let dry = run_in(
        dir.path(),
        &[
            "import-idmap",
            "--file",
            "idmap.jsonl",
            "--merge",
            "--dry-run",
        ],
    );
    assert_eq!(dry.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&dry.stdout).contains("dry-run"),
        "dry-run 摘要要标出来"
    );
}

#[test]
fn import_idmap_format_error_exits_4() {
    let dir = import_fixture();
    let broken = MINIMAL_EXPORT.replace("3d1c", "0000");
    std::fs::write(dir.path().join("broken.jsonl"), broken).expect("写坏文件");
    let output = run_in(dir.path(), &["import-idmap", "--file", "broken.jsonl"]);
    assert_eq!(
        output.status.code(),
        Some(4),
        "sha256 不符应以格式错退出，stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
