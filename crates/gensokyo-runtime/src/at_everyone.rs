//! @全员：标签归一 + 「每群每自然日 1 次」限流。
//!
//! # 归一
//!
//! 官方群聊 @全体成员的写法是字面标签 `<qqbot-at-everyone />`（文本与 markdown 都认），
//! OneBot 侧沿用 CQ 习惯写 `[CQ:at,qq=all]`。两个入口归一到同一个标签：
//!
//! | 输入 | 输出 |
//! |------|------|
//! | `[CQ:at,qq=all]` | `<qqbot-at-everyone />` |
//! | `[CQ:at,qq=everyone]` | 同上（宽松别名，防手误） |
//! | 字面 `<qqbot-at-everyone />` | 原样保留 |
//!
//! [`normalize`] 只替换这几处子串，其余字节原样保留——markdown 内容里的 `[` `]` 不会被
//! 重新转义（CQ 的 `escape` 会破坏 `[文本](链接)` 语法）。
//!
//! # 限流
//!
//! 官方限制：**每个群每个自然日 1 次**，超额上游直接报错。计数落在 idmap 的 config 表：
//! 与 Go `handlers/at_everyone.go` 的 `WriteConfig` / `ReadConfig` **格式兼容**，但存储不同源
//! （Rust redb / Go bbolt），仅同一份存储时才互认；section 取 [`SECTION`]、key 取群 OpenID，
//! 值是 `{"date":"YYYY-MM-DD","count":1}`。
//!
//! 顺序是**先占计数、发送失败回滚**：[`reserve`] 先写当天的占位记录，调用方发送失败时用
//! [`Reservation::rollback`] 恢复旧值（旧值不存在则删键）。这样同群并发的两条 @全员 只有
//! 一条能占住，也不会因为发送失败而白白吃掉当天的配额。
//!
//! # 时区
//!
//! 两侧统一按**固定 UTC+08:00** 切自然日（[`date_at`]）：Go 用本地时区
//! （`time.Now().Format("2006-01-02")`，部署在 CST 即 +08:00），Rust 侧写死偏移而不读系统
//! 时区，容器 `TZ=UTC` 时两侧不会在跨零点前后各判一天。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use gensokyo_core::{Error, Result};
use gensokyo_storage::{IdMapStore, StorageError};
use serde::{Deserialize, Serialize};

/// idmap config 表的 section（与 Go 侧 `at_everyone` 同名，同一存储时互认）。
pub const SECTION: &str = "at_everyone";

/// 群聊 @全员 标签（官方文档原文）。
pub const TAG: &str = "<qqbot-at-everyone />";

/// CQ `at` 段的前缀；比较时大小写不敏感（Go 的 `atEveryoneCQPattern` 带 `(?i)`）。
const AT_PREFIX: &str = "[CQ:at,";

/// 官方标签的开头（`/>` 前允许任意空白，Go 的正则是 `<qqbot-at-everyone\s*/>`）。
const TAG_OPEN: &str = "<qqbot-at-everyone";

/// 官方标签的结尾。
const TAG_CLOSE: &str = "/>";

/// CQ `at` 段里表示「所有人」的取值：官方只定义 `all`，`everyone` 是宽松别名（大小写不敏感）。
const EVERYONE_IDS: [&str; 2] = ["all", "everyone"];

/// 一天有多少秒（自然日切分用）。
const SECONDS_PER_DAY: u64 = 86_400;

/// 自然日固定偏移：UTC+08:00（秒）。
const DAY_OFFSET_SECONDS: u64 = 8 * 3_600;

