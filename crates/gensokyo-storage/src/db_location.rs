//! 数据库文件的自定义位置（Go `dblocation/`）：注册表 + 路径解析 + 可达性探测 + 迁移裁决。
//!
//! 注册表 [`KNOWN_DB_FILES`] 第一阶段只覆盖 **idmap 族**（身份库 / 消息库，即 Go 的
//! `idmap.db` 族在 Rust 侧的等价 redb 文件）。botstats / 短链 / union 缓存不进注册表，
//! 开关打开也不迁移、不搬迁；后续要接入只需在注册表追加一行。
//!
//! ## 启动期契约
//! 唯一的入口是 [`setup`]：配置加载后、**任何数据库打开前**调用一次（Go `main.go:167-169`
//! 的 `dblocation.Setup`）。开关关闭时零行为变化——[`resolve_file`] 原样返回文件名，所有
//! 库仍相对工作目录打开。启用失败（路径非法 / 不可达 / 迁移冲突 / 校验失败）返回错误，
//! 调用方**拒绝启动**：绝不静默回退本地，避免本地与目标两侧数据分叉。
//!
//! ## 迁移裁决（逐文件）
//! - 目标与本地同时存在 → 冲突，拒绝启动（无法自动裁决保留哪一份）；
//! - 目标有、本地无 → 视为已迁移，直接用目标；
//! - 本地有、目标无 → 文件级复制（不解读格式）→ 校验（大小 + 只读打开）→ 本地原件改名
//!   `.bak.<时间戳>` 保留；复制/校验失败删目标残留、本地不动、拒绝启动；
//! - 两边都没有 → 全新，直接用目标。
//!
//! ## 已知限制
//! 网络文件系统（NFS/SMB/挂载盘）的文件锁语义无官方保证：探针用一次性 redb 库走与正式库
//! 完全相同的锁路径，但该位置对多进程并发仍不安全，本功能按单实例运行、用户自担风险设计
//! （与 Go 引用 etcd-io/bbolt issue #257 的口径一致）。

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use thiserror::Error;
use tracing::{info, warn};

use crate::idmap::{DEFAULT_IDENTITY_PATH, DEFAULT_MSG_PATH};

/// 第一阶段注册表：只登记 idmap 族的 redb 文件（迁移裁决与运行时打开路径共用）。
pub const KNOWN_DB_FILES: [&str; 2] = [DEFAULT_IDENTITY_PATH, DEFAULT_MSG_PATH];

/// 可写性探针的临时文件名（写完即删）。
const PROBE_FILE: &str = ".gensokyo-dbprobe";
/// redb 锁探针的库文件名：固定名字才能让第二个实例撞上锁（Go 同口径）。
const PROBE_DB: &str = "gensokyo_db_probe.redb";
/// 网络挂载可能瞬时失败，重试次数与退避对齐 Go `probeDir`（0 / 500ms / 1000ms）。
const PROBE_ATTEMPTS: u32 = 3;

/// 自定义数据库位置的解析 / 探测 / 迁移错误。
#[derive(Debug, Error)]
pub enum DbLocationError {
    /// 开关打开但目录为空。
    #[error("custom_db_enabled 已开启但 custom_db_path 为空,请在 config.yml 中配置数据库目录")]
    EmptyPath,
    /// 形如协议 URL 的输入（当前版本只支持文件系统路径与已挂载路径）。
    #[error(
        "custom_db_path {0:?} 形似协议 URL: 当前版本仅支持文件系统路径(网络挂载用 UNC \\\\server\\share 或已挂载盘符),协议 URL 由未来版本支持"
    )]
    ProtocolUrl(String),
    /// 取工作目录失败。
    #[error("获取工作目录失败: {0}")]
    WorkDir(#[source] io::Error),
    /// 目录不可达 / 不可写 / 文件锁探针失败。
    #[error(
        "目录 {dir} 不可达或不可用: {detail} (若为网络挂载,请检查挂载状态后重试;若为文件锁探针失败,说明该位置的文件锁语义对 redb 数据库不可用,不能作为数据库位置)"
    )]
    Probe {
        /// 探测的目录。
        dir: String,
        /// 最后一次探测的失败原因。
        detail: String,
    },
    /// 本地与目标同时存在同一库文件。
    #[error(
        "迁移冲突: 本地与目标目录同时存在 {name} (本地 {local}: {local_size} 字节 / 目标 {remote}: {remote_size} 字节),请手动确认保留哪一份并删除另一份后重启"
    )]
    Conflict {
        /// 库文件名。
        name: String,
        /// 本地路径。
        local: PathBuf,
        /// 本地文件大小。
        local_size: u64,
        /// 目标路径。
        remote: PathBuf,
        /// 目标文件大小。
        remote_size: u64,
    },
    /// 单个文件的迁移失败（复制 / 校验 / 备份改名）。
    #[error("{detail}")]
    Migrate {
        /// 已展开的错误说明（含清理动作）。
        detail: String,
    },
    /// 检查本地 / 目标文件状态失败。
    #[error("检查{side}文件 {path} 失败: {source}")]
    Stat {
        /// `本地` / `目标`。
        side: &'static str,
        /// 文件路径。
        path: PathBuf,
        /// 底层 IO 错误。
        #[source]
        source: io::Error,
    },
}

