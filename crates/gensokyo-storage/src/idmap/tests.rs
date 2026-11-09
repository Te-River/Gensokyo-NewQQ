//! idmap 本地存储单测：往返、NotFound、TTL、并发与重开。

use std::path::Path;
use std::sync::{Arc, Barrier};
use std::time::Duration;

use super::*;
use encoding::{decode_i64, encode_i64, normalize_identity, vuin_identity_key};

fn options(dir: &Path) -> IdMapOptions {
    IdMapOptions::new(dir.join("openid-map.redb"), dir.join("msgid-map.redb")).with_app_id("10001")
}

fn open(dir: &Path) -> IdMapStore {
    IdMapStore::open(options(dir)).expect("打开 idmap 存储失败")
}

/// 直接写反表（模拟 Go 迁移遗留：同一 vUIN 下挂多个身份 key）。
fn write_reverse_entry(store: &IdMapStore, vuin: i64, identity_key: &str, raw: &str) {
    let txn = store.identity_db.begin_write().expect("写事务");
    {
        let mut reverse = txn.open_table(VUIN_TO_IDENTITY).expect("反表");
        reverse
            .insert(
                vuin_identity_key(vuin, identity_key).as_str(),
                raw.as_bytes(),
            )
            .expect("写反表");
    }
    txn.commit().expect("提交");
    // 绕过 store 的写路径，读缓存必须整片失效（导入器同口径）
    store.invalidate_lookup_cache();
}

/// 直接写 `last_seen`（公开 API 只在分配/命中时更新它）。
fn write_last_seen(store: &IdMapStore, vuin: i64, raw: &str) {
    let txn = store.identity_db.begin_write().expect("写事务");
    touch_last_seen_tx(&txn, vuin, raw).expect("写 last_seen");
    txn.commit().expect("提交");
    store.invalidate_lookup_cache();
}

#[test]
fn default_paths_match_documented_names() {
    let defaults = IdMapOptions::default();
    assert_eq!(defaults.identity_path, Path::new("openid-map.redb"));
    assert_eq!(defaults.msg_path, Path::new("msgid-map.redb"));
    assert_eq!(defaults.app_id, UNSET_APP_ID, "默认 AppID 是显式占位");
    assert_eq!(defaults.msg_ttl, Duration::from_secs(3600));
    assert_eq!(defaults.user_name_ttl, Duration::from_secs(600));
}

#[test]
fn go_byte_encoding() {
    assert_eq!(encode_i64(1), [0, 0, 0, 0, 0, 0, 0, 1]);
    assert_eq!(decode_i64(&[0, 0, 0, 0, 0, 0, 2, 88]), Some(600));
    assert_eq!(decode_i64(&[1, 2, 3]), None);
    assert_eq!(vuin_identity_key(7, "raw:x"), "7:raw:x");
    assert_eq!(vuin_prefix(7), "7:");
}

#[test]
fn identity_normalization_matches_go() {
    let openid = "A".repeat(32);
    let normalized = normalize_identity(&openid, "10001", None);
    assert_eq!(normalized.key, format!("openid:QQ:10001:{openid}"));
    assert_eq!(normalized.raw, openid);

    let raw = normalize_identity("alice", "10001", None);
    assert_eq!(raw.key, "raw:alice");
    assert_eq!(raw.raw, "alice");

    // rUIN：base32("1") == "GE"，解码后回填数字 id
    let ruin = normalize_identity("rUIN-QQ-0-GE", "10001", None);
    assert_eq!(ruin.key, "ruin:QQ:0:1");
    assert_eq!(ruin.raw, "rUIN-QQ-0-1");

    // 平台非 QQ 时不解码
    let other = normalize_identity("rUIN-qq-0-GE", "10001", None);
    assert_eq!(other.key, "ruin:qq:0:GE");

    // uin 隔离前缀在归一化前剥离
    let isolated = normalize_identity("12345:alice", "10001", Some("12345"));
    assert_eq!(isolated.key, "raw:alice");
    assert_eq!(isolated.raw, "alice");
}

