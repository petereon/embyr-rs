<!-- markdownlint-disable MD024 -->
# Feature Delta — embyr-agent

> Feature ID: embyr-agent
> Wave: DISCUSS
> Updated: 2026-05-27
> Status: Ready for DESIGN handoff
> Parent feature: embyr-rs (this feature extends Slice S13 of the embyr-rs DISCUSS wave)

---

## Wave: DISCUSS / [REF] Personas

| ID | Name | Role | Primary Jobs (this feature) |
|----|------|------|-----------------------------|
| P4 | Riley Nakamura | DevSecOps Lead, FinOps Corp | JOB-04 (credential-isolation), JOB-07 (agent-operations), JOB-09 (agent-auditproof) |
| P1 | Alex Chen | SDK Developer | JOB-01 (sdk-compat via agent), JOB-03 / JOB-08 (live-sync via agent) |

---

## Wave: DISCUSS / [REF] JTBD One-Liners

| Job ID | One-liner |
|--------|-----------|
| JOB-04 | When my security policy prohibits DB credentials leaving my VPC, I want to deploy the embyr agent alongside Postgres, so I can use the Firestore SDK without exposing credentials to any third party. |
| JOB-07 | When I need to put the embyr agent into production, I want to configure it via env vars and see clear startup logs, so I can complete the deployment in under 30 minutes and hand off a green health check. |
| JOB-08 | When SDK clients use onSnapshot on an agent-backed project, I want the agent to forward Postgres NOTIFY events to embyr SaaS, so real-time features work identically to direct-mode projects. |
| JOB-09 | When my external auditor asks for credential-egress proof, I want to show system DB evidence plus DSN-free logs, so I can close the finding without a remediation plan. |

---

## Wave: DISCUSS / [REF] Locked Decisions

| ID | Decision | Verdict |
|----|----------|---------|
| D1 | Feature type | Cross-cutting — auth (mTLS), storage (all 8 RPCs), change notification (Subscribe), lifecycle |
| D2 | Walking skeleton | Brownfield — mTLS skeleton exists; WS = GetDocument proxied end-to-end (S01A) |
| D3 | UX research depth | Lightweight — happy path focus, backend service |
| D4 | JTBD analysis | Full — JOB-04 extended; JOB-07, JOB-08, JOB-09 added to jobs.yaml |
| D5 | Subscribe proto gap | `Subscribe` RPC is NOT in current storage_agent.proto — proto extension required in S05A |
| D6 | BatchGetDocuments via agent | Proto gap: not declared; DESIGN wave decides: extend proto OR implement as N parallel GetDocument calls in AgentAdapter |
| D7 | Startup probe ordering | Postgres probe MUST complete before gRPC listener opens (SPEC.md §Lifecycle) |
| D8 | DSN log discipline | Hard invariant: DSN must not appear in any log at any level; negative test is mandatory AC |
| D9 | Slice execution order | S01A → S06A → S02A → S04A → S03A → S05A (learning leverage + dependency chain) |

---

## Wave: DISCUSS / [REF] Scope Assessment

PASS — 6 slices × ≤1 day = ≤6 days. Touches 3 bounded contexts: (1) storage adapter protocol, (2) change notification/Subscribe, (3) agent lifecycle. Within right-sized threshold.

Note: Slice S13 in embyr-rs covers the SaaS-side AgentAdapter. This feature covers the agent binary. The two are complementary.

---

## Wave: DISCUSS / [REF] User Stories

### US-A01 — GetDocument via agent (Walking Skeleton)

**As** Riley (DevSecOps Lead at FinOps Corp),
**I want** the embyr-agent binary to handle `GetDocument` RPCs from embyr SaaS and return the document from local Postgres,
**so that** I can prove the end-to-end mTLS wiring chain works before committing to a full agent deployment.

### Elevator Pitch
Before: All StorageAgent RPCs return `Unimplemented`; Riley cannot test whether the agent actually proxies anything.
After: embyr SaaS calls `StorageAgent.GetDocument` → agent returns `Document` from Postgres → `getDoc(doc(db, "users/riley"))` resolves in the SDK.
Decision enabled: Riley can verify the agent works before configuring production projects.

**job_id**: JOB-04
**slice**: S01A

### Domain Examples

**1. Happy path — document exists**: Riley's integration test inserts `users/riley` into Postgres, calls `getDoc` via the Firebase SDK (connected to embyr SaaS, project `finops-prod` with `backend_mode=agent`). The SDK receives `{ name: "Riley", role: "devops" }`. Agent logs show `msg="GetDocument"` with `project_id="finops-prod"`, `path="users/riley"`, `duration_ms=3`.

**2. Edge case — document absent**: Riley calls `getDoc(doc(db, "orders/nonexistent"))`. The SDK receives `snap.exists() === false`. Agent returns `Status::not_found`. No error thrown.

**3. Error case — name empty**: A buggy client sends an empty `name` field. Agent returns `InvalidArgument`. SDK receives a network error; agent log shows `msg="InvalidArgument: name is empty"`.

### UAT Scenarios (BDD)

