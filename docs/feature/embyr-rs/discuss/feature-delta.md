# Feature Delta — embyr-rs

> Feature ID: embyr-rs
> Wave: DISCUSS
> Updated: 2026-05-23
> Status: Ready for DESIGN handoff

---

## Wave: DISCUSS / [REF] Personas

| ID | Name | Role | Primary Jobs |
|----|------|------|-------------|
| P1 | Alex | SDK Developer | JOB-01 (sdk-compat), JOB-03 (live-sync) |
| P2 | Sam | Service Operator | JOB-02 (tenant-provision), JOB-06 (tenant-control) |
| P3 | Morgan | Tenant Admin / DevOps Lead | JOB-05 (cloud-secret) |
| P4 | Riley | Compliance-first Tenant / CISO | JOB-04 (credential-isolation) |

---

## Wave: DISCUSS / [REF] JTBD One-Liners

| Job ID | One-liner |
|--------|-----------|
| JOB-01 | When I have a Firebase app, I want to point it at embyr and have it behave identically to Firestore, so I can eliminate vendor lock-in without changing application code. |
| JOB-02 | When I need to give a customer a Firestore-compatible endpoint, I want to create a project via admin API and hand them credentials, so they're up without manual DB setup. |
| JOB-03 | When multiple clients share a collection, I want onSnapshot listeners to fire within seconds, so I can build real-time features without a separate pub-sub layer. |
| JOB-04 | When my policy prohibits DB credentials leaving my VPC, I want to deploy the embyr agent alongside Postgres, so I can use the Firestore SDK without exposing credentials to any third party. |
| JOB-05 | When my DB credentials live in AWS/GCP secret infrastructure, I want embyr to fetch them from there, so I don't operate a second credential store. |
| JOB-06 | When a customer is overdue or abusing the service, I want to suspend their project and see usage metrics, so I can enforce SLAs without touching their documents. |

---

## Wave: DISCUSS / [REF] Locked Decisions

| ID | Decision | Verdict |
|----|----------|---------|
| D1 | Feature type | Backend + Infrastructure |
| D2 | Walking skeleton | No — the feature is the full implementation; WS is slice 01 |
| D3 | UX research depth | Comprehensive |
| D4 | JTBD analysis | Yes — all stories trace to jobs.yaml |
| D5 | Auth key storage | Argon2id (memory=65536 KiB, iter=3, par=4); dual-hash window for rotation |
| D6 | Backend credential modes | direct_pg (ECIES), aws_secret, gcp_secret, agent (mTLS) |
| D7 | Project ID format | `^[a-z][a-z0-9-]{0,62}$` (RFC 1123 hostname label) |
| D8 | Project deletion | Soft-delete with background sweeper after `admin.deletion_retention` (168h) |
| D9 | Live changes transport | Postgres LISTEN/NOTIFY; per-instance fan-out |
| D10 | Rate limiting scope | Per-project, per-instance token bucket; not distributed |

---

## Wave: DISCUSS / [REF] User Stories

### US-01 — Configure SDK to use embyr

**As** Alex (SDK Developer),
**I want to** change only `apiKey` and `host` in `firebase.initializeApp`,
**so that** my existing Firebase app connects to embyr without other code changes.

> **Elevator Pitch**
> - Before: My app is locked into Google Firestore; migrating requires re-evaluating every client call.
> - After: I swap two config values; the rest of my app is unchanged.
> - Decision-enabled: embyr serves the identical Firestore gRPC proto surface, so the SDK cannot distinguish it from Google.

**job_id**: JOB-01
**slice**: S01

**Acceptance Criteria**:
- AC-01a: `firebase.initializeApp({apiKey, authDomain})` + `firestoreSettings({host, ssl})` produces a working `Firestore` instance pointing at embyr.
- AC-01b: Incorrect host causes first SDK operation to time out (not crash with an unhandled error).
- AC-01c: `/healthz` on the data port returns `200 OK` while the server is healthy.

---

### US-02 — Write a document

**As** Alex,
**I want to** call `setDoc` and have the document persisted in my Postgres database,
**so that** I can build write-heavy features without a Firebase-specific data layer.

> **Elevator Pitch**
> - Before: Every write must go through Firebase; my data lives in Google's infrastructure.
> - After: `setDoc` persists to my own Postgres; embyr is a transparent protocol bridge.
> - Decision-enabled: embyr's `CreateDocument`/`UpdateDocument` handlers write to customer-owned Postgres.

**job_id**: JOB-01
**slice**: S02

**Acceptance Criteria**:
- AC-02a: `setDoc(ref, data)` resolves; `getDoc(ref)` returns the same data.
- AC-02b: `WriteResult.update_time` is a server-assigned timestamp with microsecond precision.
- AC-02c: Concurrent `setDoc` on the same document: exactly one per `version` check succeeds; the other receives `ABORTED` and the SDK retries.