/// 解析后的数据库目录：进程级一次性状态（Go 侧是 `enabled` + `baseDir` 两个包级变量）。
static BASE_DIR: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();

fn base_dir() -> &'static RwLock<Option<PathBuf>> {
    BASE_DIR.get_or_init(|| RwLock::new(None))
}

/// 读当前目录（锁中毒时取回内部值：这里的状态只是一个路径，没有半写的风险）。
fn current_base() -> Option<PathBuf> {
    base_dir()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// 对齐 Go `dblocation.Setup`：配置加载后、任何数据库打开前调用。
///
/// - 开关关闭 → `Ok(None)`，零行为变化；
/// - 解析出的目录与工作目录相同 → `Ok(None)`（视为未启用，与 Go 一致）；
/// - 成功 → `Ok(Some(dir))`，此后 [`resolve_file`] 返回该目录下的绝对路径；
/// - 任何失败 → `Err`，调用方必须拒绝启动（不得回退本地）。
///
/// 进程级一次性：已经解析成功后再调用会直接返回既有目录（配置项本身是重启项）。
///
/// # Errors
/// 路径为空 / 形似协议 URL / 目录不可达不可写 / 迁移冲突或复制校验失败时返回
/// [`DbLocationError`]。
pub fn setup(enabled: bool, raw_path: &str) -> Result<Option<PathBuf>, DbLocationError> {
    if let Some(dir) = current_base() {
        return Ok(Some(dir));
    }
    if !enabled {
        return Ok(None);
    }
    let Some(dir) = prepare(raw_path, Path::new(""))? else {
        info!("[dblocation] custom_db_path 解析结果与工作目录相同,视为未启用(数据库位置不变)");
        return Ok(None);
    };
    *base_dir()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(dir.clone());
    info!("[dblocation] idmap 族数据库位置: {}", dir.display());
    warn!(
        "[dblocation] 网络文件系统(NFS/SMB/挂载盘)上的文件锁语义无官方保证: 本功能按单实例运行、用户自担风险设计,多进程并发访问同一网络位置上的库文件会损坏数据(etcd-io/bbolt issue #257 实测),请勿多开实例共用同一目录"
    );
    Ok(Some(dir))
}

/// idmap 族库文件的实际打开路径：开关关闭时原样返回 `name`（相对工作目录，历史行为），
/// 启用后返回解析目录下的绝对路径。所有 idmap 运行时打开点都必须经由此函数取路径。
#[must_use]
pub fn resolve_file(name: &str) -> PathBuf {
    match current_base() {
        Some(dir) => dir.join(name),
        None => PathBuf::from(name),
    }
}

/// 路径解析的唯一入口（也是未来协议客户端唯一的扩展缝：`smb://` 等远程形态在这里接入
/// 协议客户端，而不是散落到各打开点）。当前只支持文件系统路径与已挂载路径：
/// - 相对路径 → 相对工作目录解析为绝对路径；
/// - 绝对路径 / UNC（`\\server\share`）→ 直接使用；
/// - 含 `://` 的输入 → 拒绝（当前版本不支持协议 URL）。
///
/// # Errors
/// 空路径、协议 URL、或取工作目录失败时返回 [`DbLocationError`]。
pub fn normalize(raw_path: &str) -> Result<PathBuf, DbLocationError> {
    let raw = raw_path.trim();
    if raw.is_empty() {
        return Err(DbLocationError::EmptyPath);
    }
    if raw.contains("://") {
        return Err(DbLocationError::ProtocolUrl(raw.to_owned()));
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return Ok(clean(path));
    }
    let cwd = std::env::current_dir().map_err(DbLocationError::WorkDir)?;
    Ok(clean(&cwd.join(path)))
}

/// 逐层收紧的点 / 冗余分隔符：`Path::components` 会丢掉中间的 `.` 与重复分隔符
/// （等价 Go `filepath.Clean` 的主要效果）。
fn clean(path: &Path) -> PathBuf {
    path.components().collect()
}

/// 两个目录是否指向同一路径（Windows 路径大小写不敏感，与 Go `sameDir` 一致）。
fn same_dir(a: &Path, b: &Path) -> bool {
    let (a, b) = (clean(a), clean(b));
    if cfg!(windows) {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}

/// 解析 + 与工作目录比较 + 探测 + 迁移裁决；返回 `Ok(None)` 表示与工作目录相同（视为未启用）。
///
/// `local_dir` 是"本地库文件所在目录"（生产为空路径 = 工作目录下的相对文件名），测试用
/// 临时目录注入，避免改进程工作目录。
fn prepare(raw_path: &str, local_dir: &Path) -> Result<Option<PathBuf>, DbLocationError> {
    let dir = normalize(raw_path)?;
    let cwd = std::env::current_dir().map_err(DbLocationError::WorkDir)?;
    if same_dir(&dir, &cwd) {
        return Ok(None);
    }
    probe(&dir)?;
    migrate_family(&dir, local_dir)?;
    Ok(Some(dir))
}

/// 探测目录：存在（或可创建）且可写，并通过 redb 锁探针（见 [`probe_once`]）。
///
/// 网络挂载可能瞬时失败，按 [`PROBE_ATTEMPTS`] 次间隔退避重试。
fn probe(dir: &Path) -> Result<(), DbLocationError> {
    let mut last = String::new();
    for attempt in 0..PROBE_ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(500 * u64::from(attempt)));
        }
        match probe_once(dir) {
            Ok(()) => return Ok(()),
            Err(detail) => last = detail,
        }
    }
    Err(DbLocationError::Probe {
        dir: dir.display().to_string(),
        detail: last,
    })
}