```gherkin
Scenario: SDK getDoc round-trips through agent to Postgres
  Given the document "orders/ord-2026-001" exists in Postgres for project finops-prod
  And the Firebase SDK is configured for project finops-prod on embyr SaaS
  When the SDK calls getDoc(doc(db, "orders/ord-2026-001"))
  Then the SDK receives the document with the correct field values
  And the agent log contains an entry for GetDocument with project_id=finops-prod

Scenario: getDoc returns not-found for absent document
  Given the document "orders/nonexistent" does not exist in Postgres for project finops-prod
  When the SDK calls getDoc(doc(db, "orders/nonexistent"))
  Then snap.exists() is false
  And no error is thrown

Scenario: Invalid name returns error
  Given a gRPC client sends GetDocumentRequest with an empty name field
  When the agent receives the request
  Then the agent returns Status::invalid_argument with message containing "name"
```

### Acceptance Criteria
- [ ] `GetDocument` returns `Document` with correct fields for existing path (SPEC.md §gRPC Service §GetDocument)
- [ ] Returns `NotFound` for absent path (SPEC.md §GetDocument Errors)
- [ ] Returns `InvalidArgument` for empty name (SPEC.md §GetDocument Errors)
- [ ] Agent log for RPC includes project_id, path, duration_ms; excludes DSN (SPEC.md Invariant 13)

### Outcome KPIs
- **Who**: Riley (DevSecOps Lead deploying agent)
- **Does what**: Verifies GetDocument works end-to-end via agent within 30 minutes of first deploy
- **By how much**: 100% of agent deployments that reach "listening on :9191" pass the GetDocument integration test
- **Measured by**: Integration test suite pass rate in CI against real agent binary
- **Baseline**: 0% today (all RPCs Unimplemented)

---

### US-A02 — Write operations via agent (Create, Update, Delete)

**As** Alex (SDK Developer, whose project uses backend_mode=agent),
**I want** `setDoc`, `updateDoc`, and `deleteDoc` to persist changes through the agent identically to Firestore,
**so that** my write-heavy features work without any SDK code changes.

### Elevator Pitch
Before: SDK writes to agent-backed projects return `Unimplemented`; Alex's app cannot write any data.
After: `setDoc(doc(db, "orders/ord-2026-001"), { status: "pending" })` resolves; the document appears in Postgres with `version=1`.
Decision enabled: Alex can verify write round-trips work before migrating his app's project to agent mode.

**job_id**: JOB-01
**slice**: S02A

### Domain Examples

**1. Happy path — create**: Alex calls `setDoc(doc(db, "orders/ord-2026-001"), { customerId: "C-489", amount: 1250 })`. The document is created in Postgres with `version=1`. `getDoc` returns the same data.

**2. Edge case — update with mask**: Alex calls `updateDoc(doc(db, "orders/ord-2026-001"), { status: "shipped" })`. Only the `status` field changes; `customerId` and `amount` are preserved (update_mask semantics). `version` increments to 2.

**3. Error case — OCC conflict on update_time precondition**: Alex's client and a backend service both read `orders/ord-2026-001` at `version=2` and attempt `setDoc` with `update_time` precondition set to the same timestamp. One succeeds (version=3); the other receives `FailedPrecondition`.

### UAT Scenarios (BDD)

```gherkin
Scenario: SDK setDoc creates document through agent
  Given the document "orders/ord-2026-001" does not exist
  When the SDK calls setDoc(doc(db, "orders/ord-2026-001"), { status: "pending" })
  Then the setDoc Promise resolves
  And getDoc(doc(db, "orders/ord-2026-001")) returns { status: "pending" }
  And the Postgres row has version = 1

Scenario: SDK updateDoc with mask preserves unmentioned fields
  Given the document "orders/ord-2026-001" exists with { customerId: "C-489", amount: 1250 }
  When the SDK calls updateDoc(doc(db, "orders/ord-2026-001"), { status: "shipped" })
  Then getDoc returns { customerId: "C-489", amount: 1250, status: "shipped" }
  And the Postgres row has version = 2

Scenario: SDK deleteDoc is idempotent
  Given the document "orders/ord-2026-001" does not exist
  When the SDK calls deleteDoc(doc(db, "orders/ord-2026-001"))
  Then the call resolves without error

Scenario: Document write increments version monotonically
  Given the document "orders/ord-2026-001" is written 3 times
  When getDoc is called after each write
  Then the versions are 1, 2, 3 respectively (never repeating, never decreasing)

@property
Scenario: version column monotonically increases across successive writes
  Given the system is under concurrent writes to the same document
  Then the Postgres version column never decreases
  And each successful write produces a version exactly 1 greater than the previous
```

### Acceptance Criteria
- [ ] `CreateDocument` with no document_id generates 20-char `[a-zA-Z0-9]` ID (SPEC.md §Document ID Generation)
- [ ] `UpdateDocument` with `update_mask` preserves fields outside the mask (SPEC.md §Write Semantics §Mask application)
- [ ] `DeleteDocument` without precondition is idempotent (SPEC.md §DeleteDocument)
- [ ] `DeleteDocument` successful: tombstone inserted in `deleted_documents` (SPEC.md §Data Model §Tombstone)
- [ ] `version` increments by 1 on every write; `version >= 1` always (SPEC.md Invariant 3)
- [ ] `DocChange{Upsert}` emitted after every successful write (SPEC.md §CreateDocument side effects)
- [ ] `increment` transform on missing field treats missing as 0 (SPEC.md §Field Transforms)