#[test]
fn identity_roundtrip() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());
    let openid = "B".repeat(32);

    let (raw, vuin) = store.retrieve_virtual_value(&openid).expect("分配失败");
    assert_eq!(raw, openid, "返回值第一项是归一化后的原始身份");
    assert_eq!(vuin, 1);

    let (raw_again, vuin_again) = store.retrieve_virtual_value(&openid).expect("查询失败");
    assert_eq!(
        (raw_again, vuin_again),
        (openid.clone(), 1),
        "同一身份不重复分配"
    );
    assert_eq!(store.retrieve_row_by_id(1).expect("反查失败"), openid);
    assert_eq!(store.last_seen(1).expect("last_seen 失败"), Some(openid));
}

#[test]
fn unknown_keys_are_not_found() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());

    assert!(matches!(
        store.retrieve_row_by_id(999),
        Err(StorageError::NotFound(_))
    ));
    assert!(matches!(
        store.retrieve_row_by_cache(999),
        Err(StorageError::NotFound(_))
    ));
    assert_eq!(store.last_seen(999).expect("last_seen 失败"), None);
    assert_eq!(store.get_user_name(999), None);
}

#[test]
fn msg_cache_roundtrip_and_reuse() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());

    assert_eq!(store.store_cache("msg-1").expect("写入失败"), 1);
    assert_eq!(store.store_cache("msg-1").expect("复用失败"), 1);
    assert_eq!(store.store_cache("msg-2").expect("写入失败"), 2);
    assert_eq!(store.retrieve_row_by_cache(1).expect("反查失败"), "msg-1");
    assert_eq!(store.retrieve_row_by_cache(2).expect("反查失败"), "msg-2");
    assert!(matches!(
        store.store_cache(""),
        Err(StorageError::Corrupt(_))
    ));
}

#[test]
fn msg_cache_ttl_expiry() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());

    let vuin = store.store_cache("expired").expect("写入失败");
    // 手改过期时间到过去：公开 API 在 TTL ≤ 0 时会回退 1 小时，造不出已过期项
    let txn = store.msg_db.begin_write().expect("写事务");
    {
        let mut expirations = txn.open_table(EXPIRES_AT).expect("过期表");
        expirations
            .insert(
                vuin.to_string().as_str(),
                encode_i64(unix_now() - 1).as_slice(),
            )
            .expect("写过期时间");
    }
    txn.commit().expect("提交");

    let err = store.retrieve_row_by_cache(vuin).expect_err("应已过期");
    assert!(matches!(err, StorageError::Expired(_)), "实际 {err:?}");
    assert!(err.is_not_found(), "过期与缺失对调用方同为 NotFound 语义");
}

#[test]
fn msg_cache_non_positive_ttl_falls_back_to_default() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());

    // Go StoreMsgID：now + ttl 不晚于 now 时回退 3600 秒；亚秒 TTL 截断后同样回退
    for (msg_id, ttl) in [
        ("ttl-zero", Duration::ZERO),
        ("ttl-subsecond", Duration::from_millis(500)),
    ] {
        let vuin = store.store_cache_with_ttl(msg_id, ttl).expect("写入失败");
        assert_eq!(
            store
                .retrieve_row_by_cache(vuin)
                .expect("回退 1 小时后应可查"),
            msg_id
        );
    }
}

#[test]
fn latest_msg_id_roundtrip_and_expiry() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());

    // Go 对空参数静默返回；从未记录的 key 是 NotFound（不是 Expired）
    store
        .store_latest_msg_id("", "u", "m")
        .expect("空参数应忽略");
    assert!(matches!(
        store.get_latest_msg_id("", "u"),
        Err(StorageError::NotFound(_))
    ));

    store.store_latest_msg_id("g", "u", "m").expect("写入失败");
    assert_eq!(store.get_latest_msg_id("g", "u").expect("查询失败"), "m");
    drop(store);

    let expiring =
        IdMapStore::open(options(dir.path()).with_msg_ttl(Duration::ZERO)).expect("打开失败");
    expiring
        .store_latest_msg_id("g2", "u", "m")
        .expect("写入失败");
    assert!(matches!(
        expiring.get_latest_msg_id("g2", "u"),
        Err(StorageError::Expired(_))
    ));
}

