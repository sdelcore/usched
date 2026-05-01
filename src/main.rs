mod backend;
mod cron_convert;
mod dnd;
pub mod history;
mod job;
mod migrate;
mod runner;
pub mod store;
mod systemd;
mod time_input;

use anyhow::Result;
use chrono::{Local, Timelike, Utc};
use clap::{Parser, Subcommand};

use job::{Constraints, Job, Schedule};
use store::JobStore;

#[derive(Parser)]
#[command(name = "usched")]
#[command(about = "Unified scheduler CLI built on systemd user timers")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new scheduled job
    Add {
        /// Job name
        #[arg(long)]
        name: Option<String>,

        /// Cron expression for recurring jobs (e.g., "0 9 * * 1-5")
        #[arg(long, conflicts_with = "once")]
        cron: Option<String>,

        /// One-time schedule (e.g., "tomorrow 14:00", "2024-01-20 15:00")
        #[arg(long, conflicts_with = "cron")]
        once: Option<String>,

        /// Time range during which job should NOT run (e.g., "22:00-08:00")
        #[arg(long)]
        not_during: Option<String>,

        /// Check DND state before running
        #[arg(long)]
        dnd_aware: bool,

        /// Auto-remove job when command exits with success (exit code 0)
        #[arg(long)]
        remove_on_success: bool,

        /// Only run after this job succeeds (job ID or name)
        #[arg(long)]
        after: Option<String>,

        /// Command to run
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },

    /// List all scheduled jobs
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Remove a job
    Remove {
        /// Job ID
        id: String,
    },

    /// Enable a disabled job
    Enable {
        /// Job ID
        id: String,
    },

    /// Disable a job without removing it
    Disable {
        /// Job ID
        id: String,
    },

    /// Edit an existing job's schedule or constraints
    Edit {
        /// Job ID
        id: String,

        /// New cron expression
        #[arg(long)]
        cron: Option<String>,

        /// New job name
        #[arg(long)]
        name: Option<String>,

        /// Replace not-during constraint (e.g., "22:00-08:00")
        #[arg(long)]
        not_during: Option<String>,

        /// Clear not-during constraints
        #[arg(long)]
        clear_not_during: bool,

        /// Set DND awareness
        #[arg(long)]
        dnd_aware: Option<bool>,
    },

    /// Run a job immediately, honoring constraints
    Run {
        /// Job ID
        id: String,

        /// Bypass constraint checks (DND, time ranges, --after, enabled)
        #[arg(long)]
        force: bool,
    },

    /// Internal: invoked by systemd/at to actually execute a scheduled job.
    /// Reads the command from jobs.json and applies all runtime constraints.
    #[command(name = "__run-job", hide = true)]
    RunJob {
        /// Job ID
        job_id: String,

        /// Legacy positional args from old `usched-run <id> <cmd...>` callers; ignored.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        _legacy: Vec<String>,
    },

    /// Show upcoming scheduled runs
    Next,

    /// Preview when a cron expression would fire next
    Preview {
        /// Cron expression (e.g., "*/30 * * * *")
        cron: String,

        /// Number of future runs to show
        #[arg(short = 'n', long, default_value = "5")]
        count: usize,

        /// Apply not-during constraint (e.g., "22:00-08:00")
        #[arg(long)]
        not_during: Option<String>,
    },

    /// Sync: verify and reconstruct missing timers, clean orphaned timers
    Sync,

    /// Check health of all jobs and timers
    Check,

    /// Export schedule as markdown (for Obsidian/ARIA visibility)
    Export {
        /// Output file path (prints to stdout if omitted)
        #[arg(short, long)]
        output: Option<String>,
    },

    /// Show execution history
    History {
        /// Filter by job ID or name
        job: Option<String>,

        /// Only show failed executions
        #[arg(long)]
        failed: bool,

        /// Number of entries to show
        #[arg(short = 'n', long, default_value = "20")]
        limit: usize,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Manage Do Not Disturb mode
    Dnd {
        #[command(subcommand)]
        action: DndAction,
    },

    /// Migrate one-shot jobs scheduled with the legacy `at(1)` backend
    /// onto systemd user timers. Idempotent — safe to run multiple times.
    MigrateFromAt,
}

