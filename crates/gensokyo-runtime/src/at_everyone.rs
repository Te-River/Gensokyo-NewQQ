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
//! | `[CQ:at , qq = all]` | 同上（`at` 后与键值两侧允许空白，Go 正则的 `\s*` 同口径） |
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
//! 两侧统一按**固定 UTC+08:00** 切自然日（[`date_at`]）：Go 用 `time.FixedZone("CST", 8*3600)`
//! （`handlers/at_everyone.go:40-43`），Rust 侧写死偏移而不读系统时区，
//! 容器 `TZ=UTC` 时两侧不会在跨零点前后各判一天。

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

/// CQ `at` 段的起始标记 `[CQ:`；比较时大小写不敏感（Go 的 `atEveryoneCQPattern` 带 `(?i)`）。
/// 它与段名之间可隔词法切分层空白（`[CQ: at,qq=1]` 同样认，与 `cqcode::parse` 及 Go
/// `cqAtSegRe` 的 `\s*` 口径一致）。
const CQ_OPEN: &str = "[CQ:";

/// CQ `at` 段的段名（大小写不敏感）；其后是 `,`（带参数）或 `]`（裸 at 段），中间允许空白。
const AT_NAME: &str = "at";

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
/// 只做字面子串替换，不重新转义，因此对 markdown 内容也是安全的。大小写与空白口径与 Go
/// 渲染层同源的 `cqAtSegRe`（`(?i)\[CQ:\s*at\s*((?:,[^\]]*)?)\]`）一致
/// （`[cq:at,QQ=ALL]`、`[CQ:at , qq = all]`、`[CQ: at,qq=all]` 同样归位）。
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

/// 统一空白契约（与 `gensokyo_protocol::cqcode` 的 CQ 解析同一集合）：空格 / 制表 `\t` /
/// 换行 `\n` / 换页 `\f` / 回车 `\r`——即 Go 正则 `\s` = {空格, `\t`, `\n`, `\f`, `\r`}；
/// **不含**纵向制表 `\v`，不含全角空格（U+3000）等 Unicode 空白。
#[inline]
fn is_cq_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0C' | '\r')
}

/// 去掉首部词法切分层空白（Go `\[CQ:at\s*`、`<qqbot-at-everyone\s*/>` 的口径）。
/// 只用于切分位置；键与值的 trim 走 `str::trim`（Go `strings.TrimSpace`），见 [`at_mention_id`]。
fn trim_cq_start(text: &str) -> &str {
    text.trim_start_matches(is_cq_space)
}

/// `text[index..]`（`index` 处必须是 `[`）是否为 `[CQ:<空白>at` 形态；是则返回 `at` 之后的切片。
///
/// `[CQ:` 与 `at` 之间、以及 `at` 之后到 `,`/`]` 之前的空白都走统一切分口径
/// （[`trim_cq_start`]）；`at` 之后的守卫（`,`/`]`/结束）由调用方按各自形态判断。
fn at_after_cq_open(text: &str, index: usize) -> Option<&str> {
    let head = text.get(index..index + CQ_OPEN.len())?;
    if !head.eq_ignore_ascii_case(CQ_OPEN) {
        return None;
    }
    // `[CQ:` 之后先跳空白再比段名：`[CQ: at,qq=1]` 与 `[CQ:at,qq=1]` 同形态
    let name = trim_cq_start(text.get(index + CQ_OPEN.len()..)?);
    if !name.get(..AT_NAME.len())?.eq_ignore_ascii_case(AT_NAME) {
        return None;
    }
    Some(&name[AT_NAME.len()..])
}

/// 查找下一个 CQ `at` 段的起始字节下标（大小写不敏感）。
///
/// `[CQ:` 与 `at` 之间可隔空白（`[CQ: at,qq=1]` 与紧凑写法同口径，`cqcode::parse` 与
/// Go `cqAtSegRe` 都认，漏判会让 @全员 静默丢、缺 `qq` 逃过 `1400`）；`at` 之后是 `,`
/// （带参数）或 `]`（裸 at 段），**允许中间隔任意空白**（`[CQ:at ,qq=1]`、`[CQ:at ]`
/// 同样算 at 段）；`[CQ:avatar]`、`[CQ:atx]`、`[CQ: atx]` 这类形似的码仍不能误判
/// （误判会把普通文本当 at 段拒成 `1400`）。
/// **缺 `qq` 的 at 段同样算命中**——调用方要看见它才能按各自口径处理（markdown 报错、
/// 文本透传）。
///
/// 供文本 / markdown 渲染复用：两处都只替换 at 段，其余字节必须原样保留。
#[must_use]
pub fn find_at_code(text: &str, from: usize) -> Option<usize> {
    let mut at = from;
    while let Some(found) = text[at..].find('[') {
        let index = at + found;
        // 前缀之后允许空白（统一空白契约，见 [`is_cq_space`]）：`[CQ:at ,qq=1]` / `[CQ:at ]` /
        // `[CQ: at,qq=1]`（`[CQ:` 与 `at` 之间）也是 at 段
        if at_after_cq_open(text, index)
            .is_some_and(|rest| trim_cq_start(rest).starts_with([',', ']']))
        {
            return Some(index);
        }
        at = index + 1;
    }
    None
}

