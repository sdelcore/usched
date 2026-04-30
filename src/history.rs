//! Execution history stored in SQLite.
//!
//! Records when jobs ran, how long they took, and whether they succeeded.
//! Written by the [`runner`](crate::runner) module; queried by the
//! `usched history` subcommand.

use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
use std::path::PathBuf;

/// A single execution record
#[derive(Debug)]
pub struct Execution {
    pub job_id: String,
    pub job_name: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<i64>,
    pub skipped_reason: Option<String>,
}

fn db_path() -> PathBuf {
    crate::store::get_data_dir().join("history.db")
}

/// Open or create the history database
fn open_db() -> Result<rusqlite::Connection> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = rusqlite::Connection::open(&path)
        .with_context(|| format!("Failed to open history database at {}", path.display()))?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS executions (
            id INTEGER PRIMARY KEY,
            job_id TEXT NOT NULL,
            job_name TEXT NOT NULL,
            started_at TEXT NOT NULL,
            finished_at TEXT,
            exit_code INTEGER,
            duration_ms INTEGER,
            skipped_reason TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_exec_job_id ON executions(job_id);
        CREATE INDEX IF NOT EXISTS idx_exec_started ON executions(started_at);",
    )?;

    Ok(conn)
}

/// Record a job execution start. Returns the row ID for later update.
pub fn record_start(job_id: &str, job_name: &str) -> Result<i64> {
    let conn = open_db()?;
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO executions (job_id, job_name, started_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![job_id, job_name, now],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Record a job execution completion.
pub fn record_finish(row_id: i64, exit_code: i32, duration_ms: i64) -> Result<()> {
    let conn = open_db()?;
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE executions SET finished_at = ?1, exit_code = ?2, duration_ms = ?3 WHERE id = ?4",
        rusqlite::params![now, exit_code, duration_ms, row_id],
    )?;
    Ok(())
}

/// Record a skipped execution (DND, constraint, disabled).
pub fn record_skip(job_id: &str, job_name: &str, reason: &str) -> Result<()> {
    let conn = open_db()?;
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO executions (job_id, job_name, started_at, skipped_reason) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![job_id, job_name, now, reason],
    )?;
    Ok(())
}

/// Query execution history
pub fn query_history(
    job_filter: Option<&str>,
    failed_only: bool,
    limit: usize,
) -> Result<Vec<Execution>> {
    let conn = open_db()?;

    let mut sql = String::from(
        "SELECT job_id, job_name, started_at, finished_at, exit_code, duration_ms, skipped_reason
         FROM executions WHERE 1=1",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(filter) = job_filter {
        sql.push_str(" AND (job_id = ? OR job_name = ?)");
        params.push(Box::new(filter.to_string()));
        params.push(Box::new(filter.to_string()));
    }

    if failed_only {
        sql.push_str(" AND (exit_code IS NOT NULL AND exit_code != 0)");
    }

    sql.push_str(" ORDER BY started_at DESC LIMIT ?");
    params.push(Box::new(limit as i64));

    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        Ok(Execution {
            job_id: row.get(0)?,
            job_name: row.get(1)?,
            started_at: row.get::<_, String>(2)?
                .parse()
                .unwrap_or_else(|_| Utc::now()),
            finished_at: row.get::<_, Option<String>>(3)?
                .and_then(|s| s.parse().ok()),
            exit_code: row.get(4)?,
            duration_ms: row.get(5)?,
            skipped_reason: row.get(6)?,
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Print execution history in a human-readable format
pub fn print_history(executions: &[Execution]) {
    if executions.is_empty() {
        println!("No execution history.");
        return;
    }

    for exec in executions {
        let local_time: DateTime<Local> = exec.started_at.into();
        let time_str = local_time.format("%Y-%m-%d %H:%M:%S");

        if let Some(ref reason) = exec.skipped_reason {
            println!(
                "  {} {} ({}) — skipped: {}",
                time_str, exec.job_id, exec.job_name, reason
            );
        } else {
            let status = match exec.exit_code {
                Some(0) => "✓".to_string(),
                Some(code) => format!("✗ exit {}", code),
                None => "… running".to_string(),
            };
            let duration = exec
                .duration_ms
                .map(|ms| {
                    if ms < 1000 {
                        format!("{}ms", ms)
                    } else if ms < 60_000 {
                        format!("{:.1}s", ms as f64 / 1000.0)
                    } else {
                        format!("{}m {}s", ms / 60_000, (ms % 60_000) / 1000)
                    }
                })
                .unwrap_or_default();

            println!(
                "  {} {} ({}) — {} {}",
                time_str, exec.job_id, exec.job_name, status, duration
            );
        }
    }
}

/// Get summary stats for a job
pub fn job_stats(job_id: &str) -> Result<(i64, i64, i64, Option<f64>)> {
    let conn = open_db()?;

    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM executions WHERE job_id = ?1 AND skipped_reason IS NULL",
        rusqlite::params![job_id],
        |row| row.get(0),
    )?;

    let successes: i64 = conn.query_row(
        "SELECT COUNT(*) FROM executions WHERE job_id = ?1 AND exit_code = 0",
        rusqlite::params![job_id],
        |row| row.get(0),
    )?;

    let skips: i64 = conn.query_row(
        "SELECT COUNT(*) FROM executions WHERE job_id = ?1 AND skipped_reason IS NOT NULL",
        rusqlite::params![job_id],
        |row| row.get(0),
    )?;

    let avg_duration: Option<f64> = conn.query_row(
        "SELECT AVG(duration_ms) FROM executions WHERE job_id = ?1 AND exit_code = 0",
        rusqlite::params![job_id],
        |row| row.get(0),
    )?;

    Ok((total, successes, skips, avg_duration))
}
