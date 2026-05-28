# Wave Decisions — embyr-agent DESIGN Wave
> Feature: embyr-agent full StorageAgent implementation
> Wave: DESIGN
> Updated: 2026-05-27
> Mode: Propose (autonomous analysis, committed recommendations)

---

## Decision Summary

Four architectural decisions required resolution before implementation can begin. Options were analysed against the codebase state, quality attribute ranking (SPEC.md §Quality Attributes), hard constraints from DISCUSS locked decisions, and the existing embyr-rs architectural patterns.

---

## Decision A — Subscribe RPC Design

### Options

| Option | Summary |
|--------|---------|
| A1 (ADOPTED) | Server-streaming `Subscribe(SubscribeRequest) returns (stream DocChange)` — agent emits events, SaaS consumes |
| A2 (Rejected) | Bidirectional streaming with heartbeat — adds protocol complexity with no semantic benefit |
| A3 (Rejected) | Polling — violates p99 ≤ 2s KPI at non-trivial listener counts; ≥500x SQL amplification |

### Recommendation and Rationale

**A1 is adopted.** The agent is the sole event producer; the SaaS is the sole consumer. Server-streaming is the minimal gRPC primitive for this unidirectional flow. Reconnection and backpressure are already handled by the existing `AgentBackendAdapter` (1s→30s exponential backoff, channel capacity 64, overflow → RESET).

The proto extension required (D5 locked decision):
```protobuf
rpc Subscribe(SubscribeRequest) returns (stream DocChange);

message SubscribeRequest {
  string project_id = 1;
}

message DocChange {
  enum Kind {
    UPSERT = 0;
    DELETE = 1;
  }
  string project_id = 1;
  string path       = 2;
  string collection = 3;
  string parent     = 4;
  Kind   kind       = 5;
  int64  version    = 6;
  string data       = 7;  // full document JSON (re-fetched if NOTIFY payload truncated)
}
```

See ADR-A01 for full rationale.

---

## Decision B — BatchGetDocuments

### Options

| Option | Summary |
|--------|---------|
| B1 (Rejected) | Extend proto with `BatchGetDocuments` RPC — agent streams N document results |
| B2 (ADOPTED) | `AgentBackendAdapter.batch_get()` fans out N parallel `GetDocument` RPCs; no proto change |

### Recommendation and Rationale

**B2 is adopted.** The agent SQL for a batch of N documents is N independent `SELECT ... WHERE project_id = $1 AND collection_path = $2 AND document_id = $3` queries regardless of protocol choice. HTTP/2 multiplexing means N concurrent unary RPCs share one TLS connection. The gRPC per-RPC overhead is negligible at Firestore's 100-document batch cap. A dedicated `BatchGetDocuments` proto RPC would add 3+ new message types with no performance benefit at design-target scale.

`AgentBackendAdapter.batch_get()` in `embyr-server::adapters::agent` implements `BackendAdapter.batch_get()` by spawning N concurrent `get_document` futures via `tokio::join_all`.

See ADR-A02 for full rationale.

---

## Decision C — SQL Reuse

### Options

| Option | Summary |
|--------|---------|
| C1 (ADOPTED) | New `embyr-pg-storage` crate — shared sqlx-based BackendAdapter used by both embyr-server and embyr-agent |
| C2 (Rejected) | Independent reimplementation in embyr-agent — duplication of complex OCC/tombstone/transform SQL |
| C3 (Forbidden) | Agent depends on embyr-server — violates AD-01 |

### Recommendation and Rationale

**C1 is adopted.** The customer DB SQL includes OCC preconditions, field-level transforms (server timestamps), tombstone insertion on delete, transaction read-set tracking, and NOTIFY. Two copies will diverge. A 6th workspace crate `embyr-pg-storage` depends on `sqlx + embyr-core` only (no tonic/axum/rustls). Both `embyr-server` and `embyr-agent` depend on it. `cargo-deny` enforces that `embyr-pg-storage` does not pull in server-only IO crates.

Crate graph after this change:
```
embyr-proto
     ↑
embyr-core
     ↑
embyr-pg-storage   (sqlx, tokio; no tonic/axum/rustls)
     ↑          ↑
embyr-server  embyr-agent
embyr-admin
```

`PostgresBackendAdapter` and `PostgresNotifyListener` move from `embyr-server::adapters` to `embyr-pg-storage`. The `notify_channel()` function (BLAKE3 channel naming) also moves to `embyr-pg-storage` so both binaries use the same channel derivation.

See ADR-A03 for full rationale.

---

## Decision D — Project Identity in Requests

### Options

| Option | Summary |
|--------|---------|
| D1 (ADOPTED) | `EMBYR_AGENT_PROJECT_ID` env var — parsed at startup, validated per-RPC |
| D2 (Rejected) | Add `project_id` field to all 8 proto request messages — premature generalisation |
| D3 (Rejected) | Derive from TLS client cert CN — fragile implicit coupling |

### Recommendation and Rationale

**D1 is adopted.** The agent is single-project. The env var model is consistent with all existing `AgentConfig` fields. Riley sets it in her Kubernetes deployment manifest alongside DSN and TLS paths. The `StorageAgentService` stores `project_id: ProjectId` and validates it against the project_id embedded in every proto resource name. Mismatch → `permission_denied` (not silent cross-project query).

New required env var: `EMBYR_AGENT_PROJECT_ID`. Missing or empty → `require_env` exits non-zero with diagnostic.

See ADR-A04 for full rationale.

---

## Constraint Checklist

| Constraint | Source | Design Compliance |
|------------|--------|-------------------|
| embyr-agent must NOT import embyr-server | AD-01 | `embyr-pg-storage` is the shared crate; agent never imports embyr-server |
| embyr-core must NOT import IO crates | deny.toml | embyr-pg-storage is a separate crate; embyr-core is unchanged |
| DSN never in logs | D8 / Invariant 13 | `AgentConfig.db_dsn` is never passed to `tracing::` instrumentation; negative CI test required |
| Postgres probe before gRPC listener | D7 / SPEC §Lifecycle | Startup sequence: probe() → migrate() → open listener (see Application Architecture section) |
| Subscribe channel capacity = 64 | SPEC §Subscribe | AgentSubscribeService buffers on tokio mpsc(64); overflow sets flag → RESET on SaaS |
| mTLS mandatory | SPEC §Security | Tonic `ServerTlsConfig` with `client_ca_root` — unchanged from existing skeleton |
| Single-project agent | DISCUSS out-of-scope | ProjectId from D1 is a single value; no multi-project routing table |
| Statically-linkable binary | SPEC §Agent binary | `embyr-pg-storage` uses `sqlx` with static feature; no runtime shared libs |
