//! echo 内存层单测：懒消息窗口、seq 并发、SSM 队列、REFIDX、行号缓存与清理。

use std::sync::Arc;
use std::time::Instant;

use super::*;

/// 冻结在 2023-11-14T22:13:20Z 的基准时间。
const BASE_SECS: u64 = 1_700_000_000;

fn base() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(BASE_SECS)
}

/// 可推进的测试时钟：不 `sleep` 也能验证窗口。
struct FrozenClock {
    now: Arc<Mutex<SystemTime>>,
}

impl FrozenClock {
    fn new() -> Self {
        Self {
            now: Arc::new(Mutex::new(base())),
        }
    }

    fn handle(&self) -> Clock {
        let now = Arc::clone(&self.now);
        Arc::new(move || *now.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn advance(&self, delta: Duration) {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner) += delta;
    }
}

fn frozen_store() -> (EchoStore, FrozenClock) {
    let clock = FrozenClock::new();
    let store = EchoStore::new().with_clock(clock.handle());
    (store, clock)
}

// ---- 懒消息 ----

#[test]
fn lazy_returns_newest_unused_record() {
    let (store, clock) = frozen_store();
    store.add_lazy_message_id("g1", "m-old", base());
    clock.advance(Duration::from_secs(60));
    store.add_lazy_message_id("g1", "m-new", base() + Duration::from_secs(60));

    assert_eq!(store.get_lazy_messages_id("g1"), "m-new");
}

#[test]
fn lazy_repeat_call_does_not_increment_usage() {
    let (store, clock) = frozen_store();
    store.add_lazy_message_id("g1", "m1", base());
    clock.advance(Duration::from_secs(10));
    store.add_lazy_message_id("g1", "m2", base() + Duration::from_secs(10));

    let first = store.get_lazy_messages_id("g1");
    let second = store.get_lazy_messages_id("g1");
    assert_eq!(first, "m2");
    assert_eq!(second, first, "同一回复链必须复用同一 msg_id（Issue #19）");

    let lazy = read_map(&store.inner.lazy);
    let records = lazy.get("g1").expect("懒消息 key 仍应存在");
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|record| record.usage_count == 0));
}

#[test]
fn lazy_expired_returns_default() {
    let (store, clock) = frozen_store();
    store.add_lazy_message_id("g1", "m1", base());
    clock.advance(DEFAULT_LAZY_WINDOW + Duration::from_secs(1));

    assert_eq!(store.get_lazy_messages_id("g1"), DEFAULT_MESSAGE_ID);

    let lazy = read_map(&store.inner.lazy);
    let records = lazy.get("g1").expect("Go 写回空切片，key 仍保留");
    assert!(records.is_empty(), "过期记录应被裁掉");
}

#[test]
fn lazy_prunes_expired_records() {
    let (store, clock) = frozen_store();
    store.add_lazy_message_id("g1", "m1", base());
    clock.advance(Duration::from_secs(180));
    store.add_lazy_message_id("g1", "m2", base() + Duration::from_secs(180));
    // m1 已经 270 秒（超过 240 秒窗口），m2 只有 90 秒。
    clock.advance(Duration::from_secs(90));

    assert_eq!(store.get_lazy_messages_id("g1"), "m2");

    let lazy = read_map(&store.inner.lazy);
    let records = lazy.get("g1").expect("懒消息 key 仍应存在");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].message_id, "m2");
}

#[test]
fn lazy_v2_key_is_group_dot_user() {
    let (store, _clock) = frozen_store();
    store.add_lazy_message_id_v2("g1", "u1", "m-v2", base());

    assert_eq!(store.get_lazy_messages_id_v2("g1", "u1"), "m-v2");
    assert_eq!(
        store.get_lazy_messages_id("g1"),
        DEFAULT_MESSAGE_ID,
        "v2 记录不参与群维度查询"
    );

    let lazy = read_map(&store.inner.lazy);
    assert!(lazy.contains_key("g1.u1"));
}

#[test]
fn lazy_prefers_unused_record_over_newer_used() {
    let (store, clock) = frozen_store();
    {
        let mut lazy = write_map(&store.inner.lazy);
        lazy.insert(
            "g1".to_owned(),
            vec![
                LazyRecord {
                    message_id: "older-unused".to_owned(),
                    timestamp: base(),
                    usage_count: 0,
                },
                LazyRecord {
                    message_id: "newer-used".to_owned(),
                    timestamp: base() + Duration::from_secs(60),
                    usage_count: 1,
                },
            ],
        );
    }
    clock.advance(Duration::from_secs(61));

    assert_eq!(store.get_lazy_messages_id("g1"), "older-unused");
}

