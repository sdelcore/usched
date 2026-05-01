//! Parsers for user-supplied time strings.
//!
//! Three formats live here:
//! - [`parse_datetime`] — `"tomorrow 14:00"`, `"in 2 hours"`,
//!   `"2024-01-20 15:00"`, `"15:00"`. Used by `--once`.
//! - [`parse_duration`] — `"2h"`, `"30m"`, `"1h30m"`. Used by DND.
//! - [`parse_time_range`] — `"HH:MM-HH:MM"`. Used by `--not-during` /
//!   `--only-during`.
//!
//! Cron parsing/conversion is *not* here — `cron_to_oncalendar` is a
//! transformation between two machine formats, not user-input parsing.

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Local, NaiveTime, TimeZone, Utc};

use crate::job::TimeRange;

/// Parse a natural datetime string into UTC.
///
/// Supported forms:
/// - `"YYYY-MM-DD HH:MM"` / `"YYYY-MM-DD HH:MM:SS"` — absolute
/// - `"today HH:MM"` / `"today HH:MM:SS"` — today at that time
/// - `"tomorrow HH:MM"` / `"tomorrow HH:MM:SS"` — tomorrow at that time
/// - `"in N minutes"` / `"in N hours"` / `"in N days"` (singular or plural)
/// - `"HH:MM"` / `"HH:MM:SS"` — today if still future, otherwise tomorrow
///
/// Anything else returns a clear error listing the supported forms.
pub fn parse_datetime(s: &str) -> Result<DateTime<Utc>> {
    let s = s.trim().to_lowercase();
    let now = Local::now();

    if let Some(rest) = s.strip_prefix("in ") {
        if let Some(dt) = parse_in_relative(rest, now)? {
            return Ok(dt);
        }
    }

    if let Some(rest) = s.strip_prefix("today") {
        let time_part = rest.trim();
        let time = parse_hms(time_part)?;
        let dt = now.date_naive().and_time(time);
        let local_dt = Local
            .from_local_datetime(&dt)
            .single()
            .context("Invalid local datetime")?;
        return Ok(local_dt.with_timezone(&Utc));
    }

    if let Some(rest) = s.strip_prefix("tomorrow") {
        let time_part = rest.trim();
        let time = parse_hms(time_part)?;
        let tomorrow = now.date_naive() + Duration::days(1);
        let dt = tomorrow.and_time(time);
        let local_dt = Local
            .from_local_datetime(&dt)
            .single()
            .context("Invalid local datetime")?;
        return Ok(local_dt.with_timezone(&Utc));
    }

    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S") {
        let local_dt = Local
            .from_local_datetime(&dt)
            .single()
            .context("Invalid local datetime")?;
        return Ok(local_dt.with_timezone(&Utc));
    }

    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M") {
        let local_dt = Local
            .from_local_datetime(&dt)
            .single()
            .context("Invalid local datetime")?;
        return Ok(local_dt.with_timezone(&Utc));
    }

    if let Ok(time) = parse_hms(&s) {
        let dt = now.date_naive().and_time(time);
        let mut local_dt = Local
            .from_local_datetime(&dt)
            .single()
            .context("Invalid local datetime")?;
        if local_dt <= now {
            let tomorrow = now.date_naive() + Duration::days(1);
            let dt = tomorrow.and_time(time);
            local_dt = Local
                .from_local_datetime(&dt)
                .single()
                .context("Invalid local datetime")?;
        }
        return Ok(local_dt.with_timezone(&Utc));
    }

    anyhow::bail!(
        "Could not parse datetime: {:?}. Supported forms: \
         'YYYY-MM-DD HH:MM[:SS]', 'today HH:MM[:SS]', 'tomorrow HH:MM[:SS]', \
         'in N (minutes|hours|days)', 'HH:MM[:SS]'.",
        s
    )
}

