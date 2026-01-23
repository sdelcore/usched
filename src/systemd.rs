use anyhow::{Context, Result};
use std::process::Command;

use crate::cron_convert::cron_to_oncalendar;
use crate::store::get_data_dir;

/// Create a systemd user timer for a recurring job.
///
/// Uses `systemd-run --user` to create a transient timer that persists across reboots.
pub fn create_timer(
    job_id: &str,
    cron_expr: &str,
    command: &[String],
) -> Result<String> {
    let oncalendar = cron_to_oncalendar(cron_expr)?;
    let unit_name = format!("usched-{}", job_id);

    // Build the command to run via usched-run wrapper
    let wrapper_path = get_wrapper_path();
    let full_command = format!(
        "{} {} {}",
        wrapper_path,
        job_id,
        command
            .iter()
            .map(|s| shell_escape(s))
            .collect::<Vec<_>>()
            .join(" ")
    );

    let output = Command::new("systemd-run")
        .args([
            "--user",
            "--unit",
            &unit_name,
            "--on-calendar",
            &oncalendar,
            "--timer-property=Persistent=true",
            "--",
            "bash",
            "-c",
            &full_command,
        ])
        .output()
        .context("Failed to run systemd-run")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("systemd-run failed: {}", stderr);
    }

    Ok(unit_name)
}

/// Remove a systemd timer.
pub fn remove_timer(unit_name: &str) -> Result<()> {
    // Stop the timer
    let _ = Command::new("systemctl")
        .args(["--user", "stop", &format!("{}.timer", unit_name)])
        .output();

    // Also stop the service if running
    let _ = Command::new("systemctl")
        .args(["--user", "stop", &format!("{}.service", unit_name)])
        .output();

    Ok(())
}

/// Enable a disabled timer.
pub fn enable_timer(unit_name: &str) -> Result<()> {
    let output = Command::new("systemctl")
        .args(["--user", "start", &format!("{}.timer", unit_name)])
        .output()
        .context("Failed to start timer")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to start timer: {}", stderr);
    }

    Ok(())
}

/// Disable a timer without removing it.
pub fn disable_timer(unit_name: &str) -> Result<()> {
    let output = Command::new("systemctl")
        .args(["--user", "stop", &format!("{}.timer", unit_name)])
        .output()
        .context("Failed to stop timer")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to stop timer: {}", stderr);
    }

    Ok(())
}

/// Get next trigger times for all user timers.
pub fn list_timers() -> Result<String> {
    let output = Command::new("systemctl")
        .args(["--user", "list-timers", "--all"])
        .output()
        .context("Failed to list timers")?;

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn get_wrapper_path() -> String {
    // Look for usched-run in the data directory or PATH
    let data_dir = get_data_dir();
    let wrapper_in_data = data_dir.join("uusched-run");
    if wrapper_in_data.exists() {
        return wrapper_in_data.to_string_lossy().to_string();
    }

    // Fall back to expecting it in PATH
    "uusched-run".to_string()
}

fn shell_escape(s: &str) -> String {
    if s.contains(' ') || s.contains('\'') || s.contains('"') || s.contains('$') {
        format!("'{}'", s.replace('\'', "'\\''"))
    } else {
        s.to_string()
    }
}
