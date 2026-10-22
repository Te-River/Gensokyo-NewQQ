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
}

/// 直接写 `last_seen`（公开 API 只在分配/命中时更新它）。
fn write_last_seen(store: &IdMapStore, vuin: i64, raw: &str) {
    let txn = store.identity_db.begin_write().expect("写事务");
    touch_last_seen_tx(&txn, vuin, raw).expect("写 last_seen");
    txn.commit().expect("提交");
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
