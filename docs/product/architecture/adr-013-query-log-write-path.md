# ADR-013: Query Log Write Path — Fire-and-Forget Tokio Spawn

## Status

Accepted

## Context

DISCUSS locked decision D5: `query_logs` is partitioned by day, with high insert rates when `logging_enabled = true` for a project. AC-B04-06: "The storage write path (BC-2) checks this flag before inserting into `query_logs`."

The logging hook must not add material latency to SDK client operations. The p99 target for write-path latency is driven by the overall SDK KPI; inserting a log row synchronously before responding to the client would add ~5–15ms for every write when logging is enabled.

Three placement options for the `query_logs` insert:

1. **In the gRPC handler after the storage op, fire-and-forget Tokio spawn** — `tokio::spawn(async { insert_query_log(...) })` after the storage op returns. No client latency impact. Failure is logged but not propagated.
2. **In the gRPC handler, synchronous inline** — insert the log row before returning the response to the client. Client sees latency impact. Failure can be surfaced as an error.
3. **As a side effect inside the BC-2 storage adapter** — the `PostgresBackendAdapter` checks `logging_enabled` and inserts the log row within the same Postgres connection as the storage op (but outside the transaction boundary).

## Decision

**Fire-and-forget Tokio spawn from the gRPC handler after the storage op (Option 1), following the existing `MetricsPort` pattern.**

A new driven port `IQueryLogWriter` is defined in `embyr-core::admin::query_log`:

```
struct QueryLogEntry {
    project_id: String,
    operation: OperationType,   // Get, Create, Update, Delete, Query, Transaction
    collection_path: Option<String>,
    document_path: Option<String>,
    status: OpStatus,           // Ok, Error
    duration_ms: u32,
    client_ip: Option<String>,
    ts: OffsetDateTime,
}

enum OperationType { Get, Create, Update, Delete, RunQuery, BatchGet, Transaction }
enum OpStatus { Ok, Error }

trait IQueryLogWriter: Send + Sync {
    // fire-and-forget; implementation must not block the caller
    fn record(&self, entry: QueryLogEntry);
}
```

The concrete adapter `PostgresQueryLogAdapter` (in `embyr-server::adapters::query_log`) implements `IQueryLogWriter`:
- `record()`: spawns a Tokio task that inserts one row into `query_logs`
- Failure: logged with `tracing::warn!` and dropped — never propagated to the caller
- No `probe()` method required: same rationale as `MetricsPort` — best-effort append-only write; failure does not affect system correctness

**Integration with the gRPC handler:**

After every storage operation, the gRPC handler checks the project record's `logging_enabled` flag (already loaded in the auth middleware and attached to the request context via a request extension). If `true`, it calls `state.query_log_writer.record(entry)`. The call is synchronous (just spawns a Tokio task) and returns immediately.

The `project.logging_enabled` flag is loaded once during the auth middleware's project record fetch, then attached to the request context as a lightweight `ProjectMeta` extension alongside the `BackendAdapter`. No additional DB lookup per request.

**AC-B04-06 compliance:** "The storage write path (BC-2) checks this flag before inserting." Technically the check and insert happen in the driving adapter layer (`embyr-server::grpc` handlers) rather than the BC-2 domain layer (`embyr-core::storage`). This is correct: the domain layer cannot spawn Tokio tasks (it has no IO). The "storage write path" in AC-B04-06 refers to the path through the system, not specifically the domain function.

**Partition awareness:** The `query_logs` table is partitioned by `(project_id, timestamp::date)`. The INSERT targets the correct partition automatically — no application-level partition routing is needed. Old partitions are dropped by a background sweeper on a schedule derived from `projects.log_retention_days`.

**New component:** `QueryLogSweeper` Tokio task in `embyr-server::sweepers::query_log_sweeper` — runs daily (configurable via env, default 02:00 UTC), queries `projects` for `logging_enabled=true` records and their `log_retention_days`, then `DROP TABLE IF EXISTS query_logs_<project_id>_<date>` for partitions older than the retention window.

