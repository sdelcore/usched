mod at;
mod cron_convert;
mod dnd;
mod job;
mod store;
mod systemd;

use anyhow::Result;
use chrono::{Timelike, Utc};
use clap::{Parser, Subcommand};

use job::{Constraints, Job, Schedule, TimeRange};
use store::JobStore;

#[derive(Parser)]
#[command(name = "usched")]
#[command(about = "Unified scheduler CLI wrapping systemd-run and at")]
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

    /// Run a job immediately (for testing)
    Run {
        /// Job ID
        id: String,
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

    /// Manage Do Not Disturb mode
    Dnd {
        #[command(subcommand)]
        action: DndAction,
    },
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
            command,
        } => cmd_add(name, cron, once, not_during, dnd_aware, remove_on_success, command),
        Commands::List { json } => cmd_list(json),
        Commands::Remove { id } => cmd_remove(&id),
        Commands::Edit { id, cron, name, not_during, clear_not_during, dnd_aware } => {
            cmd_edit(&id, cron, name, not_during, clear_not_during, dnd_aware)
        }
        Commands::Enable { id } => cmd_enable(&id),
        Commands::Disable { id } => cmd_disable(&id),
        Commands::Run { id } => cmd_run(&id),
        Commands::Next => cmd_next(),
        Commands::Preview { cron, count, not_during } => cmd_preview(&cron, count, not_during),
        Commands::Sync => cmd_sync(),
        Commands::Check => cmd_check(),
        Commands::Dnd { action } => match action {
            DndAction::Set { duration } => dnd::set_dnd(&duration),
            DndAction::Off => dnd::clear_dnd(),
            DndAction::Status => dnd::show_dnd_status(),
        },
    }
}