#[test]
fn lazy_without_unused_picks_first_valid_record() {
    let (store, clock) = frozen_store();
    {
        let mut lazy = write_map(&store.inner.lazy);
        lazy.insert(
            "g1".to_owned(),
            vec![
                LazyRecord {
                    message_id: "first-used".to_owned(),
                    timestamp: base(),
                    usage_count: 1,
                },
                LazyRecord {
                    message_id: "second-used".to_owned(),
                    timestamp: base() + Duration::from_secs(60),
                    usage_count: 2,
                },
            ],
        );
    }
    clock.advance(Duration::from_secs(61));

    assert_eq!(
        store.get_lazy_messages_id("g1"),
        "first-used",
        "没有 usage==0 时 Go 取第一个有效记录，而不是最新记录"
    );

    let lazy = read_map(&store.inner.lazy);
    let records = lazy.get("g1").expect("懒消息 key 仍应存在");
    assert_eq!(records.len(), 2, "窗口内的记录全部写回");
    assert!(
        records.iter().all(|record| record.usage_count > 0),
        "读路径绝不递增/改写 usage_count"
    );
}

#[test]
fn lazy_used_record_blocks_older_unused_record() {
    // Go 的 `else if selectedRecord == nil` 会先选中使用过的较新记录；
    // 之后遇到 usage == 0 但更旧的记录不会替换（时间必须更新才替换）。
    let (store, clock) = frozen_store();
    {
        let mut lazy = write_map(&store.inner.lazy);
        lazy.insert(
            "g1".to_owned(),
            vec![
                LazyRecord {
                    message_id: "used-new".to_owned(),
                    timestamp: base() + Duration::from_secs(60),
                    usage_count: 1,
                },
                LazyRecord {
                    message_id: "unused-old".to_owned(),
                    timestamp: base(),
                    usage_count: 0,
                },
            ],
        );
    }
    clock.advance(Duration::from_secs(61));

    assert_eq!(store.get_lazy_messages_id("g1"), "used-new");
}

// ---- seq ----

#[test]
fn seq_concurrent_increments_are_unique() {
    let store = EchoStore::new();
    let results = Mutex::new(Vec::new());

    std::thread::scope(|scope| {
        for _ in 0..100 {
            scope.spawn(|| {
                let value = store.next_mapping_seq("msg");
                results
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(value);
            });
        }
    });

    let mut values = results.into_inner().unwrap_or_else(PoisonError::into_inner);
    values.sort_unstable();
    assert_eq!(values, (1..=100).collect::<Vec<i32>>());
}

#[test]
fn seq_current_and_increment_returns_zero_first() {
    let store = EchoStore::new();
    assert_eq!(store.current_and_increment_mapping_seq("msg"), 0);
    assert_eq!(store.current_and_increment_mapping_seq("msg"), 1);
    assert_eq!(store.get_mapping_seq("msg"), 2);
}

#[test]
fn seq_missing_without_ramdom_is_zero() {
    let store = EchoStore::new();
    assert_eq!(store.get_mapping_seq("missing"), 0);
}

#[test]
fn seq_missing_with_ramdom_in_range() {
    let store = EchoStore::new().with_ramdom_seq(true);
    for _ in 0..32 {
        let value = store.get_mapping_seq("missing");
        assert!(
            (1..=RANDOM_SEQ_MAX).contains(&value),
            "随机 seq 越界: {value}"
        );
    }
    // 随机取值不写回：key 仍然缺失。
    assert!(read_map(&store.inner.seq.values).get("missing").is_none());
}

#[test]
fn seq_add_then_next() {
    let store = EchoStore::new();
    store.add_mapping_seq("msg", 5);
    assert_eq!(store.get_mapping_seq("msg"), 5);
    assert_eq!(store.next_mapping_seq("msg"), 6);
}

#[test]
fn seq_increment_equals_next() {
    let store = EchoStore::new();
    assert_eq!(store.increment_mapping_seq("msg"), 1);
    assert_eq!(store.increment_mapping_seq("msg"), 2);
}

#[test]
fn next_mapping_seq_saturates_at_i32_max() {
    let store = EchoStore::new();
    store.add_mapping_seq("msg", i32::MAX);
    assert_eq!(
        store.next_mapping_seq("msg"),
        i32::MAX,
        "seq 饱和于 i32::MAX，不回绕、不 panic（值为 i32，Go int 为 64 位）"
    );
    assert_eq!(store.current_and_increment_mapping_seq("msg"), i32::MAX);
    assert_eq!(store.get_mapping_seq("msg"), i32::MAX);
}