#[derive(Subcommand)]
enum DndAction {
    /// Set DND for a duration (e.g., "2h", "30m")
    Set {
        /// Duration (e.g., "2h", "30m", "1h30m")
        duration: String,
    },
    /// Clear DND
    Off,
    /// Show DND status
    Status,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Add {
            name,
            cron,
            once,
            not_during,
            dnd_aware,
            remove_on_success,
            after,
            command,
        } => cmd_add(name, cron, once, not_during, dnd_aware, remove_on_success, after, command),
        Commands::List { json } => cmd_list(json),
        Commands::Remove { id } => cmd_remove(&id),
        Commands::Edit { id, cron, name, not_during, clear_not_during, dnd_aware } => {
            cmd_edit(&id, cron, name, not_during, clear_not_during, dnd_aware)
        }
        Commands::Enable { id } => cmd_enable(&id),
        Commands::Disable { id } => cmd_disable(&id),
        Commands::Run { id, force } => cmd_run(&id, force),
        Commands::RunJob { job_id, .. } => cmd_run_job(&job_id),
        Commands::Next => cmd_next(),
        Commands::Preview { cron, count, not_during } => cmd_preview(&cron, count, not_during),
        Commands::Sync => cmd_sync(),
        Commands::Check => cmd_check(),
        Commands::Export { output } => cmd_export(output),
        Commands::History { job, failed, limit, json } => cmd_history(job, failed, limit, json),
        Commands::Dnd { action } => match action {
            DndAction::Set { duration } => dnd::set_dnd(&duration),
            DndAction::Off => dnd::clear_dnd(),
            DndAction::Status => dnd::show_dnd_status(),
        },
        Commands::MigrateFromAt => cmd_migrate_from_at(),
    }
}

fn cmd_migrate_from_at() -> Result<()> {
    let (migrated, dropped, kept) = migrate::run()?;
    println!(
        "Migration complete: {} migrated to systemd, {} dropped (past due), {} unchanged",
        migrated, dropped, kept
    );
    Ok(())
}

fn cmd_add(
    name: Option<String>,
    cron: Option<String>,
    once: Option<String>,
    not_during: Option<String>,
    dnd_aware: bool,
    remove_on_success: bool,
    after: Option<String>,
    command: Vec<String>,
) -> Result<()> {
    if cron.is_none() && once.is_none() {
        anyhow::bail!("Must specify either --cron or --once");
    }

    // Validate --after dependency exists
    if let Some(ref dep) = after {
        let store = JobStore::load()?;
        let dep_exists = store.get(dep).is_some()
            || store.list().iter().any(|j| j.name == *dep);
        if !dep_exists {
            anyhow::bail!(
                "Dependency job '{}' not found. Use a valid job ID or name.",
                dep
            );
        }
    }

    let job_name = name.unwrap_or_else(|| command.first().cloned().unwrap_or_else(|| "job".to_string()));
    let job_id = Job::generate_id(&job_name);

    let schedule = if let Some(cron_expr) = &cron {
        Schedule::Cron { expr: cron_expr.clone(), unit: None }
    } else {
        let at_time = time_input::parse_datetime(once.as_ref().unwrap())?;
        Schedule::Once { at: at_time, unit: None, at_job: None }
    };

    let constraints = Constraints {
        not_during: match not_during {
            Some(s) => vec![time_input::parse_time_range(&s)?],
            None => vec![],
        },
        only_during: vec![],
        dnd_aware,
        remove_on_success,
        after,
    };

    let mut job = Job {
        id: job_id.clone(),
        name: job_name,
        schedule,
        command,
        constraints,
        enabled: true,
        created_at: Utc::now(),
        created_by: "user".to_string(),
    };

    let backend = job.schedule.backend();
    let handle = backend.schedule(&job)?;
    match &job.schedule {
        Schedule::Cron { .. } => {
            println!("Created recurring job '{}' with systemd timer '{}'", job_id, handle);
        }
        Schedule::Once { at, .. } => {
            let local: chrono::DateTime<chrono::Local> = (*at).into();
            println!(
                "Created one-off job '{}' with systemd timer '{}' (fires {})",
                job_id,
                handle,
                local.format("%Y-%m-%d %H:%M:%S")
            );
        }
    }
    job.schedule.set_handle(handle);

    let mut store = JobStore::load()?;
    store.add(job);
    store.save()?;

    Ok(())
}