/// 单次探测：可写性（快速失败）+ redb 锁探针（真实文件锁路径）。
fn probe_once(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|err| format!("创建目录失败: {err}"))?;
    let probe = dir.join(PROBE_FILE);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&probe)
        .map_err(|err| format!("写探针文件失败: {err}"))?;
    file.write_all(b"probe")
        .map_err(|err| format!("写探针文件失败: {err}"))?;
    drop(file);
    fs::remove_file(&probe).map_err(|err| format!("删除探针文件失败: {err}"))?;
    redb_probe(dir)
}

/// 用一次性 redb 库验证目标位置的文件锁语义：`Database::create` 走与正式库完全相同的
/// 锁路径，成功后立即关闭并删除探针文件。
///
/// 仅靠"建临时文件再删"发现不了网络挂载上锁静默失效；探针打开/加锁失败 = 该位置对文件
/// 数据库不可用。
fn redb_probe(dir: &Path) -> Result<(), String> {
    let probe = dir.join(PROBE_DB);
    match redb::Database::create(&probe) {
        Ok(db) => {
            drop(db);
            fs::remove_file(&probe).map_err(|err| format!("删除探针库失败: {err}"))
        }
        Err(err) => {
            // 锁被另一个进程持有说明探针文件不是本进程的残留，删掉它会让双实例同时启动的
            // 意外互斥失效，故只在非"已在打开中"错误时清理普通文件残留。
            if !matches!(err, redb::DatabaseError::DatabaseAlreadyOpen)
                && fs::metadata(&probe).is_ok_and(|meta| meta.is_file())
            {
                let _ = fs::remove_file(&probe);
            }
            Err(format!("redb 锁探针失败: {err}"))
        }
    }
}

/// 按注册表逐个文件做迁移裁决（见模块文档的四条分支）。
fn migrate_family(dir: &Path, local_dir: &Path) -> Result<(), DbLocationError> {
    for name in KNOWN_DB_FILES {
        migrate_one(dir, name, local_dir)?;
    }
    Ok(())
}

/// 单个文件的迁移裁决。
fn migrate_one(dir: &Path, name: &str, local_dir: &Path) -> Result<(), DbLocationError> {
    let local_path = local_dir.join(name);
    let remote_path = dir.join(name);
    let local = stat_opt(&local_path, "本地")?;
    let remote = stat_opt(&remote_path, "目标")?;
    match (local, remote) {
        (Some(local), Some(remote)) => Err(DbLocationError::Conflict {
            name: name.to_owned(),
            local: local_path,
            local_size: local,
            remote: remote_path,
            remote_size: remote,
        }),
        // 目标有、本地无：已迁移，直接用目标
        (None, Some(_)) => Ok(()),
        (Some(local_size), None) => migrate_file(&local_path, &remote_path, local_size),
        (None, None) => Ok(()),
    }
}