/// 把 `[CQ:at,qq=all]` / `[CQ:at,qq=everyone]` 换成官方标签；其余字节原样保留。
///
/// 只做字面子串替换，不重新转义，因此对 markdown 内容也是安全的。大小写与 Go 的
/// `(?i)\[CQ:at,qq=(?:all|everyone)(?:,[^\]]*)?\]` 一致（`[cq:at,QQ=ALL]` 同样归位）。
#[must_use]
pub fn normalize(text: &str) -> String {
    let Some(first) = find_at_code(text, 0) else {
        return text.to_owned();
    };
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..first]);
    let mut from = first;
    loop {
        let Some(start) = find_at_code(text, from) else {
            out.push_str(&text[from..]);
            return out;
        };
        out.push_str(&text[from..start]);
        let after = &text[start..];
        let Some(end) = after.find(']') else {
            // 没有闭合括号：剩余部分按原文处理（与 `cqcode::parse` 的 malformed 分支一致）
            out.push_str(after);
            return out;
        };
        let code = &after[..=end];
        if is_everyone_code(code) {
            out.push_str(TAG);
        } else {
            out.push_str(code);
        }
        from = start + end + 1;
    }
}

/// 查找下一个 `[CQ:at,` 的起始字节下标（大小写不敏感）。
///
/// 供 markdown 渲染复用：markdown 正文只替换 at 段，其余字节必须原样保留。
#[must_use]
pub fn find_at_code(text: &str, from: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut at = from;
    while let Some(found) = text[at..].find('[') {
        let index = at + found;
        if bytes
            .get(index..index + AT_PREFIX.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(AT_PREFIX.as_bytes()))
        {
            return Some(index);
        }
        at = index + 1;
    }
    None
}

/// 取 `[CQ:at,qq=…]` 里的 `qq` 值（键名大小写不敏感，值已 trim）；不是完整的 at 码返回 `None`。
#[must_use]
pub fn at_mention_id(code: &str) -> Option<&str> {
    let stripped = code.strip_suffix(']')?;
    if !stripped
        .get(..AT_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(AT_PREFIX))
    {
        return None;
    }
    stripped[AT_PREFIX.len()..].split(',').find_map(|param| {
        let (key, value) = param.split_once('=')?;
        key.trim().eq_ignore_ascii_case("qq").then(|| value.trim())
    })
}

/// 文本里是否已有归一后的标签（发送前扫 `content` / `markdown.content`）。
///
/// 与 Go `atEveryoneTagPattern` 同口径：`/>` 前允许无空格 / 多空格（`<qqbot-at-everyone/>`、
/// `<qqbot-at-everyone  />` 都算），标签本身大小写敏感（Go 的正则没有 `(?i)`）。
#[must_use]
pub fn contains_tag(text: &str) -> bool {
    let mut from = 0;
    while let Some(found) = text[from..].find(TAG_OPEN) {
        let end = from + found + TAG_OPEN.len();
        if text[end..].trim_start().starts_with("/>") {
            return true;
        }
        from = end;
    }
    false
}

/// 只保留第一个 @全员 标签，重复的删掉；正文其余字节原样保留。
///
/// markdown 正文可能同时带字面标签与 `[CQ:at,qq=all]`：两者归一后是同一个标签，
/// 连发两个等于重复 @（官方只接受一个）。文本与 markdown 两条渲染路径都要过一遍，
/// 保证一条消息里只有一个标签，当天额度也只按一条消息计一次。
#[must_use]
pub fn dedupe_tag(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    let mut seen = false;
    while let Some(found) = text[from..].find(TAG_OPEN) {
        let start = from + found;
        let open_end = start + TAG_OPEN.len();
        let rest = &text[open_end..];
        let trimmed = rest.trim_start();
        if !trimmed.starts_with(TAG_CLOSE) {
            // 不是完整标签：这段前缀原样保留，继续向后找
            out.push_str(&text[from..open_end]);
            from = open_end;
            continue;
        }
        let end = open_end + (rest.len() - trimmed.len()) + TAG_CLOSE.len();
        out.push_str(&text[from..start]);
        if !seen {
            out.push_str(&text[start..end]);
            seen = true;
        }
        from = end;
    }
    out.push_str(&text[from..]);
    out
}

/// `[CQ:at,qq=all]` / `[CQ:at,qq=everyone,…]` 判定（只看 `qq` 参数）。
fn is_everyone_code(code: &str) -> bool {
    at_mention_id(code).is_some_and(is_everyone_id)
}

