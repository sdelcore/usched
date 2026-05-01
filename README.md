# usched - Unified Scheduler

A Rust CLI tool that provides a unified interface for scheduling jobs on top of **systemd user timers** — both recurring and one-shot.

## Features

- **systemd-only backend** - both cron-style recurring jobs and `--once`
  one-shots are dispatched via persistent `.timer`/`.service` units. No
  runtime dependency on `at(1)`/`atd`/`atrm`.
- **Cron Expressions** - Full cron syntax for recurring schedules
- **Natural Datetime** - "tomorrow 14:00", "in 2 hours", "2024-01-20 15:00"
- **Do Not Disturb** - Skip jobs during DND periods
- **Time Constraints** - Never run during specific hours (e.g., "22:00-08:00")
- **Auto-Removal** - Jobs can remove themselves on success
- **Constraint Enforcement** - Runtime validation before execution

## Quick Start

```bash
# Enter development environment
nix develop

# Build
cargo build

# Schedule a recurring job (every weekday at 9 AM)
usched add --cron "0 9 * * 1-5" -- echo "Good morning"

# Schedule a one-time job
usched add --once "tomorrow 14:00" -- notify-send "Reminder"

# List all jobs
usched list

# Remove a job
usched remove <job-id>
```

## CLI Commands

### Scheduling Jobs

```bash
# Recurring job with cron
usched add --cron "0 9 * * 1-5" -- command args

# One-time job
usched add --once "tomorrow 14:00" -- command args
usched add --once "in 2 hours" -- command args
usched add --once "2024-01-20 15:00" -- command args

# With constraints
usched add --cron "*/30 * * * *" \
  --not-during "22:00-08:00" \
  --dnd-aware \
  -- command args

# Auto-remove on success
usched add --once "in 1 hour" --remove-on-success -- command args

# Custom job name
usched add --name "morning-check" --cron "0 9 * * *" -- command args
```

### Managing Jobs

```bash
usched list              # List all jobs
usched list --json       # JSON output
usched remove <job-id>   # Remove job
usched enable <job-id>   # Enable disabled job
usched disable <job-id>  # Disable without removing
usched run <job-id>      # Execute immediately (for testing)
usched next              # Show upcoming runs
```

### Do Not Disturb

```bash
usched dnd set 2h        # Enable DND for 2 hours
usched dnd set 30m       # Enable for 30 minutes
usched dnd set 1h30m     # Enable for 1.5 hours
usched dnd off           # Disable DND
usched dnd status        # Show current DND state
```

## Cron Expressions

Standard 5-field cron format: `minute hour day month day-of-week`

```
# Examples
0 9 * * *        # Every day at 9 AM
0 9 * * 1-5      # Weekdays at 9 AM
*/30 * * * *     # Every 30 minutes
0 */2 * * *      # Every 2 hours
0 9,18 * * *     # 9 AM and 6 PM daily
```

## Datetime Formats

For `--once` scheduling:

| Format | Example |
|--------|---------|
| Relative | `in 2 hours`, `in 30 minutes`, `in 3 days` |
| Today | `today 23:30` |
| Tomorrow | `tomorrow 14:00` |
| Absolute | `2024-01-20 15:00`, `2024-01-20 15:00:30` |
| Time only | `14:00` / `14:00:30` (today if still future, otherwise tomorrow) |

## Time Constraints

### `--not-during`

Prevent jobs from running during specific hours:

```bash
# Don't run between 10 PM and 8 AM
usched add --cron "*/30 * * * *" --not-during "22:00-08:00" -- command

# Midnight wrap is supported
--not-during "23:00-06:00"
```

### `--dnd-aware`

Skip execution if DND mode is active:

```bash
usched add --cron "0 * * * *" --dnd-aware -- notify-send "Hourly check"
```

## How It Works

### Constraint Enforcement

Jobs are wrapped via `usched __run-job <id>` which validates constraints
before execution:

```
systemd timer fires → usched __run-job <job-id>
                           │
                           ▼
                    Check enabled?
                           │
                           ▼
                    Check DND active?
                           │
                           ▼
                    Check time constraints?
                           │
                           ▼
                    Execute command
                           │
                           ▼
                    Handle auto-removal
```

### Migrating from at(1)

Hosts that previously ran usched against `atd` can migrate any pending
one-shots onto systemd timers idempotently:

```bash
usched migrate-from-at
```

This re-registers each `--once` job as a systemd user timer (using the same
absolute fire time), best-effort-removes the legacy at-queue entry (warns
rather than fails if `atd` is unreachable), and drops past-due entries that
can no longer be scheduled. Safe to run repeatedly.

### Storage

- Jobs: `~/.local/share/usched/jobs.json`
- State: `~/.local/share/usched/state.json` (DND timestamp)

## Building

```bash
# With Nix
nix build

# With Cargo
cargo build --release
```

The build installs:
- `usched` - Main CLI binary
- `usched-run` - Compatibility shim that forwards to `usched __run-job <id>`
  for unit files written by older versions. New installs don't need it; the
  shim exists so legacy `.service` units keep working until `usched sync`
  rewrites them.
