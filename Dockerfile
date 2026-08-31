# embyr-server multi-stage Dockerfile (ADR-017 D-PR-3)
#
# Stages:
#   1. chef    — install cargo-chef for dependency layer caching
#   2. planner — generate recipe.json from Cargo manifests
#   3. builder — cook deps (cached layer) + compile embyr-server release binary
#   4. runtime — minimal debian:bookworm-slim image, non-root user, 3 exposed ports
#
# sqlx::migrate!("../../migrations") embeds migration SQL at compile time —
# no runtime COPY of migrations/ is needed.
#
# Layer-cache contract:
#   Changing only src/**/*.rs → only the "cargo build --release" step re-runs.
#   Changing Cargo.toml/Cargo.lock → deps are re-cooked from recipe.json.

# ─── Stage 1: install cargo-chef ─────────────────────────────────────────────
# Pinned to match the workspace's actual MSRV floor, not embyr-rs's own edition
# (2021): several transitive deps in Cargo.lock require newer rustc than that —
# crypto-common v0.2.2 needs edition2024 (Rust 1.85+), cargo-chef's own
# dependency tree needs 1.88+, and the aws-sdk-secretsmanager stack needs
# 1.91.1+. Keep in lockstep with the toolchain used for local `cargo build`.
#
# `-bookworm` suffix pinned explicitly: the untagged `rust:1.95-slim` now
# resolves to Debian trixie, whose glibc is newer than debian:bookworm-slim
# (the runtime stage below) — a binary built on trixie fails to run on
# bookworm with "GLIBC_2.38 not found". Builder and runtime must share a
# Debian release.
#
# 1.95, matching rust-toolchain.toml's own pin exactly (drifted to 1.93 here
# previously — found 2026-08-31 when a transitive aws-smithy-* bump required
# rustc 1.94.1, which this stage's then-1.93 toolchain couldn't satisfy;
# cargo-chef panicked on `cargo chef cook` before any real compile started).
# Keep in lockstep with `rust-toolchain.toml` going forward — this stage has
# no independent reason to pin an older toolchain than local `cargo build`
# and CI's own test/lint jobs already use.
FROM rust:1.95-slim-bookworm AS chef
WORKDIR /app
RUN cargo install cargo-chef --locked

# ─── Stage 2: generate dependency recipe ─────────────────────────────────────
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ─── Stage 3: cook dependencies then build the release binary ─────────────────
FROM chef AS builder
# Install system build dependencies: pkg-config/libssl-dev for sqlx native-tls
# / ring, protobuf-compiler because embyr-proto's build.rs shells out to
# `protoc` (prost-build) to compile the Firestore .proto definitions.
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy recipe and cook deps — this layer is cached unless Cargo.toml/lock changes.
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Copy full source and build only the server binary.
COPY . .
# SQLX_OFFLINE=true: avoid connecting to Postgres at build time; queries are
# validated via the cached .sqlx/ directory already committed to the repo.
RUN SQLX_OFFLINE=true cargo build --release --bin embyr-server

# ─── Stage 4: minimal runtime image ──────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

# Install runtime dependencies: ca-certificates for TLS handshakes.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create a dedicated non-root user for the server process.
RUN useradd --system --no-create-home --shell /bin/false embyr

WORKDIR /app

# Copy the compiled binary from the builder stage.
COPY --from=builder --chown=embyr:embyr /app/target/release/embyr-server /app/embyr-server

# Run as non-root.
USER embyr

# Document the three TCP listeners (gRPC :8080, REST :8081, Admin :9090).
EXPOSE 8080 8081 9090

ENTRYPOINT ["/app/embyr-server"]