/// Parse "N minutes" / "N hours" / "N days" (singular or plural). Returns
/// `Ok(None)` if the suffix doesn't match — the caller may try other forms.
fn parse_in_relative(rest: &str, now: chrono::DateTime<Local>) -> Result<Option<DateTime<Utc>>> {
    let rest = rest.trim();
    if let Some(num) = rest
        .strip_suffix(" days")
        .or_else(|| rest.strip_suffix(" day"))
    {
        let d: i64 = num.trim().parse()?;
        return Ok(Some((now + Duration::days(d)).with_timezone(&Utc)));
    }
    if let Some(num) = rest
        .strip_suffix(" hours")
        .or_else(|| rest.strip_suffix(" hour"))
    {
        let h: i64 = num.trim().parse()?;
        return Ok(Some((now + Duration::hours(h)).with_timezone(&Utc)));
    }
    if let Some(num) = rest
        .strip_suffix(" minutes")
        .or_else(|| rest.strip_suffix(" minute").or_else(|| rest.strip_suffix(" min")))
    {
        let m: i64 = num.trim().parse()?;
        return Ok(Some((now + Duration::minutes(m)).with_timezone(&Utc)));
    }
    Ok(None)
}

/// Accept `HH:MM` or `HH:MM:SS`.
fn parse_hms(s: &str) -> Result<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M:%S")
        .or_else(|_| NaiveTime::parse_from_str(s, "%H:%M"))
        .with_context(|| format!("Invalid time {:?}, expected HH:MM or HH:MM:SS", s))
}

/// Parse a duration string like `"2h"`, `"30m"`, `"1h30m"`. Bare numbers
/// are treated as minutes (`"90"` → 90 minutes). Zero is rejected.
pub fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.trim().to_lowercase();
    let mut total_minutes: i64 = 0;
    let mut current_num = String::new();

    for c in s.chars() {
        if c.is_ascii_digit() {
            current_num.push(c);
        } else if c == 'h' {
            let hours: i64 = current_num.parse()?;
            total_minutes += hours * 60;
            current_num.clear();
        } else if c == 'm' {
            let mins: i64 = current_num.parse()?;
            total_minutes += mins;
            current_num.clear();
        }
    }

    if !current_num.is_empty() {
        let mins: i64 = current_num.parse()?;
        total_minutes += mins;
    }

    if total_minutes == 0 {
        anyhow::bail!("Invalid duration: {}", s);
    }

    Ok(Duration::minutes(total_minutes))
}

