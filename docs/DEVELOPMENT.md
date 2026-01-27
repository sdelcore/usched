# usched Development

Development guide for working on usched.

## Build Commands

```bash
# Enter nix development environment
cd usched && nix develop

# Build
cargo build

# Build release
cargo build --release

# Run tests
cargo test

# Format code
cargo fmt

# Lint
cargo clippy
```

## Project Structure

```
usched/
├── src/
│   ├── main.rs          # CLI entry point (clap)
│   ├── job.rs           # Job, Schedule, Constraints structs
│   ├── store.rs         # JSON persistence
│   ├── systemd.rs       # systemd timer/service creation
│   ├── at.rs            # at command integration
│   └── cron_convert.rs  # Cron to OnCalendar conversion
├── scripts/
│   └── usched-run       # Constraint enforcement wrapper
├── Cargo.toml
└── flake.nix
```

## Key Files

| File | Purpose |
|------|---------|
| `src/main.rs` | CLI parsing with clap |
| `src/job.rs` | Job/Schedule/Constraints data structures |
| `src/store.rs` | Load/save jobs.json and state.json |
| `src/systemd.rs` | Generate and manage systemd units |
| `src/at.rs` | Interface with at/atq/atrm commands |
| `src/cron_convert.rs` | Convert cron expressions to OnCalendar |
| `scripts/usched-run` | Shell script for constraint checking |

## Adding a New Constraint

1. Add field to `Constraints` struct in `src/job.rs`:
```rust
pub struct Constraints {
    pub dnd_aware: bool,
    pub not_during: Option<TimeRange>,
    pub remove_on_success: bool,
    pub new_constraint: Option<NewType>,  // Add here
}
```

2. Add CLI flag in `src/main.rs`:
```rust
#[arg(long)]
new_constraint: Option<String>,
```

3. Update `usched-run` script to check the constraint:
```bash
# Check new constraint
if [ -n "$NEW_CONSTRAINT" ]; then
    # constraint logic
fi
```

4. Update documentation

## Testing

```bash
# Run all tests
cargo test

# Run specific test
cargo test cron_convert

# Test with output
cargo test -- --nocapture
```

### Manual Testing

```bash
# Add test job (won't actually run dangerous commands)
usched add --once "in 1 minute" -- echo "test"

# Check it was created
usched list

# Watch systemd
systemctl --user list-timers

# Check logs
journalctl --user -f
```

## Debugging

### Check Job Store

```bash
cat ~/.local/share/usched/jobs.json | jq
```

### Check DND State

```bash
cat ~/.local/share/usched/state.json | jq
```

### Check systemd Units

```bash
# List usched timers
systemctl --user list-timers 'usched-*'

# Check specific timer
systemctl --user status usched-<job-id>.timer

# Check service
systemctl --user status usched-<job-id>.service

# View unit files
cat ~/.config/systemd/user/usched-*.timer
cat ~/.config/systemd/user/usched-*.service
```

### Check at Jobs

```bash
# List at jobs
atq

# View job content
at -c <job-number>
```

## Common Tasks

### Modify Cron Conversion

Edit `src/cron_convert.rs`. The conversion handles:
- Minute/hour fields
- Day-of-week mapping (0-6 vs Mon-Sun)
- Step values (*/5)
- Ranges (1-5)
- Lists (1,3,5)

### Add New CLI Command

1. Add subcommand enum variant in `src/main.rs`
2. Add handler function
3. Wire up in main match statement
4. Update documentation

### Modify usched-run Behavior

Edit `scripts/usched-run`. This is a shell script that:
1. Loads job from store
2. Checks constraints
3. Executes command
4. Handles post-execution

## Build Output

The nix build produces two artifacts:
- `usched` - Main CLI binary
- `usched-run` - Wrapper script (installed to same bin directory)

Both must be in PATH for the system to function correctly.
