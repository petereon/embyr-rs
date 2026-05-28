# Slice 06A — Agent Lifecycle: Startup Probe, Graceful Shutdown, DSN Log Discipline

**Goal**: Agent startup is atomic (Postgres verified before port opens), shutdown is clean (in-flight RPCs drain), and the Postgres DSN never appears in any log output at any level.

**Feature**: embyr-agent
**Estimated effort**: ≤1 day
**Sequence**: 2 of 6 (immediately after S01A — audit invariant blocks production deployment)

---

## IN Scope

- **Startup probe** (Postgres connectivity verified BEFORE gRPC listener opens):
  - `SELECT 1` via sqlx against `EMBYR_AGENT_DB_DSN`
  - On failure: log `ERROR` with error detail (not the DSN), exit with code 1
  - On success: log `INFO "connected to Postgres"` with `max_conns` field
  - Only after successful Postgres probe: bind gRPC listener and log `INFO "listening on :9191"`
- **Config validation extension** (`AgentConfig::from_env`):
  - Add `EMBYR_AGENT_MAX_CONNS` support (currently missing — `AgentConfig` has no `max_conns` field)
  - Add `EMBYR_AGENT_LOG_LEVEL` support (currently missing)
  - Validate `EMBYR_AGENT_LISTEN_ADDR` is a parseable `SocketAddr`; exit with named error if invalid
  - All cert files must be readable at startup; emit named error (file path + OS error) if unreadable
- **Structured JSON logging** (tracing + tracing-subscriber JSON formatter):
  - All log output is JSON (key-value pairs), not plain text
  - Log fields: `level`, `ts` (RFC3339), `msg`, plus contextual fields per log line
  - No log line at any level emits the DSN string — enforced by design (DSN only in pool creation, never echoed)
- **Graceful shutdown**:
  - Signal: `SIGTERM` (and optionally `SIGINT`)
  - On signal: stop accepting new connections
  - Drain in-flight RPCs: wait for all active handler futures to complete (or a configurable drain timeout, default 30s)
  - Close Postgres pool (all pending queries complete or are cancelled)
  - Log `INFO "shutdown complete"`, exit code 0
  - In-flight RPCs that are still running after drain timeout: cancelled; callers receive `Unavailable`
- **DSN log discipline negative test**: a test that starts the agent with a DSN containing a known sentinel string, sends 10 RPCs, and asserts zero log lines contain the sentinel

## OUT Scope

- Agent metrics endpoint (e.g., Prometheus /metrics) — separate future concern
- Agent binary distribution / Docker image packaging
- Health check HTTP endpoint on agent (Kubernetes liveness probe — acceptable to use TCP check on :9191)

---

## Learning Hypothesis

**Disproves**: "Draining in-flight Tonic RPCs on SIGTERM in Rust async requires more than one day of plumbing."

**Confirms if successful**: `tonic::transport::Server::serve_with_shutdown(shutdown_signal)` + a tokio `select!` on a Ctrl-C/SIGTERM channel is sufficient for clean drain in a single-binary deployment.

---

## Acceptance Criteria

- [ ] Startup probe: `SELECT 1` executes against `EMBYR_AGENT_DB_DSN` BEFORE the gRPC listener opens (SPEC.md §embyr Agent §Lifecycle: "agent connects pool to Postgres, verifies WAL mode / connection, starts gRPC listener")
- [ ] Startup probe failure: agent logs `ERROR` (no DSN in message), exits code 1, no port is bound
- [ ] Successful startup: log contains `msg="connected to Postgres"` followed by `msg="listening on :9191"` (in that order)
- [ ] `EMBYR_AGENT_MAX_CONNS` env var configures Postgres pool max connections; defaults to 25 (SPEC.md §embyr Agent §Configuration)
- [ ] `EMBYR_AGENT_LOG_LEVEL` env var configures log level; defaults to `info`
- [ ] Unreadable cert file: agent logs file path + OS error to stderr, exits code 1 before gRPC listener opens
- [ ] All log output is structured JSON (fields: `level`, `ts`, `msg`)
- [ ] DSN never appears in any log line at any level: negative test with sentinel DSN string finds zero matches across 10 RPC log entries (SPEC.md Invariant 13 + §Security Model)
- [ ] `SIGTERM` causes graceful shutdown: in-flight RPCs that started before SIGTERM complete; no new RPCs are accepted after SIGTERM
- [ ] After graceful shutdown: exit code 0; log contains `msg="shutdown complete"`
- [ ] In-flight RPCs outstanding >30s after SIGTERM: cancelled; callers receive `Unavailable` (SPEC.md §embyr Agent §Lifecycle §Shutdown: "agent drains in-flight RPCs, closes pool, exits. All in-flight embyr requests receive Unavailable")

---

## Spec Traceability

| AC | SPEC.md Reference |
|----|------------------|
| Startup probe ordering (Postgres first, then listener) | §embyr Agent §Lifecycle: "Startup: agent connects pool to Postgres, verifies WAL mode / connection, starts gRPC listener" |
| Startup probe failure exits agent | §embyr Agent §Lifecycle (implicit: probe is a gate) |
| Configuration env vars | §embyr Agent §Configuration (all listed env vars) |
| DSN never in logs | §Invariants Invariant 13: "Customer database credentials never exist in plaintext in the system database." Extended to: never in agent logs either. §Security Model: "Credential material (PG DSN) never leaves the customer's network." |
| Graceful shutdown (drain + Unavailable) | §embyr Agent §Lifecycle §Shutdown |

---

## Dependencies

- S01A: `required` (agent binary must be running to test shutdown)
- Note: S06A can be developed in parallel with S02A after S01A lands, since S06A only requires the agent binary to start and accept connections (GetDocument from S01A is sufficient)

---

## Note on Current State

`AgentConfig::from_env` (crates/embyr-agent/src/config.rs) currently missing:
- `max_conns: usize` field
- `log_level: String` field
- Validation for `listen_addr` (stored as String, not validated)

These gaps must be addressed in this slice.