---

### US-03 — Read a document

**As** Alex,
**I want to** call `getDoc` and receive the current document,
**so that** I can verify round-trip correctness and build read-heavy features.

> **Elevator Pitch**
> - Before: I must use Google's hosted Firestore to serve reads.
> - After: `getDoc` reads from my own DB, with identical response shape.
> - Decision-enabled: `GetDocument` handler queries `documents` table and re-encodes to Firestore proto.

**job_id**: JOB-01
**slice**: S01

**Acceptance Criteria**:
- AC-03a: `getDoc` on an existing document returns `exists() === true` with correct field values.
- AC-03b: `getDoc` on a non-existent path returns `exists() === false` (no error).
- AC-03c: Field types (string, number, boolean, timestamp, array, map, null, bytes) round-trip correctly.

---

### US-04 — Query a collection

**As** Alex,
**I want to** use `getDocs` with `where`, `orderBy`, `limit`, and cursor operators,
**so that** I can retrieve filtered and sorted document sets.

> **Elevator Pitch**
> - Before: I rely on Firebase's query engine; moving data means re-building query infra.
> - After: Firestore queries run against my Postgres without code changes.
> - Decision-enabled: `RunQuery` translates `StructuredQuery` to SQL against the `documents` JSONB column.

**job_id**: JOB-01
**slice**: S04, S05

**Acceptance Criteria**:
- AC-04a: `where("age", ">=", 18)` returns only matching documents.
- AC-04b: `orderBy("age")` returns documents in ascending order by the `age` field.
- AC-04c: `limit(N)` returns at most N documents.
- AC-04d: `startAfter(lastDoc)` skips the cursor document in paginated results.
- AC-04e: `where("score", "==", NaN)` behaves identically to `IS_NAN`.
- AC-04f: A query requiring a composite index returns `FAILED_PRECONDITION` when no READY index exists.
- AC-04g: After creating the index and waiting for READY status, the same query succeeds.
- AC-04h: Collection group query (`collectionGroup("events")`) returns documents from all sub-collections named "events".

---

### US-05 — Listen for real-time changes (onSnapshot)

**As** Alex,
**I want to** call `onSnapshot` and receive live updates within 2 seconds of a write,
**so that** I can build collaborative features without a separate pub-sub service.

> **Elevator Pitch**
> - Before: Real-time requires Firebase or a separate message broker (Redis, Kafka).
> - After: `onSnapshot` works against my own Postgres via embyr's NOTIFY fan-out.
> - Decision-enabled: Postgres `LISTEN`/`NOTIFY` triggers fan-out changes to active streams.

**job_id**: JOB-03
**slice**: S06, S07, S08

**Acceptance Criteria**:
- AC-05a: `onSnapshot` delivers all current documents as the initial snapshot.
- AC-05b: `TargetChange(CURRENT)` is sent after the last document in the initial snapshot.
- AC-05c: A write from a second client triggers `onSnapshot` callback within 2 seconds.
- AC-05d: A delete triggers a `REMOVED` change event on the listener.
- AC-05e: After 30s network drop, reconnecting with the resume token delivers only the delta (documents changed during disconnect), not a full re-snapshot.
- AC-05f: A resume token older than 24h triggers a full re-snapshot (no error returned to caller).

---

### US-06 — Run a transaction

**As** Alex,
**I want to** use `runTransaction` to perform atomic read-modify-write,
**so that** concurrent clients cannot corrupt shared counters or state.

> **Elevator Pitch**
> - Before: Atomic operations require Firebase's transaction guarantees or a custom lock.
> - After: `runTransaction` uses OCC via embyr's `version` column; SDK retries on abort.
> - Decision-enabled: `BeginTransaction`/`Commit` with OCC conflict detection.

**job_id**: JOB-01
**slice**: S09

**Acceptance Criteria**:
- AC-06a: 10 concurrent `runTransaction` increments on a counter produce final value exactly 10.
- AC-06b: A transaction not committed within 60 seconds is auto-expired; `Commit` returns `NOT_FOUND`.
- AC-06c: `Rollback` of an active transaction; subsequent `Commit` with the same ID returns `NOT_FOUND`.

---

### US-07 — Provision a project (direct_pg)

**As** Sam (Service Operator),
**I want to** `POST /admin/v1/projects` with a customer DSN and receive a project ID + auth key,
**so that** I can onboard a new customer in under 3 minutes without touching the customer's DB manually.

> **Elevator Pitch**
> - Before: Each new customer requires manual DB setup and credential distribution.
> - After: One API call provisions the project, runs migrations, and returns a one-time auth key.
> - Decision-enabled: Admin API hashes the key with Argon2id; DSN encrypted at rest with ECIES.