### Outcome KPIs
- **Who**: Alex (SDK Developer) using agent-backed project
- **Does what**: Writes and reads documents without SDK code changes vs. Firestore
- **By how much**: Zero SDK code changes required; write-read round-trip latency within 50ms of direct-mode on LAN
- **Measured by**: Integration test suite; latency measured in test
- **Baseline**: 0 successful writes through agent today

---

### US-A03 — Query operations via agent

**As** Alex,
**I want** `getDocs(query(...))`, `runAggregation(...)`, and `listCollectionIds` to work through the agent,
**so that** read-heavy features (dashboards, lists, analytics) work without SDK changes.

### Elevator Pitch
Before: Query RPCs return `Unimplemented` from agent; dashboards that use `getDocs(query)` are broken for agent-mode projects.
After: `getDocs(query(collection(db, "orders"), where("status", "==", "pending")))` returns matching orders from Postgres.
Decision enabled: Alex can run his existing query-heavy features against agent-mode projects without modifying a single query.

**job_id**: JOB-01
**slice**: S03A

### Domain Examples

**1. Happy path — filtered query**: Alex queries `orders` where `status == "pending"`. Returns 3 matching orders from Postgres. Final response includes `{done: true}`.

**2. Edge case — collection group query**: Alex queries all `line_items` collections at any depth under `projects/finops-prod`. Agent uses `collection = 'line_items'` SQL filter without restricting by parent. Returns items from `orders/*/line_items`.

**3. Error case — invalid field path**: Query filter uses field path `order..amount` (double dot). Agent returns `InvalidArgument`. SDK receives error before any SQL executes.

### UAT Scenarios (BDD)

```gherkin
Scenario: Filtered query returns matching documents only
  Given 5 orders exist; 3 have status="pending", 2 have status="shipped"
  When getDocs(query(collection(db, "orders"), where("status", "==", "pending")))
  Then 3 documents are returned
  And none have status="shipped"

Scenario: Collection group query traverses nested collections
  Given orders/ord-001/line_items/item-1 and orders/ord-002/line_items/item-2 exist
  When a collection group query runs for collectionId="line_items"
  Then both documents are returned regardless of parent path

Scenario: RunAggregationQuery returns correct COUNT
  Given 7 orders exist for project finops-prod
  When runAggregation(query(collection(db,"orders")), count()) is called
  Then the result shows count = 7

Scenario: Invalid field path returns error before SQL
  Given a query uses field path "order..amount" (invalid: double dot)
  When the query is dispatched to the agent
  Then the agent returns InvalidArgument
  And no Postgres query is executed
```

### Acceptance Criteria
- [ ] `RunQuery` with `==` filter returns only matching documents (SPEC.md §Query System §Filter Operators)
- [ ] `RunQuery` with `!=` excludes documents missing the field (SPEC.md §Query System: "missing field excluded")
- [ ] Collection group query with `all_descendants=true` matches docs in nested collections (SPEC.md §Collection Group Queries)
- [ ] Field path validated against `^[a-zA-Z_][a-zA-Z0-9_.]*$`; invalid path returns `InvalidArgument` (SPEC.md Invariant 6)
- [ ] Streaming `RunQuery` response ends with `{done: true}` (SPEC.md §RunQuery response format)
- [ ] `RunAggregationQuery` COUNT returns correct document count (SPEC.md §RunAggregationQuery)
- [ ] `ListDocuments` default page size 100; `next_page_token` absent on last page (SPEC.md §Pagination)

### Outcome KPIs
- **Who**: Alex (SDK Developer) using query-heavy features on agent-backed project
- **Does what**: Runs existing queries without SDK modification
- **By how much**: 100% of query operations that succeed in direct mode succeed in agent mode
- **Measured by**: Parity test suite: same queries against direct-mode and agent-mode projects; results must match
- **Baseline**: 0 successful queries through agent today

---

### US-A04 — Transaction lifecycle via agent

**As** Alex,
**I want** Firestore transactions (`runTransaction(async (t) => {...})`) to work through the agent with OCC semantics,
**so that** my app's concurrent-write scenarios (order processing, inventory management) are safe.

### Elevator Pitch
Before: `runTransaction` fails with `Unimplemented` on agent-mode projects; Alex cannot safely implement concurrent writes.
After: `runTransaction(async (t) => { const doc = await t.get(ref); t.set(ref, {...}); })` commits atomically; concurrent writes trigger automatic retry.
Decision enabled: Alex can verify that his order-processing logic (read-then-write) is safe against concurrent updates.

**job_id**: JOB-01
**slice**: S04A

### Domain Examples

**1. Happy path — clean commit**: Alex's transaction reads `orders/ord-2026-001` (version=3) and updates the `status` field. No concurrent writes. Commit succeeds; version becomes 4.

**2. Edge case — OCC conflict triggers retry**: Two clients simultaneously read `orders/ord-2026-001` at version=3 and both try to commit. One commits (version=4); the second receives `Aborted "version mismatch for orders/ord-2026-001"`. Firebase SDK retries the transaction automatically; it succeeds on retry (version=5).