fn cmd_list(json: bool) -> Result<()> {
    let store = JobStore::load()?;
    let jobs = store.list();

    if json {
        println!("{}", serde_json::to_string_pretty(&jobs)?);
    } else {
        if jobs.is_empty() {
            println!("No scheduled jobs");
            return Ok(());
        }

        for job in jobs {
            let schedule_str = match &job.schedule {
                Schedule::Cron { expr, .. } => format!("cron: {}", expr),
                Schedule::Once { at, .. } => format!(
                    "once: {}",
                    at.with_timezone(&Local).format("%Y-%m-%d %H:%M")
                ),
            };

            let status = if job.enabled { "enabled" } else { "disabled" };

            let mut flags: Vec<String> = vec![];
            if job.constraints.dnd_aware {
                flags.push("dnd-aware".into());
            }
            if job.constraints.remove_on_success {
                flags.push("auto-remove".into());
            }
            if !job.constraints.not_during.is_empty() {
                flags.push("has-constraints".into());
            }
            if let Some(ref dep) = job.constraints.after {
                flags.push(format!("after:{}", dep));
            }

            let flags_str = if flags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", flags.join(", "))
            };

            println!(
                "{} ({}) - {} [{}]{}",
                job.id, job.name, schedule_str, status, flags_str
            );
            println!("  command: {}", job.command.join(" "));
        }
    }

    Ok(())
}

fn cmd_remove(id: &str) -> Result<()> {
    let mut store = JobStore::load()?;

    let job = store.remove(id).ok_or_else(|| anyhow::anyhow!("Job '{}' not found", id))?;

    if let Some(handle) = job.schedule.handle() {
        job.schedule.backend().remove(handle)?;
        println!("Removed systemd timer '{}'", handle);
    }

    store.save()?;
    println!("Removed job '{}'", id);

    Ok(())
}

fn cmd_enable(id: &str) -> Result<()> {
    let mut store = JobStore::load()?;

    let job = store.get_mut(id).ok_or_else(|| anyhow::anyhow!("Job '{}' not found", id))?;

    if job.enabled {
        println!("Job '{}' is already enabled", id);
        return Ok(());
    }

    if let Some(handle) = job.schedule.handle() {
        job.schedule.backend().enable(handle)?;
    }

    job.enabled = true;
    store.save()?;
    println!("Enabled job '{}'", id);

    Ok(())
}

fn cmd_disable(id: &str) -> Result<()> {
    let mut store = JobStore::load()?;

    let job = store.get_mut(id).ok_or_else(|| anyhow::anyhow!("Job '{}' not found", id))?;

    if !job.enabled {
        println!("Job '{}' is already disabled", id);
        return Ok(());
    }

    if let Some(handle) = job.schedule.handle() {
        job.schedule.backend().disable(handle)?;
    }

    job.enabled = false;
    store.save()?;
    println!("Disabled job '{}'", id);

    Ok(())
}

fn cmd_run(id: &str, force: bool) -> Result<()> {
    let exit_code = runner::run(id, force)?;
    if exit_code == 0 {
        println!("Job completed successfully");
    } else {
        println!("Job failed with exit code: {}", exit_code);
    }
    Ok(())
}

/// Hidden subcommand invoked by systemd timers and `at` queue entries.
/// Propagates the child's exit code to the caller (so systemd can mark
/// the service unit as failed when appropriate).
fn cmd_run_job(job_id: &str) -> Result<()> {
    let exit_code = runner::run(job_id, false)?;
    std::process::exit(exit_code);
}

fn cmd_next() -> Result<()> {
    println!("=== Systemd Timers ===");
    let timers = systemd::list_timers()?;
    // Filter to only show usched timers
    for line in timers.lines() {
        if line.contains("usched-") || line.starts_with("NEXT") || line.starts_with("---") {
            println!("{}", line);
        }
    }

    Ok(())
}

