//! 自定义数据库位置的端到端集成：开关关闭零变化、开关打开迁移后启动可用。
//!
//! 只有一个用例：它会切换到临时工作目录（进程级），同二进制内的并行用例会互相干扰，
//! 所以全部断言收在同一个 `#[test]` 里顺序执行。

use std::path::{Path, PathBuf};

use gensokyo_storage::db_location::{self, KNOWN_DB_FILES};
use gensokyo_storage::idmap::{DEFAULT_IDENTITY_PATH, DEFAULT_MSG_PATH};
use gensokyo_storage::{IdMapOptions, IdMapStore};

/// 建一个可用的 redb 库（写一个 bucket，等价 Go 测试的 `writeBboltFile`）。
fn write_redb(path: &Path) {
    let db = redb::Database::create(path).expect("创建测试 redb 文件失败");
    const TABLE: redb::TableDefinition<'static, &'static str, u64> =
        redb::TableDefinition::new("ids");
    let txn = db.begin_write().expect("开启测试写事务失败");
    {
        txn.open_table(TABLE).expect("打开测试表失败");
    }
    txn.commit().expect("提交测试写事务失败");
}

fn backups(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .expect("读目录失败")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.to_string_lossy().contains(".bak."))
        .collect()
}

#[test]
fn custom_db_location_end_to_end() {
    let original_cwd = std::env::current_dir().expect("取工作目录失败");
    let local = tempfile::tempdir().expect("创建本地临时目录失败");
    let target = tempfile::tempdir().expect("创建目标临时目录失败");

    // 本地工作目录里放两个 redb 库，模拟历史数据
    for name in KNOWN_DB_FILES {
        write_redb(&local.path().join(name));
    }
    std::env::set_current_dir(local.path()).expect("切换工作目录失败");

    // ---- 场景 1：开关关闭 → 零变化（路径不变、不迁移、不建目录） ----
    assert!(
        db_location::setup(false, &target.path().join("unused").to_string_lossy())
            .expect("关闭时不得报错")
            .is_none(),
        "开关关闭时必须零行为变化"
    );
    assert_eq!(
        db_location::resolve_file(DEFAULT_IDENTITY_PATH),
        PathBuf::from(DEFAULT_IDENTITY_PATH),
        "关闭时打开路径必须仍是工作目录下的相对文件名"
    );
    assert!(
        !target.path().join("unused").exists(),
        "关闭时不得创建任何目标目录"
    );
    assert!(backups(local.path()).is_empty(), "关闭时不得产生备份");

    // ---- 场景 2：开关打开 → 迁移到目标目录，迁移后库可直接打开使用 ----
    let resolved = db_location::setup(true, &target.path().to_string_lossy())
        .expect("启用自定义数据库位置失败")
        .expect("期望返回解析后的目录");
    assert!(
        resolved.starts_with(target.path()),
        "解析目录应在目标下: {resolved:?}"
    );
    assert_eq!(
        db_location::resolve_file(DEFAULT_MSG_PATH),
        target.path().join(DEFAULT_MSG_PATH)
    );
    for name in KNOWN_DB_FILES {
        assert!(target.path().join(name).exists(), "迁移后目标缺少 {name}");
        assert!(
            !local.path().join(name).exists(),
            "迁移后本地原件应已改名: {name}"
        );
        // 只读打开校验：确实是可读的 redb 库
        redb::ReadOnlyDatabase::open(target.path().join(name))
            .unwrap_or_else(|err| panic!("迁移后 {name} 无法只读打开: {err}"));
    }
    assert_eq!(
        backups(local.path()).len(),
        KNOWN_DB_FILES.len(),
        "每个本地原件都应保留 .bak.<时间戳>"
    );

    // 迁移后启动可用：按 resolve 的路径打开 idmap，写入并读回一条消息 ID 映射
    let store = IdMapStore::open(
        IdMapOptions::new(
            db_location::resolve_file(DEFAULT_IDENTITY_PATH),
            db_location::resolve_file(DEFAULT_MSG_PATH),
        )
        .with_app_id("102001234"),
    )
    .expect("迁移后按自定义位置打开 idmap 失败");
    let msg_vuin = store.store_cache("msg-real-id").expect("写入消息映射失败");
    assert_eq!(
        store
            .retrieve_row_by_cache(msg_vuin)
            .expect("读取消息映射失败"),
        "msg-real-id"
    );

    std::env::set_current_dir(original_cwd).expect("恢复工作目录失败");
}