**3. Error case — read document deleted during transaction**: Alex's transaction reads `orders/ord-2026-001` (version=3). Before commit, another process deletes the document. Commit returns `Aborted "<path> was deleted"`. Alex's error handler receives the abort and presents a "order was cancelled" message.

### UAT Scenarios (BDD)

```gherkin
Scenario: Transaction commits atomically through agent
  Given orders/ord-2026-001 exists at version 3 with status="processing"
  When runTransaction reads the document and sets status="complete"
  And no concurrent writes occur during the transaction
  Then the transaction commits successfully
  And getDoc shows version=4, status="complete"

Scenario: OCC conflict causes Aborted and triggers SDK retry
  Given orders/ord-2026-001 exists at version 3
  And two concurrent transactions both read version 3
  When both attempt to commit
  Then exactly one commit succeeds
  And the other receives Aborted with "version mismatch"
  And the Firebase SDK retries the aborted transaction

Scenario: Transaction reading deleted document is aborted
  Given orders/ord-2026-001 is read within a transaction at version 5
  And the document is deleted by another process before commit
  When the transaction attempts to commit
  Then the commit returns Aborted with "<path> was deleted"
```

### Acceptance Criteria
- [ ] `BeginTransaction` creates record with `expires_at = started_at + transactions_ttl` (SPEC.md §Transactions §Begin)
- [ ] `GetDocument` with transaction bytes records path + version in read set (SPEC.md §Transactions §Read-Within-Transaction)
- [ ] `CommitTransaction` returns `Aborted "version mismatch for <path>"` on concurrent write (SPEC.md §Transactions §Commit step 3)
- [ ] `CommitTransaction` returns `Aborted "<path> was deleted"` when read doc is deleted (SPEC.md §Transactions §Commit step 3)
- [ ] `RollbackTransaction` deletes tx record without applying writes (SPEC.md §Transactions §Rollback)
- [ ] `SweepExpiredTransactions` deletes records with `expires_at < now` (SPEC.md §Transactions §Sweep)

### Outcome KPIs
- **Who**: Alex (SDK Developer) implementing concurrent writes
- **Does what**: Uses `runTransaction` on agent-backed projects; SDK retries on Aborted without special handling
- **By how much**: OCC abort rate stays below 5% for well-designed transactions (read-before-write on same document, low contention)
- **Measured by**: Integration test measuring abort rate under concurrent write load (10 goroutines, same document, 100 iterations)
- **Baseline**: 0 transactions succeed through agent today

---

### US-A05 — Real-time change subscription via Subscribe stream

**As** Alex,
**I want** `onSnapshot` to fire for every committed write to an agent-backed project within 2 seconds,
**so that** collaborative features (live order status, real-time dashboards) work identically to direct-mode Firestore.

### Elevator Pitch
Before: `onSnapshot` never fires for agent-mode projects (no Subscribe RPC, no change notifications); real-time features are broken.
After: `onSnapshot(collection(db, "orders"), callback)` fires within 2 seconds of any write to the `orders` collection.
Decision enabled: Alex can certify that his real-time feature works in agent mode before migrating his production project.

**job_id**: JOB-08
**slice**: S05A

### Domain Examples

**1. Happy path — live update**: Alex's dashboard has `onSnapshot` registered on `orders`. Riley's backend updates `orders/ord-2026-001 { status: "shipped" }`. Alex's callback fires within 2 seconds. The change shows `status: "shipped"`.

**2. Edge case — Subscribe overflow**: 65 writes are committed within 1 second to project finops-prod. The Subscribe channel (capacity 64) overflows. The 65th DocChange is dropped; the overflow flag is set. embyr SaaS detects overflow and sends `targetChange{RESET}` to all Listen clients. Alex's SDK re-fetches the full snapshot and fires `onSnapshot` with the complete current state.

**3. Error case — agent disconnect**: The VPC network briefly drops. The Subscribe stream disconnects. embyr SaaS starts reconnecting (1s → 2s → 4s → ... → 30s backoff). Active Listen clients receive `targetChange{RESET}`. Alex's SDK re-snapshots. When the stream reconnects, real-time delivery resumes normally.

### UAT Scenarios (BDD)

```gherkin
Scenario: onSnapshot fires within 2 seconds of write through agent
  Given Alex has called onSnapshot on the orders collection for project finops-prod
  And the Subscribe stream is active
  When updateDoc(doc(db, "orders/ord-2026-001"), { status: "shipped" }) is called
  Then Alex's onSnapshot callback fires within 2 seconds
  And the change reflects status = "shipped"

Scenario: Subscribe channel overflow triggers RESET and re-snapshot
  Given 64 pending DocChange events are buffered in the Subscribe channel
  When one more document is written
  Then the overflow flag is set
  And embyr SaaS sends targetChange{RESET} to all Listen clients for finops-prod
  And clients receive a complete fresh snapshot

Scenario: Subscribe stream reconnects after agent disconnect
  Given Alex's onSnapshot is registered and active
  When the Subscribe stream disconnects (agent restart or network failure)
  Then embyr SaaS begins reconnecting with exponential backoff starting at 1 second
  And active Listen clients receive targetChange{RESET}
  And after the agent reconnects, new writes are again delivered within 2 seconds

@property
Scenario: DocChange events contain non-truncated document data
  Given documents in finops-prod have fields totalling up to 100KB
  When these documents are written and the DocChange is pushed over Subscribe
  Then DocChange.data contains the complete document JSON (re-fetched if NOTIFY payload was truncated)
  And no field values are missing or corrupted
```

