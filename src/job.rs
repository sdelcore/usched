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
