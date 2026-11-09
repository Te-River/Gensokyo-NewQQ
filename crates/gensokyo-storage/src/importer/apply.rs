//! 校验通过的数据落 redb：先规划（只读提交态），再身份库/消息库各一个写事务。
//!
//! 规划与应用拆开是为了 `--dry-run`：同一份决策逻辑先跑一遍只读版本，冲突、幂等命中与
//! 计数器的最终值都能在**不动库**的前提下报出来。

use std::collections::BTreeSet;

use redb::{ReadOnlyTable, ReadTransaction, ReadableDatabase, ReadableTable};

use super::jsonl::{Allocator, ImportData};
use super::{ImportError, ImportOptions, ImportReport, OnVuinConflict};
use crate::StorageError;
use crate::echo::generate_row_id;
use crate::idmap::encoding::{encode_i64, vuin_identity_key};
use crate::idmap::{
    CONFIG, CURRENT_MSG_KEY, CURRENT_VUIN_KEY, EXPIRES_AT, HASH_VUIN_MAX_DIGITS,
    HASH_VUIN_MIN_DIGITS, IDENTITY_TO_VUIN, IdMapStore, LAST_SEEN, LATEST_BY_GROUP_USER,
    LATEST_EXPIRES_AT, META, MSG_TO_VIRTUAL, USER_INFO, VIRTUAL_TO_MSG, VUIN_TO_IDENTITY,
    config_key, counter_text, decode_vuin, last_seen_key, latest_key, prefix_exists, rb,
};

/// 一条身份的落地方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    /// 同 key 同 vUIN：同值 upsert。
    Unchanged,
    /// 新写入。
    Imported,
    /// 冲突后换新 vUIN，旧 vUIN 的归属保留。
    Remapped,
}

/// 导入一条身份的最终决定。
struct PlannedIdentity<'a> {
    /// Go 归一化后的身份 key（原样入库，不再归一化）。
    key: &'a str,
    /// `vuin_to_identity` 的值（Go `identity.Raw`）。
    raw: &'a str,
    vuin: i64,
}

/// 身份规划结果。
struct IdentityPlan<'a> {
    /// 要写的身份（含幂等命中：同值 upsert 不变，顺便修复缺失的反表行）。
    planned: Vec<PlannedIdentity<'a>>,
    imported: usize,
    unchanged: usize,
    remapped: usize,
    /// 导入后的 `current_vuin`。
    counter: i64,
}

/// 消息库规划结果。
struct MsgPlan {
    /// 导入后的 `current_msg`。
    counter: i64,
    /// 文件是否涉及消息库（否则一个字节都不写）。
    touched: bool,
}

/// 规划 →（非 `--dry-run` 时）写入 → 报告。
pub(crate) fn run(
    store: &IdMapStore,
    data: &ImportData,
    options: &ImportOptions,
) -> Result<ImportReport, ImportError> {
    if !options.merge && !store.is_empty()? {
        return Err(ImportError::NotEmpty(format!(
            "{} 或 {} 已有数据",
            store.options().identity_path.display(),
            store.options().msg_path.display()
        )));
    }
    let identities = plan_identities(store, data, options.on_vuin_conflict)?;
    let messages = plan_messages(store, data)?;
    let report = ImportReport {
        dry_run: options.dry_run,
        records: data.records,
        unknown_kinds: data.unknown_kinds,
        identities_imported: identities.imported,
        identities_unchanged: identities.unchanged,
        identities_remapped: identities.remapped,
        configs: data.configs.len(),
        user_infos: data.user_infos.len(),
        last_seen: data.last_seen.len(),
        msg_ids: data.msg_ids.len(),
        latest: data.latest.len(),
        vuin_counter: identities.counter,
        msg_counter: messages.counter,
    };
    if options.dry_run {
        return Ok(report);
    }
    write_identities(store, data, &identities)?;
    write_messages(store, data, &messages)?;
    Ok(report)
}