### Acceptance Criteria
- [ ] `Subscribe(project_id)` RPC declared in proto and implemented in agent (SPEC.md §Agent gRPC Protocol)
- [ ] `DocChange{Upsert}` pushed within 2s of committed write (SPEC.md §Change Notification §Agent mode parity)
- [ ] Subscribe channel capacity 64; overflow sets overflow flag (SPEC.md §Storage Backend Contract §Subscribe)
- [ ] embyr SaaS reconnects to agent with exponential backoff 1s→30s on stream disconnect (SPEC.md §embyr Agent §Lifecycle §Reconnection)
- [ ] Listen clients receive `targetChange{RESET}` on Subscribe reconnect (SPEC.md §Change Notification §Agent mode)
- [ ] `DocChange.data` always carries full document JSON, never truncated (SPEC.md §Storage Backend Contract: "DocChange.Data carries the full document JSON for upserts")
- [ ] `DocChange.version >= 1` for upserts; `DocChange.kind == Delete` for deletes (SPEC.md Invariant 2)

### Outcome KPIs
- **Who**: Alex (SDK Developer) using real-time features on agent-backed project
- **Does what**: `onSnapshot` fires within 2 seconds of committed writes, same as direct-mode projects
- **By how much**: P99 latency from write commit to `onSnapshot` callback < 2s under normal load (≤100 writes/sec)
- **Measured by**: Integration test: write → measure time-to-onSnapshot callback in test harness
- **Baseline**: onSnapshot never fires for agent-mode projects today

---

### US-A06 — Agent startup probe and graceful shutdown

**As** Riley (DevSecOps Lead),
**I want** the agent to verify Postgres connectivity before accepting RPCs, emit structured startup logs, and drain in-flight calls on SIGTERM,
**so that** I can deploy the agent safely in Kubernetes and pass SOC2 audit on log discipline.

### Elevator Pitch
Before: The agent opens a gRPC listener even when Postgres is unreachable; early RPCs fail with cryptic connection errors; startup failure is not clearly signaled.
After: The agent logs `{"msg":"connected to Postgres"}` BEFORE `{"msg":"listening on :9191"}`; on SIGTERM it drains in-flight RPCs before exiting 0.
Decision enabled: Riley can use the Kubernetes `readinessProbe` (TCP check on :9191) as a reliable indicator that the agent is fully ready to serve traffic.

**job_id**: JOB-07
**slice**: S06A

### Domain Examples

**1. Happy path — clean startup**: Riley deploys the agent with all required env vars. Agent logs `connected to Postgres`, then `listening on :9191`. Kubernetes marks the pod as Ready. Riley calls `GetDocument`; it responds immediately.

**2. Edge case — SIGTERM during in-flight RPC**: Riley triggers a rolling restart. The pod receives SIGTERM. The agent stops accepting new connections but completes the GetDocument RPC currently in flight (1s). Exits with code 0. The RPC caller receives the response normally.

**3. Audit invariant — DSN never in logs**: Riley configures the agent with DSN `postgres://admin:DO-NOT-LOG@pg.finops.internal/db`. He starts the agent, runs 10 RPCs, exports all pod logs, and greps for `DO-NOT-LOG`. Zero matches. Riley attaches this grep output to the SOC2 audit package.

### UAT Scenarios (BDD)

```gherkin
Scenario: Startup probe gates gRPC listener on Postgres readiness
  Given Postgres is reachable at EMBYR_AGENT_DB_DSN
  When the agent binary starts
  Then "connected to Postgres" appears in the log before "listening on :9191"
  And a GetDocument call to the agent succeeds immediately after both lines appear

Scenario: Startup fails if Postgres is unreachable
  Given EMBYR_AGENT_DB_DSN points to an unreachable address
  When the agent binary starts
  Then the log contains an ERROR message about the connection failure
  And the process exits with a non-zero exit code
  And no port is bound

Scenario: SIGTERM drains in-flight RPCs before shutdown
  Given the agent has an in-flight GetDocument RPC in progress
  When SIGTERM is sent to the agent process
  Then new connections are rejected immediately
  And the in-flight RPC completes and returns its response
  And the process exits with code 0 after the RPC completes
  And the log contains "shutdown complete"

Scenario: DSN never appears in agent logs
  Given EMBYR_AGENT_DB_DSN contains the string "DO-NOT-LOG"
  When the agent starts and handles 10 GetDocument RPCs
  Then no log line at any level contains "DO-NOT-LOG"
  And the agent logs are structured JSON (parseable with jq)

Scenario: Missing required env var causes immediate exit
  Given EMBYR_AGENT_DB_DSN is not set
  When the agent binary starts
  Then stderr contains "missing required environment variable: EMBYR_AGENT_DB_DSN"
  And the process exits with code 1
  And no port is bound
```

