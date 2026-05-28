# ADR-A03: SQL Reuse — New `embyr-pg-storage` Crate

## Status
Accepted

## Context

`PostgresBackendAdapter` in `embyr-server` implements all document CRUD, query execution, OCC conflict detection, tombstone insertion, and transaction management in SQL. The embyr-agent binary needs identical SQL logic to serve the same storage operations against an identical customer database schema.

Three strategies were evaluated:

- **C1**: Extract shared SQL into a new `embyr-pg-storage` crate
- **C2**: Reimplement SQL independently in `crates/embyr-agent/src/storage.rs`
- **C3**: Agent depends on `embyr-server` — **forbidden by AD-01** (embyr-agent must not import embyr-server)

## Decision

**Option C1 — new `embyr-pg-storage` crate** is adopted.

`embyr-pg-storage` is a library crate in the workspace. It depends on `sqlx`, `embyr-core`, and `embyr-proto`. It does NOT depend on `tonic`, `axum`, `tokio-rustls`, or any network/TLS crate. Both `embyr-server` and `embyr-agent` add `embyr-pg-storage` to their `[dependencies]`.

`PostgresBackendAdapter` moves from `embyr-server::adapters::postgres_backend` into `embyr-pg-storage`. `embyr-server` becomes a consumer, not an owner, of the adapter.

The workspace crate graph after this change:

```
embyr-proto  (generated stubs)
     ↑
embyr-core   (domain logic, pure)
     ↑
embyr-pg-storage  (sqlx-based BackendAdapter impl, customer DB migrations, NOTIFY listener)
     ↑               ↑
embyr-server      embyr-agent
embyr-admin
```

`deny.toml` for `embyr-pg-storage` allows `sqlx` and `tokio` but forbids `tonic`, `axum`, and `rustls`. This is enforced by `cargo-deny` in CI.

## Alternatives Considered

**Option C2 — independent reimplementation in `embyr-agent`.**
The SQL is not trivial: OCC via `update_time` precondition, tombstone insertion on delete, field-level transforms (server timestamp), transaction read-set tracking, and resume token delta queries. Two independent copies of this logic will diverge. A bug fix in `embyr-server`'s SQL must be manually propagated to the agent. At V1 this is one developer; at V2 with multiple contributors, this becomes a synchronisation hazard. Rejected: duplication violates the maintainability quality attribute ranked 6th in the system architecture (operational simplicity). The SQL complexity is too high for safe duplication.

**Option C3 — agent depends on embyr-server.**
Explicitly forbidden by AD-01. embyr-server imports tonic, axum, rustls, and the three TCP listener tasks. Pulling the entire embyr-server crate into the agent binary would bloat the binary with unused code and violate the statically-linkable constraint. Rejected without evaluation.

## Consequences

**Positive:**
- Single source of truth for customer DB SQL. Bug fixes and schema evolution propagate automatically to both binaries.
- `embyr-pg-storage` can be independently tested with a Postgres testcontainer; tests run for both consumers without duplication.
- The workspace grows from 5 to 6 crates. This is the minimum extension justified by the DISCUSS scope assessment ("SQL complexity is too high for safe duplication").
- `deny.toml` for `embyr-pg-storage` prevents accidental introduction of server-only dependencies.

**Negative:**
- Build time increases slightly (one more crate to compile). At the project's current size, this is negligible.
- `embyr-server` loses ownership of `PostgresBackendAdapter`; the file moves. All imports in `embyr-server` that reference the adapter must be updated.
- Sixth crate increases workspace complexity marginally. This complexity is warranted by duplication avoidance.

## Crate Dependency Rules (enforced by cargo-deny)

| Crate | Allowed IO deps | Forbidden |
|-------|----------------|-----------|
| `embyr-pg-storage` | `sqlx`, `tokio` (async runtime only) | `tonic`, `axum`, `rustls`, `embyr-server` |
| `embyr-agent` | `tonic`, `tokio`, `rustls`, `sqlx` (via pg-storage) | `embyr-server`, `embyr-admin` |
| `embyr-core` | none | all IO crates |