// ---- SSM ----

#[test]
fn ssm_correlation_id_format() {
    let store = EchoStore::new();
    assert_eq!(
        store.next_ssm_correlation_id("12345678901"),
        "ssm-12345678-1"
    );
    assert_eq!(
        store.next_ssm_correlation_id("12345678901"),
        "ssm-12345678-2"
    );
    // 非 ASCII：第 8 字节切在字符中间时退化为整个 group_id，不 panic。
    assert_eq!(store.next_ssm_correlation_id("群组群组"), "ssm-群组群组-3");
}

#[test]
fn ssm_correlation_id_short_group() {
    let store = EchoStore::new();
    assert_eq!(store.next_ssm_correlation_id("42"), "ssm-42-1");
    assert_eq!(store.next_ssm_correlation_id(""), "ssm--2");
}

#[test]
fn ssm_push_fills_metadata() {
    let (store, _clock) = frozen_store();
    let mut pair = MessageGroupPair::new("g1");
    pair.payload = Some("{\"content\":\"hi\"}".to_owned());
    store.push_global_stack(pair);

    let items = store.pop_global_stack_multi(1);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].group, "g1");
    assert_eq!(items[0].correlation_id, "ssm-g1-1");
    assert_eq!(items[0].enqueue_time, Some(base()));
    assert_eq!(items[0].payload.as_deref(), Some("{\"content\":\"hi\"}"));
}

#[test]
fn ssm_push_keeps_explicit_correlation() {
    let (store, _clock) = frozen_store();
    let mut pair = MessageGroupPair::new("g1");
    pair.correlation_id = "custom".to_owned();
    store.push_global_stack(pair);

    let items = store.pop_global_stack_multi(1);
    assert_eq!(items[0].correlation_id, "custom");
}