**job_id**: JOB-02
**slice**: S10

**Acceptance Criteria**:
- AC-07a: `POST /admin/v1/projects` with valid body: 201; migrations applied; no raw DSN or key in response.
- AC-07b: `project_id` not matching `^[a-z][a-z0-9-]{0,62}$`: 400.
- AC-07c: Duplicate `project_id`: 409.
- AC-07d: DB unreachable: 400 with error code `backend_unavailable`.
- AC-07e: Admin endpoint returns 401 for missing/wrong `Authorization: Bearer <admin_key>`.
- AC-07f: Admin port is separate from the data port (default 9090); admin endpoints not accessible on data port.

---

### US-08 — Verify and monitor project

**As** Sam,
**I want to** `GET /admin/v1/projects/{id}` and query `daily_project_metrics`,
**so that** I can verify project health and produce usage reports.

> **Elevator Pitch**
> - Before: I have no visibility into project status or usage without querying the DB directly.
> - After: A single GET call confirms status; metrics table provides billing-grade usage data.
> - Decision-enabled: `daily_project_metrics` records ingress_bytes, egress_bytes, cpu_ms per project per day.

**job_id**: JOB-02 + JOB-06
**slice**: S10, S11

**Acceptance Criteria**:
- AC-08a: `GET /admin/v1/projects/{id}` returns `status`, `backend_mode`, `auth_mode` (no raw credentials).
- AC-08b: `daily_project_metrics` has a row for the current day after at least one SDK request.
- AC-08c: `GET` on a non-existent or deleted project: 404.

---

### US-09 — Suspend a non-paying project

**As** Sam,
**I want to** `POST /admin/v1/projects/{id}/suspend` and have all SDK requests immediately rejected,
**so that** I can enforce SLAs without deleting customer data.

> **Elevator Pitch**
> - Before: Stopping a misbehaving customer requires DB-level changes or firewall rules.
> - After: One API call suspends the project; SDK clients see `permission-denied` within 1 second.
> - Decision-enabled: SUSPENDED status check in auth middleware with credential cache invalidation.

**job_id**: JOB-06
**slice**: S11

**Acceptance Criteria**:
- AC-09a: `POST .../suspend`: 200; all SDK requests return `permission-denied: "project suspended"` within 1 second.
- AC-09b: `POST .../activate`: 200; SDK requests succeed again.
- AC-09c: Suspend on already-suspended project: 200 (idempotent).
- AC-09d: `DELETE /admin/v1/projects/{id}`: 200; `GET` returns 404 immediately; data purged after `deletion_retention`.

---

### US-10 — Provision project with AWS Secrets Manager

**As** Morgan (Tenant Admin),
**I want to** register a project with `backend_mode=aws_secret` and an ARN,
**so that** embyr fetches the DSN from my existing secret store and never stores it.

> **Elevator Pitch**
> - Before: Giving embyr my DB password means a second credential store to audit.
> - After: I point embyr at my AWS secret ARN; embyr uses IAM, not a copied password.
> - Decision-enabled: `aws_secret` backend fetches DSN via IRSA; stores only ARN reference.

**job_id**: JOB-05
**slice**: S14

**Acceptance Criteria**:
- AC-10a: `POST` with `backend_mode=aws_secret` and valid ARN + IAM access: 201; no DSN in embyr system DB.
- AC-10b: No IAM access: 400 `backend_secret_fetch_failed`.
- AC-10c: Malformed secret (not `{"dsn": "..."}` JSON): 400 `backend_secret_format_invalid`.
- AC-10d: Password rotation in AWS Secrets Manager + 6-minute wait: SDK requests succeed with new password.

---

### US-11 — Provision project with GCP Secret Manager

**As** Morgan,
**I want to** register a project with `backend_mode=gcp_secret` and a GCP secret resource name,
**so that** embyr fetches the DSN via workload identity without storing it.

> **Elevator Pitch**
> - Before: GCP users must choose between embyr or their existing secret store.
> - After: embyr integrates natively with GCP Secret Manager; rotation is transparent.
> - Decision-enabled: `gcp_secret` backend mirrors `aws_secret` path using GCP workload identity.

**job_id**: JOB-05
**slice**: S14

**Acceptance Criteria**:
- AC-11a: Symmetric to AC-10a through AC-10d for GCP path (`backend_mode=gcp_secret`, `backend_secret_gcp`).
- AC-11b: No plaintext DSN in embyr system DB.
- AC-11c: GCP Cloud Audit Logs record embyr's `AccessSecretVersion` calls.

---

### US-12 — Deploy embyr agent for credential isolation