/// 逐条决定身份的落地方式：幂等命中 / 新写入 / （`--on-vuin-conflict=remap` 时）换新 vUIN。
fn plan_identities<'a>(
    store: &IdMapStore,
    data: &'a ImportData,
    on_conflict: OnVuinConflict,
) -> Result<IdentityPlan<'a>, ImportError> {
    let txn = store.identity_db().begin_read().map_err(rb)?;
    let forward = txn.open_table(IDENTITY_TO_VUIN).map_err(rb)?;
    let reverse = txn.open_table(VUIN_TO_IDENTITY).map_err(rb)?;
    // 新 vUIN 的取法必须与导出侧一致（`meta.allocator`）；文件没声明时沿用本机配置。
    let hash_id = match data.allocator {
        Some(Allocator::Hash) => true,
        Some(Allocator::Counter) => false,
        None => store.options().hash_id,
    };

    // 计数器只增不减：库内现值、文件声明的计数器、文件内最大 vUIN 三者取最大。
    let mut counter = read_counter(&txn, CURRENT_VUIN_KEY)?.max(data.vuin_counter.unwrap_or(0));
    for line in &data.identities {
        counter = counter.max(line.vuin);
    }

    let mut claimed: BTreeSet<i64> = BTreeSet::new();
    let mut planned = Vec::with_capacity(data.identities.len());
    let (mut imported, mut unchanged, mut remapped) = (0usize, 0usize, 0usize);

    for line in &data.identities {
        let existing = match forward.get(line.key.as_str()).map_err(rb)? {
            Some(guard) => Some(decode_vuin(guard.value())?),
            None => None,
        };
        let vuin_taken = claimed.contains(&line.vuin) || prefix_exists(&reverse, line.vuin)?;
        let (vuin, decision) = match existing {
            Some(vuin) if vuin == line.vuin => (vuin, Decision::Unchanged),
            Some(vuin) => (
                resolve_conflict(
                    on_conflict,
                    format!(
                        "身份 {} 已映射到 vUIN {vuin}，导入行要求 {}",
                        line.key, line.vuin
                    ),
                    hash_id,
                    &line.raw,
                    &mut counter,
                    &mut claimed,
                    &reverse,
                )?,
                Decision::Remapped,
            ),
            None if vuin_taken => (
                resolve_conflict(
                    on_conflict,
                    format!("vUIN {} 已被其他身份占用（导入行 {}）", line.vuin, line.key),
                    hash_id,
                    &line.raw,
                    &mut counter,
                    &mut claimed,
                    &reverse,
                )?,
                Decision::Remapped,
            ),
            None => (line.vuin, Decision::Imported),
        };
        match decision {
            Decision::Unchanged => unchanged += 1,
            Decision::Imported => imported += 1,
            Decision::Remapped => remapped += 1,
        }
        claimed.insert(vuin);
        planned.push(PlannedIdentity {
            key: &line.key,
            raw: &line.raw,
            vuin,
        });
    }

    Ok(IdentityPlan {
        planned,
        imported,
        unchanged,
        remapped,
        counter,
    })
}

/// 冲突处理：`fail` 直接报错，`remap` 分配一个未占用的新 vUIN。
fn resolve_conflict(
    on_conflict: OnVuinConflict,
    detail: String,
    hash_id: bool,
    raw: &str,
    counter: &mut i64,
    claimed: &mut BTreeSet<i64>,
    reverse: &ReadOnlyTable<&'static str, &'static [u8]>,
) -> Result<i64, ImportError> {
    if on_conflict == OnVuinConflict::Fail {
        return Err(ImportError::Conflict(detail));
    }
    let vuin = allocate_vuin(hash_id, raw, counter, claimed, reverse)?;
    claimed.insert(vuin);
    Ok(vuin)
}

/// 新 vUIN 的取法跟随存储的 `hash_id`（与 Go `allocateVuinTx` 同一口径）。
fn allocate_vuin(
    hash_id: bool,
    raw: &str,
    counter: &mut i64,
    claimed: &BTreeSet<i64>,
    reverse: &ReadOnlyTable<&'static str, &'static [u8]>,
) -> Result<i64, ImportError> {
    let taken = |vuin: i64| -> Result<bool, StorageError> {
        if claimed.contains(&vuin) {
            return Ok(true);
        }
        prefix_exists(reverse, vuin)
    };
    if hash_id {
        for digits in HASH_VUIN_MIN_DIGITS..=HASH_VUIN_MAX_DIGITS {
            let candidate = generate_row_id(raw, digits)?;
            if !taken(candidate)? {
                *counter = (*counter).max(candidate);
                return Ok(candidate);
            }
        }
        return Err(ImportError::Storage(StorageError::Corrupt(format!(
            "身份 {raw} 在 {HASH_VUIN_MIN_DIGITS}~{HASH_VUIN_MAX_DIGITS} 位内无法派生唯一 vUIN"
        ))));
    }
    let mut candidate = counter
        .checked_add(1)
        .ok_or_else(|| overflow_error("vUIN"))?;
    while taken(candidate)? {
        candidate = candidate
            .checked_add(1)
            .ok_or_else(|| overflow_error("vUIN"))?;
    }
    *counter = candidate;
    Ok(candidate)
}

/// 消息库的规划：计数器取库内现值、文件声明值、文件内最大虚拟 ID 三者最大。
fn plan_messages(store: &IdMapStore, data: &ImportData) -> Result<MsgPlan, ImportError> {
    let touched = data.msg_counter.is_some() || !data.msg_ids.is_empty() || !data.latest.is_empty();
    let txn = store.msg_db().begin_read().map_err(rb)?;
    let mut counter = read_counter(&txn, CURRENT_MSG_KEY)?.max(data.msg_counter.unwrap_or(0));
    for line in &data.msg_ids {
        counter = counter.max(line.virtual_id);
    }
    Ok(MsgPlan { counter, touched })
}