#[test]
fn ssm_pop_multi_keeps_elements() {
    let store = EchoStore::new();
    for group in ["a", "b", "c"] {
        store.push_global_stack(MessageGroupPair::new(group));
    }

    let first_two = store.pop_global_stack_multi(2);
    assert_eq!(
        first_two
            .iter()
            .map(|pair| pair.group.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(store.global_stack_len(), 3, "pop 不删除元素");

    assert_eq!(store.pop_global_stack_multi(99).len(), 3);
    assert!(store.pop_global_stack_multi(0).is_empty());
}

#[test]
fn ssm_remove_out_of_range_is_noop() {
    let store = EchoStore::new();
    store.push_global_stack(MessageGroupPair::new("a"));
    store.push_global_stack(MessageGroupPair::new("b"));

    store.remove_from_global_stack(0);
    assert_eq!(store.global_stack_len(), 1);
    assert_eq!(store.pop_global_stack_multi(1)[0].group, "b");

    store.remove_from_global_stack(99);
    store.remove_from_global_stack(usize::MAX);
    assert_eq!(store.global_stack_len(), 1);
}

// ---- REFIDX ----

#[test]
fn ref_idx_store_get_delete() {
    let store = EchoStore::new();
    assert_eq!(store.get_ref_idx("m1"), "");

    store.store_ref_idx("m1", "REFIDX_1");
    assert_eq!(store.get_ref_idx("m1"), "REFIDX_1");

    // 空参数一律忽略（Go 同）。
    store.store_ref_idx("", "REFIDX_2");
    store.store_ref_idx("m2", "");
    assert_eq!(store.get_ref_idx(""), "");
    assert_eq!(store.get_ref_idx("m2"), "");

    store.delete_ref_idx("m1");
    assert_eq!(store.get_ref_idx("m1"), "");
    // 删除不存在的 key 不 panic。
    store.delete_ref_idx("m1");
    store.delete_ref_idx("");
}

#[test]
fn ref_idx_from_scene() {
    let store = EchoStore::new();
    let scene = MessageScene::from_ext(["ref_msg_idx=TMP_x", "msg_idx=REFIDX_1", "auth_token=t"]);

    assert_eq!(scene.ref_idx(), Some("REFIDX_1"));
    store.store_ref_idx_from_scene("m1", &scene);
    assert_eq!(store.get_ref_idx("m1"), "REFIDX_1");

    // msgID 为空 → 不写。
    store.store_ref_idx_from_scene("", &scene);
    assert_eq!(store.get_ref_idx(""), "");

    // 值为空 → 不写（也不看后续元素）。
    let empty = MessageScene::from_ext(["msg_idx=", "msg_idx=REFIDX_2"]);
    assert_eq!(empty.ref_idx(), None);
    store.store_ref_idx_from_scene("m2", &empty);
    assert_eq!(store.get_ref_idx("m2"), "");

    // 没有 msg_idx → 不写。
    assert_eq!(MessageScene::default().ref_idx(), None);
}

// ---- echo 键与映射 ----

#[test]
fn echo_key_formats() {
    assert_eq!(echo_key("10001", 42), "10001_42");
    assert_eq!(echo_key_v2("10001", 1, 2), "10001_1_2");
    assert_eq!(echo_key_event_id("10001", 7), "10001_7");
    assert_eq!(echo_key_event_id_v2("10001", "openid"), "10001_openid");
    assert_eq!(echo_key_v3("10001", "abc"), "10001_abc");
}

#[test]
fn msg_type_and_msg_id_mappings() {
    let store = EchoStore::new();
    assert_eq!(store.get_msg_type_by_key("nope"), "");
    assert_eq!(store.get_msg_id_by_key("nope"), "");

    store.add_msg_type("10001", 5, "2");
    assert_eq!(store.get_msg_type_by_key(&echo_key("10001", 5)), "2");
    assert_eq!(store.get_message_type_by_group_id_v2("10001", "5"), "2");

    store.add_msg_id("10001", 5, "msg-a");
    store.add_msg_id_v2("10001", 5, 9, "msg-b");
    store.add_msg_id_v3("10001", "abc", "msg-c");

    assert_eq!(store.get_msg_id_by_key(&echo_key("10001", 5)), "msg-a");
    assert_eq!(
        store.get_msg_id_by_key(&echo_key_v2("10001", 5, 9)),
        "msg-b"
    );
    assert_eq!(store.get_msg_id_v3("10001", "abc"), "msg-c");
    assert_eq!(store.get_msg_id_v3("10001", "missing"), "");
}

#[test]
fn event_id_mappings() {
    let store = EchoStore::new();
    store.add_event_id("10001", 7, "event-a");
    store.add_event_id_v2("10001", "openid", "event-b");

    assert_eq!(
        store.get_event_id_by_key(&echo_key_event_id("10001", 7)),
        "event-a"
    );
    assert_eq!(
        store.get_event_id_by_key(&echo_key_event_id_v2("10001", "openid")),
        "event-b"
    );
    assert_eq!(store.get_event_id_by_key("nope"), "");
}

#[test]
fn int_mapping_roundtrip() {
    let store = EchoStore::new();
    assert_eq!(store.get_mapping(7), 0);
    store.add_mapping(7, 3);
    assert_eq!(store.get_mapping(7), 3);
}

// ---- 行号缓存 ----

#[test]
fn cache_in_memory_roundtrip() {
    let store = EchoStore::new();
    let row = store.store_cache_in_memory("msg-1").expect("分配行号");
    assert!(row > 0);
    assert_eq!(
        store.store_cache_in_memory("msg-1").expect("复取行号"),
        row,
        "同一 ID 必须稳定得到同一行号"
    );
    assert_eq!(
        store.get_cache_id_from_memory_by_row_id(&row.to_string()),
        Some("msg-1".to_owned())
    );
    assert_eq!(
        store.get_cache_id_from_memory_by_row_id("not-a-number"),
        None
    );
    // 18 位行号的上界远小于 i64::MAX，这个值一定查不到。
    assert_eq!(
        store.get_cache_id_from_memory_by_row_id("9223372036854775807"),
        None
    );
}

#[test]
fn cache_in_memory_uses_go_row_id() {
    let store = EchoStore::new();
    let row = store.store_cache_in_memory("abc").expect("分配行号");
    // Go `GenerateRowID("abc", 9) == 900150983`。
    assert_eq!(row, 900_150_983);
    assert_eq!(
        store.get_cache_id_from_memory_by_row_id("900150983"),
        Some("abc".to_owned())
    );
}

/// MD5 已知向量：与 Go `idmap.GenerateRowID` 逐位对齐。
#[test]
fn row_id_matches_go_generate_row_id() {
    // md5("abc") = 900150983cd24fb0d6963f7d28e17f72
    // → 数字串 90015098324069637281772 → length=9 时 900150983。
    assert_eq!(generate_row_id("abc", 9).expect("派生行号"), 900_150_983);
    assert_eq!(
        generate_row_id("abc", 18).expect("派生行号"),
        900_150_983_240_696_372
    );

    // 中文 ID：md5 = 915cdb9efe1515cba2dd84b06c0f6067（按 UTF-8 字节计算）。
    assert_eq!(
        generate_row_id("群1234567890-openid", 9).expect("派生行号"),
        915_915_152
    );
    assert_eq!(
        generate_row_id("群1234567890-openid", 18).expect("派生行号"),
        915_915_152_840_606_067
    );

    // 中英混排长 ID：md5 = 1e5d84ca6ac5de0f4c19af42c35f5599。
    assert_eq!(
        generate_row_id("CMD_2026_群聊消息_9876543210_abcdef", 9).expect("派生行号"),
        158_465_041
    );

    // 数字不足 length 时右侧补 '0'（Go 的 strings.Repeat）。
    assert_eq!(row_id_from_digits("12", 9).expect("派生行号"), 120_000_000);
    assert_eq!(row_id_from_digits("", 9).expect("派生行号"), 0);
}

#[test]
fn cache_in_memory_accepts_empty_id_like_go() {
    let store = EchoStore::new();
    let row = store.store_cache_in_memory("").expect("Go 允许空 ID");
    assert_eq!(
        store.store_cache_in_memory("").expect("复取行号"),
        row,
        "同一 ID 必须稳定得到同一行号"
    );
}

#[test]
fn cache_row_lookup_treats_unparsable_row_as_zero() {
    let store = EchoStore::new();
    write_map(&store.inner.cache_ids).insert(0, "row-zero".to_owned());

    // Go 忽略 `strconv.ParseInt` 的错误，按 0 查表。
    for row_id in ["not-a-number", "-", "", "9223372036854775808"] {
        assert_eq!(
            store.get_cache_id_from_memory_by_row_id(row_id),
            Some("row-zero".to_owned()),
            "rowID {row_id:?} 应回退到 0"
        );
    }
}

// ---- 清理 ----

#[test]
fn cleanup_clears_all_mappings() {
    let (store, _clock) = frozen_store();
    store.add_msg_type("10001", 1, "2");
    store.add_msg_id("10001", 1, "msg");
    store.add_event_id("10001", 1, "event");
    store.add_mapping(1, 2);
    store.add_mapping_seq("msg", 4);
    store.push_global_stack(MessageGroupPair::new("g1"));
    store.store_ref_idx("msg", "REFIDX_1");
    let row = store.store_cache_in_memory("msg").expect("分配行号");

    store.cleanup();

    assert_eq!(store.get_msg_type_by_key(&echo_key("10001", 1)), "");
    assert_eq!(store.get_msg_id_by_key(&echo_key("10001", 1)), "");
    assert_eq!(
        store.get_event_id_by_key(&echo_key_event_id("10001", 1)),
        ""
    );
    assert_eq!(store.get_mapping(1), 0);
    assert_eq!(store.get_mapping_seq("msg"), 0);
    assert_eq!(store.global_stack_len(), 0);
    assert_eq!(store.get_ref_idx("msg"), "");
    assert_eq!(
        store.get_cache_id_from_memory_by_row_id(&row.to_string()),
        None
    );
    // 行号缓存清空后仍能重新派生同一行号。
    assert_eq!(store.store_cache_in_memory("msg").expect("重新分配"), row);
    // SSM 计数器不随清理重置（Go 同）：入队已用掉 1。
    assert_eq!(store.next_ssm_correlation_id("g1"), "ssm-g1-2");
}

#[test]
fn cleanup_keeps_lazy_messages_within_window() {
    let (store, clock) = frozen_store();
    store.add_lazy_message_id("g1", "lazy", base());

    store.cleanup();

    // Go `cleanupGlobalMaps` 从不清理 `messageStore.records`：被动回复 msg_id 必须活到窗口结束。
    assert_eq!(
        store.get_lazy_messages_id("g1"),
        "lazy",
        "cleanup 后懒消息仍在 4 分钟窗口内可命中"
    );

    // 窗口仍按时间裁剪：满 4 分钟后照常回落到主动推送默认值。
    clock.advance(DEFAULT_LAZY_WINDOW);
    assert_eq!(store.get_lazy_messages_id("g1"), DEFAULT_MESSAGE_ID);
}

#[test]
fn cleanup_routine_tick_clears() {
    let store = EchoStore::new();
    store.add_msg_id("10001", 1, "msg");
    store
        .start_cleanup_routine_with_period(Duration::from_millis(5))
        .expect("启动清理线程");

    let deadline = Instant::now() + Duration::from_secs(2);
    while !store.get_msg_id_by_key(&echo_key("10001", 1)).is_empty() {
        assert!(Instant::now() < deadline, "清理线程未在 2 秒内清空映射");
        std::thread::sleep(Duration::from_millis(5));
    }

    store.stop_cleanup_routine();
    assert!(store.get_msg_id_by_key(&echo_key("10001", 1)).is_empty());
}
