//! Job execution path between "the timer fired" and "the command ran."
//!
//! Split into a pure decision (`evaluate`) and an impure execution
//! (`execute`). Both the systemd `.service` ExecStart and the `at` queue
//! entry call into [`run`] via the hidden `usched __run-job` subcommand.

use anyhow::Result;
use chrono::{DateTime, Local, Timelike};

use crate::history;
use crate::job::Job;
use crate::store::{JobStore, State};

/// Outcome of evaluating a job's constraints against the current world.
#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Run,
    Skip(SkipReason),
}

#[derive(Debug, PartialEq, Eq)]
pub enum SkipReason {
    Disabled,
    DndActive,
    NotDuring { start: String, end: String },
    NotInOnlyDuring,
    AfterMissingHistory(String),
    AfterFailed { dep: String, exit: i32 },
}

impl SkipReason {
    pub fn as_history_reason(&self) -> String {
        match self {
            SkipReason::Disabled => "disabled".to_string(),
            SkipReason::DndActive => "dnd".to_string(),
            SkipReason::NotDuring { start, end } => format!("not_during {}-{}", start, end),
            SkipReason::NotInOnlyDuring => "not in only_during range".to_string(),
            SkipReason::AfterMissingHistory(dep) => {
                format!("after: {} has no history", dep)
            }
            SkipReason::AfterFailed { dep, exit } => format!("after: {} exit {}", dep, exit),
        }
    }
}

/// Lookup the most recent execution exit code for an `--after` dependency.
///
/// Returns `None` when the dependency has no recorded execution (or its
/// most recent record is a skip — matches the prior bash behavior).
pub trait HistoryLookup {
    fn last_exit_for(&self, job_ref: &str) -> Result<Option<i32>>;
}

/// Production lookup: queries `history.db`.
pub struct HistoryDb;

impl HistoryLookup for HistoryDb {
    fn last_exit_for(&self, job_ref: &str) -> Result<Option<i32>> {
        let executions = history::query_history(Some(job_ref), false, 1)?;
        Ok(executions.first().and_then(|e| e.exit_code))
    }
}

/// Pure evaluator. Given the current world, decide whether to run the job.
pub fn evaluate(
    job: &Job,
    state: &State,
    history: &dyn HistoryLookup,
    now: DateTime<Local>,
) -> Result<Decision> {
    if !job.enabled {
        return Ok(Decision::Skip(SkipReason::Disabled));
    }

    if job.constraints.dnd_aware && state.is_dnd_active() {
        return Ok(Decision::Skip(SkipReason::DndActive));
    }

    let now_time = chrono::NaiveTime::from_hms_opt(now.hour(), now.minute(), 0)
        .expect("hour/minute always in range");

    for tr in &job.constraints.not_during {
        if tr.contains(now_time) {
            return Ok(Decision::Skip(SkipReason::NotDuring {
                start: tr.start.format("%H:%M").to_string(),
                end: tr.end.format("%H:%M").to_string(),
            }));
        }
    }

    if !job.constraints.only_during.is_empty() {
        let any_match = job.constraints.only_during.iter().any(|tr| tr.contains(now_time));
        if !any_match {
            return Ok(Decision::Skip(SkipReason::NotInOnlyDuring));
        }
    }

    if let Some(dep) = &job.constraints.after {
        match history.last_exit_for(dep)? {
            None => {
                return Ok(Decision::Skip(SkipReason::AfterMissingHistory(dep.clone())));
            }
            Some(0) => {}
            Some(exit) => {
                return Ok(Decision::Skip(SkipReason::AfterFailed {
                    dep: dep.clone(),
                    exit,
                }));
            }
        }
    }

    Ok(Decision::Run)
}

