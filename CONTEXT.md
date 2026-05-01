# Context

Domain language for `usched`. Use these terms exactly when describing the
system. Architectural vocabulary (module, interface, seam, depth, leverage,
locality) is defined separately in the `improve-codebase-architecture` skill.

## Terms

**Job**
A user's instruction to run a command on a schedule, plus the constraints
that gate it. Persisted in `jobs.json`. Identified by `<name>-<6-hex>`.

**Schedule**
How often a Job fires. Two variants: `Cron` (recurring) and `Once`
(one-shot, absolute fire time). Both variants are dispatched through the
same systemd backend; each carries the systemd unit name as its handle.

**Constraints**
The "should this run *right now*" predicates: `dnd_aware`, `not_during`,
`only_during`, `after`, `remove_on_success`. Distinct from Schedule —
Schedule decides *when* to wake up, Constraints decide whether to actually
run once awake.

**Backend**
The OS scheduler that owns a Job's lifecycle. Currently one variant —
`Backend::Systemd` — which writes persistent `.timer`/`.service` user
units for both recurring and one-shot jobs. The single-variant enum is
kept so a future second backend would be a compiler-enforced sweep, not
silent dispatch. The pre-systemd-only versions of usched also had
`Backend::At` (shelling out to `at(1)`/`atrm`); migration helpers live
in `migrate.rs`.

**Runner**
The execution path between "the timer fired" and "the command ran." Two
parts:
- `evaluate(&Job, &State, &dyn HistoryLookup, now) -> Decision` — pure;
  decides `Run` or `Skip(reason)` against the current world.
- `execute(&Job)` — impure; fork/exec the command, record start and
  finish in the history database.

The Runner is invoked through the hidden `usched __run-job <id>`
subcommand. The systemd `.service` `ExecStart` calls into it for both
recurring and one-shot jobs.

**State**
Out-of-band runtime state that affects Constraints — currently just the
Do Not Disturb expiry timestamp. Persisted in `state.json`.

**History**
Append-only log of executions in `history.db` (SQLite). Each row records
a start, finish (exit code + duration), or skip-with-reason. The Runner
records its own rows; nothing else writes to it.

## Storage layout

- `~/.local/share/usched/jobs.json` — Jobs (canonical command source)
- `~/.local/share/usched/state.json` — DND
- `~/.local/share/usched/history.db` — Execution log