#[test]
fn user_name_cache_expires() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = IdMapStore::open(options(dir.path()).with_user_name_ttl(Duration::from_millis(50)))
        .expect("打开失败");

    store.store_user_name(1, "alice");
    assert_eq!(store.get_user_name(1), Some("alice".to_owned()));
    assert_eq!(store.get_user_name(2), None);

    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(store.get_user_name(1), None, "超过 TTL 后必须失效");
}

#[test]
fn concurrent_allocation_is_unique_and_contiguous() {
    const THREADS: usize = 8;
    const PER_THREAD: usize = 100;

    let dir = tempfile::tempdir().expect("临时目录");
    let store = Arc::new(open(dir.path()));
    let barrier = Arc::new(Barrier::new(THREADS));

    let handles: Vec<_> = (0..THREADS)
        .map(|thread| {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                (0..PER_THREAD)
                    .map(|index| {
                        store
                            .retrieve_virtual_value(&format!("user-{thread}-{index}"))
                            .map(|(_, vuin)| vuin)
                            .expect("并发分配失败")
                    })
                    .collect::<Vec<i64>>()
            })
        })
        .collect();

    let mut allocated: Vec<i64> = handles
        .into_iter()
        .flat_map(|handle| handle.join().expect("线程 panic"))
        .collect();
    allocated.sort_unstable();
    assert_eq!(
        allocated,
        (1..=i64::try_from(THREADS * PER_THREAD).expect("数量")).collect::<Vec<i64>>(),
        "并发分配必须唯一且连续"
    );
    // 并发下 vUIN 1 属于哪个线程不确定，只断言反查能读回某个已分配身份
    let first = store.retrieve_row_by_id(1).expect("反查失败");
    assert!(first.starts_with("user-"), "反表必须可反查，实际 {first}");
}

#[test]
fn reopen_preserves_data() {
    let dir = tempfile::tempdir().expect("临时目录");
    {
        let store = open(dir.path());
        for index in 0..3 {
            store
                .retrieve_virtual_value(&format!("user-{index}"))
                .expect("分配失败");
        }
        assert_eq!(store.store_cache("msg-1").expect("写入失败"), 1);
    }

    let reopened = open(dir.path());
    assert_eq!(reopened.retrieve_row_by_id(1).expect("反查失败"), "user-0");
    assert_eq!(reopened.retrieve_row_by_id(3).expect("反查失败"), "user-2");
    assert_eq!(
        reopened.retrieve_row_by_cache(1).expect("反查失败"),
        "msg-1"
    );
    let (_, next) = reopened
        .retrieve_virtual_value("user-new")
        .expect("分配失败");
    assert_eq!(next, 4, "重开后计数器必须延续");
    assert_eq!(reopened.store_cache("msg-2").expect("写入失败"), 2);
}

#[test]
fn same_path_is_rejected() {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = dir.path().join("idmap.redb");
    let err = IdMapStore::open(IdMapOptions::new(path.clone(), path).with_app_id("1"))
        .expect_err("同一文件必须拒绝");
    assert!(matches!(err, StorageError::Corrupt(_)), "实际 {err:?}");
}

#[test]
fn empty_app_id_is_rejected() {
    let dir = tempfile::tempdir().expect("临时目录");
    let err = IdMapStore::open(IdMapOptions::new(
        dir.path().join("openid-map.redb"),
        dir.path().join("msgid-map.redb"),
    ))
    .expect_err("空 AppID 必须拒绝");
    assert!(matches!(err, StorageError::Corrupt(_)), "实际 {err:?}");
    assert!(
        err.to_string().contains("app_id"),
        "文案需点名 app_id，实际 {err}"
    );
    assert!(
        !dir.path().join("openid-map.redb").exists(),
        "拒绝必须发生在建库之前"
    );
}

