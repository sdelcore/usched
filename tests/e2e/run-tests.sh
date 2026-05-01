#!/usr/bin/env bash
# Runs as root, after systemd boots inside the e2e container.
# Drives usched as `testuser` and asserts the real systemd timer + at paths fire.

set -uo pipefail

PASS=0
FAIL=0
LOG_PREFIX="[e2e]"

log() { echo "$LOG_PREFIX $*"; }
section() { echo; echo "$LOG_PREFIX === $* ==="; }

ok() { log "  PASS: $*"; PASS=$((PASS + 1)); }
ng() { log "  FAIL: $*"; FAIL=$((FAIL + 1)); }

as_user() { runuser -u testuser -- env XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "$@"; }

section "wait for testuser systemd"
for i in $(seq 1 60); do
    if as_user systemctl --user is-system-running >/dev/null 2>&1 \
        || as_user systemctl --user is-system-running 2>&1 | grep -qE 'running|degraded'; then
        log "user@1000 ready after ${i}s"
        break
    fi
    sleep 1
done

section "smoke: schedule cron job"
# Uses a /bin/sh -c command with shell metacharacters (redirection, spaces) —
# this was the case that exposed the unit-file quoting bug.
if as_user usched add --name e2e-cron --cron "* * * * *" -- /bin/sh -c 'echo fired > /home/testuser/marker'; then
    ok "usched add --cron"
else
    ng "usched add --cron"
fi

if as_user systemctl --user list-timers --all | grep -q usched-e2e-cron; then
    ok "timer registered with systemd"
else
    ng "timer not visible in systemctl --user list-timers"
    as_user systemctl --user list-timers --all || true
fi

section "smoke: list shows job"
if as_user usched list | grep -q e2e-cron; then
    ok "list shows job"
else
    ng "list missing job"
fi

section "real fire: wait up to 180s for marker"
# The timer template sets OnBootSec=2min + RandomizedDelaySec=30 — so the
# first fire after boot can be up to ~2.5 minutes out.
rm -f /home/testuser/marker
fired=0
for i in $(seq 1 180); do
    if [ -f /home/testuser/marker ]; then
        log "marker after ${i}s"
        fired=1
        break
    fi
    sleep 1
done
if [ "$fired" = 1 ]; then
    ok "cron timer fired and command ran"
else
    ng "marker never appeared (timer did not fire within 180s)"
    as_user systemctl --user status 'usched-e2e-cron-*.timer' || true
    as_user systemctl --user status 'usched-e2e-cron-*.service' || true
    as_user journalctl --user --no-pager -n 50 || true
fi

section "history recorded"
if as_user usched history --json | grep -q e2e-cron; then
    ok "history shows execution"
else
    ng "history empty for e2e-cron"
fi

section "preview (pure CLI)"
if as_user usched preview "0 9 * * *" -n 2 | grep -q "Cron:"; then
    ok "preview works"
else
    ng "preview broken"
fi

section "DND set/clear"
as_user usched dnd set 1h && ok "dnd set" || ng "dnd set"
as_user usched dnd status | grep -q "DND active" && ok "dnd status" || ng "dnd status"
as_user usched dnd off && ok "dnd off" || ng "dnd off"

section "remove tears down timer"
job_id=$(as_user usched list | awk 'NR==1 {print $1}')
if [ -n "$job_id" ]; then
    as_user usched remove "$job_id" && ok "remove ok" || ng "remove failed"
    if as_user systemctl --user list-timers --all | grep -q usched-e2e-cron; then
        ng "timer not torn down"
    else
        ok "timer torn down"
    fi
else
    ng "could not extract job id from list"
fi

section "smoke: schedule one-shot job (systemd-only, no at(1))"
# Schedule for ~30s in the future and wait up to 90s for the marker.
once_at=$(date -u -d '+30 seconds' +'%Y-%m-%dT%H:%M:%SZ')
once_local=$(date -d "$once_at" +'%Y-%m-%d %H:%M:%S')
log "scheduling one-shot for $once_local (local)"
rm -f /home/testuser/once-marker
if as_user usched add --name e2e-once --once "$once_local" -- /bin/sh -c 'echo once-fired > /home/testuser/once-marker'; then
    ok "usched add --once"
else
    ng "usched add --once"
fi

if as_user systemctl --user list-timers --all | grep -q usched-e2e-once; then
    ok "one-shot timer registered with systemd"
else
    ng "one-shot timer not visible"
    as_user systemctl --user list-timers --all || true
fi

# Verify usched did not invoke at(1): the at binary isn't installed in this
# image, so any code path that shelled out would have failed loudly. The
# unit file is the canonical signal.
once_unit=$(as_user systemctl --user list-unit-files | awk '/usched-e2e-once-.*\.timer/ {print $1; exit}')
if [ -n "$once_unit" ]; then
    ok "one-shot dispatched via systemd unit ($once_unit)"
else
    ng "no systemd unit for one-shot"
fi

once_fired=0
for i in $(seq 1 90); do
    if [ -f /home/testuser/once-marker ]; then
        log "once marker after ${i}s"
        once_fired=1
        break
    fi
    sleep 1
done
if [ "$once_fired" = 1 ]; then
    ok "one-shot timer fired and command ran"
else
    ng "one-shot marker never appeared (timer did not fire within 90s)"
    as_user systemctl --user status 'usched-e2e-once-*.timer' || true
    as_user systemctl --user status 'usched-e2e-once-*.service' || true
    as_user journalctl --user --no-pager -n 50 || true
fi

section "summary"
log "PASS=$PASS FAIL=$FAIL"
if [ "$FAIL" -gt 0 ]; then
    systemctl exit 1
else
    systemctl exit 0
fi
