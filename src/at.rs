use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
use std::io::Write;
use std::process::{Command, Stdio};

/// Schedule a one-off job using the `at` command.
///
/// The queued shell line is `usched __run-job <id>` — the runner reads the
/// command from `jobs.json`, so the at queue doesn't carry a copy.
///
/// Returns the at job number.
pub fn schedule_at(
    job_id: &str,
    at_time: DateTime<Utc>,
    _command: &[String],
) -> Result<String> {
    let local_time: DateTime<Local> = at_time.into();
    let time_spec = local_time.format("%H:%M %Y-%m-%d").to_string();

    let usched_path = get_usched_path();
    let full_command = format!(
        "{} __run-job {}",
        shell_escape(&usched_path),
        shell_escape(job_id),
    );

    let mut child = Command::new("at")
        .arg(&time_spec)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn 'at' command")?;

    if let Some(ref mut stdin) = child.stdin {
        writeln!(stdin, "{}", full_command)?;
    }

    let output = child.wait_with_output()?;

    // at writes to stderr even on success
    let stderr = String::from_utf8_lossy(&output.stderr);

    let job_num = parse_at_job_number(&stderr).unwrap_or_else(|| "unknown".to_string());

    Ok(job_num)
}

/// Remove an at job.
///
/// On hosts where `atd` isn't running (e.g. NixOS without
/// `services.atd`), `atrm` itself fails with `Cannot get uid for atd`
/// because libc's `getpwnam("atd")` returns NULL. The job in the at
/// queue (if any) is harmless without atd to fire it, so we treat that
/// case as a warning rather than a hard error — usched's own metadata
/// is the source of truth and the caller still wants the job gone.
/// Any other atrm failure is propagated.
pub fn remove_at(job_num: &str) -> Result<()> {
    let output = Command::new("atrm")
        .arg(job_num)
        .output()
        .context("Failed to run atrm")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("cannot find") {
            // atrm couldn't find that job number — already gone.
            return Ok(());
        }
        if stderr.contains("Cannot get uid for atd") {
            eprintln!(
                "warning: atrm could not reach atd ({}). \
                 The at-queue entry (if any) was left untouched, but the \
                 job has been removed from usched. Enable services.atd if \
                 you want at-queue cleanup to succeed.",
                stderr.trim()
            );
            return Ok(());
        }
        anyhow::bail!("atrm failed: {}", stderr);
    }

    Ok(())
}

/// List pending at jobs.
pub fn list_at_jobs() -> Result<String> {
    let output = Command::new("atq").output().context("Failed to run atq")?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Resolve an absolute path to the `usched` binary for the at-queued line.
/// Mirrors `systemd::get_usched_path` — kept here to avoid a circular import.
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

    "usched".to_string()
}

fn shell_escape(s: &str) -> String {
    if s.contains(' ') || s.contains('\'') || s.contains('"') || s.contains('$') {
        format!("'{}'", s.replace('\'', "'\\''"))
    } else {
        s.to_string()
    }
}

fn parse_at_job_number(output: &str) -> Option<String> {
    for line in output.lines() {
        if line.starts_with("job ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                return Some(parts[1].to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_at_job_number_typical() {
        let out = "warning: commands will be executed using /bin/sh\njob 17 at Thu Jan 16 14:00:00 2025\n";
        assert_eq!(parse_at_job_number(out), Some("17".to_string()));
    }

    #[test]
    fn parse_at_job_number_missing() {
        assert_eq!(parse_at_job_number("nothing here"), None);
    }

    #[test]
    fn shell_escape_basic() {
        assert_eq!(shell_escape("plain"), "plain");
        assert_eq!(shell_escape("with space"), "'with space'");
        assert_eq!(shell_escape("$var"), "'$var'");
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }
}