#[test]
fn base32_validation_matches_go() {
    // 长度非法（len%8 == 3）：Go 解码器直接报错，这里保留原始 id
    let bad_len = normalize_identity("rUIN-QQ-0-GEA", "10001", None);
    assert_eq!(bad_len.key, "ruin:QQ:0:GEA");
    assert_eq!(bad_len.raw, "rUIN-QQ-0-GEA");

    // 尾随比特非 0（"GZ" → "6" + 残位 01）：Go 拒绝，这里保留原始 id
    let bad_tail = normalize_identity("rUIN-QQ-0-GZ", "10001", None);
    assert_eq!(bad_tail.key, "ruin:QQ:0:GZ");
    assert_eq!(bad_tail.raw, "rUIN-QQ-0-GZ");

    // 合法：base32("1") == "GE"（无残位），解码成数字 id
    let ok = normalize_identity("rUIN-QQ-0-GE", "10001", None);
    assert_eq!(ok.key, "ruin:QQ:0:1");
    assert_eq!(ok.raw, "rUIN-QQ-0-1");
}

#[test]
fn retrieve_row_by_id_prefers_last_seen() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());
    let openid = "C".repeat(32);
    let vuin = store.retrieve_virtual_value("alice").expect("分配失败").1;

    // 同一 vUIN 反表下再挂一个 openid 身份（Go 迁移遗留形态）
    write_reverse_entry(&store, vuin, &format!("openid:QQ:10001:{openid}"), &openid);

    // last_seen 指向 alice 且正表仍映射到该 vUIN → 优先 alice，而不是反表里的 openid
    assert_eq!(store.retrieve_row_by_id(vuin).expect("反查失败"), "alice");

    // last_seen 指向映射到别的 vUIN 的身份 → 校验失败，回退扫描反表命中 openid
    let other = store.retrieve_virtual_value("bob").expect("分配失败").1;
    assert_ne!(other, vuin);
    write_last_seen(&store, vuin, "bob");
    assert_eq!(store.retrieve_row_by_id(vuin).expect("反查失败"), openid);
}

#[test]
fn schema_version_is_rewritten_on_open() {
    let dir = tempfile::tempdir().expect("临时目录");
    {
        let store = open(dir.path());
        let txn = store.identity_db.begin_write().expect("写事务");
        {
            let mut meta = txn.open_table(META).expect("meta 表");
            meta.insert(SCHEMA_VERSION_KEY, b"1".as_slice())
                .expect("写未知版本");
        }
        txn.commit().expect("提交");
    }

    // Go initNewDBs 每次打开都无条件写 "2"
    let reopened = open(dir.path());
    let txn = reopened.identity_db.begin_read().expect("读事务");
    let meta = txn.open_table(META).expect("meta 表");
    let value = meta
        .get(SCHEMA_VERSION_KEY)
        .expect("读版本")
        .expect("版本必须存在");
    assert_eq!(value.value(), SCHEMA_VERSION, "未知版本必须被覆盖");
}

#[test]
fn storage_error_messages_are_readable() {
    let not_found: gensokyo_core::Error = StorageError::NotFound("vUIN=9".to_owned()).into();
    assert_eq!(not_found.to_string(), "存储错误: 键不存在: vUIN=9");
    let expired: gensokyo_core::Error = StorageError::Expired("虚拟消息 ID=9".to_owned()).into();
    assert_eq!(expired.to_string(), "存储错误: 键已过期: 虚拟消息 ID=9");
}