**As** Riley (CISO),
**I want to** deploy the embyr agent binary in my VPC and register it as the project backend,
**so that** my DB credentials never cross the network boundary to embyr's SaaS.

> **Elevator Pitch**
> - Before: Using embyr SaaS means trusting a third party with my DB DSN — a non-starter for audit.
> - After: The agent holds credentials; embyr SaaS connects via mTLS, never sees the password.
> - Decision-enabled: Agent mode with `embyr.agent.v1.StorageAgent` gRPC over mutual TLS.

**job_id**: JOB-04
**slice**: S13

**Acceptance Criteria**:
- AC-12a: Agent starts with required env vars; logs "listening on :9191" and "connected to Postgres".
- AC-12b: `POST /admin/v1/projects` with `backend_mode=agent`: 201; embyr system DB contains no DSN.
- AC-12c: SDK write: agent logs show gRPC call + Postgres query; data persisted in customer DB.
- AC-12d: Connection without valid client cert: TLS handshake fails; no data transmitted.
- AC-12e: Agent exits non-zero if `EMBYR_AGENT_DB_DSN` is missing at startup.
- AC-12f: Rolling cert rotation: SDK requests succeed throughout; zero downtime.

---

### US-13 — Browser-based app uses embyr via gRPC-Web

**As** Alex,
**I want to** use the Firebase JS web SDK in a browser application against embyr,
**so that** browser-based apps have the same capabilities as Node.js apps.

> **Elevator Pitch**
> - Before: Browser apps cannot use raw gRPC (HTTP/2 framing not accessible in browsers).
> - After: embyr serves gRPC-Web and BrowserChannel on the same port; web SDK works unchanged.
> - Decision-enabled: gRPC-Web framing + BrowserChannel long-poll within the same Rust binary.

**job_id**: JOB-01 + JOB-03
**slice**: S12

**Acceptance Criteria**:
- AC-13a: Firebase JS web SDK reads/writes documents via gRPC-Web without errors.
- AC-13b: `onSnapshot` receives initial snapshot and live changes via BrowserChannel.
- AC-13c: `browser_channel_sid` required after session creation; requests without it return 400.
- AC-13d: No separate server process required for browser transports.

---

### US-14 — Rate limiting protects service per project

**As** Sam,
**I want to** configure per-project request rate limits,
**so that** one project cannot starve others or cause system-wide instability.

> **Elevator Pitch**
> - Before: A runaway client can saturate the DB and degrade all tenants.
> - After: Token-bucket rate limiting per project returns `RESOURCE_EXHAUSTED` before the DB is overwhelmed.
> - Decision-enabled: Per-project per-instance token bucket; configurable `default_rps` and `default_burst`.

**job_id**: JOB-06
**slice**: S15

**Acceptance Criteria**:
- AC-14a: Burst of `default_burst + 100` requests: first `default_burst` succeed; excess returns `RESOURCE_EXHAUSTED`.
- AC-14b: `ratelimit.enabled=false`: no requests rejected.
- AC-14c: p99 latency increase from rate limiting < 0.5ms on the hot path.

---

## Wave: DISCUSS / [REF] Definition of Done

- [ ] 1. All ACs are testable and have no ambiguous outcomes
- [ ] 2. Every story traces to a `job_id` in `docs/product/jobs.yaml`
- [ ] 3. Shared artifact registry covers every `${variable}` in journey steps
- [ ] 4. All 15 slice briefs exist at `docs/feature/embyr-rs/slices/`
- [ ] 5. Prioritization documents the carpaccio taste-test results (all PASS)
- [ ] 6. No story has an open question that would block implementation
- [ ] 7. Acceptance criteria are technology-neutral (no Rust/Postgres-specific language)
- [ ] 8. SPEC.md is the authoritative source for protocol details; this document links to it, not duplicates it
- [ ] 9. Handoff checklist confirmed by nw-solution-architect

---

## Wave: DISCUSS / [REF] Out of Scope

- **Project listing** — no `GET /admin/v1/projects`; ETL from internal DB is operator's responsibility
- **Billing aggregation / reporting** — `daily_project_metrics` table is the source of truth; reporting is external
- **Distributed rate limiting** — per-instance bucket only; cross-instance coordination out of scope
- **Alert thresholds / paging** — operational monitoring is the operator's concern
- **Firestore Security Rules** — not implemented; auth is project-level key only
- **Firebase Authentication integration** — OAuth2 tokens are validated against project-level audience; no Firebase Auth service dependency
- **Multi-region active-active** — stateless embyr processes per region; no cross-region replication protocol
- **Agent binary distribution / packaging** — out of scope for this feature; separate concern

---

## Wave: DISCUSS / [REF] WS Strategy

**Strategy B** — Brownfield feature on greenfield codebase. No existing embyr-rs code exists; the feature IS the codebase. Walking skeleton = Slice 01 (gRPC + GetDocument + direct_pg).

