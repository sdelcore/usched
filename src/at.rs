use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
use std::io::Write;
use std::process::{Command, Stdio};

use crate::store::get_data_dir;

/// Schedule a one-off job using the `at` command.
///
/// Returns the at job number.
pub fn schedule_at(
    job_id: &str,
    at_time: DateTime<Utc>,
    command: &[String],
) -> Result<String> {
    // Convert UTC to local time for at command
    let local_time: DateTime<Local> = at_time.into();
    let time_spec = local_time.format("%H:%M %Y-%m-%d").to_string();

    // Build the command to run via usched-run wrapper
    let wrapper_path = get_wrapper_path();
    let full_command = format!(
        "{} {} {}",
        wrapper_path,
        job_id,
        command
            .iter()
            .map(|s| shell_escape(s))
            .collect::<Vec<_>>()
            .join(" ")
    );

    let mut child = Command::new("at")
        .arg(&time_spec)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn 'at' command")?;

    if let Some(ref mut stdin) = child.stdin {
        writeln!(stdin, "{}", full_command)?;
    }

    let output = child.wait_with_output()?;

    // at writes to stderr even on success
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Parse job number from output like "job 5 at Thu Jan 16 14:00:00 2025"
    let job_num = parse_at_job_number(&stderr)
        .unwrap_or_else(|| "unknown".to_string());

    Ok(job_num)
}

/// Remove an at job.
pub fn remove_at(job_num: &str) -> Result<()> {
    let output = Command::new("atrm")
        .arg(job_num)
        .output()
        .context("Failed to run atrm")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Ignore "cannot find job" errors (job may have already run)
        if !stderr.contains("cannot find") {
            anyhow::bail!("atrm failed: {}", stderr);
        }
    }

    Ok(())
}

/// List pending at jobs.
pub fn list_at_jobs() -> Result<String> {
    let output = Command::new("atq")
        .output()
        .context("Failed to run atq")?;

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Parse natural datetime string into DateTime<Utc>.
///
/// Supports formats like:
/// - "tomorrow 14:00"
/// - "2024-01-20 14:00"
/// - "15:00" (today)
/// - "in 2 hours"
pub fn parse_datetime(s: &str) -> Result<DateTime<Utc>> {
    let s = s.trim().to_lowercase();
    let now = Local::now();

    // Handle "in X hours/minutes"
    if s.starts_with("in ") {
        let rest = &s[3..];
        if let Some(hours) = rest.strip_suffix(" hours").or_else(|| rest.strip_suffix(" hour")) {
            let h: i64 = hours.trim().parse()?;
            let dt = now + chrono::Duration::hours(h);
            return Ok(dt.with_timezone(&Utc));
        }
        if let Some(mins) = rest.strip_suffix(" minutes").or_else(|| rest.strip_suffix(" minute").or_else(|| rest.strip_suffix(" min"))) {
            let m: i64 = mins.trim().parse()?;
            let dt = now + chrono::Duration::minutes(m);
            return Ok(dt.with_timezone(&Utc));
        }
    }

    // Handle "tomorrow HH:MM"
    if s.starts_with("tomorrow") {
        let time_part = s.strip_prefix("tomorrow").unwrap().trim();
        let time = chrono::NaiveTime::parse_from_str(time_part, "%H:%M")
            .or_else(|_| chrono::NaiveTime::parse_from_str(time_part, "%H:%M:%S"))?;
        let tomorrow = now.date_naive() + chrono::Duration::days(1);
        let dt = tomorrow.and_time(time);
        let local_dt = Local.from_local_datetime(&dt).single()
            .context("Invalid local datetime")?;
        return Ok(local_dt.with_timezone(&Utc));
    }

    // Handle "YYYY-MM-DD HH:MM"
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M") {
        let local_dt = Local.from_local_datetime(&dt).single()
            .context("Invalid local datetime")?;
        return Ok(local_dt.with_timezone(&Utc));
    }

    // Handle "HH:MM" (today)
    if let Ok(time) = chrono::NaiveTime::parse_from_str(&s, "%H:%M") {
        let dt = now.date_naive().and_time(time);
        let mut local_dt = Local.from_local_datetime(&dt).single()
            .context("Invalid local datetime")?;
        // If time has passed, schedule for tomorrow
        if local_dt <= now {
            let tomorrow = now.date_naive() + chrono::Duration::days(1);
            let dt = tomorrow.and_time(time);
            local_dt = Local.from_local_datetime(&dt).single()
                .context("Invalid local datetime")?;
        }
        return Ok(local_dt.with_timezone(&Utc));
    }

    anyhow::bail!("Could not parse datetime: {}", s)
}

