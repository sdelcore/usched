//! The OS scheduler that owns a Job's lifecycle.
//!
//! Both recurring (cron) and one-shot jobs are dispatched via systemd user
//! timers. The single-variant [`Backend`] enum is kept for symmetry with
//! the rest of the code (and to make a future second backend a
//! compiler-enforced sweep) but currently only [`Backend::Systemd`] exists.

use anyhow::Result;

use crate::job::{Job, Schedule};
use crate::systemd;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Systemd,
}

impl Backend {
    /// Schedule a job; returns the backend handle to persist with the Schedule.
    pub fn schedule(self, job: &Job) -> Result<String> {
        match (self, &job.schedule) {
            (Backend::Systemd, Schedule::Cron { expr, .. }) => {
                systemd::create_timer(&job.id, expr, &job.command)
            }
            (Backend::Systemd, Schedule::Once { at: when, .. }) => {
                systemd::create_oneshot_timer(&job.id, *when, &job.command)
            }
        }
    }

    pub fn remove(self, handle: &str) -> Result<()> {
        match self {
            Backend::Systemd => systemd::remove_timer(handle),
        }
    }

    pub fn enable(self, handle: &str) -> Result<()> {
        match self {
            Backend::Systemd => systemd::enable_timer(handle),
        }
    }

    pub fn disable(self, handle: &str) -> Result<()> {
        match self {
            Backend::Systemd => systemd::disable_timer(handle),
        }
    }

    pub fn verify(self, handle: &str) -> bool {
        match self {
            Backend::Systemd => systemd::verify_timer(handle),
        }
    }
}

impl Schedule {
    pub fn backend(&self) -> Backend {
        match self {
            Schedule::Cron { .. } => Backend::Systemd,
            Schedule::Once { .. } => Backend::Systemd,
        }
    }

    pub fn handle(&self) -> Option<&str> {
        match self {
            Schedule::Cron { unit, .. } => unit.as_deref(),
            Schedule::Once { unit, .. } => unit.as_deref(),
        }
    }

    pub fn set_handle(&mut self, h: String) {
        match self {
            Schedule::Cron { unit, .. } => *unit = Some(h),
            Schedule::Once { unit, .. } => *unit = Some(h),
        }
    }
}