/// 身份库：一个写事务写完身份/配置/用户信息/last_seen 与 `current_vuin`。
fn write_identities(
    store: &IdMapStore,
    data: &ImportData,
    plan: &IdentityPlan<'_>,
) -> Result<(), ImportError> {
    let txn = store.identity_db().begin_write().map_err(rb)?;
    {
        let mut meta = txn.open_table(META).map_err(rb)?;
        let mut forward = txn.open_table(IDENTITY_TO_VUIN).map_err(rb)?;
        let mut reverse = txn.open_table(VUIN_TO_IDENTITY).map_err(rb)?;
        let mut last_seen = txn.open_table(LAST_SEEN).map_err(rb)?;
        let mut config = txn.open_table(CONFIG).map_err(rb)?;
        let mut user_info = txn.open_table(USER_INFO).map_err(rb)?;

        for item in &plan.planned {
            let mapped = encode_i64(item.vuin);
            forward.insert(item.key, mapped.as_slice()).map_err(rb)?;
            let reverse_key = vuin_identity_key(item.vuin, item.key);
            reverse
                .insert(reverse_key.as_str(), item.raw.as_bytes())
                .map_err(rb)?;
        }
        for line in &data.configs {
            let key = config_key(&line.section, &line.key);
            config
                .insert(key.as_str(), line.value.as_bytes())
                .map_err(rb)?;
        }
        for line in &data.user_infos {
            user_info
                .insert(line.key.as_str(), line.json.as_bytes())
                .map_err(rb)?;
        }
        for line in &data.last_seen {
            let key = last_seen_key(line.vuin);
            last_seen
                .insert(key.as_str(), line.raw.as_bytes())
                .map_err(rb)?;
        }
        let counter = encode_i64(plan.counter);
        meta.insert(CURRENT_VUIN_KEY, counter.as_slice())
            .map_err(rb)?;
    }
    txn.commit().map_err(rb)?;
    // 导入绕过了 IdMapStore 的写路径（直连库句柄），读路径缓存必须整片失效
    store.invalidate_lookup_cache();
    Ok(())
}

/// 消息库：文件未涉及消息库时不开写事务。
fn write_messages(
    store: &IdMapStore,
    data: &ImportData,
    plan: &MsgPlan,
) -> Result<(), ImportError> {
    if !plan.touched {
        return Ok(());
    }
    let txn = store.msg_db().begin_write().map_err(rb)?;
    {
        let mut meta = txn.open_table(META).map_err(rb)?;
        let mut forward = txn.open_table(MSG_TO_VIRTUAL).map_err(rb)?;
        let mut reverse = txn.open_table(VIRTUAL_TO_MSG).map_err(rb)?;
        let mut expires = txn.open_table(EXPIRES_AT).map_err(rb)?;
        let mut latest = txn.open_table(LATEST_BY_GROUP_USER).map_err(rb)?;
        let mut latest_expires = txn.open_table(LATEST_EXPIRES_AT).map_err(rb)?;

        for line in &data.msg_ids {
            let mapped = encode_i64(line.virtual_id);
            forward
                .insert(line.msg_id.as_str(), mapped.as_slice())
                .map_err(rb)?;
            let key = counter_text(line.virtual_id);
            reverse
                .insert(key.as_str(), line.msg_id.as_bytes())
                .map_err(rb)?;
            if let Some(at) = line.expires {
                expires
                    .insert(key.as_str(), encode_i64(at).as_slice())
                    .map_err(rb)?;
            }
        }
        for line in &data.latest {
            let key = latest_key(&line.group, &line.user);
            latest
                .insert(key.as_str(), line.msg_id.as_bytes())
                .map_err(rb)?;
            if let Some(at) = line.expires {
                latest_expires
                    .insert(key.as_str(), encode_i64(at).as_slice())
                    .map_err(rb)?;
            }
        }
        // 写事务内再取一次 max：库内计数器可能已被同进程的其他写入推进。
        let committed = match meta.get(CURRENT_MSG_KEY).map_err(rb)? {
            Some(guard) => decode_vuin(guard.value())?,
            None => 0,
        };
        let mapped = encode_i64(plan.counter.max(committed));
        meta.insert(CURRENT_MSG_KEY, mapped.as_slice())
            .map_err(rb)?;
    }
    txn.commit().map_err(rb)?;
    Ok(())
}

/// 读 8 字节大端计数器（不存在即 0，与 Go `readInt64(meta.Get(key))` 一致）。
fn read_counter(txn: &ReadTransaction, key: &str) -> Result<i64, StorageError> {
    let meta = txn.open_table(META).map_err(rb)?;
    match meta.get(key).map_err(rb)? {
        Some(guard) => decode_vuin(guard.value()),
        None => Ok(0),
    }
}

fn overflow_error(what: &str) -> ImportError {
    ImportError::Storage(StorageError::Corrupt(format!("{what} 空间耗尽")))
}
