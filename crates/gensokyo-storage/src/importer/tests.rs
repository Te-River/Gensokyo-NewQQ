//! JSONL 导入器单测：golden 往返、幂等、冲突两态、校验失败、大整数、dry-run、hash_id 重映射。

use std::path::Path;

use redb::{ReadableDatabase, TableDefinition};
use sha2::{Digest, Sha256};

use super::jsonl::{self, ImportData};
use super::*;
use crate::echo::generate_row_id;
use crate::idmap::encoding::{decode_i64, encode_i64};
use crate::idmap::{
    CONFIG, CURRENT_MSG_KEY, CURRENT_VUIN_KEY, EXPIRES_AT, IDENTITY_TO_VUIN, IdMapOptions,
    LAST_SEEN, LATEST_BY_GROUP_USER, LATEST_EXPIRES_AT, META, MSG_TO_VIRTUAL, USER_INFO,
    VIRTUAL_TO_MSG, VUIN_TO_IDENTITY, decode_vuin,
};

const OPENID: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

fn options(dir: &Path) -> IdMapOptions {
    IdMapOptions::new(dir.join("openid-map.redb"), dir.join("msgid-map.redb")).with_app_id("10001")
}

fn open(dir: &Path) -> IdMapStore {
    IdMapStore::open(options(dir)).expect("打开 idmap 存储失败")
}