### Acceptance Criteria
- [ ] Startup log order: `"connected to Postgres"` appears BEFORE `"listening on :9191"` (SPEC.md §embyr Agent §Lifecycle §Startup)
- [ ] Startup probe failure exits with non-zero code; no port is bound (SPEC.md §Lifecycle)
- [ ] `EMBYR_AGENT_MAX_CONNS` configures pool max connections; defaults to 25 (SPEC.md §Configuration)
- [ ] `EMBYR_AGENT_LOG_LEVEL` configures log level; defaults to `info`
- [ ] All log output is structured JSON (fields: `level`, `ts`, `msg`)
- [ ] DSN never appears in any log line; negative test with sentinel string passes (SPEC.md Invariant 13)
- [ ] SIGTERM: in-flight RPCs complete; exit code 0; log shows `"shutdown complete"` (SPEC.md §Lifecycle §Shutdown)
- [ ] RPCs still in-flight after 30s drain timeout: cancelled; callers receive `Unavailable` (SPEC.md §Lifecycle §Shutdown)

### Outcome KPIs
- **Who**: Riley (DevSecOps Lead deploying agent in VPC)
- **Does what**: Verifies agent health from pod logs within 60 seconds of deploy; passes DSN-not-in-logs audit check without remediation
- **By how much**: Zero open audit findings related to agent log discipline after first deployment
- **Measured by**: SOC2 audit report; CI negative test (grep for sentinel DSN string)
- **Baseline**: No structured logs; no startup ordering guarantee; no graceful shutdown; DSN safety untested

---

## Wave: DISCUSS / [REF] Out-of-Scope

- Agent binary distribution, Docker image packaging, Helm chart
- Agent metrics / Prometheus endpoint
- Agent HTTP health endpoint (Kubernetes can use TCP check on :9191)
- Composite index enforcement for complex queries (deferred)
- Multi-project agent (agent serves a single project; multi-project is a future extension)
- Agent-side rate limiting (rate limiting is embyr SaaS responsibility)

---

## Wave: DISCUSS / [REF] System Constraints

1. **DSN never in agent logs at any level** — hard invariant. Enforced by code review + negative test AC on US-A06. (SPEC.md Invariant 13)
2. **mTLS is mandatory** — no unauthenticated mode. (SPEC.md §embyr Agent §Security Model)
3. **Subscribe channel capacity = 64** — overflow drops events and signals RESET. (SPEC.md §Storage Backend Contract §Subscribe)
4. **Startup probe ordering** — Postgres probe MUST complete before gRPC listener opens. (SPEC.md §Lifecycle)
5. **Subscribe proto gap** — `Subscribe` RPC is missing from `storage_agent.proto`; must be added in S05A before Subscribe can be implemented.
6. **BatchGetDocuments proto gap** — Not declared in `storage_agent.proto`; DESIGN wave must decide implementation strategy.

---

## Wave: DISCUSS / [REF] WS Strategy

**B — Brownfield incremental**: mTLS skeleton exists (step 09-01, step 09-02). Walking skeleton = GetDocument (S01A) proves full wiring chain. Subsequent slices extend the same wiring with additional RPCs.

---

## Wave: DISCUSS / [REF] Driving Ports

- `embyr.agent.v1.StorageAgent` gRPC service (mTLS listener on `:9191`)
- `Subscribe` RPC (proto extension required in S05A)

---

## Wave: DISCUSS / [REF] Pre-requisites

- `embyr-rs` Slice S13 (AgentAdapter on SaaS side) must be in progress or complete before integration tests can run end-to-end
- `embyr_proto::agent` crate with all required message types
- Postgres test instance (testcontainers or Docker Compose in CI)

---

## Wave: DISCUSS / [REF] Definition of Done (9-item checklist)

| Item | Status |
|------|--------|
| 1. Problem statement clear, domain language | PASS — all stories start from user pain |
| 2. User/persona with specific characteristics | PASS — P4 Riley, P1 Alex; real names + company |
| 3. 3+ domain examples with real data | PASS — all stories have 3 examples with real names, doc paths, field values |
| 4. UAT scenarios in Given/When/Then (3–7) | PASS — all stories have 3–5 scenarios |
| 5. AC derived from UAT | PASS — each AC maps to a scenario |
| 6. Right-sized (1–3 days, 3–7 scenarios) | PASS — each slice ≤1 day |
| 7. Technical notes: constraints/dependencies | PASS — System Constraints section above |
| 8. Dependencies resolved or tracked | PASS — each slice lists dependencies |
| 9. Outcome KPIs defined with measurable targets | PASS — each story has outcome KPI section |

---

## Wave: DISTILL

### Wave: DISTILL / [REF] Scenario List