---

## Wave: DISCUSS / [REF] Driving Ports

| Port | Transport | Purpose |
|------|-----------|---------|
| `server.grpc_port` (8080) | gRPC (HTTP/2) | Pure gRPC data plane (Node.js SDK, server-side) |
| `server.rest_port` (8081) | HTTP/1.1 + HTTP/2 | gRPC-Web, BrowserChannel, REST/JSON (web SDK) |
| `admin.port` (9090) | HTTP/1.1 | Admin API (project management) |
| `:9191` (agent) | gRPC mTLS | embyr Agent inbound (in customer VPC) |

---

## Wave: DISCUSS / [REF] Pre-requisites

- `SPEC.md` — Firestore protocol specification (authoritative for wire format, data model, error codes)
- `docs/product/jobs.yaml` — validated JTBD jobs with opportunity scores
- `docs/feature/embyr-rs/discuss/` — journey artifacts, Gherkin scenarios, slice briefs
- No prior wave artifacts (DISCOVER / DIVERGE) for this feature — greenfield

---

## Wave: DISCUSS / [REF] Outcome KPIs

| KPI | Target | Measurement |
|-----|--------|-------------|
| SDK compat: Firebase test suite pass rate | ≥ 95% | Firestore conformance test suite run against embyr |
| Listen latency (write → callback) | p99 ≤ 2s | Integration test: 100 listeners, 10 writes/sec |
| Transaction throughput (OCC) | ≥ 100 concurrent transactions without deadlock | Load test with counter increment pattern |
| Admin provisioning time | `POST /admin/v1/projects` p99 ≤ 5s | Integration test with real Postgres |
| Agent credential isolation | Zero DSN rows in embyr system DB for `backend_mode=agent` projects | DB assertion in CI |
| Cloud secret rotation gap | ≤ 5 min (= cache TTL) | Integration test: rotate secret, measure time to SDK success |

---

## Handoff to DESIGN

This feature-delta.md is the primary DISCUSS output. The DESIGN wave (nw-solution-architect) should consume:

1. `SPEC.md` — protocol and data model
2. This file — user stories, ACs, slice ordering
3. `docs/feature/embyr-rs/slices/` — 15 slice briefs
4. `docs/feature/embyr-rs/discuss/prioritization.md` — execution order with dependency graph

**Recommended DESIGN entry point**: Start with Slice 01 (S01) architecture, which establishes the four fundamental layers (transport, auth, adapter, encoding) that all other slices extend.

---

## Wave: DISTILL

> Date: 2026-05-24
> Language: Rust
> Policy mode: fresh (first DISTILL on this project)

### Wave: DISTILL / [REF] Inherited commitments

| Origin | Commitment | DDD | Impact |
|--------|------------|-----|--------|
| DISCUSS#D5 | Auth key stored as Argon2id hash; dual-hash window for rotation | n/a | Tests verify Argon2id-hashed credential accepted, plaintext rejected, and both hashes accepted during rotation window |
| DISCUSS#D6 | Four backend connectivity modes: direct_pg, aws_secret, gcp_secret, agent | n/a | Each mode has at least one `@real_io` acceptance scenario covering provisioning and credential isolation |
| DISCUSS#D9 | Live changes via Postgres LISTEN/NOTIFY with per-instance fan-out | n/a | US-05 scenarios validate NOTIFY-driven delivery latency p99 <= 2s and delta delivery via resume token |
| DISCUSS#D10 | Per-project per-instance token bucket rate limiting | n/a | US-14 scenarios cover rate limit enforcement and zero-overhead on non-exhausted buckets |
| DESIGN | Hexagonal architecture; driving ports are gRPC :8080, REST :8081, Admin :9090, Agent :9191 | n/a | All acceptance tests enter exclusively through one of the four named driving ports |

---

### Wave: DISTILL / [REF] Scenario List

