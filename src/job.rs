use chrono::{DateTime, NaiveTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub schedule: Schedule,
    pub command: Vec<String>,
    pub constraints: Constraints,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
}

/// How and when a Job fires. The handle issued by the owning [`Backend`]
/// (the systemd unit name or the `at` job number) lives inside the variant
/// so it cannot drift out of sync with the schedule kind.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Schedule {
    Once {
        at: DateTime<Utc>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at_job: Option<String>,
    },
    Cron {
        expr: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
    },
}

fn default_created_by() -> String {
    "user".to_string()
}

/// Custom deserializer for [`Job`] that absorbs the pre-Backend layout where
/// the handle lived as `Job::systemd_unit` / `Job::at_job` rather than inside
/// the [`Schedule`] variant. New writes use the inner-handle layout; old
/// `jobs.json` files still load and silently migrate on the next `save()`.
impl<'de> Deserialize<'de> for Job {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            id: String,
            name: String,
            schedule: RawSchedule,
            command: Vec<String>,
            #[serde(default)]
            constraints: Constraints,
            enabled: bool,
            created_at: DateTime<Utc>,
            #[serde(default = "default_created_by")]
            created_by: String,
            #[serde(default)]
            systemd_unit: Option<String>,
            #[serde(default)]
            at_job: Option<String>,
        }

        #[derive(Deserialize)]
        #[serde(tag = "type", rename_all = "snake_case")]
        enum RawSchedule {
            Once {
                at: DateTime<Utc>,
                #[serde(default)]
                at_job: Option<String>,
            },
            Cron {
                expr: String,
                #[serde(default)]
                unit: Option<String>,
            },
        }

        let r = Raw::deserialize(d)?;
        let schedule = match r.schedule {
            RawSchedule::Once { at, at_job } => Schedule::Once {
                at,
                at_job: at_job.or(r.at_job),
            },
            RawSchedule::Cron { expr, unit } => Schedule::Cron {
                expr,
                unit: unit.or(r.systemd_unit),
            },
        };
        Ok(Job {
            id: r.id,
            name: r.name,
            schedule,
            command: r.command,
            constraints: r.constraints,
            enabled: r.enabled,
            created_at: r.created_at,
            created_by: r.created_by,
        })
    }
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

    fn tr(start: (u32, u32), end: (u32, u32)) -> TimeRange {
        TimeRange { start: t(start.0, start.1), end: t(end.0, end.1) }
    }

    #[test]
    fn time_range_normal_contains() {
        let r = tr((9, 0), (17, 0));
        assert!(r.contains(t(12, 0)));
        assert!(r.contains(t(9, 0)));
        assert!(!r.contains(t(17, 0))); // end-exclusive
        assert!(!r.contains(t(8, 59)));
        assert!(!r.contains(t(18, 0)));
    }

    #[test]
    fn time_range_midnight_wrap_contains() {
        let r = tr((22, 0), (8, 0));
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
        let s = Schedule::Cron { expr: "0 9 * * *".into(), unit: None };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"type\":\"cron\""));
        assert!(json.contains("\"expr\":\"0 9 * * *\""));
        // unit is None → omitted via skip_serializing_if
        assert!(!json.contains("\"unit\""));
        let back: Schedule = serde_json::from_str(&json).unwrap();
        match back {
            Schedule::Cron { expr, unit } => {
                assert_eq!(expr, "0 9 * * *");
                assert_eq!(unit, None);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn job_deserializes_legacy_systemd_unit_into_schedule() {
        // Pre-Backend layout: handle lived at Job::systemd_unit
        let json = r#"{
            "id": "j-1",
            "name": "j",
            "schedule": {"type": "cron", "expr": "0 9 * * *"},
            "command": ["true"],
            "constraints": {},
            "enabled": true,
            "created_at": "2026-01-01T00:00:00Z",
            "created_by": "user",
            "systemd_unit": "usched-j-1"
        }"#;
        let job: Job = serde_json::from_str(json).unwrap();
        match job.schedule {
            Schedule::Cron { expr, unit } => {
                assert_eq!(expr, "0 9 * * *");
                assert_eq!(unit.as_deref(), Some("usched-j-1"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn job_deserializes_legacy_at_job_into_schedule() {
        let json = r#"{
            "id": "o-1",
            "name": "o",
            "schedule": {"type": "once", "at": "2099-01-01T00:00:00Z"},
            "command": ["true"],
            "constraints": {},
            "enabled": true,
            "created_at": "2026-01-01T00:00:00Z",
            "created_by": "user",
            "at_job": "42"
        }"#;
        let job: Job = serde_json::from_str(json).unwrap();
        match job.schedule {
            Schedule::Once { at_job, .. } => {
                assert_eq!(at_job.as_deref(), Some("42"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn job_deserializes_new_layout_handle_inside_schedule() {
        let json = r#"{
            "id": "j-2",
            "name": "j",
            "schedule": {"type": "cron", "expr": "0 9 * * *", "unit": "usched-j-2"},
            "command": ["true"],
            "constraints": {},
            "enabled": true,
            "created_at": "2026-01-01T00:00:00Z",
            "created_by": "user"
        }"#;
        let job: Job = serde_json::from_str(json).unwrap();
        match job.schedule {
            Schedule::Cron { unit, .. } => {
                assert_eq!(unit.as_deref(), Some("usched-j-2"));
            }
            _ => panic!("wrong variant"),
        }
    }
}