fn open_hash(dir: &Path) -> IdMapStore {
    IdMapStore::open(options(dir).with_hash_id(true)).expect("打开 idmap 存储失败")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

/// 组装一份合法 JSONL：`meta` + 给定记录 + `end`（`declared` 用来故意写错 `lines`）。
fn build(records: &[&str], declared: Option<usize>) -> String {
    let mut body = String::from(r#"{"kind":"meta","format":"gensokyo-idmap","version":1}"#);
    body.push('\n');
    for record in records {
        body.push_str(record);
        body.push('\n');
    }
    let lines = records.len() + 1;
    let digest = sha256_hex(body.as_bytes());
    body.push_str(&format!(
        r#"{{"kind":"end","lines":{},"sha256":"{digest}"}}"#,
        declared.unwrap_or(lines)
    ));
    body.push('\n');
    body
}

fn import(
    store: &IdMapStore,
    text: &str,
    options: &ImportOptions,
) -> Result<ImportReport, ImportError> {
    import_bytes(store, text.as_bytes(), options)
}

/// 把 `build` 产出的文件换成带 `allocator` 的 `meta`（记录数与 `end` 行一并重算）。
fn with_allocator(file: &str, allocator: &str) -> String {
    let body = file
        .split_once(r#"{"kind":"end""#)
        .expect("文件含 end 行")
        .0;
    let records = body.matches('\n').count();
    let rest = &body[body.find('\n').expect("文件含 meta 行") + 1..];
    let meta = format!(
        r#"{{"kind":"meta","format":"gensokyo-idmap","version":1,"allocator":"{allocator}"}}"#
    );
    let body = format!("{meta}\n{rest}");
    let digest = sha256_hex(body.as_bytes());
    format!("{body}{{\"kind\":\"end\",\"lines\":{records},\"sha256\":\"{digest}\"}}\n")
}

fn merge() -> ImportOptions {
    ImportOptions {
        merge: true,
        ..ImportOptions::default()
    }
}

fn remap() -> ImportOptions {
    ImportOptions {
        merge: true,
        on_vuin_conflict: OnVuinConflict::Remap,
        ..ImportOptions::default()
    }
}

fn identity_copy(store: &IdMapStore, key: &str) -> Option<i64> {
    let txn = store.identity_db().begin_read().expect("读事务");
    let table = txn.open_table(IDENTITY_TO_VUIN).expect("正表");
    table
        .get(key)
        .expect("读正表")
        .map(|guard| decode_vuin(guard.value()).expect("vUIN 8 字节"))
}

fn bytes_of(
    store: &IdMapStore,
    in_identity_db: bool,
    table: TableDefinition<'static, &'static str, &'static [u8]>,
    key: &str,
) -> Option<Vec<u8>> {
    let db = if in_identity_db {
        store.identity_db()
    } else {
        store.msg_db()
    };
    let txn = db.begin_read().expect("读事务");
    let table = txn.open_table(table).expect("表");
    table
        .get(key)
        .expect("读表")
        .map(|guard| guard.value().to_vec())
}

fn counter(store: &IdMapStore, in_identity_db: bool, key: &str) -> i64 {
    let raw = bytes_of(store, in_identity_db, META, key).expect("计数器应存在");
    decode_i64(&raw).expect("计数器 8 字节")
}

const GOLDEN: &str = concat!(
    r#"{"kind":"meta","format":"gensokyo-idmap","version":1,"exported_at":1700000000,"allocator":"counter"}"#,
    "\n",
    r#"{"kind":"identity","key":"openid:QQ:10001:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","raw":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","vuin":1}"#,
    "\n",
    r#"{"kind":"identity","key":"raw:alice","raw":"alice","vuin":9007199254740993}"#,
    "\n",
    r#"{"kind":"identity","key":"ruin:QQ:0:1","raw":"rUIN-QQ-0-1","vuin":42}"#,
    "\n",
    r#"{"kind":"vuin_counter","counter":9007199254740993}"#,
    "\n",
    r#"{"kind":"msg_counter","counter":77}"#,
    "\n",
    r#"{"kind":"config","section":"openid:QQ:10001:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","key":"group_enable","value":"true"}"#,
    "\n",
    r#"{"kind":"user_info","key":"alice:12345","value":{"UserID":"12345","Nickname":"爱丽丝","Private":true}}"#,
    "\n",
    r#"{"kind":"last_seen","vuin":1,"raw":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}"#,
    "\n",
    r#"{"kind":"msg_id","virtual":77,"real":"MSG-REAL-1"}"#,
    "\n",
    r#"{"kind":"latest","group":"GROUP-1","user":"USER-1","msg_id":"MSG-REAL-2"}"#,
    "\n",
    r#"{"kind":"future_kind","whatever":1}"#,
    "\n",
    r#"{"kind":"end","lines":12,"sha256":"ab1026eb6627a24b79abfd43e6e87fcf019ca45bafcb147147c7df18fb588eb6"}"#,
    "\n",
);

#[test]
fn golden_round_trip_writes_every_table() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());
    let report = import(&store, GOLDEN, &ImportOptions::default()).expect("golden 导入应成功");

    assert_eq!(report.records, 12, "end.lines 是 end 之前的记录数");
    assert_eq!(report.unknown_kinds, 1, "未知 kind 忽略并计数");
    assert_eq!(report.identities_imported, 3);
    assert_eq!(report.identities_unchanged, 0);
    assert_eq!(report.identities_remapped, 0);
    assert_eq!(report.configs, 1);
    assert_eq!(report.user_infos, 1);
    assert_eq!(report.last_seen, 1);
    assert_eq!(report.msg_ids, 1);
    assert_eq!(report.latest, 1);
    assert!(!report.dry_run);

    // 身份正/反表逐字节对齐 Go 编码
    assert_eq!(
        identity_copy(&store, &format!("openid:QQ:10001:{OPENID}")),
        Some(1)
    );
    assert_eq!(
        identity_copy(&store, "raw:alice"),
        Some(9_007_199_254_740_993)
    );
    assert_eq!(identity_copy(&store, "ruin:QQ:0:1"), Some(42));
    assert_eq!(
        bytes_of(
            &store,
            true,
            VUIN_TO_IDENTITY,
            "1:openid:QQ:10001:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        ),
        Some(OPENID.as_bytes().to_vec())
    );
    assert_eq!(
        bytes_of(&store, true, VUIN_TO_IDENTITY, "42:ruin:QQ:0:1"),
        Some(b"rUIN-QQ-0-1".to_vec())
    );

    // last_seen / config / user_info
    assert_eq!(
        bytes_of(&store, true, LAST_SEEN, "default:1"),
        Some(OPENID.as_bytes().to_vec())
    );
    assert_eq!(
        bytes_of(
            &store,
            true,
            CONFIG,
            &format!("openid:QQ:10001:{OPENID}:group_enable")
        ),
        Some(b"true".to_vec())
    );
    // user_info 存紧凑 JSON（对象 key 按字典序，与 serde_json 的 Map 一致）
    let expected_user_info = r#"{"Nickname":"爱丽丝","Private":true,"UserID":"12345"}"#
        .as_bytes()
        .to_vec();
    assert_eq!(
        bytes_of(&store, true, USER_INFO, "alice:12345"),
        Some(expected_user_info)
    );

    // 计数器只增不减；消息库
    assert_eq!(
        counter(&store, true, CURRENT_VUIN_KEY),
        9_007_199_254_740_993
    );
    assert_eq!(counter(&store, false, CURRENT_MSG_KEY), 77);
    assert_eq!(
        bytes_of(&store, false, MSG_TO_VIRTUAL, "MSG-REAL-1"),
        Some(encode_i64(77).to_vec())
    );
    assert_eq!(
        bytes_of(&store, false, VIRTUAL_TO_MSG, "77"),
        Some(b"MSG-REAL-1".to_vec())
    );
    assert_eq!(
        bytes_of(&store, false, EXPIRES_AT, "77"),
        None,
        "导出侧不带 TTL：expires_at 留空（读时视为不过期）"
    );
    assert_eq!(
        bytes_of(&store, false, LATEST_BY_GROUP_USER, "GROUP-1:USER-1"),
        Some(b"MSG-REAL-2".to_vec())
    );
    assert_eq!(
        bytes_of(&store, false, LATEST_EXPIRES_AT, "GROUP-1:USER-1"),
        None
    );
}

#[test]
fn optional_expires_is_written_when_present() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());
    import(
        &store,
        &build(
            &[
                r#"{"kind":"msg_id","virtual":7,"real":"MSG-1","expires":1700003600}"#,
                r#"{"kind":"latest","group":"G","user":"U","msg_id":"MSG-2","expires":1700007200}"#,
            ],
            None,
        ),
        &ImportOptions::default(),
    )
    .expect("导入");
    assert_eq!(
        bytes_of(&store, false, EXPIRES_AT, "7"),
        Some(encode_i64(1_700_003_600).to_vec())
    );
    assert_eq!(
        bytes_of(&store, false, LATEST_EXPIRES_AT, "G:U"),
        Some(encode_i64(1_700_007_200).to_vec())
    );
    assert_eq!(
        counter(&store, false, CURRENT_MSG_KEY),
        7,
        "消息计数器取文件内最大虚拟 ID"
    );
}