#[test]
fn find_config_sections_filters_value_sorts_and_dedups() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());

    assert_eq!(
        store
            .find_config_sections("type", "group")
            .expect("空库枚举应成功"),
        Vec::<String>::new(),
        "空库回空列表（Go 缺桶时回错，这里表恒存在）"
    );

    // 乱序写入：group 两个、user 一个、subtype 不同的一个
    store.write_config("g-3", "type", "group").expect("写配置");
    store.write_config("g-1", "type", "group").expect("写配置");
    store.write_config("g-2", "type", "user").expect("写配置");
    store.write_config("g-4", "memo", "group").expect("写配置");
    // 同键覆盖写（去重语义：一个 section 只出现一次）
    store
        .write_config("g-3", "type", "group")
        .expect("重复写配置");

    assert_eq!(
        store.find_config_sections("type", "group").expect("枚举"),
        ["g-1".to_owned(), "g-3".to_owned()],
        "同值多 section 全回，且结果排序"
    );
}

#[test]
fn find_config_sections_strips_suffix_not_first_colon() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());

    // section 含冒号时剥后缀得到完整 section（Go 取第一个冒号前段，OpenID 形态不受影响）
    store
        .write_config("outer:inner", "type", "group")
        .expect("写配置");
    store
        .write_config("outer", "type", "group")
        .expect("写配置");

    assert_eq!(
        store.find_config_sections("type", "group").expect("枚举"),
        ["outer".to_owned(), "outer:inner".to_owned()],
    );
}

#[test]
fn list_identities_enumerates_forward_table_in_key_order() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());

    assert!(
        store.list_identities().expect("空库枚举").is_empty(),
        "空库回空列表"
    );

    let openid_a = "A".repeat(32);
    let openid_b = "B".repeat(32);
    // 逆序写入：枚举结果按正表 key 升序（Go bbolt `ForEach` 的字节序），与 vUIN 无关
    let (_, row_b) = store.retrieve_virtual_value(&openid_b).expect("写 B");
    let (_, row_a) = store.retrieve_virtual_value(&openid_a).expect("写 A");
    store.store_user_name(row_b, "bee");

    assert_eq!(
        store.list_identities().expect("枚举"),
        vec![
            IdentityMapping {
                real_id: openid_a,
                virtual_id: row_a,
                username: String::new(),
            },
            IdentityMapping {
                real_id: openid_b,
                virtual_id: row_b,
                username: "bee".to_owned(),
            },
        ]
    );
}

#[test]
fn force_unbind_deletes_forward_and_reverse_rows() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());
    let openid = "C".repeat(32);
    let (_, row) = store.retrieve_virtual_value(&openid).expect("写身份");
    // 迁移遗留：同一 vUIN 下再挂一条指向同一身份的反表条目
    write_reverse_entry(&store, row, "openid:QQ:other", &openid);

    assert_eq!(
        store.force_unbind(&openid).expect("按身份解绑"),
        2,
        "反表两条都要删掉（Go 只计数反表）"
    );
    assert!(
        store
            .lookup_virtual_value(&openid)
            .expect_err("正表已删")
            .is_not_found()
    );
    assert!(
        store
            .retrieve_row_by_id(row)
            .expect_err("反表已删")
            .is_not_found()
    );
    assert!(store.list_identities().expect("枚举").is_empty());
}

#[test]
fn force_unbind_accepts_vuin_input() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());
    let openid = "D".repeat(32);
    let (_, row) = store.retrieve_virtual_value(&openid).expect("写身份");

    assert_eq!(
        store.force_unbind(&row.to_string()).expect("按 vUIN 解绑"),
        1
    );
    assert!(
        store
            .lookup_virtual_value(&openid)
            .expect_err("正表已删")
            .is_not_found()
    );
}

#[test]
fn force_unbind_reports_go_argument_errors() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());

    assert!(
        matches!(store.force_unbind(""), Err(StorageError::Corrupt(_))),
        "空 id 与 Go 一样报错"
    );
    let missing = store.force_unbind("424242").expect_err("未知 vUIN");
    assert!(missing.is_not_found(), "{missing}");
    assert!(missing.to_string().contains("424242"), "{missing}");
    // 不存在的身份串：Go 删不到也算成功（幂等）
    assert_eq!(store.force_unbind("nobody").expect("幂等解绑"), 0);
}

