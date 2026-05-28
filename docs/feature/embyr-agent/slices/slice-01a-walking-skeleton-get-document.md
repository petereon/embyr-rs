# Slice 01A — Walking Skeleton: GetDocument via Agent

**Goal**: Implement `StorageAgent::get_document` end-to-end, proving the full mTLS wiring chain from embyr SaaS AgentAdapter through the agent binary to Postgres.

**Feature**: embyr-agent
**Estimated effort**: ≤1 day (≤6 hours crafter dispatch)
**Sequence**: 1 of 6 (foundation — all other slices depend on this)

---

## IN Scope

- `StorageAgent::get_document` RPC implementation in `crates/embyr-agent/src/server.rs`
  - Postgres SELECT for `(project_id, path)`
  - Returns `Document` proto if found
  - Returns `Status::not_found` if absent
- Postgres pool passed into `StorageAgentService` (replaces `_pool: PgPool` stub)
- `AgentAdapter` in `embyr-server` (or integration test harness): calls `StorageAgent.GetDocument` over mTLS
- Integration test: start real agent binary (testcontainers or Docker Compose), insert row into Postgres, call `AgentAdapter.GetDocument`, assert matching `Document` returned
- Agent log: RPC name, project_id, path, duration_ms (no DSN)

## OUT Scope

- All write RPCs (S02A)
- Query RPCs (S03A)
- Transaction RPCs (S04A)
- Subscribe stream (S05A)
- Lifecycle / graceful shutdown (S06A)
- Missing document field in GetDocumentRequest (transaction bytes, read_time) — stub as not-implemented for now

---

## Learning Hypothesis

**Disproves**: "AgentAdapter in embyr-server cannot talk to embyr-agent via tonic mTLS within the same test harness (two separate binaries, shared proto types)."

**Confirms if successful**: The proto-sharing between embyr-proto, embyr-server, and embyr-agent crates works correctly for end-to-end RPC calls; mTLS cert verification passes with test certs.

---

## Acceptance Criteria

- [ ] `StorageAgent::get_document` returns `Document` with correct field values for a path that exists in Postgres (SPEC.md §Storage Backend Contract §GetDocument)
- [ ] Returns `Status::not_found` for a path absent from Postgres (SPEC.md §Error Model)
- [ ] `InvalidArgument` returned if `name` is empty (SPEC.md §gRPC Service §GetDocument)
- [ ] Integration test passes: embyr SaaS or test client (mTLS) calls GetDocument, receives Document matching Postgres row
- [ ] Agent log line for the RPC includes `project_id`, `path`, `duration_ms`, does NOT contain any Postgres DSN substring

---

## Spec Traceability

| AC | SPEC.md Reference |
|----|------------------|
| Document returned for existing path | §gRPC Service §GetDocument: "Outputs: Document" |
| NotFound for absent path | §gRPC Service §GetDocument: "Errors: NotFound (document absent)" |
| InvalidArgument for empty name | §gRPC Service §GetDocument: "Errors: InvalidArgument (name empty)" |
| DSN not in logs | §embyr Agent §Security Model + Invariant 13 |

---

## Dependencies

- mTLS skeleton (exists — step 09-01, step 09-02): `DONE`
- `embyr_proto::agent` crate with `GetDocumentRequest` / `Document` types: `DONE`
- Testcontainers / Docker Compose test harness with Postgres: `required`

---

## Reference Class

Closest analog: Slice S01 (gRPC server + GetDocument + direct_pg) in embyr-rs story map. That slice took 1 day. This slice is narrower (agent only, no SaaS auth layer) but adds mTLS complexity. Estimate holds at ≤1 day.

---

## Pre-slice SPIKE

None required — mTLS + tonic already proved in step 09-01. Proceed directly.
