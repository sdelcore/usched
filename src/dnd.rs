use anyhow::Result;
use chrono::{Duration, Utc};

use crate::store::State;

/// Parse a duration string like "2h", "30m", "1h30m".
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

    // Handle bare numbers as minutes
    if !current_num.is_empty() {
        let mins: i64 = current_num.parse()?;
        total_minutes += mins;
    }

    if total_minutes == 0 {
        anyhow::bail!("Invalid duration: {}", s);
    }

    Ok(Duration::minutes(total_minutes))
}

/// Set DND for a given duration.
pub fn set_dnd(duration_str: &str) -> Result<()> {
    let duration = parse_duration(duration_str)?;
    let until = Utc::now() + duration;

    let mut state = State::load()?;
    state.set_dnd(until);
    state.save()?;

    println!("DND set until {}", until.format("%Y-%m-%d %H:%M:%S UTC"));
    Ok(())
}

/// Clear DND.
pub fn clear_dnd() -> Result<()> {
    let mut state = State::load()?;
    state.clear_dnd();
    state.save()?;

    println!("DND cleared");
    Ok(())
}

/// Show DND status.
pub fn show_dnd_status() -> Result<()> {
    let state = State::load()?;

    if state.is_dnd_active() {
        if let Some(until) = state.dnd_until {
            let remaining = until - Utc::now();
            let hours = remaining.num_hours();
            let mins = remaining.num_minutes() % 60;
            println!("DND active until {} ({:02}h {:02}m remaining)",
                until.format("%Y-%m-%d %H:%M:%S UTC"),
                hours, mins);
        }
    } else {
        println!("DND is not active");
    }

    Ok(())
}

/// Check if DND is currently active.
pub fn is_dnd_active() -> Result<bool> {
    let state = State::load()?;
    Ok(state.is_dnd_active())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("2h").unwrap(), Duration::hours(2));
        assert_eq!(parse_duration("30m").unwrap(), Duration::minutes(30));
        assert_eq!(parse_duration("1h30m").unwrap(), Duration::minutes(90));
        assert_eq!(parse_duration("90").unwrap(), Duration::minutes(90));
    }

    #[test]
    fn test_parse_duration_uppercase() {
        assert_eq!(parse_duration("2H").unwrap(), Duration::hours(2));
        assert_eq!(parse_duration("1H30M").unwrap(), Duration::minutes(90));
    }

    #[test]
    fn test_parse_duration_whitespace() {
        assert_eq!(parse_duration("  2h  ").unwrap(), Duration::hours(2));
    }

    #[test]
    fn test_parse_duration_zero_errors() {
        assert!(parse_duration("0").is_err());
        assert!(parse_duration("0h").is_err());
        assert!(parse_duration("0h0m").is_err());
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration("abc").is_err());
        // Bare "h" — current_num is empty when we hit 'h'; "".parse::<i64>() errors
        assert!(parse_duration("h").is_err());
    }
}
