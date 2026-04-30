//! The OS scheduler that owns a Job's lifecycle.
//!
//! A closed-set enum (no plugins) — every [`Schedule`] variant maps to
//! exactly one [`Backend`] via [`Schedule::backend`]. Adapters live in
//! `systemd.rs` and `at.rs`; this module just dispatches.
//!
//! Adding a third backend is a compiler-enforced sweep: every match here
//! and every `Schedule::backend` case must grow.

use anyhow::Result;

use crate::job::{Job, Schedule};
use crate::{at, systemd};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Systemd,
    At,
}

impl Backend {
    /// Schedule a job; returns the backend handle to persist with the Schedule.
    pub fn schedule(self, job: &Job) -> Result<String> {
        match (self, &job.schedule) {
            (Backend::Systemd, Schedule::Cron { expr, .. }) => {
                systemd::create_timer(&job.id, expr, &job.command)
            }
            (Backend::At, Schedule::Once { at: when, .. }) => {
                at::schedule_at(&job.id, *when, &job.command)
            }
            // Mismatched (Backend, Schedule) combos shouldn't occur — Schedule::backend
            // always picks the right one. This branch is defense-in-depth.
            _ => anyhow::bail!("backend/schedule mismatch"),
        }
    }

    pub fn remove(self, handle: &str) -> Result<()> {
        match self {
            Backend::Systemd => systemd::remove_timer(handle),
            Backend::At => at::remove_at(handle),
        }
    }

    pub fn enable(self, handle: &str) -> Result<()> {
        match self {
            Backend::Systemd => systemd::enable_timer(handle),
            Backend::At => {
                anyhow::bail!("cannot enable a one-shot job; remove and re-add instead")
            }
        }
    }

    pub fn disable(self, handle: &str) -> Result<()> {
        match self {
            Backend::Systemd => systemd::disable_timer(handle),
            Backend::At => {
                anyhow::bail!("cannot disable a one-shot job; remove it instead")
            }
        }
    }

    pub fn verify(self, handle: &str) -> bool {
        match self {
            Backend::Systemd => systemd::verify_timer(handle),
            // The at queue has no health concept comparable to systemd's
            // is-enabled / unit-files-present check. Treat as healthy.
            Backend::At => true,
        }
    }
}

impl Schedule {
    pub fn backend(&self) -> Backend {
        match self {
            Schedule::Cron { .. } => Backend::Systemd,
            Schedule::Once { .. } => Backend::At,
        }
    }

    pub fn handle(&self) -> Option<&str> {
        match self {
            Schedule::Cron { unit, .. } => unit.as_deref(),
            Schedule::Once { at_job, .. } => at_job.as_deref(),
        }
    }

    pub fn set_handle(&mut self, h: String) {
        match self {
            Schedule::Cron { unit, .. } => *unit = Some(h),
            Schedule::Once { at_job, .. } => *at_job = Some(h),
        }
    }
}