// ---- P35：批量写 + 读路径热查询缓存 ----

/// 带缓存配置打开（容量 0 = 关闭）。
fn open_with_cache(dir: &Path, capacity: usize, ttl: Duration) -> IdMapStore {
    IdMapStore::open(
        options(dir)
            .with_lookup_cache_capacity(capacity)
            .with_lookup_cache_ttl(ttl),
    )
    .expect("打开 idmap 存储失败")
}

/// 绕过 store 直接把某个身份的正表值改成 `vuin`（模拟外部/旧版本写库）。
fn overwrite_vuin_row(store: &IdMapStore, openid: &str, vuin: i64) {
    let key = normalize_identity(openid, "10001", None).key;
    let txn = store.identity_db.begin_write().expect("写事务");
    {
        let mut forward = txn.open_table(IDENTITY_TO_VUIN).expect("正表");
        forward
            .insert(key.as_str(), encode_i64(vuin).as_slice())
            .expect("改库");
    }
    txn.commit().expect("提交");
}

#[test]
fn write_batch_commits_every_key_and_pairs_with_reads() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());
    let openid = "E".repeat(32);

    let (raw, vuin, msg_a, msg_b) = store
        .with_write_batch(|batch| {
            let (raw, vuin) = batch.retrieve_virtual_value(&openid)?;
            let msg_a = batch.store_cache("MSG-A")?;
            let msg_b = batch.store_cache("MSG-B")?;
            batch.write_config(&vuin.to_string(), "type", "group")?;
            Ok((raw, vuin, msg_a, msg_b))
        })
        .expect("批量提交");

    assert_eq!(raw, openid);
    assert_eq!(vuin, 1, "顺序模式首个身份是 1");
    assert_ne!(msg_a, msg_b, "同一批内的两次分配不能重号");
    // 写后立即读：非批量读路径（含缓存）看到的就是本批提交的值
    assert_eq!(store.lookup_virtual_value(&openid).expect("查身份"), vuin);
    assert_eq!(store.retrieve_row_by_id(vuin).expect("反查身份"), openid);
    assert_eq!(
        store.retrieve_row_by_cache(msg_a).expect("反查消息"),
        "MSG-A"
    );
    assert_eq!(
        store
            .read_config(&vuin.to_string(), "type")
            .expect("读配置"),
        "group"
    );
}

#[test]
fn write_batch_rolls_back_when_the_closure_fails() {
    let dir = tempfile::tempdir().expect("临时目录");
    let openid = "F".repeat(32);
    {
        let store = open(dir.path());
        let result: Result<(), StorageError> = store.with_write_batch(|batch| {
            batch.retrieve_virtual_value(&openid)?;
            batch.store_cache("MSG-X")?;
            Err(StorageError::Corrupt("闭包报错".to_owned()))
        });
        assert!(
            matches!(result, Err(StorageError::Corrupt(_))),
            "闭包错误原样上抛"
        );
        // 未提交事务整批丢弃：同实例读不到（读走库，不是缓存）
        assert!(
            store
                .lookup_virtual_value(&openid)
                .expect_err("身份不应存在")
                .is_not_found()
        );
    }
    // 重开：磁盘上同样没有任何批内写入
    let reopened = open(dir.path());
    assert!(
        reopened
            .lookup_virtual_value(&openid)
            .expect_err("重开后仍不应存在")
            .is_not_found()
    );
    assert!(
        reopened
            .retrieve_row_by_cache(1)
            .expect_err("消息 ID 也不应存在")
            .is_not_found()
    );
}

