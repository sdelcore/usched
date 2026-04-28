#!/usr/bin/env bash
# Build and run the e2e container locally. Mirrors what CI does.
# Usage: tests/e2e/run.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

IMAGE="usched-e2e:local"

echo "==> docker build"
docker build --target e2e -t "$IMAGE" .

echo "==> docker run"
# --privileged + cgroup mounts + tmpfs are the canonical recipe for systemd in docker.
# Container runs /sbin/init, which boots systemd, which starts usched-e2e.service,
# which calls `systemctl exit <code>` to halt the container with a meaningful code.
exec docker run --rm -t \
    --privileged \
    --cgroupns=host \
    -v /sys/fs/cgroup:/sys/fs/cgroup:rw \
    --tmpfs /run \
    --tmpfs /run/lock \
    --tmpfs /tmp \
    "$IMAGE"