fn cmd_edit(
    id: &str,
    cron: Option<String>,
    name: Option<String>,
    not_during: Option<String>,
    clear_not_during: bool,
    dnd_aware: Option<bool>,
) -> Result<()> {
    let mut store = JobStore::load()?;

    let job = store
        .get_mut(id)
        .ok_or_else(|| anyhow::anyhow!("Job '{}' not found", id))?;

    let mut changes = Vec::new();

    // Update name
    if let Some(new_name) = name {
        job.name = new_name.clone();
        changes.push(format!("name → {}", new_name));
    }

    // Update DND awareness
    if let Some(dnd) = dnd_aware {
        job.constraints.dnd_aware = dnd;
        changes.push(format!("dnd_aware → {}", dnd));
    }

    // Update not_during
    if clear_not_during {
        job.constraints.not_during.clear();
        changes.push("not_during → cleared".to_string());
    } else if let Some(nd) = not_during {
        job.constraints.not_during = vec![time_input::parse_time_range(&nd)?];
        changes.push(format!("not_during → {}", nd));
    }

    // Update cron schedule — this requires recreating the systemd timer
    if let Some(new_cron) = cron {
        match &job.schedule {
            Schedule::Cron { unit, .. } => {
                let backend = backend::Backend::Systemd;
                if let Some(handle) = unit {
                    backend.remove(handle)?;
                }

                job.schedule = Schedule::Cron { expr: new_cron.clone(), unit: None };
                let new_handle = backend.schedule(job)?;
                job.schedule.set_handle(new_handle);

                changes.push(format!("cron → {}", new_cron));
            }
            Schedule::Once { .. } => {
                anyhow::bail!("Cannot change a one-off job to cron. Remove and re-add instead.");
            }
        }
    }

    if changes.is_empty() {
        println!("No changes specified for job '{}'", id);
        return Ok(());
    }

    store.save()?;
    println!("Updated job '{}':", id);
    for change in &changes {
        println!("  {}", change);
    }

    Ok(())
}

fn cmd_preview(cron_expr: &str, count: usize, not_during: Option<String>) -> Result<()> {
    let constraint = not_during.as_ref().map(|nd| time_input::parse_time_range(nd)).transpose()?;

    // The cron crate expects 6-7 fields (seconds first). Standard cron has 5 fields.
    // Prepend "0" for seconds if we got a 5-field expression.
    let full_cron = if cron_expr.split_whitespace().count() == 5 {
        format!("0 {}", cron_expr)
    } else {
        cron_expr.to_string()
    };

    let schedule: cron::Schedule = full_cron.parse().map_err(|e| {
        anyhow::anyhow!("Invalid cron expression {:?}: {}", cron_expr, e)
    })?;

    println!("Cron: {}", cron_expr);
    if let Some(ref c) = constraint {
        println!("Not during: {:?}-{:?}", c.start, c.end);
    }
    println!();

    let mut shown = 0;
    let now = chrono::Local::now();
    let iter = schedule.upcoming(chrono::Local);

    for next in iter.take(count * 3) {
        if shown >= count {
            break;
        }

        // Check not_during constraint
        if let Some(ref c) = constraint {
            let time = next.time();
            let naive_time = chrono::NaiveTime::from_hms_opt(
                time.hour() as u32,
                time.minute() as u32,
                0,
            ).unwrap();
            if c.contains(naive_time) {
                continue;
            }
        }

        shown += 1;
        let delta = next - now;
        let delta_str = format_duration(delta);
        println!("  {}  ({})", next.format("%Y-%m-%d %H:%M %a"), delta_str);
    }

    if shown == 0 {
        println!("  (no runs within the next {} candidates)", count * 3);
    }

    Ok(())
}

fn format_duration(d: chrono::Duration) -> String {
    let total_mins = d.num_minutes();
    if total_mins < 60 {
        format!("in {} min", total_mins)
    } else if total_mins < 60 * 24 {
        let hours = total_mins / 60;
        let mins = total_mins % 60;
        if mins == 0 {
            format!("in {}h", hours)
        } else {
            format!("in {}h {}m", hours, mins)
        }
    } else {
        let days = total_mins / (60 * 24);
        let hours = (total_mins % (60 * 24)) / 60;
        format!("in {}d {}h", days, hours)
    }
}