| Scenario | Tags | Story |
|----------|------|-------|
| Agent returns document fields to an authenticated caller | @walking_skeleton @driving_port @us_a01 @real_io @skip | US-A01 |
| Agent signals document not found for absent path | @driving_port @us_a01 @real_io @error @skip | US-A01 |
| Agent rejects request with empty document path | @driving_port @us_a01 @real_io @error @skip | US-A01 |
| Creating a new document stores it with generation one | @driving_port @us_a02 @real_io @skip | US-A02 |
| Updating a document with a field mask preserves unmentioned fields | @driving_port @us_a02 @real_io @skip | US-A02 |
| Removing an absent document succeeds without error | @driving_port @us_a02 @real_io @skip | US-A02 |
| Removing a document leaves a deletion record | @driving_port @us_a02 @real_io @skip | US-A02 |
| Increment transform on absent field treats starting value as zero | @driving_port @us_a02 @real_io @skip | US-A02 |
| Creating a document without specifying an identifier generates one | @driving_port @us_a02 @real_io @skip | US-A02 |
| Concurrent write attempt on same generation is rejected | @driving_port @us_a02 @real_io @error @skip | US-A02 |
| Document generation advances by exactly one on every successful write | @driving_port @us_a02 @real_io @property @skip | US-A02 |
| Filtered query returns only matching documents | @driving_port @us_a03 @real_io @skip | US-A03 |
| Collection group query traverses nested collections | @driving_port @us_a03 @real_io @skip | US-A03 |
| Count aggregation returns the correct total | @driving_port @us_a03 @real_io @skip | US-A03 |
| Query with a malformed field path is rejected before any data is read | @driving_port @us_a03 @real_io @error @skip | US-A03 |
| Listing documents returns results in pages of up to one hundred | @driving_port @us_a03 @real_io @skip | US-A03 |
| Query excluding a field value omits documents missing that field | @driving_port @us_a03 @real_io @skip | US-A03 |
| Streaming query response indicates completion at the end | @driving_port @us_a03 @real_io @skip | US-A03 |
| Transaction with no concurrent competition commits successfully | @driving_port @us_a04 @real_io @skip | US-A04 |
| Concurrent transaction on same generation is rejected with a conflict | @driving_port @us_a04 @real_io @error @skip | US-A04 |
| Transaction reading a document that was later deleted is aborted on commit | @driving_port @us_a04 @real_io @error @skip | US-A04 |
| Committing an expired transaction returns not-found | @driving_port @us_a04 @real_io @error @skip | US-A04 |
| Rolling back a transaction discards writes without applying them | @driving_port @us_a04 @real_io @skip | US-A04 |
| Expired transactions are removed by the sweep operation | @driving_port @us_a04 @real_io @skip | US-A04 |
| Change notification arrives within two seconds of a committed write | @driving_port @us_a05 @real_io @skip | US-A05 |
| Overflow of pending change events triggers a reset notification | @driving_port @us_a05 @real_io @error @skip | US-A05 |
| Subscription stream restores delivery after a connection interruption | @driving_port @us_a05 @real_io @error @skip | US-A05 |
| Change event carries complete document contents (no truncation) | @driving_port @us_a05 @real_io @property @skip | US-A05 |
| Change event for an upsert carries a generation of at least one | @driving_port @us_a05 @real_io @skip | US-A05 |
| Change event for a deleted document carries the delete kind | @driving_port @us_a05 @real_io @skip | US-A05 |
| Agent logs storage readiness before accepting caller connections | @driving_port @us_a06 @real_io @skip | US-A06 |
| Agent exits without binding a port when storage is unreachable | @driving_port @us_a06 @real_io @error @skip | US-A06 |
| Agent completes in-flight work before exiting on shutdown signal | @driving_port @us_a06 @real_io @skip | US-A06 |
| Storage credential never appears in agent logs | @driving_port @us_a06 @real_io @skip | US-A06 |
| Agent exits immediately when required storage configuration is absent | @driving_port @us_a06 @real_io @error @skip | US-A06 |
| Agent exits immediately when required project identifier is absent | @driving_port @us_a06 @real_io @error @skip | US-A06 |

| Updating a document that does not exist returns not-found | @driving_port @us_a02 @real_io @error @skip | US-A02 |
| Creating a document that already exists is rejected | @driving_port @us_a02 @real_io @error @skip | US-A02 |
| Query over an empty collection returns no documents and a completion signal | @driving_port @us_a03 @real_io @error @skip | US-A03 |
| Rolling back a transaction that has already been committed returns not-found | @driving_port @us_a04 @real_io @error @skip | US-A04 |

**Total scenarios**: 40
**Error/edge scenario count**: 16 (40.0% — meets 40% threshold)
**Walking skeleton scenarios**: 1 (@walking_skeleton @us_a01)

---

### Wave: DISTILL / [REF] WS Strategy

**B — Brownfield incremental**: walking skeleton = GetDocument (S01A), via in-process tonic mTLS test client against a real Postgres container. mTLS skeleton already exists from steps 09-01 and 09-02; the WS proves SQL execution through the new embyr-agent binary. Subsequent slices extend the same wiring with additional RPCs. The WS answers "can Riley call GetDocument through the agent and receive the correct document?" — demo-able to Riley as stakeholder.

---

### Wave: DISTILL / [REF] Adapter Coverage

| Adapter | @real-io scenario | Covered by |
|---------|-------------------|---------|
| StorageAgent gRPC driving port (mTLS :9191) | "Agent returns document fields to an authenticated caller" | @walking_skeleton @us_a01 |
| Customer Postgres via embyr-pg-storage (documents, transactions, tombstones) | All @real_io scenarios US-A01 through US-A05 | @us_a01 through @us_a05 |
| Postgres NOTIFY/LISTEN (change notification) | "Change notification arrives within 2 seconds of a committed write" | @us_a05 @real_io |
| Agent process lifecycle (subprocess) | "Agent logs storage readiness before accepting connections", "SIGTERM drains in-flight RPCs", "Storage credential never appears in agent logs" | @us_a06 @real_io |

