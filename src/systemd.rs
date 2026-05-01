use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::cron_convert::cron_to_oncalendar;
use crate::job::{Job, Schedule};

/// Get the systemd user unit directory.
fn get_systemd_user_dir() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME not set");
    PathBuf::from(home).join(".config/systemd/user")
}

/// Create a persistent systemd user timer for a recurring job.
///
/// Writes .timer and .service unit files to ~/.config/systemd/user/,
/// then enables the timer. These units persist across reboots and daemon-reload.
///
/// The service ExecStart is `usched __run-job <id>` — the runner reads the
/// command from `jobs.json`, so the unit doesn't carry a copy.
pub fn create_timer(job_id: &str, cron_expr: &str, _command: &[String]) -> Result<String> {
    let oncalendar = cron_to_oncalendar(cron_expr)?;
    let unit_name = format!("usched-{}", job_id);
    let systemd_dir = get_systemd_user_dir();
    fs::create_dir_all(&systemd_dir)?;

    let usched_path = get_usched_path();
    let exec_start = format!(
        "{} __run-job {}",
        systemd_quote(&usched_path),
        systemd_quote(job_id),
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

    // Write .service file
    let service_content = format!(
        r#"[Unit]
Description=usched job: {job_id}

[Service]
Type=oneshot
Environment="PATH={path_env}"
ExecStart={exec_start}
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
    match &job.schedule {
        Schedule::Cron { expr, .. } => create_timer(&job.id, expr, &job.command),
        Schedule::Once { at, .. } => create_oneshot_timer(&job.id, *at, &job.command),
    }
}

/// Create a persistent systemd user timer for a one-shot job.
///
/// Writes .timer and .service unit files to ~/.config/systemd/user/, with
/// `OnCalendar=<absolute timestamp>` and `Persistent=true`. Persistent
/// matters: if the host is suspended past the fire time, systemd fires on
/// resume rather than skipping. The timer is "one-shot" by virtue of the
/// `OnCalendar` being a single absolute moment — once it has elapsed, the
/// timer goes inactive and won't re-fire.
pub fn create_oneshot_timer(
    job_id: &str,
    at: DateTime<Utc>,
    _command: &[String],
) -> Result<String> {
    let local_at: DateTime<Local> = at.into();
    let oncalendar = local_at.format("%Y-%m-%d %H:%M:%S").to_string();

    let unit_name = format!("usched-{}", job_id);
    let systemd_dir = get_systemd_user_dir();
    fs::create_dir_all(&systemd_dir)?;

    let usched_path = get_usched_path();
    let exec_start = format!(
        "{} __run-job {}",
        systemd_quote(&usched_path),
        systemd_quote(job_id),
    );

    // Capture current PATH for the service environment
    let path_env = std::env::var("PATH").unwrap_or_default();

    // One-shot timers don't need OnBootSec/RandomizedDelaySec — those are
    // for recurring jobs that should spread load on boot. A one-shot job
    // just wants to fire at its absolute time (or on the next boot if it
    // was missed, courtesy of Persistent=true).
    let timer_content = format!(
        r#"[Unit]
Description=usched one-shot job: {job_id}

[Timer]
OnCalendar={oncalendar}
Persistent=true
Unit={unit_name}.service

[Install]
WantedBy=timers.target
"#
    );
    fs::write(
        systemd_dir.join(format!("{}.timer", unit_name)),
        timer_content,
    )?;

    let service_content = format!(
        r#"[Unit]
Description=usched one-shot job: {job_id}

[Service]
Type=oneshot
Environment="PATH={path_env}"
ExecStart={exec_start}
"#
    );
    fs::write(
        systemd_dir.join(format!("{}.service", unit_name)),
        service_content,
    )?;

    let output = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output()
        .context("Failed to daemon-reload")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("daemon-reload failed: {}", stderr);
    }

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

/// Resolve an absolute path to the `usched` binary for use in systemd
/// ExecStart lines. systemd user services don't have the full user PATH,
/// so we need to bake in an absolute path at unit-creation time.
fn get_usched_path() -> String {
    if let Ok(output) = std::process::Command::new("which")
        .arg("usched")
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return path;
            }
        }
    }

    let home = std::env::var("HOME").unwrap_or_default();
    let nix_profile = format!("{}/.nix-profile/bin/usched", home);
    if std::path::Path::new(&nix_profile).exists() {
        return nix_profile;
    }

    // Last resort
    "usched".to_string()
}

/// Detect whether a timer was generated by the pre-runner version of usched
/// (ExecStart points at the standalone `usched-run` wrapper instead of
/// `usched __run-job`). Used by `cmd_sync` to opportunistically rewrite.
pub fn timer_is_legacy(unit_name: &str) -> bool {
    let systemd_dir = get_systemd_user_dir();
    let service_path = systemd_dir.join(format!("{}.service", unit_name));
    match fs::read_to_string(&service_path) {
        Ok(content) => !content.contains("__run-job"),
        Err(_) => false,
    }
}

/// Quote a string for use as one argv element in a systemd ExecStart line.
///
/// Per systemd.exec(5), arguments are split on unquoted whitespace and may be
/// wrapped in double quotes to preserve them as a single argument. Inside
/// double quotes, `\` and `"` need to be backslash-escaped, `$` becomes `$$`
/// (otherwise systemd treats `$VAR` as env-variable expansion), and `%`
/// becomes `%%` (otherwise systemd treats `%h`, `%u`, etc. as specifiers).
///
/// Strings without any special characters are returned as-is.
fn systemd_quote(s: &str) -> String {
    let needs_quote = s.is_empty()
        || s.chars().any(|c| {
            c.is_whitespace()
                || matches!(c, '"' | '\\' | '\'' | '$' | '%' | '`' | ';' | '&' | '|')
        });
    if !needs_quote {
        return s.to_string();
    }
    // Order matters: escape backslashes first so we don't double-escape ones
    // we add when escaping `"`.
    let escaped = s
        .replace('\\', r"\\")
        .replace('"', "\\\"")
        .replace('$', "$$")
        .replace('%', "%%");
    format!("\"{}\"", escaped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_plain() {
        assert_eq!(systemd_quote("plain"), "plain");
        assert_eq!(systemd_quote("/usr/bin/touch"), "/usr/bin/touch");
        assert_eq!(systemd_quote("--cron"), "--cron");
    }

    #[test]
    fn quote_whitespace() {
        assert_eq!(systemd_quote("with space"), "\"with space\"");
        assert_eq!(systemd_quote("a\tb"), "\"a\tb\"");
    }

    #[test]
    fn quote_special_chars() {
        // Single quotes force double-quote wrapping
        assert_eq!(systemd_quote("it's"), "\"it's\"");
        // Embedded double quote
        assert_eq!(systemd_quote(r#"a"b"#), "\"a\\\"b\"");
        // Backslash
        assert_eq!(systemd_quote(r"a\b"), r#""a\\b""#);
        // Dollar — systemd would expand $VAR otherwise
        assert_eq!(systemd_quote("$VAR"), "\"$$VAR\"");
        // Percent — specifier expansion
        assert_eq!(systemd_quote("100%done"), "\"100%%done\"");
    }

    #[test]
    fn quote_empty() {
        assert_eq!(systemd_quote(""), "\"\"");
    }

    #[test]
    fn quote_realistic_shell_command() {
        // The exact case that exposed the bug: a quoted shell command passed
        // as a single argv element.
        assert_eq!(
            systemd_quote("echo fired > /home/testuser/marker"),
            "\"echo fired > /home/testuser/marker\""
        );
    }
}