/// 取值是否为「所有人」（大小写不敏感：`[CQ:at,qq=ALL]` 与 Go 一样算 @全员）。
#[must_use]
pub fn is_everyone_id(value: &str) -> bool {
    EVERYONE_IDS
        .iter()
        .any(|id| value.eq_ignore_ascii_case(id))
}


/// 当日计数记录（与 Go 侧同名字段一一对应）。
#[derive(Debug, Serialize, Deserialize)]
struct Usage {
    /// 自然日 `YYYY-MM-DD`。
    date: String,
    /// 次数，恒为 1（官方只允许每天一次，没有「剩余次数」语义）。
    count: u32,
}

/// 一次成功的占位：发送失败时用它回滚。
#[derive(Debug)]
pub struct Reservation {
    /// 群 OpenID（同时是 config key）。
    group: String,
    /// 占位前的原始值；`None` 表示当天本来没有记录，回滚要删键。
    previous: Option<String>,
}

impl Reservation {
    /// 回滚到占位前：有旧值写回旧值，没有旧值就删键。
    ///
    /// 回滚失败只告警：调用方此时正在返回发送错误，不能让回滚错误盖掉它。
    pub fn rollback(self, idmap: &IdMapStore) {
        let guard = group_lock(&self.group);
        let _guard = guard.lock().unwrap_or_else(PoisonError::into_inner);
        let outcome = match &self.previous {
            Some(previous) => idmap.write_config(SECTION, &self.group, previous),
            None => idmap
                .delete_config(SECTION, &self.group)
                .map(|_removed| ()),
        };
        if let Err(err) = outcome {
            tracing::warn!(group = %self.group, error = %err, "@全员 计数回滚失败");
        }
    }

    /// 占位所属的群 OpenID（测试与日志用）。
    #[must_use]
    pub fn group(&self) -> &str {
        &self.group
    }
}

/// 读当天计数：读失败按「没有记录」放行（Go `GuardAtEveryone` 把读错当成 `prev = ""`）。
///
/// 读失败放行，避免本地存储故障导致消息全丢：若判成「已用过」，一次存储抖动就会让该群的
/// @全员 永久不可用；写占位失败仍会照常报 [`Error::Storage`]，限流不会因此被悄悄绕过。
fn previous_usage(
    result: std::result::Result<String, StorageError>,
    group_openid: &str,
) -> Option<String> {
    match result {
        Ok(value) => Some(value),
        Err(err) if err.is_not_found() => None,
        Err(err) => {
            tracing::warn!(group = %group_openid, error = %err, "读取 @全员 计数失败，按未占用放行");
            None
        }
    }
}

/// 占住本群今天的 @全员 名额（每群每自然日 1 次）。
///
/// # Errors
/// 今天已经用过返回 [`Error::Protocol`]（→ `1400`）；写 idmap 失败返回
/// [`Error::Storage`]。读失败不报错，按「未占用」放行（与 Go `GuardAtEveryone` 一致）。
pub fn reserve(idmap: &IdMapStore, group_openid: &str) -> Result<Reservation> {
    let guard = group_lock(group_openid);
    let _guard = guard.lock().unwrap_or_else(PoisonError::into_inner);

    let previous = previous_usage(idmap.read_config(SECTION, group_openid), group_openid);

    let date = today();
    if previous.as_deref().and_then(usage_date) == Some(date.clone()) {
        return Err(Error::Protocol(format!(
            "本群今天已经 @过全体成员：官方限制每群每自然日 1 次（{date}）"
        )));
    }

    let value = serde_json::to_string(&Usage { date, count: 1 })
        .map_err(|err| Error::Storage(format!("序列化 @全员 计数失败: {err}")))?;
    idmap
        .write_config(SECTION, group_openid, &value)
        .map_err(|err| Error::Storage(format!("写入 @全员 计数失败: {err}")))?;
    Ok(Reservation {
        group: group_openid.to_owned(),
        previous,
    })
}