fn get_wrapper_path() -> String {
    let data_dir = get_data_dir();
    let wrapper_in_data = data_dir.join("usched-run");
    if wrapper_in_data.exists() {
        return wrapper_in_data.to_string_lossy().to_string();
    }
    "usched-run".to_string()
}

fn shell_escape(s: &str) -> String {
    if s.contains(' ') || s.contains('\'') || s.contains('"') || s.contains('$') {
        format!("'{}'", s.replace('\'', "'\\''"))
    } else {
        s.to_string()
    }
}

fn parse_at_job_number(output: &str) -> Option<String> {
    // Parse "job 5 at Thu Jan 16 14:00:00 2025"
    for line in output.lines() {
        if line.starts_with("job ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                return Some(parts[1].to_string());
            }
        }
    }
    None
}

use chrono::TimeZone;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_in_hours() {
        let now = Local::now();
        let parsed = parse_datetime("in 2 hours").unwrap();
        let delta = parsed - now.with_timezone(&Utc);
        // Allow a couple-second drift between Local::now() calls
        assert!(delta.num_seconds() >= 7195 && delta.num_seconds() <= 7205);
    }

    #[test]
    fn test_parse_in_one_hour() {
        let now = Local::now();
        let parsed = parse_datetime("in 1 hour").unwrap();
        let delta = parsed - now.with_timezone(&Utc);
        assert!(delta.num_seconds() >= 3595 && delta.num_seconds() <= 3605);
    }

    #[test]
    fn test_parse_in_minutes() {
        let now = Local::now();
        let parsed = parse_datetime("in 30 minutes").unwrap();
        let delta = parsed - now.with_timezone(&Utc);
        assert!(delta.num_seconds() >= 1795 && delta.num_seconds() <= 1805);
    }

    #[test]
    fn test_parse_in_min_alias() {
        let parsed = parse_datetime("in 5 min");
        assert!(parsed.is_ok());
    }

    #[test]
    fn test_parse_tomorrow() {
        let parsed = parse_datetime("tomorrow 14:00").unwrap();
        let local = parsed.with_timezone(&Local);
        let tomorrow = Local::now().date_naive() + chrono::Duration::days(1);
        assert_eq!(local.date_naive(), tomorrow);
        assert_eq!(local.format("%H:%M").to_string(), "14:00");
    }

    #[test]
    fn test_parse_absolute() {
        // Use a date well in the future so the test is timezone-stable
        let parsed = parse_datetime("2099-06-15 09:30").unwrap();
        let local = parsed.with_timezone(&Local);
        assert_eq!(local.format("%Y-%m-%d %H:%M").to_string(), "2099-06-15 09:30");
    }

    #[test]
    fn test_parse_hhmm_today_or_tomorrow() {
        // If "23:59" is in the future today, scheduled for today; otherwise tomorrow.
        let parsed = parse_datetime("23:59").unwrap();
        let now = Local::now();
        let scheduled = parsed.with_timezone(&Local);
        assert!(scheduled > now);
        assert_eq!(scheduled.format("%H:%M").to_string(), "23:59");
    }

    #[test]
    fn test_parse_invalid() {
        assert!(parse_datetime("garbage").is_err());
        assert!(parse_datetime("").is_err());
        assert!(parse_datetime("in two hours").is_err());
        assert!(parse_datetime("tomorrow notatime").is_err());
    }

    #[test]
    fn test_parse_at_job_number_typical() {
        let out = "warning: commands will be executed using /bin/sh\njob 17 at Thu Jan 16 14:00:00 2025\n";
        assert_eq!(parse_at_job_number(out), Some("17".to_string()));
    }

    #[test]
    fn test_parse_at_job_number_missing() {
        assert_eq!(parse_at_job_number("nothing here"), None);
    }

    #[test]
    fn test_shell_escape() {
        assert_eq!(shell_escape("plain"), "plain");
        assert_eq!(shell_escape("with space"), "'with space'");
        assert_eq!(shell_escape("$var"), "'$var'");
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }
}