#[test]
fn lookup_cache_serves_reads_and_a_write_invalidates_it() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());
    let openid = "G".repeat(32);

    // 负结果先进缓存，随后分配必须让它失效（写后立即可读）
    assert!(
        store
            .lookup_virtual_value(&openid)
            .expect_err("未登记")
            .is_not_found()
    );
    let (_, vuin) = store.retrieve_virtual_value(&openid).expect("分配 vUIN");
    assert_eq!(
        store.lookup_virtual_value(&openid).expect("写后立即读"),
        vuin,
        "分配身份必须让负缓存失效"
    );

    // 绕过 store 直接改库：命中缓存 → 读到的仍是旧值（证明缓存确实在服务读路径）
    overwrite_vuin_row(&store, &openid, 4242);
    assert_eq!(
        store.lookup_virtual_value(&openid).expect("命中缓存"),
        vuin,
        "缓存命中：库里已被外部改成 4242"
    );
    // 导入器等直接写库的路径用整片失效入口
    store.invalidate_lookup_cache();
    assert_eq!(
        store.lookup_virtual_value(&openid).expect("失效后读库"),
        4242
    );
}

#[test]
fn row_cache_is_invalidated_by_force_unbind() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());
    let openid = "H".repeat(32);
    let (_, vuin) = store.retrieve_virtual_value(&openid).expect("分配 vUIN");

    // 先填充反查缓存，再解绑：解绑必须让它失效，否则会反查到已删除的身份
    assert_eq!(store.retrieve_row_by_id(vuin).expect("反查"), openid);
    store.force_unbind(&openid).expect("解绑");
    assert!(
        store
            .retrieve_row_by_id(vuin)
            .expect_err("已解绑")
            .is_not_found(),
        "解绑后反查缓存必须失效"
    );
}

#[test]
fn disabled_or_zero_ttl_cache_reads_the_database_every_time() {
    let dir = tempfile::tempdir().expect("临时目录");
    let openid = "I".repeat(32);

    for capacity in [0_usize, DEFAULT_LOOKUP_CACHE_CAPACITY] {
        let ttl = if capacity == 0 {
            DEFAULT_LOOKUP_CACHE_TTL
        } else {
            Duration::ZERO
        };
        let store = open_with_cache(dir.path(), capacity, ttl);
        let (_, vuin) = store.retrieve_virtual_value(&openid).expect("分配 vUIN");
        assert_eq!(store.lookup_virtual_value(&openid).expect("查"), vuin);
        // 关闭缓存（容量 0 或 TTL 0）时，外部写库立刻可见——与不缓存完全一致
        overwrite_vuin_row(&store, &openid, 4242);
        assert_eq!(
            store.lookup_virtual_value(&openid).expect("读库"),
            4242,
            "容量 {capacity} / TTL {ttl:?} 时必须直读库"
        );
        drop(store);
    }
}

#[test]
fn lookup_cache_evicts_the_least_recently_used_entry() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open_with_cache(dir.path(), 2, DEFAULT_LOOKUP_CACHE_TTL);
    let (a, b, c) = ("J".repeat(32), "K".repeat(32), "L".repeat(32));
    let mut vuins = Vec::new();
    for openid in [&a, &b, &c] {
        vuins.push(store.retrieve_virtual_value(openid).expect("分配").1);
        store.lookup_virtual_value(openid).expect("回填缓存");
    }

    // 容量 2：插入第三条时最久未用的 a 被淘汰；b 仍在缓存里 → 外部改库看不见
    overwrite_vuin_row(&store, &b, 4243);
    assert_eq!(
        store.lookup_virtual_value(&b).expect("命中缓存"),
        vuins[1],
        "未淘汰的项仍由缓存服务"
    );
    // a 已被淘汰（最后一次读它是在插入 b 之前）→ 外部改库立刻可见
    overwrite_vuin_row(&store, &a, 4242);
    assert_eq!(
        store.lookup_virtual_value(&a).expect("读库"),
        4242,
        "容量满后最久未用的一条应被淘汰"
    );
}