fn cmd_export(output: Option<String>) -> Result<()> {
    let store = JobStore::load()?;
    let jobs = store.list();

    let now = chrono::Local::now();
    let mut md = String::new();

    md.push_str(&format!(
        "# Scheduled Tasks\n\n*Generated: {}*\n\n",
        now.format("%Y-%m-%d %H:%M")
    ));

    md.push_str("| Job | Schedule | Status | Constraints |\n");
    md.push_str("|-----|----------|--------|-------------|\n");

    for job in &jobs {
        let schedule_str = match &job.schedule {
            Schedule::Cron { expr, .. } => format!("`{}`", expr),
            Schedule::Once { at, .. } => format!(
                "once: {}",
                at.with_timezone(&Local).format("%Y-%m-%d %H:%M")
            ),
        };

        let status = if job.enabled { "enabled" } else { "disabled" };

        let mut constraints = Vec::new();
        if job.constraints.dnd_aware {
            constraints.push("DND-aware".to_string());
        }
        for nd in &job.constraints.not_during {
            constraints.push(format!("not {}-{}", nd.start.format("%H:%M"), nd.end.format("%H:%M")));
        }
        let constraints_str = if constraints.is_empty() {
            "—".to_string()
        } else {
            constraints.join(", ")
        };

        md.push_str(&format!(
            "| **{}** (`{}`) | {} | {} | {} |\n",
            job.name, job.id, schedule_str, status, constraints_str
        ));
    }

    // Add recent history
    if let Ok(executions) = history::query_history(None, false, 10) {
        if !executions.is_empty() {
            md.push_str("\n## Recent Executions\n\n");
            for exec in &executions {
                let local_time: chrono::DateTime<chrono::Local> = exec.started_at.into();
                let time_str = local_time.format("%Y-%m-%d %H:%M");

                if let Some(ref reason) = exec.skipped_reason {
                    md.push_str(&format!(
                        "- {} **{}** — skipped ({})\n",
                        time_str, exec.job_name, reason
                    ));
                } else {
                    let status = match exec.exit_code {
                        Some(0) => "✓".to_string(),
                        Some(code) => format!("✗ exit {}", code),
                        None => "running…".to_string(),
                    };
                    let dur = exec.duration_ms
                        .map(|ms| if ms < 1000 { format!("{}ms", ms) } else { format!("{:.1}s", ms as f64 / 1000.0) })
                        .unwrap_or_default();
                    md.push_str(&format!(
                        "- {} **{}** — {} {}\n",
                        time_str, exec.job_name, status, dur
                    ));
                }
            }
        }
    }

    match output {
        Some(path) => {
            std::fs::write(&path, &md)?;
            println!("Exported to {}", path);
        }
        None => print!("{}", md),
    }

    Ok(())
}

