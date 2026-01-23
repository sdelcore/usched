mod at;
mod cron_convert;
mod dnd;
mod job;
mod store;
mod systemd;

use anyhow::Result;
use chrono::Utc;
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

    /// Run a job immediately (for testing)
    Run {
        /// Job ID
        id: String,
    },

    /// Show upcoming scheduled runs
    Next,

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
        Commands::Enable { id } => cmd_enable(&id),
        Commands::Disable { id } => cmd_disable(&id),
        Commands::Run { id } => cmd_run(&id),
        Commands::Next => cmd_next(),
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