/// 旧记录的日期；值损坏时按「没有记录」处理（不让一条脏数据永久锁死某个群）。
fn usage_date(raw: &str) -> Option<String> {
    match serde_json::from_str::<Usage>(raw) {
        Ok(usage) => Some(usage.date),
        Err(err) => {
            tracing::warn!(error = %err, "@全员 计数记录无法解析，按无记录处理");
            None
        }
    }
}

/// 进程内 per-group 互斥：同一群的「读-改-写」串行，不同群互不阻塞。
///
/// 跨进程互斥不做（与 Go 的 `sync.Map` 存 mutex 同口径）：多实例部署时两进程仍可能各占
/// 一次，最终由上游的每日限制兜底。
fn group_lock(group_openid: &str) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    Arc::clone(locks.entry(group_openid.to_owned()).or_default())
}

/// 今天（固定 UTC+08:00）的 `YYYY-MM-DD`。
fn today() -> String {
    date_at(unix_now_seconds())
}

/// 当前 Unix 秒；系统时钟早于 1970 时按 0 处理（与旧实现一致，不为时钟回拨 panic）。
fn unix_now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Unix 秒 → 固定 UTC+08:00 自然日的 `YYYY-MM-DD`。
///
/// 偏移写死而不读系统时区：容器默认 `TZ=UTC` 时若按本地时区算，会和 Go 侧（CST 部署）在
/// 本地 00:00~08:00 之间各判一天，同一条群昨天发过今天却放行。
fn date_at(unix_seconds: u64) -> String {
    format_date(unix_seconds.saturating_add(DAY_OFFSET_SECONDS) / SECONDS_PER_DAY)
}

