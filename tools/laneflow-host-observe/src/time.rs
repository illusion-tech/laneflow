//! UTC 时刻格式化（RFC 3339，毫秒精度）。

use std::time::{SystemTime, UNIX_EPOCH};

/// 当前 UTC 时刻，如 `2026-11-16T03:04:05.678Z`。系统时钟早于 1970 年时返回纪元时刻。
pub(crate) fn now_utc() -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format_unix_millis(elapsed.as_millis())
}

/// Unix 毫秒数格式化为 RFC 3339 UTC。
pub(crate) fn format_unix_millis(millis: u128) -> String {
    let seconds = millis / 1000;
    let days = i64::try_from(seconds / 86_400).unwrap_or(i64::MAX);
    let second_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        second_of_day / 3600,
        second_of_day / 60 % 60,
        second_of_day % 60,
        millis % 1000
    )
}

/// 自 1970-01-01 起的天数换算为公历年月日（Howard Hinnant 算法）。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = u32::try_from(day_of_year - (153 * month_index + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    })
    .unwrap_or(1);
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}