/// 取 `[CQ:at,qq=…]` 里的 `qq` 值（键名大小写不敏感，值已 trim）；不是完整的 at 码返回 `None`。
///
/// `[CQ:` 与 `at` 之间、`at` 之后与键名、取值两侧都允许空白（`[CQ: at , qq = ALL]`），与 Go
/// `\[CQ:at\s*,\s*qq\s*=` 及 `cqAtSegRe` 的段名前 `\s*` 同口径。
/// 裸 at 段（`[CQ:at]` / `[CQ:at ]`）与缺 `qq` 的形态（`[CQ:at,foo]`）都返回 `None`，
/// `[CQ:at,qq=]` 返回 `Some("")`：调用方据此区分「没有 qq」与「qq 为空值」。
#[must_use]
pub fn at_mention_id(code: &str) -> Option<&str> {
    let stripped = code.strip_suffix(']')?;
    // `[CQ:` 与 `at` 之间的空白、`at` 之后的空白都先按统一契约跳过（Go `\[CQ:\s*(?i:at)\s*,`
    // 同口径）；随后只能是 `,`（带参数）或直接结束（裸 at 段）：
    // `[CQ:atx,…]` / `[CQ: at x]` 不是 at 段（与 find_at_code 同口径）
    let rest = trim_cq_start(at_after_cq_open(stripped, 0)?);
    let params = match rest.strip_prefix(',') {
        Some(params) => params,
        None if rest.is_empty() => "",
        None => return None,
    };
    params.split(',').find_map(|param| {
        let (key, value) = param.split_once('=')?;
        // 键/值首尾 trim 用 Unicode 语义（`str::trim`），对偶 Go 按键取值路径的
        // `strings.TrimSpace`（`handlers/message_parser.go:3130-3141` `cqCodeParam`）：
        // `\v`、全角空格（U+3000）、NBSP 都吞。`[CQ:at` 之后与 `,` 两侧的跳空白仍是
        // ASCII 五元集（[`trim_cq_start`]），两层各司其职
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
        // `/>` 前的空白用统一契约集合（Go `\s`，见 [`is_cq_space`]）
        if trim_cq_start(&text[end..]).starts_with("/>") {
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
        // `/>` 前的空白用统一契约集合（与 contains_tag 同一口径）
        let trimmed = trim_cq_start(rest);
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
    EVERYONE_IDS.iter().any(|id| value.eq_ignore_ascii_case(id))
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
            None => idmap.delete_config(SECTION, &self.group).map(|_removed| ()),
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
/// 偏移写死而不读系统时区：容器默认 `TZ=UTC` 时若按本地时区算，会和 Go 侧（同样固定
/// +08:00）在本地 00:00~08:00 之间各判一天，同一条群昨天发过今天却放行。
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
        assert_eq!(normalize("[CQ:at,qq=everyone]早安"), format!("{TAG}早安"));
        // 带额外参数（顺序无关）也认
        assert_eq!(normalize("[CQ:at,qq=all,name=全体]"), TAG.to_owned());
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
    fn normalize_accepts_whitespace_before_params() {
        // `at` 后与键值两侧的空白不影响归位（Go 正则 `\[CQ:at\s*,\s*qq\s*=`）
        assert_eq!(normalize("[CQ:at ,qq=all]开会"), format!("{TAG}开会"));
        assert_eq!(normalize("[CQ:at , qq = ALL]开会"), format!("{TAG}开会"));
        assert_eq!(
            normalize("[CQ:at  , qq = everyone ]开会"),
            format!("{TAG}开会")
        );
        // 空白形态的普通 @ 与裸 at 段不归位
        assert_eq!(normalize("[CQ:at ,qq=123]嗯"), "[CQ:at ,qq=123]嗯");
        assert_eq!(normalize("[CQ:at ]嗯"), "[CQ:at ]嗯");
        // `[CQ:` 之后、`at` 之前的空白同样归位（与 cqcode::parse / Go cqAtSegRe 同口径）
        assert_eq!(normalize("[CQ: at,qq=all]开会"), format!("{TAG}开会"));
        assert_eq!(normalize("[CQ: at , qq = ALL]开会"), format!("{TAG}开会"));
        assert_eq!(normalize("[cq: at,qq=everyone]开会"), format!("{TAG}开会"));
        assert_eq!(normalize("[CQ: at ,qq=123]嗯"), "[CQ: at ,qq=123]嗯");
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
        assert_eq!(at_mention_id("[CQ:at,foo]"), None, "裸参数不是 qq");
        assert_eq!(
            at_mention_id("[CQ:at,qq=]"),
            Some(""),
            "空值与没有 qq 可区分"
        );
        assert_eq!(at_mention_id("[CQ:image,file=a]"), None);
        assert_eq!(at_mention_id("[CQ:avatar]"), None, "形似的码不是 at 段");
        assert_eq!(at_mention_id("[CQ:atx,qq=1]"), None, "形似的码不是 at 段");
    }

    #[test]
    fn at_mention_id_allows_whitespace_before_params() {
        // `at` 与 `,` 之间、键名/取值两侧的空白都要照样取参（Go `\[CQ:at\s*,\s*qq\s*=`）
        assert_eq!(at_mention_id("[CQ:at ,qq=1]"), Some("1"));
        assert_eq!(at_mention_id("[CQ:at  ,qq=123]"), Some("123"));
        assert_eq!(at_mention_id("[CQ:at , qq = ALL]"), Some("ALL"));
        assert_eq!(at_mention_id("[CQ:at ,qq=all]"), Some("all"));
        // 空白之后直接结束或只有不成对的参数：仍然是缺 qq
        assert_eq!(at_mention_id("[CQ:at ]"), None, "空白后面直接结束：缺 qq");
        assert_eq!(
            at_mention_id("[CQ:at ,foo]"),
            None,
            "空白形态的裸参数不是 qq"
        );
        // 支持空白不能扩大误判面
        assert_eq!(at_mention_id("[CQ:at x]"), None);
        assert_eq!(at_mention_id("[CQ:avatar]"), None);
        // `[CQ:` 与 `at` 之间的空白同样可跳（Go `cqAtSegRe` 段名前 `\s*`）
        assert_eq!(at_mention_id("[CQ: at,qq=1]"), Some("1"));
        assert_eq!(at_mention_id("[CQ: at , qq = ALL]"), Some("ALL"));
        assert_eq!(at_mention_id("[CQ: at,foo]"), None, "段名前空白的裸参数不是 qq");
        assert_eq!(at_mention_id("[CQ: at ]"), None, "段名前空白的裸 at 段：缺 qq");
        assert_eq!(at_mention_id("[CQ: at x]"), None);
        assert_eq!(at_mention_id("[CQ: atx,qq=1]"), None);
    }

    #[test]
    fn find_at_code_matches_bare_at_segment() {
        // 裸 at 段与缺 qq 的形态都要命中：调用方靠它把「缺 qq」判成 1400 / 透传
        assert_eq!(find_at_code("[CQ:at]", 0), Some(0));
        assert_eq!(find_at_code("x[CQ:at,foo]", 0), Some(1));
        assert_eq!(find_at_code("[cq:at,qq=all]", 0), Some(0), "大小写不敏感");
        // 命中后仍可继续向后找
        assert_eq!(find_at_code("a[CQ:at]b[CQ:at,qq=1]", 2), Some(9));
    }

    #[test]
    fn find_at_code_skips_lookalike_codes() {
        // `[CQ:at` 后必须是 `,` 或 `]`（中间允许空白）：形似的码不能被当成 at 段
        assert_eq!(find_at_code("[CQ:avatar]", 0), None);
        assert_eq!(find_at_code("[CQ:atx]", 0), None);
        assert_eq!(find_at_code("[CQ:atom,qq=1]", 0), None);
        assert_eq!(find_at_code("没有 CQ 码", 0), None);
    }

    #[test]
    fn find_at_code_allows_whitespace_before_params() {
        // `at` 与 `,` / `]` 之间隔空白同样是 at 段：漏判会让 markdown 原样发出 CQ 码
        assert_eq!(find_at_code("[CQ:at ,qq=1]", 0), Some(0));
        assert_eq!(find_at_code("[CQ:at ]", 0), Some(0));
        assert_eq!(find_at_code("[CQ:at  ,qq=all]", 0), Some(0));
        // 空白之后不是 `,` / `]` 的仍不算 at 段
        assert_eq!(find_at_code("[CQ:at x]", 0), None);
        assert_eq!(find_at_code("[CQ:avatar]", 0), None);
        assert_eq!(find_at_code("[CQ:atx]", 0), None);
    }

    #[test]
    fn find_at_code_allows_whitespace_after_cq_prefix() {
        // `[CQ:` 之后、`at` 之前可隔空白（`[CQ: at,qq=1]`）：`cqcode::parse` 与 Go `cqAtSegRe`
        // 都认这形态。漏判的代价在渲染层：@全员 静默丢、未绑定 qq 既不 1400 也不渲染（G2 回归）
        assert_eq!(find_at_code("[CQ: at,qq=1]", 0), Some(0));
        assert_eq!(find_at_code("[CQ: at ,qq=all]", 0), Some(0));
        assert_eq!(find_at_code("[CQ: at ]", 0), Some(0), "段名前空白的裸 at 段");
        assert_eq!(find_at_code("x[CQ: at,qq=1]", 0), Some(1));
        // 段名前空白与大小写可叠加（`[cq:` 现状保持大小写不敏感）
        assert_eq!(find_at_code("[cq: at,qq=all]", 0), Some(0));
        assert_eq!(find_at_code("[CQ: AT,qq=1]", 0), Some(0));
        // 段名前空白不得扩大误判面：形似的码照旧不是 at 段
        assert_eq!(find_at_code("[CQ: atx]", 0), None);
        assert_eq!(find_at_code("[CQ: avatar]", 0), None);
        assert_eq!(find_at_code("[CQ: atom,qq=1]", 0), None);
        assert_eq!(find_at_code("[CQ: at x]", 0), None);
        assert_eq!(find_at_code("[CQ: ]", 0), None, "跳完空白没有 at 段名");
    }

    #[test]
    fn whitespace_after_cq_prefix_uses_ascii_split_set() {
        // `[CQ:` 与 `at` 之间的空白同样走统一词法切分契约（ASCII 五元集）：`\f` 等跳过，
        // `\v` 与全角空格（U+3000）不算空白不跳过（键/值 trim 层的 Unicode 语义不受影响）
        assert_eq!(find_at_code("[CQ:\u{0C}at ,qq=1]", 0), Some(0));
        assert_eq!(find_at_code("[CQ:\t\n\r at,qq=1]", 0), Some(0));
        assert_eq!(find_at_code("[CQ:\u{0B}at,qq=1]", 0), None);
        assert_eq!(find_at_code("[CQ:\u{3000}at,qq=1]", 0), None);
        assert_eq!(
            at_mention_id("[CQ:\u{0C}at,\u{0C}qq = 1]"),
            Some("1"),
            "段名前空白与 `at`/`,` 两侧空白同码并存"
        );
    }

    /// 空白契约用例（两层）：切分位置用 Go 正则 `\s` 的 ASCII 五元集，`\f` 计入
    /// （空格/`\t`/`\n`/`\f`/`\r`）、`\v` 与全角空格（U+3000）不算空白不跳过；
    /// 键/值首尾 trim 用 Unicode（Go `strings.TrimSpace`），`\v`、U+3000 一并吞。
    #[test]
    fn at_whitespace_contract_split_ascii_value_trim_unicode() {
        // `\f`（换页）是空白：段名后、`,` 两侧、键值两侧统一 trim
        assert_eq!(find_at_code("[CQ:at\u{0C} ,\u{0C}qq = 1]", 0), Some(0));
        assert_eq!(
            at_mention_id("[CQ:at\u{0C} ,\u{0C}qq\u{0C}=\u{0C}123\u{0C}]"),
            Some("123")
        );
        // ---- 词法切分层：ASCII 五元集（对 Go `\[CQ:at\s*`）----
        // `\v` 不是空白：段名后出现就不再是 at 段形态
        assert_eq!(find_at_code("[CQ:at\u{0B},qq=1]", 0), None);
        assert_eq!(at_mention_id("[CQ:at\u{0B} ,qq=1]"), None);
        // 全角空格在切分位置不是空白：段名后出现同样不认
        assert_eq!(find_at_code("[CQ:at\u{3000},qq=1]", 0), None);
        // ---- 键/值 trim 层：Unicode（对 Go `cqCodeParam` 的 `strings.TrimSpace`）----
        // 值首尾的 U+3000 被吞（Go 按键取值路径全部 TrimSpace）
        assert_eq!(at_mention_id("[CQ:at,qq=\u{3000}1]"), Some("1"));
        // 键两侧同理：U+3000 包裹的 `qq` 仍取到值
        assert_eq!(
            at_mention_id("[CQ:at,\u{3000}qq\u{3000}=\u{3000}123]"),
            Some("123")
        );
        // `\v` 同属 Unicode 空白：键值两侧一并吞
        assert_eq!(
            at_mention_id("[CQ:at,\u{0B}qq\u{0B}=\u{0B}45\u{0B}]"),
            Some("45")
        );
    }

    #[test]
    fn date_uses_fixed_utc_plus_eight() {
        // 2026-01-01 00:30 (+08:00) = 2025-12-31 16:30 UTC：东八区已跨日，UTC 还停在昨天
        let local_0030 = 20_453 * SECONDS_PER_DAY + 16 * 3_600 + 30 * 60;
        assert_eq!(
            date_at(local_0030),
            "2026-01-01",
            "东八区 00:30 必须算新的一天"
        );

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