fn cmd_history(job: Option<String>, failed: bool, limit: usize, json: bool) -> Result<()> {
    let executions = history::query_history(job.as_deref(), failed, limit)?;

    if json {
        let json_val: Vec<serde_json::Value> = executions
            .iter()
            .map(|e| {
                serde_json::json!({
                    "job_id": e.job_id,
                    "job_name": e.job_name,
                    "started_at": e.started_at.to_rfc3339(),
                    "finished_at": e.finished_at.map(|t| t.to_rfc3339()),
                    "exit_code": e.exit_code,
                    "duration_ms": e.duration_ms,
                    "skipped_reason": e.skipped_reason,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_val)?);
    } else {
        if let Some(ref j) = job {
            println!("History for '{}':", j);
            // Show stats
            if let Ok((total, successes, skips, avg_dur)) = history::job_stats(j) {
                let fail_count = total - successes;
                let avg_str = avg_dur
                    .map(|ms| {
                        if ms < 1000.0 { format!("{:.0}ms", ms) }
                        else if ms < 60_000.0 { format!("{:.1}s", ms / 1000.0) }
                        else { format!("{:.0}m", ms / 60_000.0) }
                    })
                    .unwrap_or_else(|| "n/a".to_string());
                println!(
                    "  runs: {} | success: {} | failed: {} | skipped: {} | avg duration: {}",
                    total, successes, fail_count, skips, avg_str
                );
            }
            println!();
        } else {
            println!("Recent execution history:");
            println!();
        }
        history::print_history(&executions);
    }

    Ok(())
}

fn cmd_sync() -> Result<()> {
    let mut store = JobStore::load()?;
    let jobs: Vec<_> = store.list().iter().map(|j| (*j).clone()).collect();

    let mut fixed = 0;
    let mut ok = 0;
    let mut skipped = 0;
    let mut orphans_cleaned = 0;

    // Phase 1: Ensure all jobs have working timers.
    // One-shot jobs whose fire time has already elapsed are skipped — there
    // is nothing to recreate; the timer has done its job.
    for job in &jobs {
        let already_fired = matches!(
            &job.schedule,
            Schedule::Once { at, .. } if *at <= chrono::Utc::now()
        );
        if already_fired {
            skipped += 1;
            continue;
        }

        let unit_name = job.schedule.handle().unwrap_or("");

        let needs_rewrite = if unit_name.is_empty() {
            Some("no unit recorded")
        } else if !systemd::verify_timer(unit_name) {
            Some("missing timer")
        } else if systemd::timer_is_legacy(unit_name) {
            Some("legacy ExecStart format")
        } else {
            None
        };

        match needs_rewrite {
            None => ok += 1,
            Some(reason) => {
                println!("Rewriting timer for '{}' ({})...", job.id, reason);
                match systemd::recreate_timer(job) {
                    Ok(new_unit) => {
                        if let Some(j) = store.get_mut(&job.id) {
                            j.schedule.set_handle(new_unit.clone());
                        }
                        println!("  Wrote: {}", new_unit);
                        fixed += 1;
                    }
                    Err(e) => {
                        eprintln!("  Failed to recreate timer for '{}': {}", job.id, e);
                    }
                }
            }
        }
    }

    // Phase 2: Clean orphaned timers (exist in systemd but not in jobs.json)
    let known_units: std::collections::HashSet<String> = jobs
        .iter()
        .filter_map(|j| j.schedule.handle().map(|h| h.to_string()))
        .collect();

    let orphaned = systemd::find_orphaned_timers(&known_units)?;
    for orphan in &orphaned {
        println!("Cleaning orphaned timer: {}", orphan);
        if let Err(e) = systemd::remove_timer(orphan) {
            eprintln!("  Failed to remove orphan '{}': {}", orphan, e);
        } else {
            orphans_cleaned += 1;
        }
    }

    store.save()?;

    println!();
    println!(
        "Sync complete: {} ok, {} fixed, {} orphans cleaned, {} skipped (already-fired one-offs)",
        ok, fixed, orphans_cleaned, skipped
    );

    Ok(())
}

fn cmd_check() -> Result<()> {
    let store = JobStore::load()?;
    let jobs = store.list();

    if jobs.is_empty() {
        println!("No jobs configured.");
        return Ok(());
    }

    let mut errors = 0;

    for job in &jobs {
        let mut issues = Vec::new();

        // Check if command binary exists
        if !job.command.is_empty() {
            let bin = &job.command[0];
            let exists = std::process::Command::new("which")
                .arg(bin)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if !exists {
                issues.push(format!("command '{}' not found in PATH", bin));
            }
        }

        // Check timer health via the owning backend.
        // For one-shot jobs whose fire time has elapsed, the timer is
        // expected to be "inactive" — that's a successful run, not an error.
        let already_fired = matches!(
            &job.schedule,
            Schedule::Once { at, .. } if *at <= chrono::Utc::now()
        );
        if !already_fired {
            match job.schedule.handle() {
                Some(handle) => {
                    if !job.schedule.backend().verify(handle) {
                        issues.push(format!("timer '{}' not enabled/healthy", handle));
                    }
                }
                None => {
                    issues.push("no systemd unit recorded".to_string());
                }
            }
        }

        // Check for duplicate names
        let name_count = jobs.iter().filter(|j| j.name == job.name).count();
        if name_count > 1 {
            issues.push(format!("duplicate name '{}' ({} jobs)", job.name, name_count));
        }

        // Print result
        if issues.is_empty() {
            println!("✓ {} ({})", job.id, job.name);
        } else {
            println!("✗ {} ({}):", job.id, job.name);
            for issue in &issues {
                println!("    - {}", issue);
                errors += 1;
            }
        }
    }

    // Check for orphaned timers
    let known_units: std::collections::HashSet<String> = jobs
        .iter()
        .filter_map(|j| j.schedule.handle().map(|h| h.to_string()))
        .collect();

    let orphaned = systemd::find_orphaned_timers(&known_units)?;
    if !orphaned.is_empty() {
        println!();
        println!("Orphaned timers ({}):", orphaned.len());
        for orphan in &orphaned {
            println!("  ✗ {} (not in jobs.json — run 'usched sync' to clean)", orphan);
            errors += 1;
        }
    }

    println!();
    if errors == 0 {
        println!("All checks passed.");
    } else {
        println!("{} issue(s) found. Run 'usched sync' to fix timer issues.", errors);
    }

    Ok(())
}
