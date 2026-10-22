//! M1c 验收：快照热重载、重启项、模板生成与真实文件监听。

use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gensokyo_config::{
    CONFIG_TEMPLATE, ConfigManager, ConfigWatcher, ReloadOutcome, ensure_config_file, parse_str,
};
use tempfile::tempdir;

fn yaml(app_id: u64, log_level: i64, port: &str, bind_prefix: &str) -> String {
    format!(
        "version: 1\nsettings:\n  app_id: {app_id}\n  log_level: {log_level}\n  \
         port: \"{port}\"\n  bind_prefix: \"{bind_prefix}\"\n"
    )
}

#[test]
fn reload_applies_hot_field_and_reports_unchanged_afterwards() {
    let dir = tempdir().expect("临时目录");
    let path = dir.path().join("config.yml");
    fs::write(&path, yaml(102001234, 1, "5700", "/bind")).expect("写配置失败");

    let manager = ConfigManager::new(&path).expect("加载失败");
    assert_eq!(manager.snapshot().bind_prefix, "/bind");
    assert_eq!(
        manager.reload().expect("reload 失败"),
        ReloadOutcome::Unchanged
    );

    fs::write(&path, yaml(102001234, 1, "5700", "/bind2")).expect("写配置失败");
    let outcome = manager.reload().expect("reload 失败");
    assert_eq!(
        outcome,
        ReloadOutcome::Applied {
            changed: vec!["bind_prefix".to_owned()]
        }
    );
    assert_eq!(manager.snapshot().bind_prefix, "/bind2", "快照必须已更新");
    assert_eq!(
        manager.reload().expect("reload 失败"),
        ReloadOutcome::Unchanged
    );
}

#[test]
fn restart_required_fields_block_snapshot_swap() {
    let cases = [
        ("port", yaml(102001234, 1, "5701", "/bind")),
        ("app_id", yaml(102001235, 1, "5700", "/bind")),
        ("log_level", yaml(102001234, 2, "5700", "/bind")),
    ];

    for (expected_tag, next) in cases {
        let dir = tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        fs::write(&path, yaml(102001234, 1, "5700", "/bind")).expect("写配置失败");
        let manager = ConfigManager::new(&path).expect("加载失败");

        fs::write(&path, next).expect("写配置失败");
        match manager.reload().expect("reload 失败") {
            ReloadOutcome::RestartRequired { changed } => {
                assert_eq!(changed, vec![expected_tag.to_owned()], "变更项");
            }
            other => panic!("{expected_tag} 必须触发 RestartRequired，实际 {other:?}"),
        }

        let snapshot = manager.snapshot();
        assert_eq!(snapshot.port, "5700", "重启项不进入快照");
        assert_eq!(snapshot.app_id, 102001234);
        assert_eq!(snapshot.log_level, 1);
    }
}

#[test]
fn template_parses_with_current_loader() {
    let settings = parse_str(CONFIG_TEMPLATE).expect("模板必须能被当前 loader 解析");
    assert_eq!(settings.port, "15630");
    assert_eq!(settings.app_id, 12345);
    assert_eq!(settings.log_level, 1);
    assert!(
        !settings.ws_address.is_empty(),
        "模板里的 ws_address 必须有值"
    );
}

#[test]
fn ensure_config_file_generates_from_template() {
    let dir = tempdir().expect("临时目录");
    let path = dir.path().join("config.yml");
    assert!(
        ensure_config_file(&path).expect("生成失败"),
        "缺失时必须生成"
    );

    let manager = ConfigManager::new(&path).expect("生成的配置必须能被加载");
    assert_eq!(manager.snapshot().port, "15630");

    fs::write(&path, "version: 2\n").expect("写配置失败");
    assert!(
        !ensure_config_file(&path).expect("二次调用失败"),
        "已存在时不覆盖"
    );
    assert_eq!(
        fs::read_to_string(&path).expect("读回失败"),
        "version: 2\n",
        "已存在的文件必须原样保留"
    );
}

#[tokio::test]
async fn watcher_reports_real_file_change() {
    let dir = tempdir().expect("临时目录");
    let path = dir.path().join("config.yml");
    fs::write(&path, yaml(102001234, 1, "5700", "/bind")).expect("写配置失败");

    let manager = Arc::new(ConfigManager::new(&path).expect("加载失败"));
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let handle = ConfigWatcher::start(Arc::clone(&manager), tx).expect("启动监听失败");

    // 监听在 start() 内同步注册，这里只是给个别平台的文件系统留出稳定窗口
    tokio::time::sleep(Duration::from_millis(200)).await;
    fs::write(&path, yaml(102001234, 1, "5700", "/bind2")).expect("写配置失败");

    let outcome = tokio::time::timeout(Duration::from_secs(20), rx.recv())
        .await
        .expect("20 秒内必须收到重载结果")
        .expect("channel 不应关闭");
    assert!(
        matches!(&outcome, ReloadOutcome::Applied { changed } if changed == &vec!["bind_prefix".to_owned()]),
        "实际结果 {outcome:?}"
    );
    assert_eq!(manager.snapshot().bind_prefix, "/bind2");
    handle.stop();
}

#[test]
fn stop_does_not_hang_when_receiver_is_full() {
    let dir = tempdir().expect("临时目录");
    let path = dir.path().join("config.yml");
    fs::write(&path, yaml(102001234, 1, "5700", "/bind")).expect("写配置失败");

    let manager = Arc::new(ConfigManager::new(&path).expect("加载失败"));
    // 容量 1 + 全程不消费：第二次重载必然撞上 Full
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let handle = ConfigWatcher::start(Arc::clone(&manager), tx).expect("启动监听失败");

    fs::write(&path, yaml(102001234, 1, "5700", "/bind2")).expect("写配置失败");
    std::thread::sleep(Duration::from_millis(1500));
    fs::write(&path, yaml(102001234, 1, "5700", "/bind3")).expect("写配置失败");
    std::thread::sleep(Duration::from_millis(1500));

    // stop() 若挂在阻塞式发送上（旧实现）会让整个测试进程卡死而非断言失败；
    // 因此把 stop() 放进独立线程，主线程带超时等结果：超时就 panic，且不 join 该线程，
    // 进程仍能正常退出。
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let started = Instant::now();
        handle.stop();
        // 主线程已超时放弃等待时发送失败，属预期
        let _ = done_tx.send(started.elapsed());
    });

    match done_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(elapsed) => assert!(
            elapsed < Duration::from_secs(8),
            "receiver 不读 + channel 满时 stop 必须在 8 秒内返回（blocking_send 会永久挂起），实际 {elapsed:?}"
        ),
        Err(_) => panic!("stop() 10 秒内未返回：监听线程阻塞在发送路径上"),
    }
    assert!(rx.try_recv().is_ok(), "channel 里应留着第一次的重载结果");
}