#[test]
fn golden_sha256_matches_external_digest() {
    // 钉死字节契约：`end.sha256` 是 `end` 行之前全部原始字节的 SHA-256。
    let body = GOLDEN
        .split_once(r#"{"kind":"end""#)
        .expect("golden 含 end 行")
        .0;
    assert_eq!(
        sha256_hex(body.as_bytes()),
        "ab1026eb6627a24b79abfd43e6e87fcf019ca45bafcb147147c7df18fb588eb6",
        "golden 的 sha256 由 Windows CNG 独立算出，改字节就必须改 end 行（len={}）",
        body.len()
    );
}

#[test]
fn repeat_import_is_idempotent() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());
    let first = import(&store, GOLDEN, &ImportOptions::default()).expect("首次导入");
    let second = import(&store, GOLDEN, &merge()).expect("重复导入应成功");

    assert_eq!(first.identities_imported, 3);
    assert_eq!(second.identities_imported, 0, "同 key 同 vUIN 不再是新写入");
    assert_eq!(second.identities_unchanged, 3);
    assert_eq!(second.identities_remapped, 0);
    assert_eq!(second.vuin_counter, first.vuin_counter);
    assert_eq!(
        identity_copy(&store, "raw:alice"),
        Some(9_007_199_254_740_993)
    );
    assert_eq!(
        bytes_of(&store, true, LAST_SEEN, "default:1"),
        Some(OPENID.as_bytes().to_vec())
    );
}

