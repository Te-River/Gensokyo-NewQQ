//! 配置热重载接线：真实文件变动驱动快照更新，关闭后监听线程退出。

mod support;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gensokyo_config::ConfigManager;
use gensokyo_runtime::{AppState, Runtime, RuntimeHandle, RuntimeOptions};
use tempfile::TempDir;

/// 只改 `bind_prefix`（热字段），并保持两个 listener 都不启动：用例不占端口。
fn write_config(path: &Path, bind_prefix: &str) {
    std::fs::write(
        path,
        format!(
            "version: 1\nsettings:\n  enable_ws_server: false\n  http_address: \"\"\n  bind_prefix: \"{bind_prefix}\"\n"
        ),
    )
    .expect("写配置失败");
}

/// 轮询等快照更新：5 秒兜底防 flake（notify 在 Windows 上偶有延迟）。
async fn wait_for_bind_prefix(config: &ConfigManager, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if config.snapshot().bind_prefix == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "5 秒内未观察到配置热重载，当前 bind_prefix = {:?}",
        config.snapshot().bind_prefix
    );
}

async fn start_runtime(config: Arc<ConfigManager>, idmap_dir: &TempDir) -> RuntimeHandle {
    let state = support::with_temp_stores(AppState::with_builtin_handlers(config), idmap_dir);
    Runtime::start_with(
        state,
        RuntimeOptions {
            config_watch: true,
            ..RuntimeOptions::default()
        },
    )
    .await
    .expect("启动运行时失败")
}

#[tokio::test]
async fn runtime_applies_config_change_and_stops_watcher_on_shutdown() {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = dir.path().join("config.yml");
    write_config(&path, "/before");

    let config = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
    let handle = start_runtime(Arc::clone(&config), &dir).await;
    assert!(
        handle.config_watch_enabled(),
        "config_watch=true 时应启动监听"
    );

    // 监听在 start() 内同步注册；给文件系统留出稳定窗口（与 m1c 用例一致）。
    tokio::time::sleep(Duration::from_millis(200)).await;
    write_config(&path, "/after");
    wait_for_bind_prefix(&config, "/after").await;

    handle.shutdown_with_timeout(Duration::from_secs(5)).await;

    // 关闭后再改文件不应生效：证明监听线程已退出，而不是留在后台继续重载。
    write_config(&path, "/after-shutdown");
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(
        config.snapshot().bind_prefix,
        "/after",
        "关闭后不应再重载（监听线程应已退出）"
    );
}

#[tokio::test]
async fn watcher_start_failure_only_warns() {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = dir.path().join("config.yml");
    write_config(&path, "/before");
    let config = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
    // idmap 用独立临时目录：下面要删掉 config 目录来模拟监听不可用。
    let idmap_dir = tempfile::tempdir().expect("临时目录");
    // 快照已读入内存；删掉目录让 notify 注册监听失败，模拟监听不可用的环境。
    std::fs::remove_dir_all(dir.path()).expect("删除临时目录失败");

    let handle = start_runtime(config, &idmap_dir).await;
    assert!(
        !handle.config_watch_enabled(),
        "监听启动失败应降级为未启用，而不是阻断启动"
    );
    handle.shutdown_with_timeout(Duration::from_secs(5)).await;
}
