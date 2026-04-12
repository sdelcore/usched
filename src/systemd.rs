use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::cron_convert::cron_to_oncalendar;
use crate::job::Job;
use crate::store::get_data_dir;

/// Get the systemd user unit directory.
fn get_systemd_user_dir() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME not set");
    PathBuf::from(home).join(".config/systemd/user")
}

/// Create a persistent systemd user timer for a recurring job.
///
/// Writes .timer and .service unit files to ~/.config/systemd/user/,
/// then enables the timer. These units persist across reboots and daemon-reload.
pub fn create_timer(job_id: &str, cron_expr: &str, command: &[String]) -> Result<String> {
    let oncalendar = cron_to_oncalendar(cron_expr)?;
    let unit_name = format!("usched-{}", job_id);
    let systemd_dir = get_systemd_user_dir();
    fs::create_dir_all(&systemd_dir)?;

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

    // Capture current PATH for the service environment
    let path_env = std::env::var("PATH").unwrap_or_default();

    // Write .timer file with persistence enhancements
    let timer_content = format!(
        r#"[Unit]
Description=usched job: {job_id}

[Timer]
OnCalendar={oncalendar}
Persistent=true
OnBootSec=2min
RandomizedDelaySec=30

[Install]
WantedBy=timers.target
"#
    );
    fs::write(
        systemd_dir.join(format!("{}.timer", unit_name)),
        timer_content,
    )?;

    // Resolve bash path (NixOS doesn't have /bin/bash)
    let bash_path = get_bash_path();

    // Write .service file
    let service_content = format!(
        r#"[Unit]
Description=usched job: {job_id}

[Service]
Type=oneshot
Environment="PATH={path_env}"
ExecStart={bash_path} -c '{full_command}'
"#
    );
    fs::write(
        systemd_dir.join(format!("{}.service", unit_name)),
        service_content,
    )?;

    // Reload systemd to pick up new units
    let output = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output()
        .context("Failed to daemon-reload")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("daemon-reload failed: {}", stderr);
    }

    // Enable and start the timer
    let output = Command::new("systemctl")
        .args(["--user", "enable", "--now", &format!("{}.timer", unit_name)])
        .output()
        .context("Failed to enable timer")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to enable timer: {}", stderr);
    }

    Ok(unit_name)
}

/// Remove a systemd timer and its unit files.
pub fn remove_timer(unit_name: &str) -> Result<()> {
    // Stop the timer
    let _ = Command::new("systemctl")
        .args(["--user", "stop", &format!("{}.timer", unit_name)])
        .output();

    // Disable the timer
    let _ = Command::new("systemctl")
        .args(["--user", "disable", &format!("{}.timer", unit_name)])
        .output();

    // Also stop the service if running
    let _ = Command::new("systemctl")
        .args(["--user", "stop", &format!("{}.service", unit_name)])
        .output();

    // Delete unit files
    let systemd_dir = get_systemd_user_dir();
    let _ = fs::remove_file(systemd_dir.join(format!("{}.timer", unit_name)));
    let _ = fs::remove_file(systemd_dir.join(format!("{}.service", unit_name)));

    // Reload daemon to reflect removed units
    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output();

    Ok(())
}

/// Enable a disabled timer.
pub fn enable_timer(unit_name: &str) -> Result<()> {
    let output = Command::new("systemctl")
        .args(["--user", "enable", "--now", &format!("{}.timer", unit_name)])
        .output()
        .context("Failed to enable timer")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to enable timer: {}", stderr);
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

    // Also disable so it doesn't start on next boot
    let _ = Command::new("systemctl")
        .args(["--user", "disable", &format!("{}.timer", unit_name)])
        .output();

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

/// Verify that a timer exists and is properly configured.
/// Returns true if the timer is healthy, false if it needs recreation.
pub fn verify_timer(unit_name: &str) -> bool {
    let systemd_dir = get_systemd_user_dir();

    // Check if unit files exist
    let timer_path = systemd_dir.join(format!("{}.timer", unit_name));
    let service_path = systemd_dir.join(format!("{}.service", unit_name));

    if !timer_path.exists() || !service_path.exists() {
        return false;
    }

    // Check if timer is loaded in systemd
    let output = Command::new("systemctl")
        .args([
            "--user",
            "is-enabled",
            &format!("{}.timer", unit_name),
        ])
        .output();

    match output {
        Ok(o) => {
            let status = String::from_utf8_lossy(&o.stdout).trim().to_string();
            // Timer should be "enabled" or "static"
            status == "enabled" || status == "static"
        }
        Err(_) => false,
    }
}

/// Find systemd usched timers that aren't in the known units set (orphans).
pub fn find_orphaned_timers(known_units: &std::collections::HashSet<String>) -> Result<Vec<String>> {
    let systemd_dir = get_systemd_user_dir();
    let mut orphaned = Vec::new();

    if !systemd_dir.exists() {
        return Ok(orphaned);
    }

    for entry in fs::read_dir(&systemd_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();

        // Only check usched timer files
        if name.starts_with("usched-") && name.ends_with(".timer") {
            let unit_name = name.trim_end_matches(".timer").to_string();
            if !known_units.contains(&unit_name) {
                orphaned.push(unit_name);
            }
        }
    }

    Ok(orphaned)
}

/// Recreate a timer from job metadata.
pub fn recreate_timer(job: &Job) -> Result<String> {
    if let crate::job::Schedule::Cron { expr } = &job.schedule {
        create_timer(&job.id, expr, &job.command)
    } else {
        anyhow::bail!("Cannot recreate timer for non-cron job")
    }
}

/// Resolve the absolute path to bash.
/// On NixOS, /bin/bash doesn't exist so we need to find it dynamically.
fn get_bash_path() -> String {
    if let Ok(output) = std::process::Command::new("which")
        .arg("bash")
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return path;
            }
        }
    }

    // Fallback for standard Linux
    "/bin/bash".to_string()
}

fn get_wrapper_path() -> String {
    // Look for usched-run in the data directory
    let data_dir = get_data_dir();
    let wrapper_in_data = data_dir.join("usched-run");
    if wrapper_in_data.exists() {
        return wrapper_in_data.to_string_lossy().to_string();
    }

    // Try to find usched-run using `which` to get absolute path
    // (systemd user services don't have the full user PATH)
    if let Ok(output) = std::process::Command::new("which")
        .arg("usched-run")
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return path;
            }
        }
    }

    // Check common nix profile locations
    let home = std::env::var("HOME").unwrap_or_default();
    let nix_profile = format!("{}/.nix-profile/bin/usched-run", home);
    if std::path::Path::new(&nix_profile).exists() {
        return nix_profile;
    }

    // Last resort - hope it's in PATH (unlikely to work for systemd)
    "usched-run".to_string()
}

fn shell_escape(s: &str) -> String {
    if s.contains(' ') || s.contains('\'') || s.contains('"') || s.contains('$') {
        format!("'{}'", s.replace('\'', "'\\''"))
    } else {
        s.to_string()
    }
}