/// Parse `"HH:MM-HH:MM"` into a [`TimeRange`]. The end is exclusive; if
/// `end <= start` the range wraps around midnight.
pub fn parse_time_range(s: &str) -> Result<TimeRange> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid time range format, expected 'HH:MM-HH:MM'");
    }
    let start = NaiveTime::parse_from_str(parts[0], "%H:%M")?;
    let end = NaiveTime::parse_from_str(parts[1], "%H:%M")?;
    Ok(TimeRange { start, end })
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- parse_datetime --

    #[test]
    fn datetime_in_hours() {
        let now = Local::now();
        let parsed = parse_datetime("in 2 hours").unwrap();
        let delta = parsed - now.with_timezone(&Utc);
        assert!(delta.num_seconds() >= 7195 && delta.num_seconds() <= 7205);
    }

    #[test]
    fn datetime_in_one_hour() {
        let now = Local::now();
        let parsed = parse_datetime("in 1 hour").unwrap();
        let delta = parsed - now.with_timezone(&Utc);
        assert!(delta.num_seconds() >= 3595 && delta.num_seconds() <= 3605);
    }

    #[test]
    fn datetime_in_minutes() {
        let now = Local::now();
        let parsed = parse_datetime("in 30 minutes").unwrap();
        let delta = parsed - now.with_timezone(&Utc);
        assert!(delta.num_seconds() >= 1795 && delta.num_seconds() <= 1805);
    }

    #[test]
    fn datetime_in_min_alias() {
        assert!(parse_datetime("in 5 min").is_ok());
    }

    #[test]
    fn datetime_tomorrow() {
        let parsed = parse_datetime("tomorrow 14:00").unwrap();
        let local = parsed.with_timezone(&Local);
        let tomorrow = Local::now().date_naive() + Duration::days(1);
        assert_eq!(local.date_naive(), tomorrow);
        assert_eq!(local.format("%H:%M").to_string(), "14:00");
    }

    #[test]
    fn datetime_absolute() {
        let parsed = parse_datetime("2099-06-15 09:30").unwrap();
        let local = parsed.with_timezone(&Local);
        assert_eq!(local.format("%Y-%m-%d %H:%M").to_string(), "2099-06-15 09:30");
    }

    #[test]
    fn datetime_hhmm_today_or_tomorrow() {
        let parsed = parse_datetime("23:59").unwrap();
        let now = Local::now();
        let scheduled = parsed.with_timezone(&Local);
        assert!(scheduled > now);
        assert_eq!(scheduled.format("%H:%M").to_string(), "23:59");
    }

    #[test]
    fn datetime_in_days() {
        let now = Local::now();
        let parsed = parse_datetime("in 3 days").unwrap();
        let delta = parsed - now.with_timezone(&Utc);
        let expected = 3 * 24 * 3600;
        assert!(
            delta.num_seconds() >= expected - 5 && delta.num_seconds() <= expected + 5,
            "delta was {}",
            delta.num_seconds()
        );
    }

    #[test]
    fn datetime_in_one_day_singular() {
        assert!(parse_datetime("in 1 day").is_ok());
    }

    #[test]
    fn datetime_today() {
        let parsed = parse_datetime("today 23:59").unwrap();
        let local = parsed.with_timezone(&Local);
        assert_eq!(local.date_naive(), Local::now().date_naive());
        assert_eq!(local.format("%H:%M").to_string(), "23:59");
    }

    #[test]
    fn datetime_today_with_seconds() {
        let parsed = parse_datetime("today 23:59:30").unwrap();
        let local = parsed.with_timezone(&Local);
        assert_eq!(local.format("%H:%M:%S").to_string(), "23:59:30");
    }

    #[test]
    fn datetime_tomorrow_with_seconds() {
        let parsed = parse_datetime("tomorrow 14:00:30").unwrap();
        let local = parsed.with_timezone(&Local);
        assert_eq!(local.format("%H:%M:%S").to_string(), "14:00:30");
    }

    #[test]
    fn datetime_absolute_with_seconds() {
        let parsed = parse_datetime("2099-06-15 09:30:45").unwrap();
        let local = parsed.with_timezone(&Local);
        assert_eq!(
            local.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2099-06-15 09:30:45"
        );
    }

    #[test]
    fn datetime_hhmmss_bare() {
        // 23:59:30 should parse the same as 23:59 logic — today if future,
        // else tomorrow. Either way, it should round-trip the seconds.
        let parsed = parse_datetime("23:59:30").unwrap();
        let local = parsed.with_timezone(&Local);
        assert_eq!(local.format("%H:%M:%S").to_string(), "23:59:30");
    }

    #[test]
    fn datetime_invalid() {
        assert!(parse_datetime("garbage").is_err());
        assert!(parse_datetime("").is_err());
        assert!(parse_datetime("in two hours").is_err());
        assert!(parse_datetime("tomorrow notatime").is_err());
        assert!(parse_datetime("today notatime").is_err());
        assert!(parse_datetime("in 5 fortnights").is_err());
    }

    #[test]
    fn datetime_error_lists_supported_forms() {
        let err = parse_datetime("nonsense").unwrap_err().to_string();
        assert!(err.contains("YYYY-MM-DD"), "err was: {}", err);
        assert!(err.contains("tomorrow"), "err was: {}", err);
        assert!(err.contains("in N"), "err was: {}", err);
    }

    // -- parse_duration --

    #[test]
    fn duration_basic() {
        assert_eq!(parse_duration("2h").unwrap(), Duration::hours(2));
        assert_eq!(parse_duration("30m").unwrap(), Duration::minutes(30));
        assert_eq!(parse_duration("1h30m").unwrap(), Duration::minutes(90));
        assert_eq!(parse_duration("90").unwrap(), Duration::minutes(90));
    }

    #[test]
    fn duration_uppercase() {
        assert_eq!(parse_duration("2H").unwrap(), Duration::hours(2));
        assert_eq!(parse_duration("1H30M").unwrap(), Duration::minutes(90));
    }

    #[test]
    fn duration_whitespace() {
        assert_eq!(parse_duration("  2h  ").unwrap(), Duration::hours(2));
    }

    #[test]
    fn duration_zero_errors() {
        assert!(parse_duration("0").is_err());
        assert!(parse_duration("0h").is_err());
        assert!(parse_duration("0h0m").is_err());
    }

    #[test]
    fn duration_invalid() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("h").is_err());
    }

    // -- parse_time_range --

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn time_range_ok() {
        let r = parse_time_range("09:00-17:00").unwrap();
        assert_eq!(r.start, t(9, 0));
        assert_eq!(r.end, t(17, 0));
    }

    #[test]
    fn time_range_invalid() {
        assert!(parse_time_range("9-17").is_err());
        assert!(parse_time_range("09:00").is_err());
        assert!(parse_time_range("25:00-26:00").is_err());
        assert!(parse_time_range("").is_err());
    }
}
