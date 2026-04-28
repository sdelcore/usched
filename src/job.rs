use chrono::{DateTime, NaiveTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub schedule: Schedule,
    pub command: Vec<String>,
    pub constraints: Constraints,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub systemd_unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_job: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Schedule {
    Once { at: DateTime<Utc> },
    Cron { expr: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Constraints {
    #[serde(default)]
    pub not_during: Vec<TimeRange>,
    #[serde(default)]
    pub only_during: Vec<TimeRange>,
    #[serde(default)]
    pub dnd_aware: bool,
    /// Auto-remove this job when the command exits with success (exit code 0)
    #[serde(default)]
    pub remove_on_success: bool,
    /// Only run after this job's last execution succeeded (job ID)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: NaiveTime,
    pub end: NaiveTime,
}

impl TimeRange {
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() != 2 {
            anyhow::bail!("Invalid time range format, expected 'HH:MM-HH:MM'");
        }
        let start = NaiveTime::parse_from_str(parts[0], "%H:%M")?;
        let end = NaiveTime::parse_from_str(parts[1], "%H:%M")?;
        Ok(TimeRange { start, end })
    }

    /// Check if a given time is within this range.
    /// Handles midnight wrap (e.g., 22:00-08:00)
    pub fn contains(&self, time: NaiveTime) -> bool {
        if self.start <= self.end {
            // Normal range (e.g., 09:00-17:00)
            time >= self.start && time < self.end
        } else {
            // Midnight wrap (e.g., 22:00-08:00)
            time >= self.start || time < self.end
        }
    }
}

impl Job {
    pub fn generate_id(name: &str) -> String {
        let suffix = uuid::Uuid::new_v4().to_string()[..6].to_string();
        format!("{}-{}", name, suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn time_range_parse_ok() {
        let r = TimeRange::parse("09:00-17:00").unwrap();
        assert_eq!(r.start, t(9, 0));
        assert_eq!(r.end, t(17, 0));
    }

    #[test]
    fn time_range_parse_invalid() {
        assert!(TimeRange::parse("9-17").is_err());
        assert!(TimeRange::parse("09:00").is_err());
        assert!(TimeRange::parse("25:00-26:00").is_err());
        assert!(TimeRange::parse("").is_err());
    }

    #[test]
    fn time_range_normal_contains() {
        let r = TimeRange::parse("09:00-17:00").unwrap();
        assert!(r.contains(t(12, 0)));
        assert!(r.contains(t(9, 0)));
        assert!(!r.contains(t(17, 0))); // end-exclusive
        assert!(!r.contains(t(8, 59)));
        assert!(!r.contains(t(18, 0)));
    }

    #[test]
    fn time_range_midnight_wrap_contains() {
        // 22:00-08:00 covers late night through early morning
        let r = TimeRange::parse("22:00-08:00").unwrap();
        assert!(r.contains(t(23, 0)));
        assert!(r.contains(t(0, 0)));
        assert!(r.contains(t(7, 59)));
        assert!(r.contains(t(22, 0)));
        assert!(!r.contains(t(8, 0))); // end-exclusive
        assert!(!r.contains(t(12, 0)));
        assert!(!r.contains(t(21, 59)));
    }

    #[test]
    fn generate_id_contains_name() {
        let id = Job::generate_id("morning-check");
        assert!(id.starts_with("morning-check-"));
        // suffix is 6 hex chars
        let suffix = id.strip_prefix("morning-check-").unwrap();
        assert_eq!(suffix.len(), 6);
    }

    #[test]
    fn generate_id_unique() {
        let a = Job::generate_id("x");
        let b = Job::generate_id("x");
        assert_ne!(a, b);
    }

    #[test]
    fn schedule_serde_roundtrip_cron() {
        let s = Schedule::Cron { expr: "0 9 * * *".into() };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"type\":\"cron\""));
        assert!(json.contains("\"expr\":\"0 9 * * *\""));
        let back: Schedule = serde_json::from_str(&json).unwrap();
        match back {
            Schedule::Cron { expr } => assert_eq!(expr, "0 9 * * *"),
            _ => panic!("wrong variant"),
        }
    }
}
