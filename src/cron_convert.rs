use anyhow::Result;

/// Convert a standard cron expression to systemd OnCalendar format.
///
/// Cron format: minute hour day month day-of-week
/// OnCalendar format: [DayOfWeek] YYYY-MM-DD HH:MM:SS
///
/// Examples:
/// - `0 9 * * *` -> `*-*-* 09:00:00`
/// - `*/30 * * * *` -> `*-*-* *:00,30:00`
/// - `0 9 * * 1-5` -> `Mon..Fri *-*-* 09:00:00`
/// - `0,30 8-22 * * *` -> `*-*-* 08..22:00,30:00`
pub fn cron_to_oncalendar(cron_expr: &str) -> Result<String> {
    let parts: Vec<&str> = cron_expr.split_whitespace().collect();
    if parts.len() != 5 {
        anyhow::bail!(
            "Invalid cron expression: expected 5 fields, got {}",
            parts.len()
        );
    }

    let minute = parts[0];
    let hour = parts[1];
    let day = parts[2];
    let month = parts[3];
    let dow = parts[4];

    // Convert day of week
    let dow_part = convert_dow(dow)?;

    // Convert date part
    let date_part = format!("*-{}-{}", convert_month(month), convert_day(day));

    // Convert time part
    let time_part = format!("{}:{}", convert_hour(hour), convert_minute(minute));

    // Combine
    let result = if dow_part.is_empty() {
        format!("{} {}", date_part, time_part)
    } else {
        format!("{} {} {}", dow_part, date_part, time_part)
    };

    Ok(result)
}

fn convert_minute(field: &str) -> String {
    convert_field(field, 60, 2)
}

fn convert_hour(field: &str) -> String {
    convert_field(field, 24, 2)
}

fn convert_day(field: &str) -> String {
    if field == "*" {
        return "*".to_string();
    }
    convert_field(field, 31, 2)
}

fn convert_month(field: &str) -> String {
    if field == "*" {
        return "*".to_string();
    }
    convert_field(field, 12, 2)
}

fn convert_dow(field: &str) -> Result<String> {
    if field == "*" {
        return Ok(String::new());
    }

    let dow_names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

    // Handle ranges like 1-5
    if field.contains('-') && !field.contains(',') {
        let parts: Vec<&str> = field.split('-').collect();
        if parts.len() == 2 {
            let start: usize = parts[0].parse()?;
            let end: usize = parts[1].parse()?;
            if start < 7 && end < 7 {
                return Ok(format!("{}..{}", dow_names[start], dow_names[end]));
            }
        }
    }

    // Handle single day
    if let Ok(day) = field.parse::<usize>() {
        if day < 7 {
            return Ok(dow_names[day].to_string());
        }
    }

    // Handle comma-separated list
    if field.contains(',') {
        let days: Result<Vec<_>, _> = field
            .split(',')
            .map(|d| {
                d.parse::<usize>()
                    .map(|n| dow_names.get(n).copied().unwrap_or("*"))
            })
            .collect();
        return Ok(days?.join(","));
    }

    Ok(field.to_string())
}

fn convert_field(field: &str, max: u32, pad: usize) -> String {
    // Handle step values like */30
    if field.starts_with("*/") {
        if let Ok(step) = field[2..].parse::<u32>() {
            let values: Vec<String> = (0..max)
                .step_by(step as usize)
                .map(|v| format!("{:0width$}", v, width = pad))
                .collect();
            return values.join(",");
        }
    }

    // Handle ranges like 8-22
    if field.contains('-') && !field.contains(',') {
        let parts: Vec<&str> = field.split('-').collect();
        if parts.len() == 2 {
            if let (Ok(start), Ok(end)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                return format!(
                    "{:0width$}..{:0width$}",
                    start,
                    end,
                    width = pad
                );
            }
        }
    }

    // Handle comma-separated values like 0,30
    if field.contains(',') {
        let values: Vec<String> = field
            .split(',')
            .filter_map(|v| v.parse::<u32>().ok())
            .map(|v| format!("{:0width$}", v, width = pad))
            .collect();
        return values.join(",");
    }

    // Handle single values
    if let Ok(val) = field.parse::<u32>() {
        return format!("{:0width$}", val, width = pad);
    }

    // Handle wildcard
    if field == "*" {
        return "*".to_string();
    }

    field.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_daily() {
        assert_eq!(cron_to_oncalendar("0 9 * * *").unwrap(), "*-*-* 09:00");
    }

    #[test]
    fn test_every_30_min() {
        assert_eq!(
            cron_to_oncalendar("*/30 * * * *").unwrap(),
            "*-*-* *:00,30"
        );
    }

    #[test]
    fn test_weekdays() {
        assert_eq!(
            cron_to_oncalendar("0 9 * * 1-5").unwrap(),
            "Mon..Fri *-*-* 09:00"
        );
    }

    #[test]
    fn test_hour_range() {
        assert_eq!(
            cron_to_oncalendar("0,30 8-22 * * *").unwrap(),
            "*-*-* 08..22:00,30"
        );
    }
}
