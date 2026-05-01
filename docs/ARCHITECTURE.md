# usched Architecture

Technical architecture documentation for the unified scheduler.

## Overview

usched provides a unified interface for scheduling jobs on top of **systemd
user timers**. Both recurring (`--cron`) and one-shot (`--once`) jobs are
backed by persistent `.timer` + `.service` unit pairs. There is no runtime
dependency on `at(1)`/`atd`/`atrm`.

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
│                             ▼                                       │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │                     systemd Backend                          │   │
│  │  recurring:  OnCalendar=<converted cron>  Persistent=true    │   │
│  │  one-shot:   OnCalendar=<absolute time>   Persistent=true    │   │
│  │  units:      ~/.config/systemd/user/usched-<id>.{timer,service}
│  └─────────────────────────────────────────────────────────────┘   │
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

## One-shot Jobs

`--once` jobs are dispatched via systemd user timers, the same machinery
that backs cron jobs. The only difference is the `OnCalendar` field — a
single absolute timestamp like `2099-06-15 09:00:00` instead of a
recurring calendar spec.

```ini
[Timer]
OnCalendar=2099-06-15 09:00:00
Persistent=true
Unit=usched-<id>.service

[Install]
WantedBy=timers.target
```

`Persistent=true` means systemd fires the timer on resume if the host was
suspended past the fire time (the right behavior for reminders). Once the
timer has elapsed, the unit goes inactive and won't re-fire. `usched
sync` skips already-fired one-shots; `usched remove` cleans them up
explicitly.

### Migration from at(1)

Pre-systemd-only versions of usched stored an at-queue job number in
`Schedule::Once::at_job`. `usched migrate-from-at` walks `jobs.json`,
re-registers each pending one-shot as a systemd timer, best-effort-removes
the old at-queue entry (warns rather than fails when `atd` is unreachable),
and drops past-due entries. Idempotent — safe to run repeatedly.