#[test]
fn same_key_different_vuin_fails_by_default() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());
    import(
        &store,
        &build(
            &[r#"{"kind":"identity","key":"raw:alice","raw":"alice","vuin":10}"#],
            None,
        ),
        &ImportOptions::default(),
    )
    .expect("首次导入");

    let conflict = build(
        &[r#"{"kind":"identity","key":"raw:alice","raw":"alice","vuin":20}"#],
        None,
    );
    match import(&store, &conflict, &merge()) {
        Err(ImportError::Conflict(detail)) => {
            assert!(detail.contains("raw:alice"), "冲突信息应点名身份: {detail}");
            assert!(detail.contains("10") && detail.contains("20"));
        }
        other => panic!("默认策略应报 Conflict，实际 {other:?}"),
    }
    assert_eq!(identity_copy(&store, "raw:alice"), Some(10), "冲突时不写库");
}

#[test]
fn remap_assigns_new_vuin_and_keeps_old_owner() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());
    import(
        &store,
        &build(
            &[r#"{"kind":"identity","key":"raw:alice","raw":"alice","vuin":10}"#],
            None,
        ),
        &ImportOptions::default(),
    )
    .expect("首次导入");

    let conflict = build(
        &[r#"{"kind":"identity","key":"raw:alice","raw":"alice","vuin":20}"#],
        None,
    );
    let report = import(&store, &conflict, &remap()).expect("remap 应成功");
    assert_eq!(report.identities_remapped, 1);
    assert_eq!(report.identities_unchanged, 0);
    assert_eq!(
        report.vuin_counter, 21,
        "counter 模式：max(库内 10, 文件 20) + 1"
    );
    assert_eq!(identity_copy(&store, "raw:alice"), Some(21));
    assert_eq!(
        bytes_of(&store, true, VUIN_TO_IDENTITY, "10:raw:alice"),
        Some(b"alice".to_vec()),
        "旧 vUIN 的归属必须保留"
    );
    assert_eq!(
        bytes_of(&store, true, VUIN_TO_IDENTITY, "21:raw:alice"),
        Some(b"alice".to_vec())
    );
}

#[test]
fn vuin_owned_by_another_identity_conflicts_then_remaps() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());
    import(
        &store,
        &build(
            &[r#"{"kind":"identity","key":"raw:alice","raw":"alice","vuin":10}"#],
            None,
        ),
        &ImportOptions::default(),
    )
    .expect("首次导入");

    let taken = build(
        &[r#"{"kind":"identity","key":"raw:bob","raw":"bob","vuin":10}"#],
        None,
    );
    assert!(
        matches!(
            import(&store, &taken, &merge()),
            Err(ImportError::Conflict(_))
        ),
        "vUIN 已被别的 key 占用必须报冲突"
    );
    assert_eq!(identity_copy(&store, "raw:bob"), None, "冲突时不写库");

    let report = import(&store, &taken, &remap()).expect("remap 应成功");
    assert_eq!(report.identities_remapped, 1);
    assert_eq!(identity_copy(&store, "raw:bob"), Some(11));
    assert_eq!(identity_copy(&store, "raw:alice"), Some(10), "原归属不变");
}

#[test]
fn bad_sha256_is_a_format_error() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());
    let mut broken = build(
        &[r#"{"kind":"identity","key":"raw:alice","raw":"alice","vuin":1}"#],
        None,
    );
    let position = broken.rfind("\"sha256\":\"").expect("含 sha256") + 10;
    let digit = broken[position..position + 1].to_owned();
    let flipped = if digit == "0" { "1" } else { "0" };
    broken.replace_range(position..position + 1, flipped);

    match import(&store, &broken, &ImportOptions::default()) {
        Err(ImportError::Format(detail)) => assert!(detail.contains("sha256"), "{detail}"),
        other => panic!("sha256 不符应为格式错，实际 {other:?}"),
    }
    assert!(store.is_empty().expect("判空"), "校验失败不得写库");
}

#[test]
fn wrong_counts_and_meta_are_format_errors() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());

    let counts = build(
        &[r#"{"kind":"identity","key":"raw:alice","raw":"alice","vuin":1}"#],
        Some(99),
    );
    assert!(matches!(
        import(&store, &counts, &ImportOptions::default()),
        Err(ImportError::Format(detail)) if detail.contains("end.lines")
    ));

    let wrong_version = concat!(
        r#"{"kind":"meta","format":"gensokyo-idmap","version":2}"#,
        "\n",
        r#"{"kind":"end","lines":1,"sha256":"00"}"#,
        "\n"
    );
    assert!(matches!(
        import(&store, wrong_version, &ImportOptions::default()),
        Err(ImportError::Format(detail)) if detail.contains("版本")
    ));

    let no_end = concat!(
        r#"{"kind":"meta","format":"gensokyo-idmap","version":1}"#,
        "\n"
    );
    assert!(matches!(
        import(&store, no_end, &ImportOptions::default()),
        Err(ImportError::Format(detail)) if detail.contains("end")
    ));
}