fn cmd_add(
    name: Option<String>,
    cron: Option<String>,
    once: Option<String>,
    not_during: Option<String>,
    dnd_aware: bool,
    remove_on_success: bool,
    command: Vec<String>,
) -> Result<()> {
    if cron.is_none() && once.is_none() {
        anyhow::bail!("Must specify either --cron or --once");
    }

    let job_name = name.unwrap_or_else(|| command.first().cloned().unwrap_or_else(|| "job".to_string()));
    let job_id = Job::generate_id(&job_name);

    let schedule = if let Some(cron_expr) = &cron {
        Schedule::Cron { expr: cron_expr.clone() }
    } else {
        let at_time = at::parse_datetime(once.as_ref().unwrap())?;
        Schedule::Once { at: at_time }
    };

    let constraints = Constraints {
        not_during: match not_during {
            Some(s) => vec![TimeRange::parse(&s)?],
            None => vec![],
        },
        only_during: vec![],
        dnd_aware,
        remove_on_success,
    };

    let mut job = Job {
        id: job_id.clone(),
        name: job_name,
        schedule: schedule.clone(),
        command: command.clone(),
        constraints,
        enabled: true,
        created_at: Utc::now(),
        created_by: "user".to_string(),
        systemd_unit: None,
        at_job: None,
    };

    // Schedule the job
    match &schedule {
        Schedule::Cron { expr } => {
            let unit = systemd::create_timer(&job_id, expr, &command)?;
            job.systemd_unit = Some(unit.clone());
            println!("Created recurring job '{}' with systemd timer '{}'", job_id, unit);
        }
        Schedule::Once { at } => {
            let job_num = at::schedule_at(&job_id, *at, &command)?;
            job.at_job = Some(job_num.clone());
            println!("Created one-off job '{}' with at job #{}", job_id, job_num);
        }
    }

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
                Schedule::Cron { expr } => format!("cron: {}", expr),
                Schedule::Once { at } => format!("once: {}", at.format("%Y-%m-%d %H:%M")),
            };

            let status = if job.enabled { "enabled" } else { "disabled" };

            let mut flags = vec![];
            if job.constraints.dnd_aware {
                flags.push("dnd-aware");
            }
            if job.constraints.remove_on_success {
                flags.push("auto-remove");
            }
            if !job.constraints.not_during.is_empty() {
                flags.push("has-constraints");
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

    // Clean up the underlying scheduler
    match &job.schedule {
        Schedule::Cron { .. } => {
            if let Some(unit) = &job.systemd_unit {
                systemd::remove_timer(unit)?;
                println!("Removed systemd timer '{}'", unit);
            }
        }
        Schedule::Once { .. } => {
            if let Some(job_num) = &job.at_job {
                at::remove_at(job_num)?;
                println!("Removed at job #{}", job_num);
            }
        }
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

    if let Some(unit) = &job.systemd_unit {
        systemd::enable_timer(unit)?;
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

    if let Some(unit) = &job.systemd_unit {
        systemd::disable_timer(unit)?;
    }

    job.enabled = false;
    store.save()?;
    println!("Disabled job '{}'", id);

    Ok(())
}

fn cmd_run(id: &str) -> Result<()> {
    let store = JobStore::load()?;

    let job = store.get(id).ok_or_else(|| anyhow::anyhow!("Job '{}' not found", id))?;

    println!("Running job '{}': {}", id, job.command.join(" "));

    let status = std::process::Command::new(&job.command[0])
        .args(&job.command[1..])
        .status()?;

    if status.success() {
        println!("Job completed successfully");
    } else {
        println!("Job failed with exit code: {:?}", status.code());
    }

    Ok(())
}

fn cmd_next() -> Result<()> {
    println!("=== Systemd Timers ===");
    let timers = systemd::list_timers()?;
    // Filter to only show sched timers
    for line in timers.lines() {
        if line.contains("usched-") || line.starts_with("NEXT") || line.starts_with("---") {
            println!("{}", line);
        }
    }

    println!("\n=== At Jobs ===");
    let at_jobs = at::list_at_jobs()?;
    if at_jobs.is_empty() {
        println!("No pending at jobs");
    } else {
        print!("{}", at_jobs);
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
        job.constraints.not_during = vec![TimeRange::parse(&nd)?];
        changes.push(format!("not_during → {}", nd));
    }

    // Update cron schedule — this requires recreating the systemd timer
    if let Some(new_cron) = cron {
        match &job.schedule {
            Schedule::Cron { .. } => {
                // Remove old timer
                if let Some(unit) = &job.systemd_unit {
                    systemd::remove_timer(unit)?;
                }

                // Update schedule
                job.schedule = Schedule::Cron { expr: new_cron.clone() };

                // Create new timer with same job ID
                let unit = systemd::create_timer(&job.id, &new_cron, &job.command)?;
                job.systemd_unit = Some(unit);

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
    let constraint = not_during.as_ref().map(|nd| TimeRange::parse(nd)).transpose()?;

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

fn cmd_sync() -> Result<()> {
    let mut store = JobStore::load()?;
    let jobs: Vec<_> = store.list().iter().map(|j| (*j).clone()).collect();

    let mut fixed = 0;
    let mut ok = 0;
    let mut skipped = 0;
    let mut orphans_cleaned = 0;

    // Phase 1: Ensure all jobs have working timers
    for job in &jobs {
        if let Schedule::Cron { .. } = &job.schedule {
            let unit_name = job.systemd_unit.as_ref().map(|s| s.as_str()).unwrap_or("");

            if unit_name.is_empty() {
                println!("Recreating timer for '{}' (no unit recorded)...", job.id);
                match systemd::recreate_timer(job) {
                    Ok(new_unit) => {
                        if let Some(j) = store.get_mut(&job.id) {
                            j.systemd_unit = Some(new_unit.clone());
                        }
                        println!("  Created: {}", new_unit);
                        fixed += 1;
                    }
                    Err(e) => {
                        eprintln!("  Failed to recreate timer for '{}': {}", job.id, e);
                    }
                }
            } else if !systemd::verify_timer(unit_name) {
                println!("Recreating missing timer for '{}' ({})...", job.id, unit_name);
                match systemd::recreate_timer(job) {
                    Ok(new_unit) => {
                        if let Some(j) = store.get_mut(&job.id) {
                            j.systemd_unit = Some(new_unit.clone());
                        }
                        println!("  Recreated: {}", new_unit);
                        fixed += 1;
                    }
                    Err(e) => {
                        eprintln!("  Failed to recreate timer for '{}': {}", job.id, e);
                    }
                }
            } else {
                ok += 1;
            }
        } else {
            skipped += 1;
        }
    }

    // Phase 2: Clean orphaned timers (exist in systemd but not in jobs.json)
    let known_units: std::collections::HashSet<String> = jobs
        .iter()
        .filter_map(|j| j.systemd_unit.clone())
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
        "Sync complete: {} ok, {} fixed, {} orphans cleaned, {} skipped (one-off)",
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

        // Check systemd timer health for cron jobs
        if let Schedule::Cron { .. } = &job.schedule {
            match &job.systemd_unit {
                Some(unit) => {
                    if !systemd::verify_timer(unit) {
                        issues.push(format!("timer '{}' not enabled/healthy", unit));
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
        .filter_map(|j| j.systemd_unit.clone())
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
