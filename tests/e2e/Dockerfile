# Multi-stage e2e image: build usched, then run inside a systemd-enabled
# Debian container so we can validate the real timer firing path.

FROM rust:1-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY scripts ./scripts
RUN cargo build --release --locked

FROM debian:bookworm-slim
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        systemd systemd-sysv libpam-systemd dbus at jq bash ca-certificates procps \
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

COPY --from=builder /src/target/release/usched /usr/local/bin/usched
COPY --from=builder /src/scripts/usched-run /usr/local/bin/usched-run
RUN chmod +x /usr/local/bin/usched /usr/local/bin/usched-run

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