/// Impure execution: fork/exec the command, record start + finish in history.
///
/// Returns the child's exit code (or -1 if the child was killed by a signal).
pub fn execute(job: &Job) -> Result<i32> {
    if job.command.is_empty() {
        anyhow::bail!("Job '{}' has empty command", job.id);
    }

    let row_id = history::record_start(&job.id, &job.name)?;
    let start = std::time::Instant::now();

    let status = std::process::Command::new(&job.command[0])
        .args(&job.command[1..])
        .status()?;

    let duration_ms = start.elapsed().as_millis() as i64;
    let exit_code = status.code().unwrap_or(-1);
    history::record_finish(row_id, exit_code, duration_ms)?;

    Ok(exit_code)
}

/// Top-level entry: load the world, evaluate, execute (or skip), handle
/// `remove_on_success` cleanup.
///
/// `force` bypasses constraint evaluation but still records execution.
pub fn run(job_id: &str, force: bool) -> Result<i32> {
    let store = JobStore::load()?;
    let state = State::load()?;
    let job = store
        .get(job_id)
        .ok_or_else(|| anyhow::anyhow!("Job '{}' not found", job_id))?
        .clone();

    if !force {
        let decision = evaluate(&job, &state, &HistoryDb, Local::now())?;
        if let Decision::Skip(reason) = decision {
            let reason_str = reason.as_history_reason();
            history::record_skip(&job.id, &job.name, &reason_str)?;
            eprintln!(
                "[usched] skipping job '{}' ({}): {}",
                job.id, job.name, reason_str
            );
            return Ok(0);
        }
    }

    let exit_code = execute(&job)?;

    if exit_code != 0 {
        eprintln!(
            "[usched] job '{}' ({}) exited with code {}",
            job.id, job.name, exit_code
        );
    }

    if job.constraints.remove_on_success && exit_code == 0 {
        eprintln!(
            "[usched] job '{}' succeeded; auto-removing",
            job.id
        );
        if let Err(e) = remove_job_after_success(&job.id) {
            eprintln!("[usched] auto-remove failed: {}", e);
        }
    }

    Ok(exit_code)
}