#[test]
fn integers_are_never_floatified() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());
    let big = build(
        &[
            r#"{"kind":"identity","key":"raw:big","raw":"big","vuin":9007199254740993}"#,
            r#"{"kind":"vuin_counter","counter":9223372036854775807}"#,
        ],
        None,
    );
    let report =
        import(&store, &big, &ImportOptions::default()).expect("2^53 以上的整数应原样保留");
    assert_eq!(report.vuin_counter, i64::MAX);
    assert_eq!(
        identity_copy(&store, "raw:big"),
        Some(9_007_199_254_740_993)
    );

    // 浮点写法必须被拒绝，不能悄悄截断成整数
    let float = build(&[r#"{"kind":"vuin_counter","counter":1.5}"#], None);
    assert!(matches!(
        import(&store, &float, &merge()),
        Err(ImportError::Format(detail)) if detail.contains("记录")
    ));
}

#[test]
fn dry_run_leaves_the_database_untouched() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());
    let options = ImportOptions {
        dry_run: true,
        ..ImportOptions::default()
    };
    let report = import(&store, GOLDEN, &options).expect("dry-run 应通过校验");
    assert!(report.dry_run);
    assert_eq!(report.identities_imported, 3, "dry-run 仍要报出计划");
    assert!(store.is_empty().expect("判空"), "dry-run 不得写库");

    // 非空库上的 dry-run 同样不得改动任何已有值
    import(&store, GOLDEN, &merge()).expect("导入");
    let before = identity_copy(&store, "raw:alice");
    let conflict = build(
        &[r#"{"kind":"identity","key":"raw:alice","raw":"alice","vuin":20}"#],
        None,
    );
    let options = ImportOptions {
        dry_run: true,
        ..remap()
    };
    import(&store, &conflict, &options).expect("dry-run remap 不写库");
    assert_eq!(identity_copy(&store, "raw:alice"), before);
}

#[test]
fn non_empty_database_needs_merge() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());
    import(&store, GOLDEN, &ImportOptions::default()).expect("首次导入");
    match import(&store, GOLDEN, &ImportOptions::default()) {
        Err(ImportError::NotEmpty(_)) => {}
        other => panic!("库非空且无 --merge 应报 NotEmpty，实际 {other:?}"),
    }
}

