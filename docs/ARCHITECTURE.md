# usched Architecture

Technical architecture documentation for the unified scheduler.

## Overview

usched provides a unified interface for scheduling jobs across two backends:
- **systemd timers** for recurring jobs
- **at command** for one-time jobs

## Component Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                           usched CLI                                 │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐                 │
│  │     add     │  │    list     │  │   remove    │                 │
│  │  --cron     │  │   --json    │  │  <job-id>   │                 │
│  │  --once     │  │             │  │             │                 │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘                 │
│         │                │                │                         │
│         └────────────────┼────────────────┘                         │
│                          ▼                                          │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │                        Job Store                             │   │
│  │              ~/.local/share/usched/jobs.json                 │   │
│  └──────────────────────────┬──────────────────────────────────┘   │
│                             │                                       │
│              ┌──────────────┴──────────────┐                       │
│              ▼                             ▼                       │
│  ┌─────────────────────┐       ┌─────────────────────┐            │
│  │   systemd Backend   │       │     at Backend      │            │
│  │   (recurring jobs)  │       │   (one-time jobs)   │            │
│  │                     │       │                     │            │
│  │  - .timer units     │       │  - at command       │            │
│  │  - .service units   │       │  - atq/atrm         │            │
│  └─────────────────────┘       └─────────────────────┘            │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

## Constraint Enforcement

Jobs are wrapped with `usched-run` for runtime constraint checking:

```
┌─────────────────────────────────────────────────────────────────┐
│                      usched-run <job-id>                         │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  1. Load job from store                                         │
│     └─ Exit if job not found                                    │
│                                                                 │
│  2. Check job.enabled                                           │
│     └─ Exit silently if disabled                                │
│                                                                 │
│  3. Check DND status (if job.dnd_aware)                         │
│     └─ Load ~/.local/share/usched/state.json                    │
│     └─ Exit if DND active and not expired                       │
│                                                                 │
│  4. Check time constraints (if job.not_during)                  │
│     └─ Parse time range (e.g., "22:00-08:00")                   │
│     └─ Exit if current time within range                        │
│                                                                 │
│  5. Execute job command                                         │
│     └─ Capture exit code                                        │
│                                                                 │
│  6. Handle post-execution (if job.remove_on_success)            │
│     └─ Remove job if exit code == 0                             │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

## Data Model

### Job Structure

```rust
pub struct Job {
    pub id: String,
    pub name: Option<String>,
    pub command: Vec<String>,
    pub schedule: Schedule,
    pub constraints: Constraints,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

pub enum Schedule {
    Cron(String),           // "0 9 * * 1-5"
    Once(DateTime<Local>),  // One-time execution
}

pub struct Constraints {
    pub dnd_aware: bool,
    pub not_during: Option<TimeRange>,
    pub remove_on_success: bool,
}
```

### State Structure

```rust
pub struct State {
    pub dnd_until: Option<DateTime<Local>>,
}
```

## Cron to OnCalendar Conversion

systemd uses OnCalendar syntax instead of cron. usched converts:

| Cron | OnCalendar |
|------|------------|
| `0 9 * * *` | `*-*-* 09:00:00` |
| `0 9 * * 1-5` | `Mon..Fri *-*-* 09:00:00` |
| `*/30 * * * *` | `*-*-* *:00/30:00` |

## systemd Unit Generation

For each recurring job, usched creates:

**Timer unit** (`~/.config/systemd/user/usched-<id>.timer`):
```ini
[Unit]
Description=usched job: <name>

[Timer]
OnCalendar=<converted-schedule>
Persistent=true

[Install]
WantedBy=timers.target
```

**Service unit** (`~/.config/systemd/user/usched-<id>.service`):
```ini
[Unit]
Description=usched job: <name>

[Service]
Type=oneshot
ExecStart=/path/to/usched-run <job-id>
```

## at Integration

One-time jobs use the system `at` command:

```bash
# Schedule
echo "usched-run <job-id>" | at <datetime>

# List
atq

# Remove
atrm <at-job-id>
```

The at job ID is stored in the job metadata for later removal.