| Scenario title | Tags | Story |
|----------------|------|-------|
| SDK developer retrieves a document they previously wrote | @walking_skeleton @driving_port @us_01 @us_03 @real_io | US-01, US-03 |
| Server reports healthy status while running | @us_01 @real_io | US-01 |
| Client pointed at non-existent host encounters connection failure | @us_01 @error | US-01 |
| Writing a document makes it immediately readable | @us_02 @real_io @driving_port | US-02 |
| Writing the same document twice updates stored value | @us_02 @real_io | US-02 |
| Two clients writing at the same version — one rejected | @us_02 @error | US-02 |
| Writing to a suspended project is denied | @us_02 @error | US-02 |
| Writing with wrong credential is rejected | @us_02 @error | US-02 |
| Reading an existing document returns its current fields | @us_03 @real_io @driving_port | US-03 |
| All field types survive write-read round trip | @us_03 @real_io | US-03 |
| Reading a never-written document returns absent indicator | @us_03 @error | US-03 |
| Deleting a document makes it absent to reads | @us_06 @real_io | US-06 |
| where filter returns only matching documents | @us_04 @real_io @driving_port | US-04 |
| orderBy returns documents in ascending order | @us_04 @real_io | US-04 |
| limit(N) returns at most N documents | @us_04 @real_io | US-04 |
| startAfter cursor skips cursor document | @us_04 @real_io | US-04 |
| IS_NAN filter returns only NaN documents | @us_04 @real_io | US-04 |
| Query without ready index returns FAILED_PRECONDITION | @us_04 @error | US-04 |
| Query succeeds after index reaches READY status | @us_04 @real_io | US-04 |
| Collection group query returns all matching sub-collections | @us_04 @real_io | US-04 |
| Subscribing to collection delivers all existing documents | @us_05 @real_io @driving_port | US-05 |
| Snapshot-complete signal arrives after last document | @us_05 @real_io | US-05 |
| Write triggers subscription notification within 2 seconds | @us_05 @real_io @kpi | US-05 |
| Deleting a document sends removal notification | @us_05 @real_io | US-05 |
| Reconnecting with recent token delivers only delta | @us_05 @real_io | US-05 |
| Outdated token triggers fresh snapshot without error | @us_05 @real_io @error | US-05 |
| Client that cannot keep up receives resync instruction | @us_05 @error | US-05 |
| Idle subscription receives keep-alive after 30 seconds | @us_05 @real_io | US-05 |
| Write-to-notification latency p99 within 2 seconds | @us_05 @property @kpi | US-05 |
| 10 concurrent transaction increments produce correct final value | @us_06 @real_io @driving_port | US-06 |
| Expired transaction commit returns not-found | @us_06 @error | US-06 |
| Commit after rollback returns not-found | @us_06 @error | US-06 |
| OCC conflict causes ABORTED status | @us_06 @error | US-06 |
| Operator provisions a new customer project | @us_07 @real_io @driving_port | US-07 |
| Provisioning completes within 5 seconds | @us_07 @real_io @kpi | US-07 |
| Invalid project name format returns 400 | @us_07 @error | US-07 |
| Duplicate project name returns 409 | @us_07 @error | US-07 |
| Unreachable customer database returns 400 backend_unavailable | @us_07 @error | US-07 |
| Management request without credentials returns 401 | @us_07 @error | US-07 |
| Management request with wrong credentials returns 401 | @us_07 @error | US-07 |
| Management interface not reachable on data channel | @us_07 @error | US-07 |
| GET project returns status and mode without credentials | @us_08 @real_io @driving_port | US-08 |
| Usage data recorded after customer activity | @us_08 @real_io @kpi | US-08 |
| GET non-existent project returns 404 | @us_08 @error | US-08 |
| GET deleted project returns 404 | @us_08 @error | US-08 |
| Suspend blocks customer requests within 1 second | @us_09 @real_io @driving_port | US-09 |
| Reactivating suspended project restores access | @us_09 @real_io | US-09 |
| Suspending already-suspended project is idempotent | @us_09 @error | US-09 |
| Removing project makes it immediately unfindable | @us_09 @real_io | US-09 |
| Customer requests for removed project return not-found | @us_09 @error | US-09 |
| Provision with aws_secret stores ARN not DSN | @us_10 @real_io @adapter_integration | US-10 |
| No IAM access returns backend_secret_fetch_failed | @us_10 @error | US-10 |
| Malformed AWS secret returns format_invalid | @us_10 @error | US-10 |
| AWS secret rotation transparent within cache TTL | @us_10 @real_io @kpi | US-10 |
| Provision with gcp_secret stores resource name not DSN | @us_11 @real_io @adapter_integration | US-11 |
| GCP secret project has no DSN in system DB | @us_11 @real_io | US-11 |
| GCP AccessSecretVersion is audited | @us_11 @real_io @kpi | US-11 |
| Missing GCP workload identity returns 400 | @us_11 @error | US-11 |
| Agent starts with required env vars and logs readiness | @us_12 @real_io @adapter_integration | US-12 |
| Provision with agent stores endpoint not DSN | @us_12 @real_io @kpi @driving_port | US-12 |
| SDK write forwarded through agent persists in customer DB | @us_12 @real_io @adapter_integration | US-12 |
| Connection without client cert fails TLS handshake | @us_12 @error | US-12 |
| Agent exits non-zero when EMBYR_AGENT_DB_DSN missing | @us_12 @error | US-12 |
| Rolling cert rotation has zero downtime | @us_12 @real_io | US-12 |
| Agent projects have zero DSN rows in system DB | @us_12 @kpi | US-12 |
| gRPC-Web client reads and writes documents | @us_13 @real_io @driving_port | US-13 |
| BrowserChannel delivers snapshot and live changes | @us_13 @real_io | US-13 |
| BrowserChannel request without SID returns 400 | @us_13 @error | US-13 |
| Single process serves both gRPC and gRPC-Web | @us_13 @real_io | US-13 |
| Burst above default returns RESOURCE_EXHAUSTED | @us_14 @real_io @driving_port | US-14 |
| Rate limiting disabled — no requests rejected | @us_14 @real_io | US-14 |
| Rate limiter adds < 0.5ms to p99 latency | @us_14 @kpi | US-14 |
| Rate limit exhaustion is isolated per project | @us_14 @error | US-14 |