---

### Wave: DISTILL / [REF] Scaffolds

| File | SCAFFOLD marker | Purpose |
|------|-----------------|---------|
| `tests/acceptance/embyr_agent/mod.rs` | `// SCAFFOLD: true` | Shared fixtures: `start_test_agent`, `test_tls_config`, `start_test_postgres`, `AgentHandle` |
| `tests/acceptance/embyr_agent/us_a01_get_document.rs` | `// SCAFFOLD: true` | US-A01 test functions — 3 scenarios, all `#[ignore]` |
| `tests/acceptance/embyr_agent/us_a02_write_operations.rs` | `// SCAFFOLD: true` | US-A02 test functions — 8 scenarios, all `#[ignore]` |
| `tests/acceptance/embyr_agent/us_a03_query_operations.rs` | `// SCAFFOLD: true` | US-A03 test functions — 7 scenarios, all `#[ignore]` |
| `tests/acceptance/embyr_agent/us_a04_transactions.rs` | `// SCAFFOLD: true` | US-A04 test functions — 6 scenarios, all `#[ignore]` |
| `tests/acceptance/embyr_agent/us_a05_subscribe.rs` | `// SCAFFOLD: true` | US-A05 test functions — 6 scenarios, all `#[ignore]` |
| `tests/acceptance/embyr_agent/us_a06_lifecycle.rs` | `// SCAFFOLD: true` | US-A06 test functions — 6 scenarios, all `#[ignore]` |
| `tests/features/agent/us_a01_get_document.feature` | n/a (Gherkin) | Business-language scenarios for US-A01 |
| `tests/features/agent/us_a02_write_operations.feature` | n/a (Gherkin) | Business-language scenarios for US-A02 |
| `tests/features/agent/us_a03_query_operations.feature` | n/a (Gherkin) | Business-language scenarios for US-A03 |
| `tests/features/agent/us_a04_transactions.feature` | n/a (Gherkin) | Business-language scenarios for US-A04 |
| `tests/features/agent/us_a05_subscribe.feature` | n/a (Gherkin) | Business-language scenarios for US-A05 |
| `tests/features/agent/us_a06_lifecycle.feature` | n/a (Gherkin) | Business-language scenarios for US-A06 |

All Rust test functions end with `panic!("Not yet implemented — RED scaffold")` and carry `#[ignore]` — they compile to `MISSING_FUNCTIONALITY` RED (not BROKEN) when the ignore is removed.

---

### Wave: DISTILL / [REF] Test Placement

`tests/acceptance/embyr_agent/` and `tests/features/agent/` — follows the established embyr-rs pattern: `tests/acceptance/` for per-story Rust test files, `tests/features/` for `.feature` files. A sub-directory `embyr_agent/` (and `agent/`) groups the new slices to avoid colliding with the existing 14 embyr-rs acceptance files.

---

### Wave: DISTILL / [REF] Driving Adapter Coverage

| Driving entry point | Mechanism | Scenario |
|--------------------|-----------|---------|
| StorageAgent.GetDocument | tonic mTLS in-process client (`Channel::from_shared` + test certs from `rcgen`) | @walking_skeleton @us_a01 |
| StorageAgent.CreateDocument | tonic mTLS in-process client | @us_a02 |
| StorageAgent.UpdateDocument | tonic mTLS in-process client | @us_a02 |
| StorageAgent.DeleteDocument | tonic mTLS in-process client | @us_a02 |
| StorageAgent.RunQuery | tonic mTLS in-process client | @us_a03 |
| StorageAgent.RunAggregationQuery | tonic mTLS in-process client (post proto extension) | @us_a03 |
| StorageAgent.ListDocuments | tonic mTLS in-process client (post proto extension) | @us_a03 |
| StorageAgent.BeginTransaction | tonic mTLS in-process client | @us_a04 |
| StorageAgent.Commit | tonic mTLS in-process client | @us_a04 |
| StorageAgent.Rollback | tonic mTLS in-process client | @us_a04 |
| StorageAgent.Subscribe (server-streaming) | tonic mTLS streaming client (post proto extension) | @us_a05 |
| agent binary process | `std::process::Command` subprocess; stdout/stderr captured | @us_a06 |

---

### Wave: DISTILL / [REF] Pre-requisites

- `embyr-pg-storage` crate created (new 6th crate per ADR-A03) and included in the `embyr-agent` binary
- `storage_agent.proto` extended with `Subscribe`, `Ping`, `ListDocuments`, `RunAggregationQuery` RPCs (ADR-A01)
- `EMBYR_AGENT_PROJECT_ID` env var added to `AgentConfig` (ADR-A04)
- Optional env vars wired: `EMBYR_AGENT_MAX_CONNS` (default 25), `EMBYR_AGENT_LOG_LEVEL` (default "info"), `EMBYR_AGENT_SHUTDOWN_TIMEOUT_SECS` (default 30)
- `testcontainers-rs` available in `Cargo.toml` (already present from embyr-rs acceptance tests)
- `rcgen` available in `Cargo.toml` (already present from existing mTLS tests in steps 09-01/09-02)
- Walking skeleton (US-A01) must be green before subsequent slices are unskipped