#[test]
fn messages_are_untouched_when_file_has_none() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());
    import(
        &store,
        &build(
            &[r#"{"kind":"identity","key":"raw:alice","raw":"alice","vuin":1}"#],
            None,
        ),
        &ImportOptions::default(),
    )
    .expect("导入");
    assert_eq!(
        bytes_of(&store, false, META, CURRENT_MSG_KEY),
        None,
        "文件没提消息库时连 current_msg 都不写"
    );
    assert_eq!(bytes_of(&store, false, MSG_TO_VIRTUAL, "MSG"), None);

    // 只有计数器、没有消息行时也要写 current_msg（导出空消息库的情形）
    let counted = build(&[r#"{"kind":"msg_counter","counter":500}"#], None);
    let report = import(&store, &counted, &merge()).expect("导入计数器");
    assert_eq!(report.msg_counter, 500);
    assert_eq!(counter(&store, false, CURRENT_MSG_KEY), 500);
    assert_eq!(bytes_of(&store, false, MSG_TO_VIRTUAL, "MSG"), None);
}

#[test]
fn hash_id_remap_uses_md5_and_skips_imported_vuins() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open_hash(dir.path());
    let derived = generate_row_id("alice", 9).expect("MD5 派生");

    // 先用导入把一个与 MD5 派生值无关的 vUIN 占住，再制造冲突：新值应是 9 位派生值。
    import(
        &store,
        &build(
            &[r#"{"kind":"identity","key":"raw:alice","raw":"alice","vuin":10}"#],
            None,
        ),
        &ImportOptions::default(),
    )
    .expect("首次导入");

    let conflict = build(
        &[r#"{"kind":"identity","key":"raw:alice","raw":"alice","vuin":20}"#],
        None,
    );
    let report = import(&store, &conflict, &remap()).expect("hash_id remap 应成功");
    assert_eq!(report.identities_remapped, 1);
    assert_eq!(identity_copy(&store, "raw:alice"), Some(derived));
    assert_ne!(derived, 10, "必须避开已占用的 vUIN");
    assert_eq!(report.vuin_counter, derived.max(20), "计数器只增不减");
    assert_eq!(
        bytes_of(&store, true, VUIN_TO_IDENTITY, "10:raw:alice"),
        Some(b"alice".to_vec()),
        "旧 vUIN 归属保留"
    );
}

#[test]
fn meta_allocator_overrides_local_hash_id_setting() {
    // `meta.allocator` 是导出侧的真相：本机配置是 counter（默认），文件声明 hash 时
    // 新 vUIN 仍必须走 MD5 派生，否则与库里已有 vUIN 的形态不一致。
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());
    let derived = generate_row_id("bob", 9).expect("MD5 派生");

    let first = with_allocator(
        &build(
            &[r#"{"kind":"identity","key":"raw:alice","raw":"alice","vuin":10}"#],
            None,
        ),
        "hash",
    );
    import(&store, &first, &ImportOptions::default()).expect("首次导入");

    let takeover = with_allocator(
        &build(
            &[r#"{"kind":"identity","key":"raw:bob","raw":"bob","vuin":10}"#],
            None,
        ),
        "hash",
    );
    let report = import(&store, &takeover, &remap()).expect("remap 应成功");
    assert_eq!(report.identities_remapped, 1);
    assert_eq!(identity_copy(&store, "raw:bob"), Some(derived));
    assert_eq!(identity_copy(&store, "raw:alice"), Some(10));
}

#[test]
fn parse_rejects_blank_lines_and_duplicate_identity_keys() {
    let mut blank = String::from(r#"{"kind":"meta","format":"gensokyo-idmap","version":1}"#);
    blank.push_str("\n\n");
    blank.push_str(&format!(
        "{{\"kind\":\"end\",\"lines\":1,\"sha256\":\"{}\"}}\n",
        sha256_hex(blank.as_bytes())
    ));
    assert!(matches!(
        jsonl::parse(blank.as_bytes()),
        Err(ImportError::Format(detail)) if detail.contains("空行")
    ));

    let duplicated = build(
        &[
            r#"{"kind":"identity","key":"raw:alice","raw":"alice","vuin":1}"#,
            r#"{"kind":"identity","key":"raw:alice","raw":"alice","vuin":2}"#,
        ],
        None,
    );
    assert!(matches!(
        jsonl::parse(duplicated.as_bytes()),
        Err(ImportError::Format(detail)) if detail.contains("重复")
    ));

    // 文件里同一 vUIN 挂两个身份也是冲突（默认策略拒绝）
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());
    let shared = build(
        &[
            r#"{"kind":"identity","key":"raw:alice","raw":"alice","vuin":7}"#,
            r#"{"kind":"identity","key":"raw:bob","raw":"bob","vuin":7}"#,
        ],
        None,
    );
    assert!(matches!(
        import(&store, &shared, &ImportOptions::default()),
        Err(ImportError::Conflict(_))
    ));
    let report = import(&store, &shared, &remap()).expect("remap 后两个身份各有 vUIN");
    assert_eq!(report.identities_imported, 1);
    assert_eq!(report.identities_remapped, 1);
    assert_eq!(identity_copy(&store, "raw:alice"), Some(7));
    assert_eq!(identity_copy(&store, "raw:bob"), Some(8));
}

#[test]
fn parse_exposes_typed_records() {
    let data: ImportData = jsonl::parse(build(&[], None).as_bytes()).expect("最小文件");
    assert_eq!(data.records, 1, "只有 meta");
    assert!(data.identities.is_empty());
}
