# Story Map — embyr-agent

> Feature: embyr-agent full StorageAgent implementation
> Wave: DISCUSS / Phase 2.5
> Updated: 2026-05-27
> Personas: P4 Riley (DevSecOps — deployment journey), P1 Alex (SDK Developer — proxying journey)

---

## Scope Assessment

**PASS** — This feature is brownfield (mTLS skeleton exists) with a clear bounded context: the `embyr-agent` binary and the `AgentAdapter` in `embyr-server`. Estimated 6 thin slices × ≤1 day = ≤6 days total. Touches 3 bounded contexts: (1) storage adapter protocol, (2) change notification / Subscribe stream, (3) agent lifecycle. This is within the right-sized threshold.

Note: S13 in embyr-rs story map covers agent backend from the SaaS side. This feature (`embyr-agent`) covers the agent binary side (all 8 RPCs, Subscribe stream, lifecycle). The two feature maps are complementary, not duplicates.

---

## Activity Backbone

| A1: Configure + Start | A2: Auth (mTLS) | A3: Document CRUD | A4: Queries | A5: Transactions | A6: Change Subscription | A7: Lifecycle |
|-----------------------|-----------------|-------------------|-------------|-----------------|------------------------|---------------|
| Env var config loading | Agent verifies embyr SaaS cert | GetDocument | RunQuery (filter + order) | BeginTransaction | Subscribe stream to Postgres LISTEN | Startup probe |
| Exit on missing var | SaaS verifies agent cert | CreateDocument | RunQuery (collection group) | GetForTransaction (read in tx) | DocChange fan-out to SaaS | Graceful shutdown |
| Postgres probe on start | Reject unauthenticated conn | UpdateDocument (upsert/update/insert-only + mask + transforms) | ListDocuments | CommitTransaction (OCC) | Reconnect (SaaS-side backoff) | Drain in-flight RPCs |
| mTLS listener start | — | DeleteDocument + tombstone | RunAggregationQuery | RollbackTransaction | Overflow → RESET signal | DSN-never-in-logs invariant |
| Structured startup log | — | BatchWrite (isolated per-write) | ListCollectionIds | SweepExpiredTransactions | — | — |
| — | — | BatchGetDocuments | — | — | — | — |

---

## Walking Skeleton

**Minimum slice** (S01A): the `GetDocument` RPC implemented end-to-end through the full chain:

```
embyr SaaS (AgentAdapter) → mTLS gRPC → embyr-agent → Postgres SELECT → Document response
```

This single RPC proves: mTLS connection, proto marshalling, Postgres pool usage, and response translation. All other document operations extend this pattern.

**Walking skeleton estimate: ≤6 hours**
**Learning hypothesis**: disproves "mTLS + proto bridging across two separate Rust binaries is too complex to get right in one day."

---

## Elephant Carpaccio Slices (6 slices)

### S01A: Walking Skeleton — GetDocument proxied through agent (≤1 day)

**Goal**: prove the full SaaS→agent→Postgres wiring chain by implementing a single read RPC.

**IN scope**:
- `StorageAgent::get_document` implementation in `embyr-agent`
- Postgres `SELECT` for `(project_id, path)`
- `NotFound` when absent, `Document` proto response when present
- Integration test: embyr SaaS sends GetDocument to live agent binary; Postgres contains the doc

**OUT scope**: all write operations, transactions, queries, Subscribe stream, lifecycle

**Learning hypothesis**: disproves "AgentAdapter in embyr-server can't talk to embyr-agent via tonic mTLS in the same test suite"

**Dependencies**: mTLS skeleton (exists — step 09-01 complete)

**Acceptance criteria**:
- `StorageAgent::get_document` returns `Document` for existing path in Postgres
- Returns `NotFound` for absent path
- Integration test: embyr SaaS `AgentAdapter.GetDocument` routed through mTLS to agent returns same data as direct Postgres query
- Agent log shows RPC name, project_id, path (no DSN)

---

### S02A: Write Operations — Create, Update, Delete via agent (≤1 day)

**Goal**: all document mutations work through the agent with OCC semantics identical to direct mode.

**IN scope**:
- `CreateDocument` (with and without document_id generation, 20-char random ID)
- `UpdateDocument` (upsert / update / insert-only preconditions, update_mask, field transforms: SERVER_TIME, increment, appendMissingElements, removeAllFromArray)
- `DeleteDocument` (idempotent + mustExist preconditions, tombstone insertion)
- `BatchWrite` (isolated per-write transactions)
- `DocChange{Upsert}` / `DocChange{Delete}` events emitted after each committed write

**OUT scope**: Subscribe stream delivery of DocChange to SaaS (S05A), transaction RPC (S04A)

**Learning hypothesis**: disproves "Field transforms (increment, array ops) are too complex to implement correctly in the agent layer"

**Dependencies**: S01A

---

### S03A: Query Operations — RunQuery, ListDocuments, BatchGet, Aggregation, ListCollectionIds (≤1 day)

**Goal**: all read-only query RPCs work through the agent identically to Postgres adapter.

**IN scope**:
- `RunQuery` (filter operators: ==, !=, <, <=, >, >=, in, not-in, array-contains, array-contains-any; unary: IS_NULL, IS_NOT_NULL, IS_NAN, IS_NOT_NAN; AND/OR composites; collection group with all_descendants)
- `RunQuery` with cursor pagination (startAt, startAfter, endAt, endBefore)
- `RunAggregationQuery` (COUNT, SUM, AVG)
- `ListDocuments` (pagination, collection filter)
- `BatchGetDocuments` (including new_transaction option)
- `ListCollectionIds`