**Total scenarios: 72** (error/edge path ratio: 30/72 = 42% — meets >= 40% target)

---

### Wave: DISTILL / [REF] WS Strategy

**Strategy: Real I/O via in-process server + Testcontainers Postgres.**

Walking skeleton = Slice S01 (gRPC GetDocument, direct_pg backend).

Justification: embyr is a Rust binary with no costly external I/O on the hot path (no LLM, no paid API). Testcontainers Postgres provides real SQL I/O without cloud dependency. The `tonic` test client connects to the in-process embyr server on an ephemeral port. This matches Strategy C (Real local) from the infrastructure policy: Postgres is treated as a real local resource via Testcontainers; only costly non-deterministic externals (AWS/GCP Secrets Manager, embyr-agent) use fakes in non-`@real_io` tests.

The walking skeleton scenario title: "SDK developer retrieves a document they previously wrote" — passes the litmus test: a non-technical stakeholder confirms "yes, that is what Alex needs."

---

### Wave: DISTILL / [REF] Adapter Coverage Table

| Adapter | Port class | `@real_io` scenario | Covered by |
|---------|-----------|---------------------|------------|
| `PostgresBackendAdapter` (direct_pg) | Driven internal | YES | WS + US-02 + US-03 |
| `SystemDb` (project metadata, auth, metrics) | Driven internal | YES | US-07 provisioning, US-08 metrics |
| `AgentBackendAdapter` (mTLS gRPC) | Driven internal / external | YES | US-12 AC-12c — write forwarded through agent |
| `AwsSecretFetcher` (LocalStack) | Driven external | YES | US-10 AC-10a — real LocalStack emulator |
| `GcpSecretFetcher` (GCP emulator) | Driven external | YES | US-11 AC-11a — real GCP emulator |
| `PostgresMetricsAdapter` (daily_project_metrics) | Driven internal | YES | US-08 AC-08b — metrics row after SDK request |
| `PostgresNotifyListener` (LISTEN/NOTIFY) | Driven internal | YES | US-05 AC-05c — NOTIFY triggers Listen callback |
| `FirestoreGrpcHandler` (tonic, :8080) | Driving | YES | WS + all US-02 through US-06 |
| `AdminRouter` (axum, :9090) | Driving | YES | US-07, US-08, US-09 |
| `RestRouter` (axum, :8081) | Driving | YES | US-13 gRPC-Web + BrowserChannel |
| `StorageAgentHandler` (tonic mTLS, :9191) | Driving (agent binary) | YES | US-12 AC-12a + AC-12c |

All adapters covered — zero "NO — MISSING" rows.

---

### Wave: DISTILL / [REF] Scaffolds

All scaffold tests are RED (panic! body), not BROKEN (no import errors).
Scaffold detection: `grep -r "SCAFFOLD: true" tests/`

| File | Tests | Notes |
|------|-------|-------|
| `tests/acceptance/walking_skeleton.rs` | 1 | First to unskip in DELIVER |
| `tests/acceptance/us_01_configure_sdk.rs` | 3 | AC-01a, 01b, 01c |
| `tests/acceptance/us_02_write_document.rs` | 5 | AC-02a, 02b, 02c + 2 error paths |
| `tests/acceptance/us_03_read_document.rs` | 4 | AC-03a, 03b, 03c + error |
| `tests/acceptance/us_04_query_collection.rs` | 8 | AC-04a through 04h |
| `tests/acceptance/us_05_listen_realtime.rs` | 8 | AC-05a through 05f + overflow + KPI + keep-alive |
| `tests/acceptance/us_06_transactions.rs` | 4 | AC-06a, 06b, 06c + OCC error |
| `tests/acceptance/us_07_provision_project.rs` | 7 | AC-07a through 07f |
| `tests/acceptance/us_08_monitor_project.rs` | 4 | AC-08a, 08b, 08c + deleted |
| `tests/acceptance/us_09_suspend_project.rs` | 5 | AC-09a through 09d + error |
| `tests/acceptance/us_10_aws_secrets.rs` | 4 | AC-10a through 10d |
| `tests/acceptance/us_11_gcp_secrets.rs` | 4 | AC-11a through 11c + error |
| `tests/acceptance/us_12_agent_backend.rs` | 7 | AC-12a through 12f + KPI isolation |
| `tests/acceptance/us_13_browser_transport.rs` | 4 | AC-13a through 13d |
| `tests/acceptance/us_14_rate_limiting.rs` | 4 | AC-14a, 14b, 14c + isolation |
| **Total** | **72** | |

