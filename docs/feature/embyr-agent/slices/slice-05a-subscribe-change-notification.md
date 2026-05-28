# Slice 05A — Subscribe: Change Subscription via Postgres NOTIFY

**Goal**: `Subscribe(project_id) → stream DocChange` server-streaming RPC works end-to-end: agent listens to Postgres NOTIFY, pushes DocChange to embyr SaaS, SaaS fans out to active Listen targets.

**Feature**: embyr-agent
**Estimated effort**: ≤1 day (after pre-slice SPIKE)
**Sequence**: 6 of 6 (last — depends on S01A and S02A; most novel plumbing)

---

## Pre-slice SPIKE (Required — 2 hours max)

**Question 1**: Does Postgres `pg_notify` payload (max ~8KB) reliably carry the full `DocChange` payload for large documents? Confirm: agent must handle payload truncation — extract project_id + path from truncated payload and re-fetch the document via `GetDocument` before pushing on the Subscribe stream.

**Question 2**: Can a tonic server-streaming RPC in an `async fn` safely hold a separate `sqlx` Postgres LISTEN connection (distinct from the pool) in the same tokio runtime without requiring `unsafe` code? Expected answer: yes (tokio tasks, mpsc channel); confirm with minimal test.

**SPIKE output**: If both answers are "yes, straightforward," proceed to full implementation. If "no," document the workaround (e.g., separate LISTEN connection managed as a tokio task with an mpsc channel bridging to the tonic streaming response).

---

## Proto Extension Required

The current `storage_agent.proto` does not declare the `Subscribe` RPC. This slice requires adding:

```protobuf
// Change notification subscription.
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
  string data       = 7;  // proto3-JSON of Document fields (may be empty for Delete)
}
```

---

## IN Scope

- Proto extension: `Subscribe` RPC + `SubscribeRequest` + `DocChange` messages added to `storage_agent.proto`
- Agent implementation:
  - On `Subscribe(project_id)`: open a Postgres `LISTEN doc_changes` connection (NOT from the pool — dedicated connection)
  - On `pg_notify` received: parse payload to extract `project_id`, `path`, `kind`, `version`, `data`
  - If payload truncated (>8KB): re-fetch document via `GetDocument` to obtain full `data`
  - Push `DocChange` onto the server-streaming response channel
  - Channel capacity: 64 (configurable via environment variable)
  - On channel full: set overflow flag in Subscription; stop buffering; SaaS detects `Subscription.Overflowed()` and sends `targetChange{RESET}` to Listen clients
  - On `Subscribe` stream disconnect: clean up LISTEN connection; stop the tokio task
- Postgres trigger (SQL migration): `pg_notify('doc_changes', payload)` fires on every INSERT/UPDATE/DELETE in `documents` table; payload includes `project_id`, `path`, `kind`, `version`
- embyr SaaS Subscribe reconnection: on stream disconnect, SaaS reconnects with exponential backoff 1s→30s (SPEC.md §embyr Agent §Lifecycle §Reconnection)
- embyr SaaS sends `targetChange{RESET}` to all active Listen clients for the project when Subscribe stream reconnects after disconnect (SPEC.md §Change Notification §Agent mode)

## OUT Scope

- Multiple concurrent Subscribe streams for the same project (one stream per project is the contract; SaaS manages this)
- Subscribe stream authentication (mTLS already handles this at the connection level)

---

## Learning Hypothesis

**Disproves**: "Postgres NOTIFY round-trip through the agent adds >2 seconds of latency for typical write rates (≤100 writes/sec)."

**Confirms if successful**: DocChange is delivered to embyr SaaS within 500ms of the Postgres NOTIFY event for writes at ≤100 writes/sec, measured from commit to Subscribe stream message received.

---

## Acceptance Criteria

- [ ] `Subscribe(project_id)` returns a server-streaming response (proto declares the RPC) (SPEC.md §Agent gRPC Protocol: "Subscribe(project_id) → stream DocChange")
- [ ] After `CreateDocument`, agent pushes `DocChange{Upsert}` on the Subscribe stream within 2 seconds (SPEC.md §Change Notification §Agent mode)
- [ ] After `DeleteDocument`, agent pushes `DocChange{Delete}` on the Subscribe stream within 2 seconds (SPEC.md §Change Notification §Agent mode)
- [ ] `DocChange.version >= 1` for upserts; `DocChange.kind == Delete` for deletes (SPEC.md Invariant 2)
- [ ] Subscribe channel reaches capacity 64 → overflow flag set → SaaS sends `targetChange{RESET}` to Listen clients (SPEC.md §Storage Backend Contract §Subscribe: "The channel is buffered (capacity 64). Slow subscribers drop changes rather than blocking the notification path")
- [ ] After Subscribe stream disconnection, SaaS reconnects with exponential backoff starting at 1s, capped at 30s (SPEC.md §embyr Agent §Lifecycle §Reconnection)
- [ ] After reconnect, active Listen clients receive `targetChange{RESET}` and re-snapshot (SPEC.md §Change Notification §Agent mode: "On stream disconnection, embyr reconnects with backoff; active Listen clients for the project receive RESET and re-snapshot")
- [ ] `DocChange.data` is never a truncated payload — full document data is always present (either from NOTIFY payload or re-fetched via GetDocument) (SPEC.md §Storage Backend Contract: "DocChange.Data carries the full document JSON for upserts")
- [ ] Agent LISTEN connection is a separate dedicated connection, not taken from the pool (requirement: pool exhaustion must not interrupt change notifications)
- [ ] Subscribe stream cleanup: on stream disconnect, the Postgres LISTEN connection is closed and the associated tokio task exits cleanly

---

## Spec Traceability

| AC | SPEC.md Reference |
|----|------------------|
| Subscribe RPC declaration | §Agent gRPC Protocol |
| DocChange within 2s | §Change Notification §Agent mode (implied by agent-mode parity with direct NOTIFY mode) |
| DocChange version invariant | §Invariants Invariant 2 |
| Channel capacity 64 + overflow | §Storage Backend Contract §Subscribe |
| SaaS reconnection backoff 1s→30s | §embyr Agent §Lifecycle §Reconnection |
| Listen clients RESET on reconnect | §Change Notification §Agent mode |
| Full DocChange.Data (re-fetch) | §Storage Backend Contract: "DocChange.Data carries the full document JSON for upserts. The Listen handler re-fetches via GetDocument" |

---

## Dependencies

- S01A: `required` (agent wiring; GetDocument needed for re-fetch on truncated NOTIFY payloads)
- S02A: `required` (write operations must emit Postgres NOTIFY triggers for test scenarios)
- Proto extension: `Subscribe` RPC added to `storage_agent.proto` as part of this slice
- Postgres trigger migration: `pg_notify('doc_changes', ...)` trigger on `documents` table