/// 1970-01-01 起的天数 → `YYYY-MM-DD`（Howard Hinnant 的 `civil_from_days`）。
fn format_date(days_since_epoch: u64) -> String {
    let z = i64::try_from(days_since_epoch).unwrap_or(i64::MAX) + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = i64::try_from(yoe).unwrap_or(0) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gensokyo_storage::IdMapOptions;
    use tempfile::TempDir;

    fn store() -> (TempDir, IdMapStore) {
        let dir = tempfile::tempdir().expect("临时目录");
        let options = IdMapOptions::new(
            dir.path().join("openid-map.redb"),
            dir.path().join("msgid-map.redb"),
        )
        .with_app_id("10001");
        let store = IdMapStore::open(options).expect("打开 idmap 存储失败");
        (dir, store)
    }

    const GROUP: &str = "GROUP-OPENID";

    // ---- 归一 ----

    #[test]
    fn normalize_rewrites_both_cq_entries() {
        assert_eq!(normalize("[CQ:at,qq=all]早上好"), format!("{TAG}早上好"));
        assert_eq!(
            normalize("[CQ:at,qq=everyone]早安"),
            format!("{TAG}早安")
        );
        // 带额外参数（顺序无关）也认
        assert_eq!(
            normalize("[CQ:at,qq=all,name=全体]"),
            TAG.to_owned()
        );
    }

    #[test]
    fn normalize_keeps_other_bytes_intact() {
        // 普通 @、字面标签、markdown 链接都不受影响（不重新转义 `[` `]`）
        assert_eq!(normalize("[CQ:at,qq=123456]你好"), "[CQ:at,qq=123456]你好");
        assert_eq!(normalize(TAG), TAG);
        assert_eq!(
            normalize("看这个 [文档](https://example.com/a)"),
            "看这个 [文档](https://example.com/a)"
        );
        // 未闭合的 CQ 段按原文保留
        assert_eq!(normalize("[CQ:at,qq=all"), "[CQ:at,qq=all");
    }

    #[test]
    fn contains_tag_matches_literal_tag_only() {
        assert!(contains_tag(&format!("{TAG}开会")));
        assert!(!contains_tag("[CQ:at,qq=all]开会"));
    }

    #[test]
    fn format_date_matches_known_days() {
        assert_eq!(format_date(0), "1970-01-01");
        assert_eq!(format_date(19_723), "2024-01-01");
        assert_eq!(format_date(20_454), "2026-01-01");
    }

    #[test]
    fn normalize_is_case_insensitive() {
        // CQ 侧大小写不敏感：前缀、参数名、取值口径一致（Go 快速路径的大小写敏感问题
        // 本轮同步修正，修好后两侧均为大小写不敏感）
        assert_eq!(normalize("[cq:at,QQ=ALL]早安"), format!("{TAG}早安"));
        assert_eq!(normalize("[CQ:AT,qq=All]早安"), format!("{TAG}早安"));
        assert_eq!(
            normalize("[CQ:at, qq = Everyone ,name=x]早安"),
            format!("{TAG}早安")
        );
        // 普通 @ 不做大小写归一（数值 ID 原样）
        assert_eq!(normalize("[CQ:AT,qq=123]嗯"), "[CQ:AT,qq=123]嗯");
    }

    #[test]
    fn contains_tag_accepts_extra_whitespace() {
        // Go 的 atEveryoneTagPattern 是 <qqbot-at-everyone\s*/>
        assert!(contains_tag("<qqbot-at-everyone/>"));
        assert!(contains_tag("<qqbot-at-everyone  />"));
        assert!(contains_tag("开会<qqbot-at-everyone\n/>"));
        assert!(!contains_tag("<qqbot-at-everyone>"));
        assert!(!contains_tag("[CQ:at,qq=all]开会"));
    }

    #[test]
    fn dedupe_tag_keeps_the_first_only() {
        assert_eq!(
            dedupe_tag(&format!("{TAG} A {TAG} B")),
            format!("{TAG} A  B"),
            "重复标签删掉，正文其余字节原样保留"
        );
        // `<qqbot-at-everyone/>` 这类空白变体同样算一个标签
        assert_eq!(
            dedupe_tag("<qqbot-at-everyone/>x<qqbot-at-everyone  />"),
            "<qqbot-at-everyone/>x"
        );
        // 不是完整标签的原样保留
        assert_eq!(
            dedupe_tag("<qqbot-at-everyone>开会"),
            "<qqbot-at-everyone>开会"
        );
        assert_eq!(dedupe_tag("没有标签"), "没有标签");
    }

    #[test]
    fn at_mention_id_reads_the_qq_param() {
        assert_eq!(at_mention_id("[CQ:at,qq=123]"), Some("123"));
        assert_eq!(at_mention_id("[CQ:at,name=x,qq=123]"), Some("123"));
        assert_eq!(at_mention_id("[CQ:at,qq=all]"), Some("all"));
        assert_eq!(at_mention_id("[CQ:at]"), None);
        assert_eq!(at_mention_id("[CQ:image,file=a]"), None);
    }

    #[test]
    fn date_uses_fixed_utc_plus_eight() {
        // 2026-01-01 00:30 (+08:00) = 2025-12-31 16:30 UTC：东八区已跨日，UTC 还停在昨天
        let local_0030 = 20_453 * SECONDS_PER_DAY + 16 * 3_600 + 30 * 60;
        assert_eq!(date_at(local_0030), "2026-01-01", "东八区 00:30 必须算新的一天");

        // 2025-12-31 23:30 (+08:00) = 2025-12-31 15:30 UTC：两侧都还是 2025-12-31
        let local_2330 = 20_453 * SECONDS_PER_DAY + 15 * 3_600 + 30 * 60;
        assert_eq!(date_at(local_2330), "2025-12-31");

        // 2026-01-01 08:30 (+08:00) = 2026-01-01 00:30 UTC：固定偏移后仍是元旦
        let local_0830 = 20_454 * SECONDS_PER_DAY + 30 * 60;
        assert_eq!(date_at(local_0830), "2026-01-01");

        // 2026-01-01 07:59 (+08:00) = 2025-12-31 23:59 UTC：差 1 分钟，UTC 口径会判成昨天
        let local_0759 = 20_453 * SECONDS_PER_DAY + 23 * 3_600 + 59 * 60;
        assert_eq!(date_at(local_0759), "2026-01-01");
    }

    #[test]
    fn date_at_rolls_over_at_local_midnight() {
        // +08:00 的 1970-01-01 覆盖 UTC 1969-12-31 16:00 ~ 1970-01-01 15:59:59
        assert_eq!(date_at(0), "1970-01-01");
        assert_eq!(date_at(57_600 - 1), "1970-01-01", "本地 23:59:59 仍是当天");
        assert_eq!(date_at(57_600), "1970-01-02", "本地 00:00 必须翻页");
    }

    // ---- 限流 ----

    #[test]
    fn first_reserve_succeeds_second_same_day_is_rejected() {
        let (_dir, idmap) = store();

        let reservation = reserve(&idmap, GROUP).expect("首次 @全员 应放行");
        assert_eq!(reservation.group(), GROUP);

        let err = reserve(&idmap, GROUP).expect_err("同日第二次必须拒绝");
        assert!(matches!(err, Error::Protocol(_)), "超限应映射 1400: {err}");
        assert!(
            err.to_string().contains("每群每自然日 1 次"),
            "文案要说明官方限制: {err}"
        );
    }

    #[test]
    fn reserve_records_go_compatible_kv_shape() {
        let (_dir, idmap) = store();
        let _reservation = reserve(&idmap, GROUP).expect("首次应放行");

        let raw = idmap.read_config(SECTION, GROUP).expect("计数应已落盘");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("值必须是 JSON");
        assert_eq!(value["date"], today().as_str());
        assert_eq!(value["count"], 1);
    }

    #[test]
    fn reserve_allows_a_new_day() {
        let (_dir, idmap) = store();
        // 换成「很久以前」的日期：与今天必然不同
        idmap
            .write_config(SECTION, GROUP, r#"{"date":"1970-01-01","count":1}"#)
            .expect("写历史计数失败");

        reserve(&idmap, GROUP).expect("跨日必须放行");
    }

    #[test]
    fn rollback_releases_the_quota() {
        let (_dir, idmap) = store();
        let reservation = reserve(&idmap, GROUP).expect("首次应放行");
        reservation.rollback(&idmap);

        assert!(
            idmap.read_config(SECTION, GROUP).is_err(),
            "占位前没有记录，回滚后键必须被删掉"
        );
        reserve(&idmap, GROUP).expect("回滚后应能再次占位");
    }

    #[test]
    fn rollback_restores_the_previous_value() {
        let (_dir, idmap) = store();
        let previous = r#"{"date":"1970-01-01","count":1}"#;
        idmap
            .write_config(SECTION, GROUP, previous)
            .expect("写历史计数失败");

        let reservation = reserve(&idmap, GROUP).expect("跨日应放行");
        reservation.rollback(&idmap);

        assert_eq!(
            idmap.read_config(SECTION, GROUP).expect("旧值应写回"),
            previous
        );
    }

    #[test]
    fn corrupt_record_is_treated_as_absent() {
        let (_dir, idmap) = store();
        idmap
            .write_config(SECTION, GROUP, "not-json")
            .expect("写脏数据失败");

        reserve(&idmap, GROUP).expect("脏数据不应锁死该群");
    }

    #[test]
    fn read_failure_is_treated_as_absent() {
        // 读失败放行：存储抖动不能让该群的 @全员 永久不可用（Go GuardAtEveryone 同口径）
        assert_eq!(
            previous_usage(Err(StorageError::Corrupt("坏记录".to_owned())), GROUP),
            None
        );
        assert_eq!(
            previous_usage(Err(StorageError::NotFound("config".to_owned())), GROUP),
            None
        );
        // 读成功仍返回原值，后续照常按日期判定
        let raw = r#"{"date":"1970-01-01","count":1}"#;
        assert_eq!(
            previous_usage(Ok(raw.to_owned()), GROUP).as_deref(),
            Some(raw)
        );
    }
}