/// Auto-removal helper used by [`run`] when `remove_on_success` is set.
fn remove_job_after_success(job_id: &str) -> Result<()> {
    let mut store = JobStore::load()?;
    let job = store
        .remove(job_id)
        .ok_or_else(|| anyhow::anyhow!("Job '{}' not found", job_id))?;

    if let Some(handle) = job.schedule.handle() {
        job.schedule.backend().remove(handle)?;
    }

    store.save()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::{Constraints, Job, Schedule, TimeRange};
    use chrono::{NaiveTime, TimeZone, Utc};

    struct StubHistory {
        last: Option<i32>,
    }

    impl HistoryLookup for StubHistory {
        fn last_exit_for(&self, _: &str) -> Result<Option<i32>> {
            Ok(self.last)
        }
    }

    fn at(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn now_at(h: u32, m: u32) -> DateTime<Local> {
        Local
            .from_local_datetime(
                &chrono::NaiveDate::from_ymd_opt(2026, 1, 15)
                    .unwrap()
                    .and_hms_opt(h, m, 0)
                    .unwrap(),
            )
            .single()
            .unwrap()
    }

    fn job_with(constraints: Constraints, enabled: bool) -> Job {
        Job {
            id: "test-id".to_string(),
            name: "test".to_string(),
            schedule: Schedule::Cron { expr: "0 * * * *".to_string(), unit: None },
            command: vec!["true".to_string()],
            constraints,
            enabled,
            created_at: Utc::now(),
            created_by: "test".to_string(),
        }
    }

    #[test]
    fn evaluate_disabled_skips() {
        let job = job_with(Constraints::default(), false);
        let state = State::default();
        let h = StubHistory { last: None };
        let d = evaluate(&job, &state, &h, now_at(12, 0)).unwrap();
        assert_eq!(d, Decision::Skip(SkipReason::Disabled));
    }

    #[test]
    fn evaluate_dnd_active_skips_when_aware() {
        let mut job = job_with(Constraints::default(), true);
        job.constraints.dnd_aware = true;
        let mut state = State::default();
        state.set_dnd(Utc::now() + chrono::Duration::hours(1));
        let h = StubHistory { last: None };
        let d = evaluate(&job, &state, &h, now_at(12, 0)).unwrap();
        assert_eq!(d, Decision::Skip(SkipReason::DndActive));
    }

    #[test]
    fn evaluate_dnd_ignored_when_not_aware() {
        let job = job_with(Constraints::default(), true); // dnd_aware = false
        let mut state = State::default();
        state.set_dnd(Utc::now() + chrono::Duration::hours(1));
        let h = StubHistory { last: None };
        let d = evaluate(&job, &state, &h, now_at(12, 0)).unwrap();
        assert_eq!(d, Decision::Run);
    }

    #[test]
    fn evaluate_not_during_skips_inside_window() {
        let mut c = Constraints::default();
        c.not_during.push(TimeRange { start: at(22, 0), end: at(8, 0) }); // midnight wrap
        let job = job_with(c, true);
        let state = State::default();
        let h = StubHistory { last: None };
        let d = evaluate(&job, &state, &h, now_at(23, 30)).unwrap();
        assert!(matches!(d, Decision::Skip(SkipReason::NotDuring { .. })));
    }

    #[test]
    fn evaluate_not_during_passes_outside_window() {
        let mut c = Constraints::default();
        c.not_during.push(TimeRange { start: at(22, 0), end: at(8, 0) });
        let job = job_with(c, true);
        let state = State::default();
        let h = StubHistory { last: None };
        let d = evaluate(&job, &state, &h, now_at(12, 0)).unwrap();
        assert_eq!(d, Decision::Run);
    }

    #[test]
    fn evaluate_only_during_skips_outside() {
        let mut c = Constraints::default();
        c.only_during.push(TimeRange { start: at(9, 0), end: at(17, 0) });
        let job = job_with(c, true);
        let state = State::default();
        let h = StubHistory { last: None };
        let d = evaluate(&job, &state, &h, now_at(20, 0)).unwrap();
        assert_eq!(d, Decision::Skip(SkipReason::NotInOnlyDuring));
    }

    #[test]
    fn evaluate_only_during_passes_inside() {
        let mut c = Constraints::default();
        c.only_during.push(TimeRange { start: at(9, 0), end: at(17, 0) });
        let job = job_with(c, true);
        let state = State::default();
        let h = StubHistory { last: None };
        let d = evaluate(&job, &state, &h, now_at(12, 0)).unwrap();
        assert_eq!(d, Decision::Run);
    }

    #[test]
    fn evaluate_after_no_history_skips() {
        let mut c = Constraints::default();
        c.after = Some("parent".to_string());
        let job = job_with(c, true);
        let state = State::default();
        let h = StubHistory { last: None };
        let d = evaluate(&job, &state, &h, now_at(12, 0)).unwrap();
        assert_eq!(
            d,
            Decision::Skip(SkipReason::AfterMissingHistory("parent".to_string()))
        );
    }

    #[test]
    fn evaluate_after_failed_skips() {
        let mut c = Constraints::default();
        c.after = Some("parent".to_string());
        let job = job_with(c, true);
        let state = State::default();
        let h = StubHistory { last: Some(1) };
        let d = evaluate(&job, &state, &h, now_at(12, 0)).unwrap();
        assert_eq!(
            d,
            Decision::Skip(SkipReason::AfterFailed { dep: "parent".to_string(), exit: 1 })
        );
    }

    #[test]
    fn evaluate_after_succeeded_runs() {
        let mut c = Constraints::default();
        c.after = Some("parent".to_string());
        let job = job_with(c, true);
        let state = State::default();
        let h = StubHistory { last: Some(0) };
        let d = evaluate(&job, &state, &h, now_at(12, 0)).unwrap();
        assert_eq!(d, Decision::Run);
    }

    #[test]
    fn evaluate_clean_job_runs() {
        let job = job_with(Constraints::default(), true);
        let state = State::default();
        let h = StubHistory { last: None };
        let d = evaluate(&job, &state, &h, now_at(12, 0)).unwrap();
        assert_eq!(d, Decision::Run);
    }
}