**OUT scope**: index enforcement for complex queries (deferred — simple queries only in this slice; composite index validation is a later concern)

**Learning hypothesis**: disproves "SQL translation of all Firestore filter operators requires more than one day"

**Dependencies**: S01A, S02A (for test data)

---

### S04A: Transaction Lifecycle — Begin, GetForTransaction, Commit (OCC), Rollback, Sweep (≤1 day)

**Goal**: full transaction RPC set with OCC semantics works through agent.

**IN scope**:
- `BeginTransaction` (creates transaction record in Postgres)
- `GetDocument` with transaction bytes (reads document + records version in tx read set)
- `CommitTransaction` (OCC re-validation, write application, DocChange emission)
- `RollbackTransaction` (deletes tx record, no writes)
- `SweepExpiredTransactions` (background sweep of expired tx records)
- `BatchGetDocuments` with `new_transaction` option

**OUT scope**: BeginTransaction in the Firestore service RPC sense (that's in embyr-server); this slice covers the StorageAgent protocol methods only

**Learning hypothesis**: disproves "OCC read-set re-validation via agent adds >50ms latency vs direct mode"

**Dependencies**: S02A (writes must work before tx commit can test OCC)

---

### S05A: Change Subscription — Subscribe streaming RPC + Postgres LISTEN (≤1 day)

**Goal**: `Subscribe` server-streaming RPC works end-to-end: agent listens to Postgres NOTIFY, pushes DocChange to embyr SaaS, SaaS fans out to active Listen targets.

**IN scope**:
- `Subscribe(project_id) → stream DocChange` RPC in agent
- Agent opens Postgres `LISTEN doc_changes` on a dedicated connection
- Postgres trigger on `documents` INSERT/UPDATE fires `pg_notify('doc_changes', payload)` with project_id
- Agent pushes `DocChange{Upsert/Delete}` to open Subscribe stream
- embyr SaaS SaaS-side reconnection with exponential backoff 1s→30s on stream disconnect
- Subscribe channel capacity 64; overflow → `Subscription.Overflowed()` flag set
- embyr SaaS sends `targetChange{RESET}` when overflow detected

**OUT scope**: this proto (storage_agent.proto) does not yet declare Subscribe — proto extension required as part of this slice

**Learning hypothesis**: disproves "Postgres NOTIFY round-trip through agent adds >2s latency for typical write rates"

**Dependencies**: S02A (writes must emit triggers), S01A (agent wiring established)

**Pre-slice SPIKE**: confirm pg_notify payload size with Postgres 8KB limit; confirm tonic server-streaming in an async context with a separate LISTEN connection doesn't require unsafe code.

---

### S06A: Agent Lifecycle — Startup Probe, Graceful Shutdown, DSN Log Discipline (≤1 day)

**Goal**: agent startup is atomic (all-or-nothing), shutdown is clean, and the DSN never appears in any log output.

**IN scope**:
- Startup probe: Postgres connectivity verified (sqlx `SELECT 1`) BEFORE gRPC listener opens
- Config validation: all required env vars present; cert files readable; listen address parseable
- Structured JSON log output (tracing + tracing-subscriber JSON format)
- Graceful shutdown: on SIGTERM, stop accepting new connections, drain in-flight RPCs, close Postgres pool, exit 0
- DSN log discipline: DSN string MUST NOT appear in any log output at any level; negative test required
- `EMBYR_AGENT_MAX_CONNS` support in config (currently missing from AgentConfig::from_env)

**OUT scope**: agent metrics endpoint, agent binary distribution/packaging

**Learning hypothesis**: disproves "Draining in-flight Tonic RPCs on SIGTERM requires >1 day of Rust async plumbing"

**Dependencies**: S01A (agent binary must be running to test shutdown)

---

## Slice Execution Order and Prioritization

| Priority | Slice | Rationale |
|----------|-------|-----------|
| 1 | S01A — Walking Skeleton | Foundation; unblocks all other slices; highest learning leverage |
| 2 | S06A — Lifecycle | Audit invariant (DSN-never-in-logs); enables production deployment; parallel with S02A once S01A lands |
| 3 | S02A — Write Operations | Unblocks S03A, S04A, S05A; write path is the second-highest frequency |
| 4 | S04A — Transactions | OCC latency is the riskiest assumption; test early |
| 5 | S03A — Query Operations | Complex but well-understood from Postgres adapter; test after writes work |
| 6 | S05A — Subscribe | Novel async plumbing; SPIKE first; schedule after S02A proves DocChange emission |

**Riskiest assumptions first**:
- S01A: "mTLS + proto bridging works" — must prove before anything else
- S04A: "OCC latency via agent is acceptable" — schedule 4th (early enough to pivot if slow)
- S05A: "NOTIFY round-trip via agent is <2s" — pre-slice SPIKE de-risks

---

## Elephant Carpaccio Taste Test Results

| Test | Result |
|------|--------|
| No slice lists 4+ new components | PASS — each slice adds 1-2 RPCs max |
| No slice depends on a new abstraction that must ship first | PASS — S01A establishes AgentAdapter; others extend it |
| Each slice disproves a pre-commitment hypothesis | PASS — all 6 slices have named learning hypotheses |
| No slice uses only synthetic data | PASS — integration tests use real Postgres (testcontainers or Docker Compose) |
| No two slices are identical except for scale | PASS — each slice covers distinct RPC groups |

All 5 taste tests PASS.
