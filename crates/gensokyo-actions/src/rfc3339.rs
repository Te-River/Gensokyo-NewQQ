//! 最小 RFC3339 工具：QQ 接口只用到「Unix 秒 ⇄ UTC 时间串」两个方向。
//!
//! 为什么不引 `time`：本 crate 的依赖白名单只有内部 crate + serde（见 [`crate`] 文档）。
//! 这里只覆盖 `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)` 一种形态，闰年/世纪规则由
//! `days_from_civil` 的整数算法保证（Howard Hinnant 的 civil-from-days 反函数），
//! 不依赖时区库，也不读本地时区——QQ 侧只要求合法 RFC3339，统一输出 `Z`。

/// 一天的秒数。
const SECS_PER_DAY: i64 = 86_400;

/// 当前 Unix 秒；系统时钟早于 1970 时返回 0（禁言到期时间按「已到期」处理）。
pub(crate) fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

/// Unix 秒 → `YYYY-MM-DDTHH:MM:SSZ`（禁言到期时间用这个形态）。
pub(crate) fn format_unix_secs(secs: i64) -> String {
    let days = secs.div_euclid(SECS_PER_DAY);
    let rest = secs.rem_euclid(SECS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// 解析 RFC3339（带偏移或 `Z`）→ Unix 秒；格式非法返回 `None`。
///
/// 调用方按中性值处理（Go 侧 `joined_at` 解析失败置 0），因此这里不区分错误种类。
pub(crate) fn parse(text: &str) -> Option<i64> {
    let (date, time) = text.trim().split_once(['T', 't'])?;

    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let (clock, offset_secs) = split_offset(time)?;
    let mut clock_parts = clock.split(':');
    let hour: i64 = clock_parts.next()?.parse().ok()?;
    let minute: i64 = clock_parts.next()?.parse().ok()?;
    // 秒可以带小数（QQ 偶有 `.000`），小数部分不参与秒级精度
    let second: i64 = clock_parts.next()?.split('.').next()?.parse().ok()?;
    if clock_parts.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let days = days_from_civil(year, month, day);
    Some(days * SECS_PER_DAY + hour * 3600 + minute * 60 + second - offset_secs)
}

/// 拆出时钟部分与偏移秒数：`Z` / `z` 为 0，否则 `±HH:MM`。
fn split_offset(time: &str) -> Option<(&str, i64)> {
    if let Some(clock) = time.strip_suffix(['Z', 'z']) {
        return Some((clock, 0));
    }
    // 偏移符号出现在秒之后：`HH:MM:SS` 已占 8 个字符，从第 9 个字符起找
    let (index, _) = time
        .char_indices()
        .skip(8)
        .find(|(_, character)| *character == '+' || *character == '-')?;
    let (clock, offset) = time.split_at(index);

    let sign = if offset.starts_with('-') { -1 } else { 1 };
    let (hours, minutes) = offset.get(1..)?.split_once(':')?;
    let hours: i64 = hours.parse().ok()?;
    let minutes: i64 = minutes.parse().ok()?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some((clock, sign * (hours * 3600 + minutes * 60)))
}

/// 公历日期 → 距 1970-01-01 的天数（month 为 `1..=12`）。
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_from_march = i64::from((month + 9) % 12);
    let day_of_year = (153 * month_from_march + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// 距 1970-01-01 的天数 → `(年, 月, 日)`，[`days_from_civil`] 的逆运算。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    (
        if month <= 2 { year + 1 } else { year },
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}

#[cfg(test)]
mod tests {
    use super::{format_unix_secs, parse};

    #[test]
    fn formats_known_instants() {
        assert_eq!(format_unix_secs(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_unix_secs(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(format_unix_secs(1_785_910_749), "2026-08-05T06:19:09Z");
        assert_eq!(format_unix_secs(-1), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn parses_utc_and_offset_forms() {
        assert_eq!(parse("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse("2023-11-14T22:13:20Z"), Some(1_700_000_000));
        assert_eq!(parse("2026-08-05T14:19:09+08:00"), Some(1_785_910_749));
        assert_eq!(parse("2026-08-05T01:19:09-05:00"), Some(1_785_910_749));
        assert_eq!(parse("2026-08-05T06:19:09.512Z"), Some(1_785_910_749));
    }

    #[test]
    fn leap_day_round_trips() {
        assert_eq!(parse("2024-02-29T12:00:00Z"), Some(1_709_208_000));
        assert_eq!(format_unix_secs(1_709_208_000), "2024-02-29T12:00:00Z");
    }

    #[test]
    fn rejects_malformed_input() {
        for text in [
            "",
            "not-a-time",
            "2026-08-05",
            "2026-13-01T00:00:00Z",
            "2026-08-05T24:00:00Z",
            "2026-08-05T00:60:00Z",
            "2026-08-05T00:00:00+24:00",
            "2026-08-05T00:00:00Z+01:00",
        ] {
            assert_eq!(parse(text), None, "{text} 应被拒绝");
        }
    }

    #[test]
    fn formatting_is_parseable_again() {
        for secs in [0, 1_000_000_000, 1_785_910_749, 2_000_000_000] {
            assert_eq!(parse(&format_unix_secs(secs)), Some(secs));
        }
    }
}
