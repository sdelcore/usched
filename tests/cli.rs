//! CLI integration tests.
//!
//! Exercises the `usched` binary end-to-end with stubbed `systemctl`/`at` so
//! we don't touch the host. See `tests/common/mod.rs` for the sandbox.

mod common;

use common::Sandbox;
use predicates::prelude::*;

#[test]
fn list_when_empty() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No scheduled jobs"));
}

#[test]
fn add_cron_writes_jobs_and_unit_files() {
    let sb = Sandbox::new();
    sb.cmd()
        .args([
            "add",
            "--name",
            "morning",
            "--cron",
            "0 9 * * 1-5",
            "--",
            "echo",
            "hi",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created recurring job"));

    // jobs.json exists and contains the job
    let jobs = std::fs::read_to_string(sb.jobs_json_path()).unwrap();
    assert!(jobs.contains("morning"));
    assert!(jobs.contains("0 9 * * 1-5"));
    assert!(jobs.contains("\"echo\""));

    // .timer and .service unit files written
    let unit_dir = sb.systemd_user_dir();
    let entries: Vec<_> = std::fs::read_dir(&unit_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        entries
            .iter()
            .any(|n| n.starts_with("usched-morning-") && n.ends_with(".timer")),
        "expected a .timer file, got {:?}",
        entries
    );
    assert!(
        entries
            .iter()
            .any(|n| n.starts_with("usched-morning-") && n.ends_with(".service")),
        "expected a .service file, got {:?}",
        entries
    );

    // systemctl was invoked: daemon-reload then enable --now
    let calls = sb.invocations_for("systemctl");
    assert!(
        calls.iter().any(|c| c.contains("daemon-reload")),
        "expected daemon-reload, got {:?}",
        calls
    );
    assert!(
        calls
            .iter()
            .any(|c| c.contains("enable") && c.contains("--now")),
        "expected enable --now, got {:?}",
        calls
    );
}

#[test]
fn add_then_list_then_remove_roundtrip() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["add", "--name", "rt", "--cron", "0 9 * * *", "--", "true"])
        .assert()
        .success();

    let stdout = sb
        .cmd()
        .args(["list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let listing = String::from_utf8(stdout).unwrap();
    assert!(listing.contains("rt"), "list output: {}", listing);
    assert!(listing.contains("cron: 0 9 * * *"));

    // Pull job ID out of the listing — format is "<id> (<name>) - ..."
    let id = listing
        .lines()
        .find_map(|l| {
            let prefix = l.split_whitespace().next()?;
            if prefix.starts_with("rt-") {
                Some(prefix.to_string())
            } else {
                None
            }
        })
        .expect("could not extract job id");

    sb.cmd()
        .args(["remove", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed"));

    // Files cleaned up
    let unit_dir = sb.systemd_user_dir();
    let remaining: Vec<_> = std::fs::read_dir(&unit_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        !remaining.iter().any(|n| n.contains("rt-")),
        "unit files not cleaned: {:?}",
        remaining
    );
}

#[test]
fn add_once_writes_systemd_timer() {
    let sb = Sandbox::new();
    sb.cmd()
        .args([
            "add",
            "--name",
            "once-job",
            "--once",
            "2099-06-15 09:00",
            "--",
            "echo",
            "hi",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("systemd timer"))
        .stdout(predicate::str::contains("2099-06-15 09:00:00"));

    // .timer and .service unit files written
    let unit_dir = sb.systemd_user_dir();
    let entries: Vec<_> = std::fs::read_dir(&unit_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        entries
            .iter()
            .any(|n| n.starts_with("usched-once-job-") && n.ends_with(".timer")),
        "expected a .timer file, got {:?}",
        entries
    );
    assert!(
        entries
            .iter()
            .any(|n| n.starts_with("usched-once-job-") && n.ends_with(".service")),
        "expected a .service file, got {:?}",
        entries
    );

    // The timer file should use OnCalendar=<absolute timestamp> with Persistent=true
    let timer_path = entries
        .iter()
        .find(|n| n.starts_with("usched-once-job-") && n.ends_with(".timer"))
        .unwrap();
    let timer_body = std::fs::read_to_string(unit_dir.join(timer_path)).unwrap();
    assert!(
        timer_body.contains("OnCalendar=2099-06-15 09:00:00"),
        "timer body: {}",
        timer_body
    );
    assert!(
        timer_body.contains("Persistent=true"),
        "timer body: {}",
        timer_body
    );

    // systemctl was invoked: daemon-reload then enable --now
    let calls = sb.invocations_for("systemctl");
    assert!(
        calls.iter().any(|c| c.contains("daemon-reload")),
        "expected daemon-reload, got {:?}",
        calls
    );
    assert!(
        calls
            .iter()
            .any(|c| c.contains("enable") && c.contains("--now")),
        "expected enable --now, got {:?}",
        calls
    );

    // No at-suite invocations — usched no longer depends on at(1)/atd/atrm.
    assert!(
        sb.invocations_for("at").is_empty(),
        "at(1) should not be invoked"
    );
    assert!(
        sb.invocations_for("atrm").is_empty(),
        "atrm should not be invoked"
    );
    assert!(
        sb.invocations_for("atq").is_empty(),
        "atq should not be invoked"
    );
}

#[test]
fn add_requires_cron_or_once() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["add", "--name", "x", "--", "echo", "hi"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--cron").or(predicate::str::contains("--once")));
}

#[test]
fn add_rejects_bad_cron() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["add", "--cron", "not a cron", "--", "echo", "hi"])
        .assert()
        .failure();
}

#[test]
fn list_json_emits_array() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["add", "--name", "j1", "--cron", "0 9 * * *", "--", "true"])
        .assert()
        .success();

    let out = sb
        .cmd()
        .args(["list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8(out).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&s).expect("valid JSON");
    assert!(parsed.is_array());
    assert_eq!(parsed.as_array().unwrap().len(), 1);
}

#[test]
fn dnd_set_persists_then_clears() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["dnd", "set", "1h"])
        .assert()
        .success()
        .stdout(predicate::str::contains("DND set"));

    let state = std::fs::read_to_string(sb.state_json_path()).unwrap();
    assert!(state.contains("dnd_until"), "state.json: {}", state);

    sb.cmd()
        .args(["dnd", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("DND active"));

    sb.cmd()
        .args(["dnd", "off"])
        .assert()
        .success()
        .stdout(predicate::str::contains("DND cleared"));

    sb.cmd()
        .args(["dnd", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not active"));
}

#[test]
fn dnd_set_rejects_bad_duration() {
    let sb = Sandbox::new();
    sb.cmd().args(["dnd", "set", "abc"]).assert().failure();
    sb.cmd().args(["dnd", "set", "0"]).assert().failure();
}

#[test]
fn preview_prints_upcoming_runs() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["preview", "0 9 * * *", "-n", "3"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Cron: 0 9 * * *"));
}

#[test]
fn preview_rejects_invalid_cron() {
    let sb = Sandbox::new();
    sb.cmd().args(["preview", "not a cron"]).assert().failure();
}

#[test]
fn check_with_no_jobs() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["check"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No jobs configured"));
}

#[test]
fn export_markdown_to_file() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["add", "--name", "exp", "--cron", "0 9 * * *", "--", "true"])
        .assert()
        .success();

    let out_path = sb.root.path().join("schedule.md");
    sb.cmd()
        .args(["export", "-o", out_path.to_str().unwrap()])
        .assert()
        .success();

    let md = std::fs::read_to_string(&out_path).unwrap();
    assert!(md.contains("# Scheduled Tasks"));
    assert!(md.contains("exp"));
    assert!(md.contains("`0 9 * * *`"));
}

#[test]
fn disable_then_enable_toggles_state() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["add", "--name", "tog", "--cron", "0 9 * * *", "--", "true"])
        .assert()
        .success();

    let listing = String::from_utf8(
        sb.cmd()
            .args(["list"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    let id = listing
        .lines()
        .find_map(|l| {
            l.split_whitespace()
                .next()
                .filter(|p| p.starts_with("tog-"))
        })
        .map(|s| s.to_string())
        .unwrap();

    sb.cmd()
        .args(["disable", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Disabled"));

    sb.cmd()
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("disabled"));

    sb.cmd()
        .args(["enable", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Enabled"));
}

/// Helper: pull the freshly-added job id out of `usched list` output.
fn first_job_id_with_prefix(sb: &Sandbox, prefix: &str) -> String {
    let listing = String::from_utf8(
        sb.cmd()
            .args(["list"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    listing
        .lines()
        .find_map(|l| {
            l.split_whitespace()
                .next()
                .filter(|p| p.starts_with(prefix))
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| panic!("no job id with prefix {:?} in:\n{}", prefix, listing))
}

#[test]
fn runjob_executes_command_and_records_history() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["add", "--name", "exec", "--cron", "0 9 * * *", "--", "true"])
        .assert()
        .success();

    let id = first_job_id_with_prefix(&sb, "exec-");

    sb.cmd().args(["__run-job", &id]).assert().success();

    let history_out = sb
        .cmd()
        .args(["history", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8(history_out).unwrap()).unwrap();
    let arr = parsed.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["job_id"], id);
    assert_eq!(arr[0]["exit_code"], 0);
}

#[test]
fn runjob_skips_disabled_job() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["add", "--name", "dis", "--cron", "0 9 * * *", "--", "true"])
        .assert()
        .success();

    let id = first_job_id_with_prefix(&sb, "dis-");
    sb.cmd().args(["disable", &id]).assert().success();

    sb.cmd().args(["__run-job", &id]).assert().success();

    let out = sb
        .cmd()
        .args(["history", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_str(&String::from_utf8(out).unwrap()).unwrap();
    assert_eq!(parsed[0]["skipped_reason"], "disabled");
}

#[test]
fn run_with_force_bypasses_disabled() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["add", "--name", "frc", "--cron", "0 9 * * *", "--", "true"])
        .assert()
        .success();

    let id = first_job_id_with_prefix(&sb, "frc-");
    sb.cmd().args(["disable", &id]).assert().success();

    // Without --force, runner records a skip
    sb.cmd().args(["run", &id]).assert().success();
    let out = sb
        .cmd()
        .args(["history", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_str(&String::from_utf8(out).unwrap()).unwrap();
    assert_eq!(parsed[0]["skipped_reason"], "disabled");

    // With --force, runner executes (records a real run)
    sb.cmd().args(["run", &id, "--force"]).assert().success();
    let out = sb
        .cmd()
        .args(["history", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_str(&String::from_utf8(out).unwrap()).unwrap();
    // Most recent entry first; should be a real run with exit 0
    assert!(parsed[0]["skipped_reason"].is_null());
    assert_eq!(parsed[0]["exit_code"], 0);
}

#[test]
fn add_with_after_dependency_validates() {
    let sb = Sandbox::new();
    // Reference to non-existent job → fail
    sb.cmd()
        .args([
            "add",
            "--name",
            "child",
            "--cron",
            "0 9 * * *",
            "--after",
            "ghost",
            "--",
            "true",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));

    // Add parent, then child with --after
    sb.cmd()
        .args([
            "add",
            "--name",
            "parent",
            "--cron",
            "0 8 * * *",
            "--",
            "true",
        ])
        .assert()
        .success();

    sb.cmd()
        .args([
            "add",
            "--name",
            "child",
            "--cron",
            "0 9 * * *",
            "--after",
            "parent",
            "--",
            "true",
        ])
        .assert()
        .success();
}

#[test]
fn remove_unknown_job_errors() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["remove", "does-not-exist"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

/// Removing a one-shot job tears down its systemd timer the same way a
/// recurring job's removal does — both flows go through the same backend.
#[test]
fn remove_once_job_tears_down_systemd_timer() {
    let sb = Sandbox::new();

    sb.cmd()
        .args([
            "add",
            "--name",
            "ghost",
            "--once",
            "2099-06-15 09:00",
            "--",
            "echo",
            "hi",
        ])
        .assert()
        .success();

    let id = first_job_id_with_prefix(&sb, "ghost-");

    sb.cmd()
        .args(["remove", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed systemd timer"))
        .stdout(predicate::str::contains("Removed job"));

    // Unit files cleaned up.
    let unit_dir = sb.systemd_user_dir();
    let remaining: Vec<_> = std::fs::read_dir(&unit_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        !remaining.iter().any(|n| n.contains("ghost-")),
        "unit files not cleaned: {:?}",
        remaining
    );

    // Job gone from jobs.json.
    let listing = String::from_utf8(
        sb.cmd()
            .args(["list"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert!(
        !listing.contains(&id),
        "job {} should be removed from jobs.json, list was:\n{}",
        id,
        listing
    );

    // No at-suite invocations on the remove path either.
    assert!(
        sb.invocations_for("atrm").is_empty(),
        "atrm should not be invoked"
    );
}

/// Migration entry point: a jobs.json file authored by an older version of
/// usched (with `at_job` set on a `once` schedule and no systemd `unit`)
/// gets re-registered as a systemd timer. The legacy `atrm` is best-effort —
/// failures are warned about, never propagated.
#[test]
fn migrate_from_at_re_registers_legacy_once_jobs_on_systemd() {
    let sb = Sandbox::new();

    // Hand-craft a jobs.json that looks like what the at-backed version
    // of usched used to write.
    std::fs::create_dir_all(sb.data_dir()).unwrap();
    let jobs_json = r#"{
        "jobs": {
            "legacy-1": {
                "id": "legacy-1",
                "name": "legacy",
                "schedule": {
                    "type": "once",
                    "at": "2099-06-15T09:00:00Z",
                    "at_job": "42"
                },
                "command": ["echo", "hi"],
                "constraints": {},
                "enabled": true,
                "created_at": "2026-01-01T00:00:00Z",
                "created_by": "user"
            }
        }
    }"#;
    std::fs::write(sb.jobs_json_path(), jobs_json).unwrap();

    sb.cmd()
        .args(["migrate-from-at"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 migrated"));

    // After migration, jobs.json should have `unit` set and `at_job` cleared.
    let after = std::fs::read_to_string(sb.jobs_json_path()).unwrap();
    assert!(
        after.contains("\"unit\""),
        "expected unit handle on once entry: {}",
        after
    );
    assert!(
        !after.contains("\"at_job\""),
        "at_job should be cleared after migration: {}",
        after
    );

    // .timer + .service files were written.
    let unit_dir = sb.systemd_user_dir();
    let entries: Vec<_> = std::fs::read_dir(&unit_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        entries.iter().any(|n| n == "usched-legacy-1.timer"),
        "expected timer file, got {:?}",
        entries
    );
    assert!(
        entries.iter().any(|n| n == "usched-legacy-1.service"),
        "expected service file, got {:?}",
        entries
    );

    // Best-effort atrm was attempted on the legacy at-queue handle.
    let atrm_calls = sb.invocations_for("atrm");
    assert!(
        atrm_calls.iter().any(|c| c.contains(" 42")),
        "expected atrm to be called with legacy job number, got {:?}",
        atrm_calls
    );
}

/// Migration is idempotent: a second run leaves an already-migrated job
/// alone.
#[test]
fn migrate_from_at_is_idempotent() {
    let sb = Sandbox::new();

    sb.cmd()
        .args([
            "add",
            "--name",
            "fresh",
            "--once",
            "2099-06-15 09:00",
            "--",
            "echo",
            "hi",
        ])
        .assert()
        .success();

    sb.cmd()
        .args(["migrate-from-at"])
        .assert()
        .success()
        .stdout(predicate::str::contains("0 migrated"))
        .stdout(predicate::str::contains("1 unchanged"));
}

/// Migration drops one-shots whose absolute fire time is already in the
/// past — they cannot be meaningfully re-scheduled.
#[test]
fn migrate_from_at_drops_past_due_jobs() {
    let sb = Sandbox::new();

    std::fs::create_dir_all(sb.data_dir()).unwrap();
    let jobs_json = r#"{
        "jobs": {
            "stale-1": {
                "id": "stale-1",
                "name": "stale",
                "schedule": {
                    "type": "once",
                    "at": "2000-01-01T00:00:00Z",
                    "at_job": "13"
                },
                "command": ["echo", "hi"],
                "constraints": {},
                "enabled": true,
                "created_at": "2000-01-01T00:00:00Z",
                "created_by": "user"
            }
        }
    }"#;
    std::fs::write(sb.jobs_json_path(), jobs_json).unwrap();

    sb.cmd()
        .args(["migrate-from-at"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 dropped"));

    // Job is gone from jobs.json.
    let after = std::fs::read_to_string(sb.jobs_json_path()).unwrap();
    assert!(
        !after.contains("stale-1"),
        "stale job should be dropped: {}",
        after
    );
}
