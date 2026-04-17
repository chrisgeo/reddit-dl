# syntax=docker/dockerfile:1.7
#
# reddit-dl — containerized runner
#
# Build:  docker build -t reddit-dl .
# Run:    docker run --rm reddit-dl <options>
#
# Typical invocation (bind-mount config + downloads dir):
#   docker run --rm \
#       -v "$PWD/config.toml:/data/config.toml:ro" \
#       -v "$PWD/downloads:/data/downloads" \
#       reddit-dl sync
#
# Config is auto-discovered at /data/config.toml (WORKDIR) or
# /home/app/.config/reddit-dl/config.toml. Pass --config for anything else.
#
# If host UID != 1000, override with:  --user "$(id -u):$(id -g)"

FROM rust:1-bookworm AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Cache mounts don't persist into the image, so the binary must be copied
# out of target/ before the RUN layer closes.
RUN --mount=type=cache,target=/build/target,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo build --release --locked && \
    cp target/release/reddit-dl /usr/local/bin/reddit-dl && \
    strip /usr/local/bin/reddit-dl


FROM debian:bookworm-slim AS runtime

# tini: PID-1 for clean SIGINT/SIGTERM propagation to the tokio runtime.
# ca-certificates: harmless fallback if the TLS stack is ever swapped
# away from rustls+webpki-roots.
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates tini && \
    rm -rf /var/lib/apt/lists/*

RUN useradd --uid 1000 --create-home --home-dir /home/app --shell /usr/sbin/nologin app && \
    mkdir -p /data /home/app/.config/reddit-dl && \
    chown -R app:app /data /home/app

COPY --from=builder /usr/local/bin/reddit-dl /usr/local/bin/reddit-dl

USER app
WORKDIR /data
ENV RUST_LOG=reddit_dl=info

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/reddit-dl"]
CMD ["--help"]