#[test]
fn zero_ttl_entry_expires_immediately() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = open_with_cache(dir.path(), 16, Duration::from_nanos(1));
    let openid = "M".repeat(32);
    let (_, vuin) = store.retrieve_virtual_value(&openid).expect("分配 vUIN");
    store.lookup_virtual_value(&openid).expect("回填缓存");

    overwrite_vuin_row(&store, &openid, 4242);
    assert_eq!(
        store.lookup_virtual_value(&openid).expect("读库"),
        4242,
        "1ns 的 TTL 过后续期读库，不会返回 {vuin} 这个旧值"
    );
}

#[test]
fn concurrent_reads_and_writes_stay_consistent_with_cache() {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = Arc::new(open(dir.path()));
    let barrier = Arc::new(Barrier::new(8));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            for i in 0..16 {
                let openid = format!("{}{i}", "N".repeat(24));
                let (raw, vuin) = store.retrieve_virtual_value(&openid).expect("分配 vUIN");
                assert_eq!(raw, openid);
                // 并发分配同一身份：所有线程必须拿到同一个 vUIN，且反查一致
                assert_eq!(store.lookup_virtual_value(&openid).expect("查身份"), vuin);
                assert_eq!(store.retrieve_row_by_id(vuin).expect("反查身份"), openid);
                assert_eq!(store.lookup_virtual_value(&openid).expect("再查"), vuin);
            }
        }));
    }
    for handle in handles {
        handle.join().expect("并发线程不得 panic");
    }
}

/// 性能探针（默认不跑）：`cargo test -p gensokyo-storage -- --ignored --nocapture perf_probe`。
///
/// 只打印数量级：200 个身份 × 20 轮「逐条提交」vs「单批提交」，以及 4000 次
/// `lookup_virtual_value` 在缓存开/关下的耗时。
#[test]
#[ignore = "手动性能探针，不进常规 suite"]
fn perf_probe_batch_and_cache() {
    use std::time::Instant;

    const IDENTITIES: usize = 200;
    const ROUNDS: usize = 20;

    let dir = tempfile::tempdir().expect("临时目录");
    let store = open(dir.path());

    let sequential = Instant::now();
    for _ in 0..ROUNDS {
        for i in 0..IDENTITIES {
            store
                .retrieve_virtual_value(&format!("batch-{i}"))
                .expect("逐条写");
        }
    }
    let sequential = sequential.elapsed();

    let batched = Instant::now();
    for _ in 0..ROUNDS {
        store
            .with_write_batch(|batch| {
                for i in 0..IDENTITIES {
                    batch.retrieve_virtual_value(&format!("batch-{i}"))?;
                }
                Ok(())
            })
            .expect("批量写");
    }
    let batched = batched.elapsed();

    let hits = Instant::now();
    for _ in 0..ROUNDS {
        for i in 0..IDENTITIES {
            store
                .lookup_virtual_value(&format!("batch-{i}"))
                .expect("缓存读");
        }
    }
    let hits = hits.elapsed();

    let dir_off = tempfile::tempdir().expect("临时目录");
    let cold = open_with_cache(dir_off.path(), 0, DEFAULT_LOOKUP_CACHE_TTL);
    for i in 0..IDENTITIES {
        cold.retrieve_virtual_value(&format!("cold-{i}"))
            .expect("准备");
    }
    let misses = Instant::now();
    for _ in 0..ROUNDS {
        for i in 0..IDENTITIES {
            cold.lookup_virtual_value(&format!("cold-{i}"))
                .expect("无缓存读");
        }
    }
    let misses = misses.elapsed();

    println!(
        "batch: {} 次写 / {} 轮：逐条提交 {:?}，单批提交 {:?}（{:.1}x）",
        IDENTITIES * ROUNDS,
        ROUNDS,
        sequential,
        batched,
        sequential.as_secs_f64() / batched.as_secs_f64()
    );
    println!(
        "cache: {} 次 lookup：命中 {:?}，无缓存 {:?}（{:.1}x）",
        IDENTITIES * ROUNDS,
        hits,
        misses,
        misses.as_secs_f64() / hits.as_secs_f64()
    );
}
