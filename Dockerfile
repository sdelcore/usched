# syntax=docker/dockerfile:1
#
# Multi-target image for usched.
#
#   docker build -t usched .                    # default → e2e (last stage)
#   docker build --target runtime -t usched .   # minimal deploy image
#   docker build --target e2e     -t usched-e2e .
#
# - `builder` compiles the binary
# - `runtime` is a minimal Debian image with usched + usched-run installed
# - `e2e`     extends runtime with systemd as PID 1 + atd + the test harness,
#             used by tests/e2e/run.sh and CI

# ---------- builder ----------
FROM rust:1-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY scripts ./scripts
RUN cargo build --release --locked

# ---------- runtime ----------
FROM debian:bookworm-slim AS runtime
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash jq ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/usched /usr/local/bin/usched
COPY --from=builder /src/scripts/usched-run /usr/local/bin/usched-run
RUN chmod +x /usr/local/bin/usched /usr/local/bin/usched-run

ENTRYPOINT [ "/usr/local/bin/usched" ]
CMD [ "--help" ]

# ---------- e2e ----------
FROM runtime AS e2e

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        systemd systemd-sysv libpam-systemd dbus at procps \
    && rm -rf /var/lib/apt/lists/*

# Strip units that hang or fail in containers
RUN systemctl mask \
        systemd-firstboot.service \
        systemd-resolved.service \
        systemd-machined.service \
        systemd-networkd.service \
        systemd-binfmt.service \
        sys-kernel-debug.mount \
        sys-kernel-tracing.mount \
        sys-kernel-config.mount \
        getty.target

RUN useradd -m -s /bin/bash testuser \
    && mkdir -p /var/lib/systemd/linger \
    && touch /var/lib/systemd/linger/testuser

COPY tests/e2e/run-tests.sh /usr/local/bin/run-tests.sh
COPY tests/e2e/usched-e2e.service /etc/systemd/system/usched-e2e.service
RUN chmod +x /usr/local/bin/run-tests.sh \
    && systemctl enable usched-e2e.service atd.service

VOLUME [ "/sys/fs/cgroup", "/run", "/run/lock", "/tmp" ]
STOPSIGNAL SIGRTMIN+3
ENTRYPOINT [ "/sbin/init" ]
CMD []
