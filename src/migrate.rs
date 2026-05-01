//! One-shot migration from the legacy `at(1)` backend to systemd timers.
//!
//! Older versions of usched dispatched `--once` jobs through `at(1)`/`atd`
//! and stored the resulting at-queue job number in `Schedule::Once::at_job`.
//! Current versions back one-shots with systemd user timers and store the
//! unit name in `Schedule::Once::unit`.
//!
//! `usched migrate-from-at` walks `jobs.json` and, for each `Once` job that
//! still has an `at_job` handle (and no systemd `unit`):
//!
//! 1. If the fire time is in the past, drop the entry — it can't be
//!    re-scheduled and there's nothing meaningful to do.
//! 2. Otherwise, create a systemd one-shot timer for the same fire time and
//!    record the new `unit` handle. Then best-effort `atrm` the old at-queue
//!    entry; an `atd`-down host will fail this with a warning rather than
//!    bail (matches the warn-and-continue logic in sdelcore/usched#5).
//!
//! The migration is idempotent: jobs without an `at_job` handle (or with one
//! that's already been migrated to a `unit`) are left alone.

use anyhow::Result;
use std::process::Command;

use crate::backend::Backend;
use crate::job::Schedule;
use crate::store::JobStore;

/// Run the migration in place. Returns `(migrated, dropped, kept)`:
/// - `migrated`: legacy `at_job` jobs that now have a working systemd timer
/// - `dropped`: legacy `at_job` jobs whose fire time was in the past
/// - `kept`:    jobs that were already on systemd or had no legacy handle
pub fn run() -> Result<(usize, usize, usize)> {
    let mut store = JobStore::load()?;
    let job_ids: Vec<String> = store.list().iter().map(|j| j.id.clone()).collect();

    let mut migrated = 0usize;
    let mut dropped = 0usize;
    let mut kept = 0usize;

    for id in job_ids {
        let snapshot = match store.get(&id) {
            Some(j) => j.clone(),
            None => continue,
        };

        let (at_time, legacy_handle) = match &snapshot.schedule {
            Schedule::Once { at, unit, at_job } if unit.is_none() && at_job.is_some() => {
                (*at, at_job.clone().unwrap())
            }
            _ => {
                kept += 1;
                continue;
            }
        };

        if at_time <= chrono::Utc::now() {
            eprintln!(
                "[migrate] dropping past-due one-shot '{}' (fire time {})",
                snapshot.id,
                at_time.format("%Y-%m-%d %H:%M:%S")
            );
            best_effort_atrm(&legacy_handle);
            store.remove(&snapshot.id);
            dropped += 1;
            continue;
        }

        match Backend::Systemd.schedule(&snapshot) {
            Ok(new_unit) => {
                if let Some(j) = store.get_mut(&snapshot.id) {
                    if let Schedule::Once { unit, at_job, .. } = &mut j.schedule {
                        *unit = Some(new_unit.clone());
                        // Clear the legacy handle so future reads no longer
                        // look like a migration candidate.
                        *at_job = None;
                    }
                }
                eprintln!(
                    "[migrate] migrated '{}' to systemd timer '{}'",
                    snapshot.id, new_unit
                );
                best_effort_atrm(&legacy_handle);
                migrated += 1;
            }
            Err(e) => {
                eprintln!(
                    "[migrate] failed to migrate '{}': {} (left as-is)",
                    snapshot.id, e
                );
                kept += 1;
            }
        }
    }

    store.save()?;
    Ok((migrated, dropped, kept))
}

/// Try to remove the legacy at-queue entry. Failures are logged but never
/// propagated — usched's metadata is the source of truth, and the at-queue
/// entry is harmless without `atd` to fire it. Mirrors the warn-and-continue
/// logic from sdelcore/usched#5.
fn best_effort_atrm(handle: &str) {
    if handle.is_empty() || handle == "unknown" {
        return;
    }
    match Command::new("atrm").arg(handle).output() {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            eprintln!(
                "[migrate] warning: atrm {} failed (left in at-queue): {}",
                handle,
                stderr.trim()
            );
        }
        Err(e) => {
            eprintln!(
                "[migrate] warning: could not invoke atrm for {} ({}). \
                 The at-queue entry (if any) was left untouched.",
                handle, e
            );
        }
    }
}