/// `fs::metadata` 的"存在即大小"版本：不存在返回 `None`，其余错误上抛。
fn stat_opt(path: &Path, side: &'static str) -> Result<Option<u64>, DbLocationError> {
    match fs::metadata(path) {
        Ok(meta) => Ok(Some(meta.len())),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(DbLocationError::Stat {
            side,
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// 单文件迁移：复制 → 校验 → 本地原件改名 `.bak.<时间戳>` → 摘要日志。
///
/// 校验失败时删除目标副本（本地原件不动）并拒绝启动：留着半截副本会让下次启动落入
/// 迁移冲突分支，把瞬时故障变成需要人工删文件的永久拒绝启动。
fn migrate_file(local: &Path, remote: &Path, local_size: u64) -> Result<(), DbLocationError> {
    let fail = |detail: String| {
        let _ = fs::remove_file(remote);
        DbLocationError::Migrate { detail }
    };
    copy_file(local, remote).map_err(|err| {
        fail(format!(
            "复制 {} 到 {} 失败: {err},已删除目标残留副本",
            local.display(),
            remote.display()
        ))
    })?;
    let remote_size = fs::metadata(remote).map_err(|err| {
        fail(format!(
            "校验目标文件 {} 失败: {err},已删除目标残留副本",
            remote.display()
        ))
    })?;
    if remote_size.len() != local_size {
        return Err(fail(format!(
            "迁移 {} 校验失败(大小不一致: 本地 {local_size} 字节 / 目标 {} 字节),已删除目标副本,本地原件未动",
            local.display(),
            remote_size.len()
        )));
    }
    // 0 字节文件无数据可校验，跳过只读打开（正式打开时会自动初始化）
    if local_size > 0 {
        verify_redb(remote).map_err(|err| {
            fail(format!(
                "迁移 {} 校验失败(只读打开 {} 失败): {err},已删除目标副本,本地原件未动",
                local.display(),
                remote.display()
            ))
        })?;
    }
    let bak = backup_path(local);
    fs::rename(local, &bak).map_err(|err| {
        fail(format!(
            "备份本地文件 {} 到 {} 失败: {err},已删除目标副本",
            local.display(),
            bak.display()
        ))
    })?;
    info!(
        "[dblocation] 已迁移 {} -> {} ({} 字节),本地原件保留于 {}",
        local.display(),
        remote.display(),
        local_size,
        bak.display()
    );
    Ok(())
}

/// 纯文件级复制，不解读格式（与 Go `copyFile` 一致），落盘后 `sync_all` 保证网络盘上的
/// 只读校验读到的是完整内容。
fn copy_file(src: &Path, dst: &Path) -> io::Result<()> {
    fs::copy(src, dst)?;
    fs::OpenOptions::new().write(true).open(dst)?.sync_all()
}

/// 只读打开校验：文件可读且确实是 redb 库（与 Go `verifyBbolt` 的 ReadOnly 口径一致）。
fn verify_redb(path: &Path) -> Result<(), redb::DatabaseError> {
    redb::ReadOnlyDatabase::open(path).map(drop)
}

/// 本地原件的备份名：`<file>.bak.<YYYYMMDD-HHMMSS>`（Go 用本地时间 `time.Format`）。
fn backup_path(local: &Path) -> PathBuf {
    let stamp = time::OffsetDateTime::now_local()
        .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
        .format(time::macros::format_description!(
            "[year][month][day]-[hour][minute][second]"
        ))
        .unwrap_or_default();
    let mut name = local.as_os_str().to_owned();
    name.push(format!(".bak.{stamp}"));
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建一个可用的 redb 库文件（写一个 bucket，等价 Go 测试的 `writeBboltFile`）。
    fn write_redb(path: &Path) {
        let db = redb::Database::create(path).expect("创建测试 redb 文件失败");
        const TABLE: redb::TableDefinition<'static, &'static str, u64> =
            redb::TableDefinition::new("ids");
        let txn = db.begin_write().expect("开启测试写事务失败");
        {
            txn.open_table(TABLE).expect("打开测试表失败");
        }
        txn.commit().expect("提交测试写事务失败");
        drop(db);
    }

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("创建临时目录失败")
    }

    #[test]
    fn normalize_table() {
        let abs_input = std::env::temp_dir()
            .join("gsk-data")
            .to_string_lossy()
            .into_owned();
        let cases: Vec<(&str, String, &str)> = vec![
            ("绝对路径", abs_input, ""),
            ("相对路径", "sub/data".to_owned(), ""),
            ("UNC路径", r"\\server\share\gsk".to_owned(), ""),
            ("前后空格", "  sub/data  ".to_owned(), ""),
            ("空路径", "   ".to_owned(), "为空"),
            ("smb协议URL", "smb://server/share".to_owned(), "协议 URL"),
            (
                "http协议URL",
                "http://example.com/db".to_owned(),
                "协议 URL",
            ),
            (
                "https协议URL",
                "https://example.com/db".to_owned(),
                "协议 URL",
            ),
        ];
        for (name, input, want_err) in &cases {
            let got = normalize(input);
            if !want_err.is_empty() {
                let err = got.expect_err(&format!("{name}: 期望报错 {want_err}"));
                assert!(
                    err.to_string().contains(want_err),
                    "{name}: 错误信息 {err} 未包含 {want_err}"
                );
                continue;
            }
            let path = got.unwrap_or_else(|err| panic!("{name}: 意外报错: {err}"));
            // UNC 在非 Windows 上是普通相对路径（Go 的 `filepath.IsAbs` 同样如此）
            if cfg!(windows) || !input.starts_with(r"\\") {
                assert!(path.is_absolute(), "{name}: 期望绝对路径,实际 {path:?}");
            }
        }
    }

    #[test]
    fn probe_existing_writable_dir() {
        let dir = tempdir();
        probe(dir.path()).expect("探测可写目录失败");
        assert!(!dir.path().join(PROBE_DB).exists(), "探针库应被删除");
        assert!(!dir.path().join(PROBE_FILE).exists(), "探针文件应被删除");
    }

    #[test]
    fn probe_creates_missing_subdirs() {
        let dir = tempdir();
        probe(&dir.path().join("a").join("b")).expect("探测可创建目录失败");
    }

    #[test]
    fn probe_rejects_file_as_dir() {
        let dir = tempdir();
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, b"x").expect("创建占位文件失败");
        let err = probe(&blocker).expect_err("期望探测失败");
        assert!(err.to_string().contains("不可达或不可用"), "{err}");
    }

    #[test]
    fn probe_fails_when_probe_db_locked() {
        let dir = tempdir();
        // 持锁的 redb 库占住固定探针名：模拟双实例 / 锁语义失效
        let holder = redb::Database::create(dir.path().join(PROBE_DB)).expect("创建持锁探针失败");
        let err = redb_probe(dir.path()).expect_err("期望 probe 失败(锁被占用)");
        assert!(err.contains("redb 锁探针失败"), "{err}");
        assert!(
            dir.path().join(PROBE_DB).exists(),
            "他进程持有的探针文件不得被删除"
        );
        drop(holder);
    }

    #[test]
    fn migrate_decisions_table() {
        const NAME: &str = DEFAULT_IDENTITY_PATH;
        let cases: &[(&str, bool, bool, &str, bool)] = &[
            ("两边都没有_全新直接用目标", false, false, "", false),
            ("本地有_迁移", true, false, "", true),
            ("目标有_直接用目标", false, true, "", false),
            ("两边都有_冲突拒绝启动", true, true, "迁移冲突", false),
        ];
        for (name, local_exists, remote_file, want_err, want_migrate) in cases {
            let local_dir = tempdir();
            let dir = tempdir();
            let local_path = local_dir.path().join(NAME);
            let remote_path = dir.path().join(NAME);
            if *local_exists {
                write_redb(&local_path);
            }
            if *remote_file {
                write_redb(&remote_path);
            }

            let result = migrate_one(dir.path(), NAME, local_dir.path());
            if !want_err.is_empty() {
                let err = result.expect_err(&format!("{name}: 期望报错 {want_err}"));
                assert!(
                    err.to_string().contains(want_err),
                    "{name}: 错误信息 {err} 未包含 {want_err}"
                );
                continue;
            }
            result.unwrap_or_else(|err| panic!("{name}: 意外报错: {err}"));
            if *want_migrate {
                assert!(remote_path.exists(), "{name}: 迁移后目标文件不存在");
                verify_redb(&remote_path)
                    .unwrap_or_else(|err| panic!("{name}: 迁移后目标文件校验失败: {err}"));
                assert!(!local_path.exists(), "{name}: 迁移后本地原件应已改名");
                let backups: Vec<_> = fs::read_dir(local_dir.path())
                    .expect("读本地目录失败")
                    .filter_map(|entry| entry.ok())
                    .filter(|entry| entry.file_name().to_string_lossy().contains(".bak."))
                    .collect();
                assert_eq!(backups.len(), 1, "{name}: 期望本地原件保留为 .bak.*");
            }
            if *remote_file {
                assert!(remote_path.exists(), "{name}: 目标文件应保持不变");
            }
        }
    }

    #[test]
    fn migrate_keeps_local_on_verify_failure() {
        let local_dir = tempdir();
        let dir = tempdir();
        let local_path = local_dir.path().join(DEFAULT_MSG_PATH);
        // 非 redb 内容：复制成功、大小一致，但只读打开必然失败
        fs::write(&local_path, b"not-a-redb-file").expect("写测试文件失败");

        let err = migrate_one(dir.path(), DEFAULT_MSG_PATH, local_dir.path())
            .expect_err("期望校验失败拒绝启动");
        assert!(err.to_string().contains("只读打开"), "{err}");
        assert!(local_path.exists(), "校验失败时本地原件不得改动");
        assert!(
            !dir.path().join(DEFAULT_MSG_PATH).exists(),
            "校验失败时目标残留副本必须删除"
        );
    }

    #[test]
    fn prepare_rejects_protocol_url_and_empty() {
        let err = prepare("smb://server/share", Path::new("")).expect_err("期望协议 URL 被拒绝");
        assert!(err.to_string().contains("协议 URL"), "{err}");
        let err = prepare("  ", Path::new("")).expect_err("期望空路径被拒绝");
        assert!(err.to_string().contains("为空"), "{err}");
    }

    #[test]
    fn prepare_treats_workdir_as_disabled() {
        let cwd = std::env::current_dir().expect("取工作目录失败");
        let got = prepare(&cwd.to_string_lossy(), Path::new("")).expect("意外报错");
        assert!(got.is_none(), "期望返回 None(视为未启用),实际 {got:?}");
    }

    #[test]
    fn prepare_migrates_into_new_dir() {
        let local_dir = tempdir();
        let dir = tempdir();
        write_redb(&local_dir.path().join(DEFAULT_IDENTITY_PATH));

        let got = prepare(&dir.path().to_string_lossy(), local_dir.path())
            .expect("意外报错")
            .expect("期望返回目标目录");
        assert!(same_dir(&got, dir.path()), "期望 {dir:?},实际 {got:?}");
        assert!(
            dir.path().join(DEFAULT_IDENTITY_PATH).exists(),
            "迁移后目标文件不存在"
        );
    }

    #[test]
    fn setup_disabled_is_zero_change_and_resolve_round_trips() {
        // 单线程内完成：进程级状态只在这里被写入，避免与其他用例交叉
        assert!(
            setup(false, "  ").expect("关闭时不得报错").is_none(),
            "开关关闭时必须零行为变化"
        );
        assert_eq!(
            resolve_file(DEFAULT_IDENTITY_PATH),
            PathBuf::from(DEFAULT_IDENTITY_PATH)
        );

        let dir = tempdir();
        let path = dir.path().join("nested");
        let got = setup(true, &path.to_string_lossy())
            .expect("启用失败")
            .expect("期望返回目标目录");
        assert!(same_dir(&got, &path), "期望 {path:?},实际 {got:?}");
        assert_eq!(resolve_file(DEFAULT_MSG_PATH), path.join(DEFAULT_MSG_PATH));
        // 复用同一份状态：再次 setup 不重复迁移
        assert_eq!(
            setup(true, &path.to_string_lossy()).expect("二次 setup 失败"),
            Some(path.clone())
        );
    }

    #[test]
    fn backup_path_has_go_shaped_suffix() {
        let path = backup_path(Path::new("openid-map.redb"));
        let text = path.to_string_lossy();
        let suffix = text
            .strip_prefix("openid-map.redb.bak.")
            .unwrap_or_else(|| panic!("备份名格式不符: {text}"));
        let bytes = suffix.as_bytes();
        assert_eq!(bytes.len(), 15, "期望 YYYYMMDD-HHMMSS: {suffix}");
        assert_eq!(bytes[8], b'-', "期望日期与时间之间有 -: {suffix}");
    }
}
