# Slice PR-01: Config Struct + main() Skeleton

## Slice Metadata

| Field | Value |
|-------|-------|
| Slice ID | PR-01 |
| Parent Story | US-PR-01 (Real Server Startup from Environment Variables) |
| job_id | JOB-13 |
| Estimate | 0.5 day |
| Depends on | Nothing (this is the walking skeleton) |
| Blocks | PR-02 (startup probe), PR-03 (Dockerfile), PR-04 (CI) |

## Deliverable

After this slice, running:

```
DATABASE_URL=postgres://... EMBYR_ADMIN_KEY=secret EMBYR_ENCRYPTION_KEY=a3f1... \
cargo run -p embyr-server
```

starts the server (reaches port-bind), logs structured startup lines, and accepts
`GET :9090/healthz → HTTP 200`. Migrations and DB probe are not yet called (those land in PR-02);
the binary simply validates config and attempts to bind ports.

**Slice boundary rationale (Elephant Carpaccio):** Config validation + port binding is the
minimum observable working behavior. Adding migrations and the DB probe in this slice would
risk making it > 0.5 day. Splitting at "ports bound" gives a demonstrable result in a single
session: Sam can verify the binary starts and `healthz` responds.

## Decisions

| ID | Decision |
|----|----------|
| D-PR-1 | Env vars: `DATABASE_URL` (required), `EMBYR_ADMIN_KEY` (required), `EMBYR_ENCRYPTION_KEY` (32-byte hex, required), `EMBYR_RATE_LIMIT_RPS` (default 1000.0), `GRPC_PORT` (default 8080), `REST_PORT` (default 8081), `ADMIN_PORT` (default 9090), `RUST_LOG` (default "info") |
| D-PR-7 | Config struct in `crates/embyr-server/src/config.rs`, validated at parse time, not scattered `std::env::var` calls |

## New Files

### `crates/embyr-server/src/config.rs`

Parsed and validated struct from environment variables. Mirrors `embyr_agent::config::AgentConfig`.

Required env vars (exit 1 if missing or invalid):
- `DATABASE_URL` — Postgres DSN
- `EMBYR_ADMIN_KEY` — non-empty string
- `EMBYR_ENCRYPTION_KEY` — 32-byte hex string (64 hex chars); parse to `[u8; 32]` at startup

Optional env vars (use defaults):
- `EMBYR_RATE_LIMIT_RPS` — f64 > 0, default 1000.0
- `GRPC_PORT` — u16, default 8080
- `REST_PORT` — u16, default 8081
- `ADMIN_PORT` — u16, default 9090
- `RUST_LOG` — log filter string, default "info"

### Modified: `crates/embyr-server/src/main.rs`

Replaces the stub `println!` with an async `tokio::main` entry point that:

1. Parses `ServerConfig::from_env()` — exits 1 on error, prints to stderr
2. Initializes tracing (`tracing_subscriber::fmt()` with `EnvFilter` from `cfg.log_level`)
3. Calls `get_or_install_prometheus_handle()` (ADR-016 ordering)
4. Binds three `TcpListener`s on configured ports — exits 1 on `EADDRINUSE`
5. Logs "embyr-server ready" with all three addresses
6. **Placeholder**: passes listeners to `spawn_servers()` stub (real wiring lands in PR-02)
7. Awaits shutdown signal (SIGTERM or CTRL-C via `tokio::signal`)
8. Exits 0

## Acceptance Criteria for This Slice

- [ ] `cargo run -p embyr-server` (with all required env vars set) binds on :8080/:8081/:9090
- [ ] `GET :9090/healthz` returns HTTP 200 (via existing healthz handler wired from lib.rs)
- [ ] Missing `DATABASE_URL` → stderr error + exit 1 within 100ms
- [ ] Missing `EMBYR_ADMIN_KEY` → stderr error + exit 1
- [ ] `EMBYR_ENCRYPTION_KEY` of wrong length → stderr error + exit 1
- [ ] `GRPC_PORT=18080` env var causes server to bind on port 18080
- [ ] `RUST_LOG=debug` produces debug-level log output

## Out of Scope for This Slice

- Migrations (PR-02)
- SystemDb probe (PR-02)
- Graceful shutdown drain (signal handling is wired but drain is a PR-02 concern)
- Dockerfile (PR-03)
- CI (PR-04)

## Test Guidance

Unit tests for `ServerConfig::from_env()`:
- Missing required var → `Err`
- Invalid hex for `EMBYR_ENCRYPTION_KEY` → `Err`
- All required vars set → `Ok(ServerConfig { ... })`
- `GRPC_PORT` override → `Ok` with custom port

Integration test (process-level or using lib.rs `TestServer` pattern):
- Server reaches `healthz` 200 when all env vars set
- Process exits 1 when `DATABASE_URL` absent