**Multi-instance coordination:** In multi-replica deployments, all instances run the sweeper on the same schedule, which would cause duplicate partition drop attempts (harmless due to `IF EXISTS` but wasteful and noisy). Each sweeper cycle acquires a Postgres session-level advisory lock before executing: `SELECT pg_try_advisory_lock(fnv1a_hash('embyr_query_log_sweep'))`. If the lock is held by another instance, the cycle is skipped and retried on the next schedule tick. The lock is released via `pg_advisory_unlock` in the task's `finally` block. Crash-safe: advisory locks are session-scoped and released automatically on connection close, at the cost of at most one missed cycle.

## Alternatives Considered

### Option 2: Synchronous inline insert (rejected)

Insert the log row synchronously before returning the gRPC response to the client.

**Rejected because:**
- Adds 5–15ms to every SDK write when logging is enabled. The write-path p99 ≤ 2s KPI (write → onSnapshot callback) relies on the embyr write handler being fast; adding a synchronous DB write per SDK operation moves the bottleneck to the logging table.
- If the `query_logs` table is unavailable (e.g., maintenance, partition creation issue), all logged projects' SDK writes fail with an internal error — cascading failure from a logging concern into the data plane.
- Inconsistent with the existing `MetricsPort` pattern (which is also fire-and-forget).

### Option 3: Side effect inside BC-2 storage adapter (rejected)

The `PostgresBackendAdapter` checks `logging_enabled` internally and inserts into `query_logs` as part of its own connection handling.

**Rejected because:**
- The `BackendAdapter` trait in `embyr-core` cannot include `logging_enabled` awareness — that would import a tenant management concept (the project configuration) into the storage adapter abstraction. This breaks the BC-2 / BC-1 boundary.
- `AgentBackendAdapter` (for agent-mode projects) would also need to implement log insertion — but `query_logs` lives in the System DB (not the customer DB), so agent-mode cannot write to it directly. This creates an inconsistency where logging only works for some backend modes.
- The `BackendAdapter` trait is defined in `embyr-core` (no IO). Making it responsible for logging requires the trait to include a `log_writer: Arc<dyn IQueryLogWriter>` field — polluting the storage abstraction with infrastructure.

## Consequences

**Positive:**
- Zero client latency impact from logging. Logging failures are completely invisible to SDK clients.
- Consistent with `MetricsPort` (also best-effort fire-and-forget): one architectural pattern for all best-effort side effects.
- `logging_enabled` flag is checked once (in the auth middleware path, reusing an already-loaded value) — no extra DB query per request.
- Log writes are decoupled from the storage write's transaction: if the storage op rolls back, a partially-written log row may still appear. This is acceptable for a diagnostic log (the log is informational, not transactionally consistent with the storage layer).

**Negative / Trade-offs:**
- Log rows may be silently dropped on Postgres connection failures. At high insert rates, the Tokio task queue can grow if `query_logs` is slow. The adapter should use a bounded channel rather than unbounded `tokio::spawn` to prevent unbounded task accumulation under load. Recommended bound: 1000 queued inserts; beyond that, inserts are dropped with a warn log.
- `QueryLogSweeper` is a new background task that must handle partition DDL (`DROP TABLE`). DDL in a live production database carries low but non-zero risk. The sweeper must be validated against a test environment partition table before production deployment (B-04 slice gate). Multi-instance duplicate execution is prevented by Postgres advisory lock (`pg_try_advisory_lock`).
- A companion sweeper `SessionCleaner` (hourly) must also be added to `embyr-server::sweepers::session_cleaner` to prevent unbounded growth of the `sessions` table. It deletes rows where `expires_at < now() - INTERVAL '30 days'` and expired unaccepted invitations. Same advisory lock pattern.

## Enforcement

- No `probe()` on `PostgresQueryLogAdapter` — consistent with `MetricsPort`. The underlying `SystemDb` connection is already probed at startup.
- The `IQueryLogWriter.record()` signature is synchronous (returns unit, not `Result`) — it is structurally impossible for callers to propagate errors.
- Integration tests verify that (1) with `logging_enabled=false`, no rows appear in `query_logs`; (2) with `logging_enabled=true`, SDK writes appear in `query_logs` within 500ms.
