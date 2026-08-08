# Slice PR-03: Dockerfile (Multi-Stage, Non-Root, < 100 MB)

## Slice Metadata

| Field | Value |
|-------|-------|
| Slice ID | PR-03 |
| Parent Story | US-PR-02 (Docker Image for Production Deployment) |
| job_id | JOB-13 |
| Estimate | 0.5 day |
| Depends on | PR-01, PR-02 (real `main()` must exist; binary must be useful to containerize) |
| Blocks | PR-04 (`docker` CI job) |

## Deliverable

After this slice:

```bash
docker build . -t embyr-server:latest
docker run -e DATABASE_URL=postgres://... \
           -e EMBYR_ADMIN_KEY=secret \
           -e EMBYR_ENCRYPTION_KEY=a3f1... \
           -p 8080:8080 -p 8081:8081 -p 9090:9090 \
           embyr-server:latest
```

starts the server inside a container. `curl localhost:9090/healthz` returns HTTP 200.

## Decision (D-PR-3)

Multi-stage Dockerfile with `cargo-chef` for layer caching:
- Stage 1: `rust:1.80-slim` — build environment with `cargo-chef`
- Stage 2: `debian:bookworm-slim` — minimal runtime (glibc, no Rust toolchain)
- Final image < 100 MB

## New Files

### `Dockerfile` (at workspace root)

Structure following `cargo-chef` best practice:

```
Stage: chef
  FROM rust:1.80-slim AS chef
  RUN cargo install cargo-chef --locked
  WORKDIR /app

Stage: planner
  FROM chef AS planner
  COPY . .
  RUN cargo chef prepare --recipe-path recipe.json

Stage: builder
  FROM chef AS builder
  COPY --from=planner /app/recipe.json recipe.json
  # Build dependencies only (cached layer)
  RUN cargo chef cook --release --recipe-path recipe.json
  # Build application
  COPY . .
  RUN cargo build --release -p embyr-server

Stage: runtime
  FROM debian:bookworm-slim AS runtime
  RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
  RUN useradd -ms /bin/bash embyr
  WORKDIR /app
  COPY --from=builder --chown=embyr:embyr /app/target/release/embyr-server /usr/local/bin/embyr-server
  USER embyr
  EXPOSE 8080 8081 9090
  ENTRYPOINT ["/usr/local/bin/embyr-server"]
```

### `.dockerignore` (at workspace root)

```
target/
docs/
tests/
.git/
*.md
```

Excludes `target/` (large) and documentation from the build context.
Excludes `tests/` to avoid leaking test credentials or testcontainers configs into the image.

## Implementation Notes

### Why `rust:1.80-slim`

Pin to Rust 1.80 to match the workspace's `rust-toolchain.toml` or the minimum required
version. If no `rust-toolchain.toml` exists, use `rust:1.80-slim` as the baseline. The
crafter should verify the actual toolchain version at implementation time.

### Why `debian:bookworm-slim` (not Alpine)

`sqlx` uses the system's OpenSSL for TLS. Alpine uses `musl-libc` and `openssl-dev` in
a different path structure. `debian:bookworm-slim` provides glibc and `libssl3` via apt,
matching the linker target of `rust:1.80-slim`. A musl target (`x86_64-unknown-linux-musl`)
would enable a smaller Alpine runtime but requires additional toolchain configuration
(cross-compilation or `musl-tools`). Defer musl to a future optimization slice.

### Layer caching strategy

`cargo-chef` separates dependency compilation from source compilation:
1. `planner` stage: `cargo chef prepare` extracts dependency graph to `recipe.json`
2. `builder` stage: `cargo chef cook --release` compiles only dependencies (cached unless
   `Cargo.toml`/`Cargo.lock` changes)
3. `cargo build --release -p embyr-server` compiles only workspace crates (re-runs on any
   `src/` change)

Result: clean `src/` change rebuild takes ~30s (deps cached); `Cargo.lock` change invalidates
dep cache and takes ~3-4 minutes.

### Size verification

After build: `docker image inspect embyr-server:latest --format='{{.Size}}'` should be
< 104857600 (100 MB in bytes). The runtime stage typically reaches 60-80 MB (Debian base
~30 MB + libssl3 ~5 MB + embyr-server binary ~15-30 MB).

### Non-root user

```dockerfile
RUN useradd -ms /bin/bash embyr
USER embyr
```

The process runs as UID 1000 (not root). Security audit: `whoami` inside the container
returns `embyr`, not `root`.

## Acceptance Criteria for This Slice

- [ ] `Dockerfile` exists at the repository root
- [ ] `docker build . -t embyr-server` succeeds from a clean checkout
- [ ] Final image size < 100 MB (verified with `docker image inspect`)
- [ ] Container process runs as non-root user (UID != 0)
- [ ] `EXPOSE 8080 8081 9090` present in Dockerfile
- [ ] Container started with required env vars: `healthz` returns HTTP 200
- [ ] Container started without `DATABASE_URL`: exits code 1 with error message
- [ ] Rebuild after `src/` change only (no `Cargo.toml` change) uses cached dependency layer
- [ ] `.dockerignore` excludes `target/` directory (prevents stale binaries in build context)

## Out of Scope for This Slice

- CI (PR-04)
- Multi-architecture builds (arm64) — deferred
- `docker-compose.yml` — deferred
- Pushing to a registry — deferred

## Test Guidance

Manual verification steps (automated in PR-04 CI job):

```bash
# Build
docker build . -t embyr-server:test

# Size check
docker image inspect embyr-server:test --format='{{.Size}}'

# Non-root check
docker run --rm --entrypoint whoami embyr-server:test

# Health check (requires running Postgres)
docker run -d --name embyr-test \
  -e DATABASE_URL=postgres://postgres:postgres@host.docker.internal:5432/embyr \
  -e EMBYR_ADMIN_KEY=test \
  -e EMBYR_ENCRYPTION_KEY=$(openssl rand -hex 32) \
  -p 9090:9090 embyr-server:test
curl -f http://localhost:9090/healthz
docker stop embyr-test && docker rm embyr-test

# Missing env var check
docker run --rm embyr-server:test 2>&1 | grep "DATABASE_URL"
```