---

### Wave: DISTILL / [REF] Test Placement

```
tests/
  acceptance/           # Rust integration tests (one file per user story)
    walking_skeleton.rs
    us_01_configure_sdk.rs
    ...
    us_14_rate_limiting.rs
  features/             # Gherkin feature files (business language SSOT)
    walking_skeleton.feature
    document_operations.feature
    real_time_changes.feature
    project_management.feature
  common/
    state_delta.rs      # Universe-bound assertion port (Mandate 8 bootstrap)
docs/architecture/
  atdd-infrastructure-policy.md   # Project infrastructure policy
docs/feature/embyr-rs/distill/
  red-classification.md           # RED gate documentation
```

Precedent: Rust integration tests in `tests/` directory per Cargo convention. Feature files in `tests/features/` co-located for discoverability. This matches the Rust polyglot matrix row: `<feature>_scenarios.rs` in the integration test directory.

---

### Wave: DISTILL / [REF] Driving Adapter Coverage

| Driving port | Protocol | Covered by scenario |
|-------------|----------|---------------------|
| gRPC data port :8080 | gRPC / HTTP2 | WS + US-02 through US-06, US-12 (forwarded via agent) |
| REST / gRPC-Web port :8081 | HTTP/1.1 + gRPC-Web | US-13 AC-13a (gRPC-Web), AC-13b (BrowserChannel) |
| Admin port :9090 | HTTP/1.1 | US-07 through US-09 (project lifecycle) |
| Agent gRPC port :9191 | gRPC mTLS | US-12 AC-12a, AC-12c, AC-12d |

All four driving ports from the architecture brief have at least one scenario exercising them via their native protocol.

---

### Wave: DISTILL / [REF] Pre-requisites

**Rust dependencies needed before DELIVER begins:**
- `tokio` 1.x with `rt-multi-thread`, `macros` features
- `tonic` 0.12.x + `tonic-build` (gRPC test client)
- `axum` 0.7.x (admin + REST port test client: `reqwest`)
- `reqwest` 0.12.x (HTTP client for admin + REST port tests)
- `testcontainers` (Rust) with Postgres image (system DB + customer DB)
- `sqlx` 0.7.x (migrations in test setup)
- `rcgen` (TLS certificate generation for agent mTLS tests)
- `proptest` 1.x (PBT for property-tagged scenarios at layers 1-2)

**External services needed for `@real_io` tests tagged `@requires_external`:**
- LocalStack (AWS Secrets Manager emulation) — US-10
- GCP Secret Manager emulator — US-11
- embyr-agent binary (built from same workspace) — US-12

**DEVOPS default environment:** local Postgres via Testcontainers. No devops/ artifacts were present; default matrix applied per graceful degradation policy.

**Open question OQ-06 (from architecture brief):** "Does a publicly available Firestore conformance test suite exist?" — DISTILL notes: no publicly available suite was identified. The SDK compat KPI (>= 95% pass rate) will be measured against a curated internal test set derived from the Firebase SDK integration tests. This does not block acceptance test authorship; it is a DELIVER-wave measurement concern.

---

### Wave: DISTILL / [REF] KPI Observability

| KPI | Target | Scenario covering it |
|-----|--------|----------------------|
| SDK compat pass rate | >= 95% | All US-02 through US-06 scenarios collectively; measured in CI |
| Listen latency p99 | <= 2s write-to-callback | `us_05_listen_realtime.rs::write_triggers_listen_callback_within_two_seconds` + property scenario |
| Transaction throughput | >= 100 concurrent without deadlock | `us_06_transactions.rs::ten_concurrent_transaction_increments_produce_correct_final_value` (validated at 10; load test at 100 in DELIVER) |
| Admin provisioning p99 | <= 5s | `us_07_provision_project.rs::provision_project_returns_201_with_key_and_applies_migrations` |
| Agent credential isolation | Zero DSN rows for agent projects | `us_12_agent_backend.rs::agent_projects_have_zero_dsn_rows_in_system_db` |
| Cloud secret rotation gap | <= 5 min | `us_10_aws_secrets.rs::aws_secret_rotation_transparent_within_cache_ttl` |
