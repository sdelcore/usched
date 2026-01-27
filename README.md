# usched - Unified Scheduler

A Rust CLI tool that provides a unified interface for scheduling jobs across multiple backends: **systemd timers** for recurring jobs and **at** for one-time jobs.

## Features

- **Dual Backends** - systemd timers (recurring) + at command (one-time)
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
| Relative | `in 2 hours`, `in 30 minutes` |
| Tomorrow | `tomorrow 14:00` |
| Absolute | `2024-01-20 15:00` |
| Time only | `14:00` (today or tomorrow) |

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

Jobs are wrapped with `usched-run` which validates constraints before execution:

```
at/systemd triggers → usched-run <job-id> <command>
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

### Storage

- Jobs: `~/.local/share/usched/jobs.json`
- State: `~/.local/share/usched/state.json` (DND timestamp)

## Integration with ARIA

usched is designed to work with ARIA for:
- Scheduling periodic vault reviews
- Time-based notifications with DND awareness
- Remote job monitoring patterns
- Maintenance tasks

Example ARIA integration:

```bash
# Morning briefing
usched add --name "briefing" --cron "0 9 * * *" --dnd-aware -- aria briefing

# Proactive checks (not during sleep)
usched add --name "proactive" --cron "*/30 * * * *" \
  --not-during "22:00-08:00" --dnd-aware -- aria review
```

## Building

```bash
# With Nix
nix build

# With Cargo
cargo build --release
```

The build installs both:
- `usched` - Main CLI binary
- `usched-run` - Wrapper script for constraint enforcement
