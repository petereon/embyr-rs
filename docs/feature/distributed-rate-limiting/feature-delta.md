<!-- markdownlint-disable MD024 -->
# Feature Delta — distributed-rate-limiting

**Feature ID:** `distributed-rate-limiting`
**Wave:** DISCUSS
**Date:** 2026-08-07
**Status:** Complete — ready for DESIGN handoff
**Walking Skeleton Strategy:** B — brownfield replacement of in-process limiter
**Replaces:** `crates/embyr-server/src/middleware/rate_limit.rs` (current per-instance TokenBucket)
**Must keep green:** `tests/acceptance/us_14_rate_limiting.rs` (4 existing tests, AC-14a through AC-14c + isolation test)

---

## Wave: DISCUSS / [REF] Persona

**Primary — P2: Sam Chen (Service Operator / Platform Engineer)**
`job_id: JOB-11` — fair-multitenancy

Embyr SaaS operator running a multi-node deployment. Sam provisions projects, quotes tenants on SLA limits, and owns the deployment topology. Sam does not write SDK code — Sam writes infrastructure configuration and observes system behavior through metrics and logs.

**Secondary — P1: Alex Reyes (SDK Developer / App Developer)**
`job_id: JOB-11`

Builds applications against the Firestore SDK pointed at embyr. Alex experiences rate limiting through gRPC status codes and response headers. Alex implements retry logic in client code and needs machine-readable signals to do so correctly.

---

## Wave: DISCUSS / [REF] JTBD

### JOB-06 coverage check

JOB-06 (`tenant-control`) covers: suspension via `PermissionDenied`, usage visibility via `daily_project_metrics`. It does NOT address fairness when multiple instances share the same tenant pool. Specifically:

- JOB-06: "so I can enforce SLAs **without touching their documents**" — focused on administrative control, not traffic fairness
- JOB-06 has no forcing constraint around horizontal scaling multiplying effective rate limits
- The fairness-under-scale angle is genuinely unrepresented

**Conclusion: JOB-11 (fair-multitenancy) is new and required.** Added to `docs/product/jobs.yaml`.

### Jobs consumed in this feature

| Job | Coverage |
|-----|----------|
| JOB-11 (fair-multitenancy) | All 4 user stories — distributed enforcement, response headers, graceful fallback, provisioning integration |
| JOB-02 (tenant-provision) | US-DRL-04 secondary — `provision_project` inserts `rate_buckets` row in same transaction |

No other existing jobs are affected.

---

## Wave: DISCUSS / [REF] Scope Assessment

**Scope Assessment: PASS** — 4 stories, 1 bounded context (BC-1 Tenant Management via system DB), estimated 4–5 days total across 5 slices. No independent user outcomes that could ship separately — all 4 stories are part of one coherent behavior change (distributed rate enforcement). No slice requires its own feature directory.

### Architecture constraint being lifted

`docs/product/architecture/brief.md` currently states:
> "Per-project rate limiting is per-instance token bucket. No distributed rate-limiting coordination."

This feature lifts that constraint. The system DB (`rate_buckets` table) becomes the coordination point. No new coordination plane is added — Postgres is already the system DB. The architectural quality attribute "Operational simplicity" (rank 6) is preserved.

---

## Wave: DISCUSS / [REF] Locked Architecture Decisions

All 10 decisions are pre-resolved and locked. DESIGN wave must not re-open them without a new ADR.

| ID | Decision | Impact |
|----|----------|--------|
| D1 | Coordination backend: Postgres system DB; migration `0018_rate_buckets.sql` | No Redis, no external store — system DB already present |
| D2 | Algorithm: atomic `UPDATE rate_buckets SET tokens = LEAST($capacity, tokens + EPOCH_DIFF * $refill_rate) - 1, last_refill = now() WHERE project_id = $1 AND tokens + EPOCH_DIFF * $refill_rate >= 1 RETURNING tokens` | Single round-trip; no SELECT + UPDATE race; row lock per project_id (no cross-project contention) |
| D3 | Failure mode: fail open with per-instance in-process fallback capped at 1× configured limit; 20ms Postgres timeout hard-coded; not configurable | Availability over strict enforcement during degradation; 1× not 2× cap |
| D4 | Configuration: `EMBYR_RATE_LIMIT_RPS` env var (default 1000); replaces hardcoded `1000.0`; operator-wide, no per-project knob | Single control point; simplicity over per-tenant granularity (future ADR if needed) |
| D5 | Response headers: full `x-ratelimit-limit`, `x-ratelimit-remaining`, `x-ratelimit-reset` on ALL gRPC responses; `retry-after-ms` on rejections | On all responses (not just errors) — enables SDK-side adaptive throttling |
| D6 | Header attachment: inline at each of the 9 gRPC handler call sites (not tower middleware) | `project_id` lives in proto body, not metadata; middleware cannot extract it before auth |
| D7 | `RateLimiter::check()` return type: `Result<RateLimitInfo, RateLimitInfo>` where `RateLimitInfo { remaining: f64, limit: f64, reset_ms: u64 }` | Both arms carry rate-limit info — headers attached regardless of allow/reject outcome |
| D8 | New project row: `provision_project` inserts `rate_buckets` row (full bucket, `tokens = capacity`); FK `ON DELETE CASCADE` | Atomic with project INSERT; no cold-start burst window on new tenants |
| D9 | Scope: gRPC `:8080` + gRPC-Web/BrowserChannel `:8081` share the same `Arc<RateLimiter>` already — both covered; Admin `:9090` excluded | Admin port is operator-only, low-volume; excluding prevents self-DoS during provisioning bursts |
| D10 | `rate_buckets` schema: `(project_id VARCHAR(63) PK FK → projects ON DELETE CASCADE, tokens DOUBLE PRECISION, last_refill TIMESTAMPTZ)` | `DOUBLE PRECISION` matches existing `TokenBucket.tokens: f64`; no schema type conversion needed |

---

## Wave: DISCUSS / [REF] User Stories

### System Constraints (cross-cutting)

- All 4 stories share the same `Arc<RateLimiter>` instance injected into `FirestoreService`
- `RateLimiter` must remain `Send + Sync` — no interior mutability beyond `Mutex` or atomic ops
- The existing `RateLimiter::disabled()` constructor must continue to work (used by test harness)
- `EMBYR_RATE_LIMIT_RPS` is read once at startup; changing it requires restart
- Admin port `:9090` is NOT rate-limited (D9)
- US-14 existing 4 acceptance tests must remain green after refactor (AC-14a, AC-14b, AC-14c, isolation test)
- The atomic UPDATE must not hold a table-level lock — row-level lock on `project_id` PK only

---

## US-DRL-01: Distributed rate enforcement across cluster nodes

### Elevator Pitch

**Before:** Sam runs 3 embyr nodes behind a load balancer. `project-acme` is configured for 1000 RPS. Each node enforces its own in-process bucket independently — `project-acme` can fire 3000 RPS cluster-wide before any single node rejects a request. Sam cannot offer tenants a meaningful rate-limit SLA.

**After:** Call `GetDocument` on node-B when the shared `rate_buckets` row for `project-acme` has 0 tokens → gRPC trailing metadata: `x-ratelimit-remaining: 0`, status `RESOURCE_EXHAUSTED`. The cluster enforces 1000 RPS regardless of node count. Sam quotes `project-acme` a 1000 RPS limit and it holds.

**Decision enabled:** Sam decides how many embyr nodes to deploy for capacity without worrying that horizontal scaling multiplies each tenant's effective rate limit. Scaling 2× nodes no longer means 2× rate limit per project.

### Problem

Sam Chen is a service operator who runs a multi-node embyr deployment for multiple customer projects. Sam finds it impossible to enforce meaningful per-project rate limits because the current per-instance `HashMap<String, TokenBucket>` means each node enforces limits independently — a project hitting all 3 nodes can send 3× the configured limit before any rejection occurs.

### Who

- Sam Chen (P2) | Service operator, 3-node embyr cluster | Needs cluster-wide enforcement so tenants cannot starve each other under horizontal scale

### Solution

Replace the in-process-only `check()` path with an atomic Postgres `UPDATE rate_buckets ... RETURNING tokens` (D2). Both the allow and reject branches return `RateLimitInfo` so headers can be attached at all 9 call sites. The per-instance `TokenBucket` is retained as a fallback only (D3, see US-DRL-03).

`job_id: JOB-11`

### Domain Examples

#### Example 1: Happy path — request allowed within distributed limit
Sam's cluster has 3 nodes. `project-acme` is configured at 1000 RPS (EMBYR_RATE_LIMIT_RPS=1000). 600 requests arrive at node-A and 300 at node-B in the same second. The `rate_buckets` row for `project-acme` starts with 1000 tokens. The 900th aggregate request is allowed (OK or NOT_FOUND). Request #901 on any node returns RESOURCE_EXHAUSTED.

#### Example 2: Per-project isolation preserved
`project-alpha` exhausts its 1000-token distributed bucket. Simultaneously, `project-beta` has 800 tokens remaining in its own row. Sam verifies that `project-beta` requests continue to succeed — the two projects have independent `rate_buckets` rows and the PK lock is per-`project_id`.

#### Example 3: Existing per-instance tests continue passing
Sam runs `cargo test --test us_14_rate_limiting`. The 4 existing tests (AC-14a burst, AC-14b disabled, AC-14c latency, isolation) all pass. The new distributed logic does not break the single-node token semantics observed by those tests.

### UAT Scenarios

#### Scenario: Aggregate cluster limit is respected when multiple nodes share a bucket

Given Sam has a 3-node embyr cluster all connected to the same system DB
And `project-acme` has a `rate_buckets` row with `tokens=1000`
And `EMBYR_RATE_LIMIT_RPS=1000` is set on all 3 nodes
When 1500 gRPC `GetDocument` requests reach the cluster spread across all nodes
Then the first 1000 requests return OK or NOT_FOUND (allowed)
And the remaining 500 requests return gRPC status RESOURCE_EXHAUSTED (code 8)

#### Scenario: Project-level isolation is preserved under distributed limiting

Given `project-alpha` and `project-beta` share the same 3-node cluster
And `project-alpha`'s `rate_buckets` row has 0 tokens (exhausted)
When Sam sends 10 `GetDocument` requests to `project-beta`
Then all 10 `project-beta` requests return OK or NOT_FOUND
And none return RESOURCE_EXHAUSTED

#### Scenario: Existing US-14 per-instance tests remain green

Given the distributed `RateLimiter` is wired in place of the old in-process one
When the `us_14_rate_limiting` test suite runs
Then all 4 existing tests pass (burst, disabled, latency p99, per-project isolation)

### Acceptance Criteria

- [ ] When cluster-wide requests for a project exceed `EMBYR_RATE_LIMIT_RPS` in 1 second, additional requests return gRPC status RESOURCE_EXHAUSTED (code 8)
- [ ] Per-project isolation: project-alpha exhausting its bucket does not cause project-beta to receive RESOURCE_EXHAUSTED
- [ ] US-14 existing 4 acceptance tests remain green after the refactor
- [ ] The atomic UPDATE uses a row-level lock on `project_id` PK, not a table-level lock
- [ ] `RateLimiter::check()` return type is `Result<RateLimitInfo, RateLimitInfo>` (D7)

### Outcome KPIs

- **Who:** Sam Chen (service operator)
- **Does what:** observes that no single project exceeds EMBYR_RATE_LIMIT_RPS cluster-wide, verified by load test
- **By how much:** 0 projects exceed the configured limit under a 3-node load test (100% enforcement accuracy)
- **Measured by:** load test at 3× configured RPS across 3 simulated nodes; count of allowed vs. RESOURCE_EXHAUSTED responses
- **Baseline:** current behavior allows N × 1000 RPS cluster-wide (N = node count; no cluster enforcement)

### Technical Notes

- Depends on: Slice 01 (migration `0018_rate_buckets.sql` must exist before this can run)
- Depends on: Slice 01 (`EMBYR_RATE_LIMIT_RPS` env var must be wired before this runs)
- `SystemDb` pool is already on `FirestoreService` — no new field required
- The `EPOCH_DIFF` expression in the atomic UPDATE is `EXTRACT(EPOCH FROM (now() - last_refill))`
- The UPDATE returns 0 rows if the token check fails — that is the rejection signal
- `start_test_server_with_rate_limit` in lib.rs must be updated to wire the new `RateLimiter` signature

---

## US-DRL-02: Rate-limit response headers for SDK adaptive backoff

### Elevator Pitch

**Before:** When Alex's `GetDocument` call is rate-limited, the gRPC response is a bare `RESOURCE_EXHAUSTED` status with no metadata about the current limit, remaining capacity, or when to retry. Alex hard-codes a 1-second fixed delay in the SDK retry wrapper — all rate-limited clients retry simultaneously 1 second later, causing a thundering herd.

**After:** Call `GetDocument` → gRPC trailing metadata: `x-ratelimit-limit: 1000`, `x-ratelimit-remaining: 0`, `x-ratelimit-reset: 1754000000000`, `retry-after-ms: 247`. Alex reads `retry-after-ms: 247` and waits exactly 247ms before the next attempt. Retry waves are staggered naturally because each client's `retry-after-ms` reflects its actual position in the refill queue.

**Decision enabled:** Alex decides whether to implement adaptive backoff in the SDK wrapper (using `retry-after-ms`) versus keeping the fixed-delay retry. The headers make the decision data-driven. Alex can also build a real-time usage dashboard by polling `x-ratelimit-remaining`.

### Problem

Alex Reyes is an SDK developer who builds multi-user applications against embyr. Alex finds it frustrating that rate-limited gRPC responses provide no machine-readable signal for when to retry, forcing naïve fixed-delay retry strategies that create thundering herds and make rate-limiting feel unreliable rather than helpful.

### Who

- Alex Reyes (P1) | SDK developer, Node.js Firebase SDK pointed at embyr | Needs retry-after signal to implement smart backoff without guessing

### Solution

Attach `x-ratelimit-limit`, `x-ratelimit-remaining`, `x-ratelimit-reset` to all gRPC response trailing metadata at all 9 call sites (D5, D6). Add `retry-after-ms` to RESOURCE_EXHAUSTED responses only. Values are derived from the `RateLimitInfo` returned by `RateLimiter::check()` (D7).

`job_id: JOB-11`

### Domain Examples

#### Example 1: Happy path — headers present on successful GetDocument
Alex calls `GetDocument` on `project-startup`. The response trailing metadata includes `x-ratelimit-limit: 1000`, `x-ratelimit-remaining: 743`, `x-ratelimit-reset: 1754000123456`. Alex's SDK wrapper logs current usage for observability.

#### Example 2: Retry signal on rate-limited RunQuery
Alex's `RunQuery` on `project-startup` returns RESOURCE_EXHAUSTED. Trailing metadata includes `retry-after-ms: 157`. Alex's exponential-backoff wrapper overrides the backoff with `max(backoff, 157ms)` and retries once. The retry succeeds because the bucket refilled in 157ms.

#### Example 3: Headers consistent across all 9 gRPC methods
Alex tests `BatchGetDocuments`, `Listen`, and `CommitTransaction` in sequence. All three responses include `x-ratelimit-limit`, `x-ratelimit-remaining`, and `x-ratelimit-reset` in trailing metadata. The headers are not limited to `GetDocument`.

### UAT Scenarios

#### Scenario: Rate-limit headers present on every allowed gRPC response

Given `project-startup` has 743 tokens remaining in its distributed bucket
When Alex calls `GetDocument` on `project-startup`
Then the gRPC response trailing metadata includes `x-ratelimit-limit: 1000`
And `x-ratelimit-remaining: 743`
And `x-ratelimit-reset` is set to a future epoch millisecond value

#### Scenario: retry-after-ms present on rate-limited response

Given `project-startup`'s distributed bucket has 0 tokens
When Alex calls `GetDocument` on `project-startup`
Then the gRPC response status is RESOURCE_EXHAUSTED (code 8)
And trailing metadata includes `retry-after-ms` with a value > 0
And `x-ratelimit-remaining: 0`

#### Scenario: Headers present on all 9 gRPC methods, not just GetDocument

Given `project-startup` is within its rate limit
When Alex calls `RunQuery` on `project-startup`
Then `RunQuery` trailing metadata includes `x-ratelimit-limit`, `x-ratelimit-remaining`, `x-ratelimit-reset`

#### Scenario: x-ratelimit-limit reflects current EMBYR_RATE_LIMIT_RPS

Given Sam has set `EMBYR_RATE_LIMIT_RPS=500` and restarted embyr
When Alex calls any gRPC method on `project-startup`
Then `x-ratelimit-limit` in trailing metadata is `500`

### Acceptance Criteria

- [ ] All 9 gRPC handler call sites attach `x-ratelimit-limit`, `x-ratelimit-remaining`, `x-ratelimit-reset` as trailing metadata
- [ ] RESOURCE_EXHAUSTED responses additionally attach `retry-after-ms` as trailing metadata
- [ ] `x-ratelimit-remaining` is never negative (floor at 0)
- [ ] `x-ratelimit-limit` matches the current `EMBYR_RATE_LIMIT_RPS` configuration at the time of the request
- [ ] `retry-after-ms` represents milliseconds until at least 1 token will be available at the current refill rate

### Outcome KPIs

- **Who:** Alex Reyes (SDK developer)
- **Does what:** reads `retry-after-ms` from RESOURCE_EXHAUSTED responses and uses it to govern retry timing
- **By how much:** 100% of RESOURCE_EXHAUSTED responses include `retry-after-ms` (0% bare rejections)
- **Measured by:** integration test assertion on trailing metadata presence; header key count in response
- **Baseline:** 0% of current responses include any rate-limit metadata (bare status code only)

### Technical Notes

- Depends on: US-DRL-01 (return type `Result<RateLimitInfo, RateLimitInfo>` must exist first)
- `attach_headers(info: &RateLimitInfo, resp: &mut impl MetadataMapMut)` helper function — not middleware
- `retry-after-ms` computation: `ceil((1.0 - info.remaining) / refill_rate * 1000.0)` where `remaining` is from RETURNING tokens
- tonic trailing metadata is attached via `Response::metadata_mut()` or equivalent extension point
- gRPC-Web responses on `:8081` also carry trailing metadata (tonic-web translates them); verify in integration test

---

## US-DRL-03: Graceful per-instance fallback when system DB is unavailable

### Elevator Pitch

**Before:** When the system DB is under load and the `rate_buckets UPDATE` query takes longer than a few hundred milliseconds, the gRPC request hangs waiting for Postgres. There is no timeout, no fallback, and no way for Sam to know what the limiter is doing. An unrelated DB incident causes rate-limit-induced request hangs that look like application slowness.

**After:** The `rate_buckets UPDATE` has a hard 20ms timeout. When it fires on node-B, the per-instance `TokenBucket` for `project-acme` takes over — capped at exactly 1× `EMBYR_RATE_LIMIT_RPS`. Sam observes `rate_limit_pg_timeout_total` incrementing in metrics. `GetDocument` completes within its normal latency budget. When Postgres recovers, distributed limiting resumes automatically.

**Decision enabled:** Sam decides whether the "fail open with per-instance cap" posture is acceptable for their availability SLA, or whether a stricter "fail closed" posture is required. The current design chooses availability (fail open) with a documented 1× cap as the documented degradation boundary. Sam can escalate to a new ADR if the posture must change.

### Problem

Sam Chen is a service operator who runs embyr in production with a shared Postgres system DB. Sam finds it worrying that a transient Postgres slowdown (long-running migration, maintenance window, network hiccup) could cause the rate-limiting check to block indefinitely, making embyr appear slow or unresponsive when the underlying cause is a rate-limiting infrastructure problem unrelated to application data.

### Who

- Sam Chen (P2) | Service operator, production deployment | Needs rate limiting to degrade gracefully rather than hanging under DB stress

### Solution

Wrap the `rate_buckets UPDATE` in a `tokio::time::timeout(Duration::from_millis(20), ...)`. On timeout: fall back to the per-instance `TokenBucket` at 1× `EMBYR_RATE_LIMIT_RPS`. Increment `rate_limit_pg_timeout_total` counter. On Postgres recovery: next request will succeed within 20ms and resume distributed coordination automatically.

`job_id: JOB-11`

### Domain Examples

#### Example 1: Postgres maintenance window — requests continue
Sam schedules a `VACUUM FULL` on the system DB, which causes the `rate_buckets` UPDATE to block for 5 seconds. The 20ms timeout fires on every request during those 5 seconds. The per-instance `TokenBucket` caps `project-acme` at 1000 RPS on node-A. Sam's monitoring shows `rate_limit_pg_timeout_total` spiking. Requests complete normally. When `VACUUM FULL` finishes, the next `UPDATE` succeeds within 20ms and distributed limiting resumes.

#### Example 2: Per-instance fallback enforces 1× not 2×
During the fallback period on node-A, `project-acme` has already consumed 800 of its per-instance 1000-token bucket. Sam sends 400 more requests to node-A. The first 200 are allowed; the next 200 return RESOURCE_EXHAUSTED. The cap is 1× (1000), not 2× (2000) or uncapped.

#### Example 3: Automatic recovery — no operator action required
Postgres recovers after a 2-minute network partition. The next `rate_buckets UPDATE` for `project-acme` succeeds in 4ms. Sam does not need to restart embyr, flush caches, or take any action. The `rate_limit_pg_timeout_total` counter stops incrementing. Distributed enforcement resumes from whatever the current token count is in the row.

### UAT Scenarios

#### Scenario: Requests continue when rate_buckets UPDATE times out

Given the Postgres system DB is unavailable (simulated by injecting a 500ms delay)
And the `rate_buckets UPDATE` exceeds the 20ms hard timeout on node-A
When Sam sends `GetDocument` requests to `project-acme` on node-A
Then requests are allowed up to 1000 RPS (per-instance fallback cap)
And no RESOURCE_EXHAUSTED responses are returned due to the Postgres timeout itself

#### Scenario: Per-instance fallback enforces exactly 1x limit

Given the per-instance fallback is active on node-A (Postgres unavailable)
And node-A's in-process bucket for `project-acme` has 0 tokens remaining
When Sam sends 100 additional `GetDocument` requests to `project-acme` on node-A
Then all 100 requests return RESOURCE_EXHAUSTED
And the cap is 1× EMBYR_RATE_LIMIT_RPS (not 2×, not uncapped)

#### Scenario: Distributed limiting resumes automatically after Postgres recovery

Given the per-instance fallback was active for 30 seconds due to Postgres unavailability
When Postgres recovers and the `rate_buckets UPDATE` succeeds within 20ms
Then the next request uses the distributed bucket from Postgres (not the per-instance fallback)
And Sam does not need to restart embyr or take any manual action

### Acceptance Criteria

- [ ] When the `rate_buckets UPDATE` exceeds 20ms, it times out and the per-instance `TokenBucket` is used for that request
- [ ] The per-instance fallback cap is exactly 1× `EMBYR_RATE_LIMIT_RPS` (not 2×, not configurable, not uncapped)
- [ ] When Postgres recovers, subsequent `UPDATE` calls within 20ms resume distributed coordination automatically (no restart required)
- [ ] Each Postgres timeout increments a `rate_limit_pg_timeout_total` counter observable via metrics
- [ ] The p99 latency of `RateLimiter::check()` remains < 500µs on the non-timeout path (AC-14c property test must stay green)

### Outcome KPIs

- **Who:** Sam Chen (service operator) and end-users of Alex's applications
- **Does what:** continue receiving successful gRPC responses during system DB degradation, with no user-visible outage attributable to rate-limiting infrastructure
- **By how much:** 0% request failure rate due to Postgres rate_buckets timeout (failures must be due to per-instance cap enforcement, not DB unavailability per se)
- **Measured by:** chaos test — simulate Postgres timeout; count RESOURCE_EXHAUSTED responses; verify all are due to per-instance cap, not DB error propagation
- **Baseline:** unknown — current code has no timeout path; a slow Postgres UPDATE would block the gRPC handler indefinitely

### Technical Notes

- Depends on: US-DRL-01 (per-instance `TokenBucket` is the fallback — both must exist simultaneously)
- The per-instance fallback `TokenBucket` is a secondary in-process map retained in `RateLimiter`; it is only consulted when Postgres times out
- The 20ms timeout is hard-coded per D3 — no env var, no config field
- `rate_limit_pg_timeout_total` counter: use the existing `MetricsAdapter` pattern or a standalone `AtomicU64` if metrics integration is deferred to DESIGN wave
- The fallback `TokenBucket` must use the same `capacity` as the distributed bucket to preserve the 1× invariant

---

## US-DRL-04: Provisioning integration — new projects start with a rate bucket

### Elevator Pitch

**Before:** `POST /admin/v1/projects` creates the project row but no `rate_buckets` row. The first gRPC call on any node creates an in-process `TokenBucket` lazily with a full 1000-token starting balance. On a 5-node cluster, each node creates its own bucket independently on first touch — `project-freshstart` can fire 5000 requests in the first second before any node's bucket refills. There is a cold-start burst window of unbounded duration.

**After:** `POST /admin/v1/projects` inserts a `rate_buckets` row with `tokens=1000, last_refill=now()` in the same database transaction that creates the project. The first `GetDocument` call on any node finds the shared row populated. No cold-start burst. The FK `ON DELETE CASCADE` removes the row when the project is hard-deleted by the sweeper.

**Decision enabled:** Sam decides to onboard new tenants knowing that their rate limit takes effect from the very first request, with no grace period or burst window that must be manually closed after provisioning.

### Problem

Sam Chen is a service operator who provisions new customer projects via the admin API. Sam finds it concerning that newly provisioned projects have no rate bucket in the distributed system, meaning the cold-start behavior of each node creating its own full-capacity in-process bucket creates a burst window that violates the fairness guarantee Sam wants to offer existing tenants.

### Who

- Sam Chen (P2) | Service operator running the admin API | Needs new projects to have rate enforcement from first request, not from when the first in-process bucket is lazily created

### Solution

Update `provision_project` in the admin handler (or the system DB migration/domain function it calls) to `INSERT INTO rate_buckets (project_id, tokens, last_refill) VALUES ($1, $2, now())` inside the same transaction as the `INSERT INTO projects`. FK `ON DELETE CASCADE` on `rate_buckets.project_id → projects.id` handles cleanup automatically (D8, D10).

`job_id: JOB-11`

### Domain Examples

#### Example 1: Happy path — rate bucket created atomically with project
Sam calls `POST /admin/v1/projects` with `project_id: project-freshstart`. The handler opens a transaction, inserts the `projects` row, inserts a `rate_buckets` row with `tokens=1000, last_refill=now()`. Both inserts succeed. The first `GetDocument` request for `project-freshstart` from any node finds the `rate_buckets` row already present.

#### Example 2: FK cascade — bucket removed when project is purged
The sweeper hard-deletes `project-obsolete` 168h after soft-deletion. The FK `ON DELETE CASCADE` on `rate_buckets.project_id` removes the `rate_buckets` row automatically. Sam verifies no stale rows accumulate in `rate_buckets` after the sweep.

#### Example 3: Atomic rollback — no orphaned rows on provision failure
A bug causes the `rate_buckets INSERT` to fail (e.g., capacity value is NULL). The transaction rolls back both the `projects` row and the `rate_buckets` row. Sam sees a 500 error from `POST /admin/v1/projects`. The system DB has no inconsistent state (no project without a rate bucket, no rate bucket without a project).

### UAT Scenarios

#### Scenario: Rate bucket row exists immediately after project creation

Given Sam calls `POST /admin/v1/projects` with `project_id: project-freshstart`
And the project is created successfully (201 response)
When Sam queries `SELECT * FROM rate_buckets WHERE project_id = 'project-freshstart'`
Then 1 row is returned with `tokens = 1000` and `last_refill` within 1 second of the project creation time

#### Scenario: First gRPC call on new project uses distributed bucket, not lazy in-process creation

Given `project-freshstart` was just provisioned (rate_buckets row exists)
When Alex sends the first `GetDocument` request to `project-freshstart` on any node
Then the request is handled by the distributed atomic UPDATE path
And no in-process lazy bucket is created for this project

#### Scenario: Project hard-deletion cascades to rate_buckets row

Given `project-obsolete` has been soft-deleted and the 168h sweeper fires
When the sweeper hard-deletes the `projects` row for `project-obsolete`
Then the `rate_buckets` row for `project-obsolete` is also deleted (CASCADE)
And `SELECT COUNT(*) FROM rate_buckets WHERE project_id = 'project-obsolete'` returns 0

#### Scenario: Failed provision transaction rolls back both rows atomically

Given a fault is injected that causes the `rate_buckets INSERT` to fail
When Sam calls `POST /admin/v1/projects` with `project_id: project-broken`
Then the `projects` row is also rolled back (not inserted)
And neither `projects` nor `rate_buckets` contains a row for `project-broken`

### Acceptance Criteria

- [ ] `POST /admin/v1/projects` inserts a `rate_buckets` row with `tokens = EMBYR_RATE_LIMIT_RPS` and `last_refill = now()` in the same transaction as the `projects` row
- [ ] If the `rate_buckets INSERT` fails, the entire provision transaction rolls back (no orphaned project row)
- [ ] When a project row is hard-deleted, the corresponding `rate_buckets` row is deleted via FK CASCADE (no manual cleanup required)
- [ ] A newly provisioned project's first gRPC call is rate-limited by the distributed bucket, not an in-process lazy bucket
- [ ] The `tokens` value at provision time equals `capacity` (full bucket — new tenants start with maximum capacity, not zero)

### Outcome KPIs

- **Who:** Sam Chen (service operator)
- **Does what:** provisions new projects knowing rate enforcement applies from the first request, with no cold-start burst window
- **By how much:** 0ms gap between project creation and distributed rate enforcement being active (vs. current unbounded cold-start window per node)
- **Measured by:** integration test — provision project, immediately send N×capacity requests across simulated nodes, verify aggregate RESOURCE_EXHAUSTED behavior matches configured limit
- **Baseline:** current cold-start burst = N nodes × capacity tokens on first request to each node; for a 5-node cluster at 1000 RPS = 5000 unmetered requests

### Technical Notes

- Depends on: Slice 01 (`0018_rate_buckets.sql` migration must exist and FK must be active)
- The `provision_project` function is called by `POST /admin/v1/projects` handler; it already uses a transaction for the project INSERT
- `tokens` at provision time must equal the current `capacity` value (read from EMBYR_RATE_LIMIT_RPS at startup), not a hardcoded constant
- The sweeper hard-delete for projects (168h retention) already deletes the projects row; CASCADE handles rate_buckets automatically with no sweeper code change

---

## Wave: DISCUSS / [REF] Outcome KPIs

### Objective

Sam can run any number of embyr instances without any single tenant consuming more than their configured `EMBYR_RATE_LIMIT_RPS` across the cluster — and Alex can implement smart retry logic using machine-readable rate-limit headers.

### Outcome KPI Table

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|-----|-----------|-------------|----------|-------------|------|
| 1 | Sam Chen (service operator) | Observes that no project exceeds EMBYR_RATE_LIMIT_RPS cluster-wide under a 3-node load test | 100% enforcement accuracy (0 projects exceed configured limit) | N × 1000 RPS cluster-wide (N = node count; no cluster enforcement exists today) | Load test: 3× configured RPS across 3 simulated nodes; count allowed vs. RESOURCE_EXHAUSTED | Leading |
| 2 | Alex Reyes (SDK developer) | Reads `retry-after-ms` from RESOURCE_EXHAUSTED responses and uses it to govern retry timing | 100% of RESOURCE_EXHAUSTED responses include `retry-after-ms` (0% bare rejections) | 0% of current responses include any rate-limit metadata | Integration test assertion on trailing metadata key presence | Leading |
| 3 | Sam + end-users | Continue receiving successful gRPC responses during system DB degradation (no outage due to rate-limiter) | 0% request failure rate attributable to Postgres rate_buckets timeout (vs. per-instance cap as intended behavior) | Unknown — current code has no timeout path | Chaos test: simulate Postgres timeout; verify all RESOURCE_EXHAUSTED due to capacity cap, not DB error | Leading |
| 4 | Sam Chen | New projects have distributed rate enforcement from first request with no cold-start burst | 0ms gap between project creation and enforcement (vs. unbounded lazy initialization window) | Up to N × capacity unmetered requests on cold start across N nodes | Integration test: provision project, send N × capacity requests, verify RESOURCE_EXHAUSTED at aggregate limit | Leading |

### Metric Hierarchy

- **North Star:** cluster-wide rate enforcement accuracy — 100% of projects respect EMBYR_RATE_LIMIT_RPS regardless of node count
- **Leading Indicators:** x-ratelimit-remaining header present on all responses (KPI 2); rate_limit_pg_timeout_total counter incrementing during degradation but request failure rate stays 0% (KPI 3)
- **Guardrail Metrics:** `RateLimiter::check()` p99 latency < 500µs (AC-14c — must not regress); US-14 existing 4 tests must remain green (regression gate)

### Hypothesis

We believe that replacing the in-process per-node `TokenBucket` with a Postgres atomic UPDATE on `rate_buckets` for Sam Chen's multi-node embyr deployment will achieve cluster-wide rate enforcement at 1× configured RPS per project.

We will know this is true when Sam observes that `project-acme` cannot exceed 1000 RPS aggregate across a 3-node cluster in load testing, with the atomic UPDATE staying within the 500µs p99 latency budget.

---

## Wave: DISCUSS / [REF] Story Map

### Backbone (brownfield — no walking skeleton)

```
Provision Project → Receive gRPC Request → Check Distributed Limit → Attach Headers → Degrade Gracefully
```

### Slice map

| Slice | Stories | Focus | Est. |
|-------|---------|-------|------|
| DRL-01 — Schema + env var | US-DRL-04 (partial) | `0018_rate_buckets.sql` migration + `EMBYR_RATE_LIMIT_RPS` env var replaces hardcoded `1000.0` | 0.5 d |
| DRL-02 — Atomic Postgres UPDATE | US-DRL-01 | Distributed enforcement core: `RateLimitInfo` type, atomic UPDATE, new `check()` signature | 1 d |
| DRL-03 — Graceful fallback | US-DRL-03 | 20ms timeout, per-instance fallback bucket, `rate_limit_pg_timeout_total` counter | 1 d |
| DRL-04 — Response headers | US-DRL-02 | `attach_headers()` helper + 9 call sites updated + `retry-after-ms` on rejections | 0.5 d |
| DRL-05 — Provisioning integration | US-DRL-04 (complete) | `provision_project` inserts rate_buckets row in same transaction | 0.5 d |

### Priority rationale

1. **DRL-01 (seed)** — schema and env var must exist before any other slice can run; lowest risk, unblocks all others
2. **DRL-02** — core distributed enforcement; validates the primary hypothesis (D2 atomic UPDATE); must keep US-14 green
3. **DRL-03** — the anxiety scenario (latency impact); must be implemented before DRL-04 so the fallback path is exercised in header tests too
4. **DRL-04** — headers are observable by Alex; can be independently verified; low risk once DRL-02 is green
5. **DRL-05** — provisioning integration; depends on DRL-01 schema; independent of DRL-02/03/04; sequenced last for review clarity

---

## Wave: DISCUSS / [REF] DoR Validation

| DoR Item | US-DRL-01 | US-DRL-02 | US-DRL-03 | US-DRL-04 |
|----------|-----------|-----------|-----------|-----------|
| Problem statement clear, domain language | PASS | PASS | PASS | PASS |
| User/persona with specific characteristics | PASS — Sam Chen, 3-node cluster | PASS — Alex Reyes, SDK developer | PASS — Sam Chen, production deployment | PASS — Sam Chen, admin API operator |
| 3+ domain examples with real data | PASS — 3 examples, project-acme/alpha/beta, 3-node cluster | PASS — 3 examples, project-startup, retry-after-ms value | PASS — 3 examples, VACUUM FULL scenario, fallback cap, auto-recovery | PASS — 3 examples, project-freshstart/obsolete/broken, sweeper |
| UAT scenarios in Given/When/Then (3-7) | PASS — 3 scenarios | PASS — 4 scenarios | PASS — 3 scenarios | PASS — 4 scenarios |
| AC derived from UAT | PASS | PASS | PASS | PASS |
| Right-sized (1-3 days, 3-7 scenarios) | PASS — 1 day | PASS — 0.5 day | PASS — 1 day | PASS — 0.5 day |
| Technical notes: constraints/dependencies | PASS | PASS | PASS | PASS |
| Dependencies resolved or tracked | PASS — depends on DRL-01 (schema) | PASS — depends on DRL-02 (RateLimitInfo type) | PASS — depends on DRL-02 (per-instance fallback coexists) | PASS — depends on DRL-01 (migration) |
| Outcome KPIs defined | PASS | PASS | PASS | PASS |

### DoR Status: PASSED — all 4 stories pass all 9 items

### Anti-pattern check

| Pattern | Checked | Verdict |
|---------|---------|---------|
| Implement-X titles | US-DRL-01/02/03/04 all start from user pain | CLEAN |
| Generic data | project-acme, project-startup, project-freshstart, project-obsolete, Sam Chen, Alex Reyes | CLEAN |
| Technical AC | All AC describe observable outcomes (RESOURCE_EXHAUSTED response, header presence, fallback cap, CASCADE deletion) | CLEAN |
| Technical scenario titles | "Aggregate cluster limit is respected", "Requests continue when rate_buckets UPDATE times out" — business outcomes | CLEAN |
| Oversized stories | 3-4 UAT scenarios each, 0.5-1 day each | CLEAN |
| Abstract requirements | 3+ concrete examples with real project IDs per story | CLEAN |

---

## Wave: DISCUSS / [REF] Handoff Notes

### For solution-architect (DESIGN wave)

**Critical constraints to preserve:**
- `embyr-core` must not import `sqlx`, `tokio`, or any IO crate — `RateLimitInfo` struct belongs in `embyr-core` as a pure domain type; the Postgres UPDATE logic belongs in `embyr-server`
- The 20ms timeout is non-negotiable per D3 — do not surface as a config parameter
- `EMBYR_RATE_LIMIT_RPS` is operator-wide, not per-project — any per-project granularity requires a new ADR and is out of scope
- Admin port `:9090` is NOT rate-limited (D9) — do not add the `RateLimiter` to admin handlers

**Files to modify (identified in discovery):**
- `crates/embyr-server/src/middleware/rate_limit.rs` — replace in-place
- `crates/embyr-server/src/grpc/handler.rs` — update 9 call sites
- `crates/embyr-server/src/adapters/system_db.rs` — add rate_buckets UPDATE method (or new adapter)
- `crates/embyr-core/src/domain/` or `crates/embyr-core/src/` — add `RateLimitInfo` type
- `migrations/system/0018_rate_buckets.sql` — new file

**New test file:**
- `tests/acceptance/us_drl_01_distributed_enforcement.rs` — new distributed-specific tests
- `tests/acceptance/us_drl_02_rate_limit_headers.rs` — header presence tests
- `tests/acceptance/us_drl_03_graceful_fallback.rs` — timeout injection tests

---

## Wave: DESIGN / [REF] Component Decomposition

> Mode: Propose (autonomous analysis). All D1-D10 decisions locked. This section records new design-level decisions not pre-resolved in DISCUSS.

### File Change Table

| File | Change Type | What Changes |
|------|-------------|--------------|
| `crates/embyr-core/src/rate_limit.rs` | NEW | `RateLimitInfo { remaining: f64, limit: f64, reset_ms: u64 }` — pure domain type; zero IO imports |
| `crates/embyr-core/src/lib.rs` | MODIFY | Add `pub mod rate_limit;` |
| `crates/embyr-server/src/middleware/rate_limit.rs` | EXTEND | Add `pg_pool: Option<Arc<PgPool>>` and `timeout_counter` fields; add `check_pg()` internal method; change `check()` return type from `Result<(), ()>` to `Result<RateLimitInfo, RateLimitInfo>`; add `attach_rate_limit_headers()` free function; retain `TokenBucket` as fallback |
| `crates/embyr-server/src/grpc/handler.rs` | MODIFY | 9 call sites: match on `Result<RateLimitInfo, RateLimitInfo>`; attach trailing metadata to both success responses and rejection `Status` values |
| `crates/embyr-server/src/admin/handlers/provision.rs` | MODIFY | Wrap project INSERT in `sqlx::Transaction`; add `rate_buckets` INSERT in same transaction; `OperatorState` gains `rate_limit_capacity: f64` field |
| `crates/embyr-server/src/admin/state.rs` | MODIFY | `OperatorState` gains `rate_limit_capacity: f64` |
| `crates/embyr-server/src/lib.rs` | MODIFY | Read `EMBYR_RATE_LIMIT_RPS` from env (default `1000.0`); replace 3 hardcoded `RateLimiter::new(1000.0, 1000.0)` calls; update `RateLimiter::new()` to pass `pg_pool`; `start_test_server_with_rate_limit()` passes `None` for pg_pool (backward compatible) |
| `migrations/0018_rate_buckets.sql` | NEW | `CREATE TABLE rate_buckets (...)` with FK `ON DELETE CASCADE` |

Note: The DISCUSS handoff listed `migrations/system/0018_rate_buckets.sql`. The actual migrations directory for the system DB is `migrations/` (root), not `migrations/system/` — confirmed by inspection of existing migrations 0001–0017. The correct path is `migrations/0018_rate_buckets.sql`.

---

## Wave: DESIGN / [REF] Port Interface Decision

`RateLimiter` is NOT behind a port/trait interface. See ADR-015 for full rationale.

**Summary:** rate limiting has exactly one implementation (Postgres + in-process fallback) and no current requirement to swap it. A trait interface would add indirection with zero benefit for this feature. Test isolation is achieved via `RateLimiter::disabled()` and `RateLimiter::new(capacity, refill_rate, None)` (in-process only mode) — no test double is needed. ADR-015 documents this choice with the rejected alternative.

---

## Wave: DESIGN / [REF] `RateLimitInfo` Domain Type

**Location:** `crates/embyr-core/src/rate_limit.rs`

Pure struct — no IO, no async, no third-party dependencies beyond `std`. Derivable by both `embyr-server` (header construction) and any future `embyr-admin` metrics display.

```
RateLimitInfo {
    remaining: f64,  // tokens remaining after this request (floored at 0.0)
    limit: f64,      // configured capacity == EMBYR_RATE_LIMIT_RPS
    reset_ms: u64,   // epoch milliseconds when >= 1 token will be available
}
```

**`remaining` semantics:**
- `Ok(info)` path (allowed): `info.remaining` = tokens left after consuming 1. Always `>= 0.0`.
- `Err(info)` path (rejected): `info.remaining = 0.0` (floored). The bucket was empty.

**`reset_ms` computation:**

From the Postgres path (token count known from `RETURNING tokens`):
- Let `t = returned_tokens` (post-deduction value from RETURNING clause).
- If `t >= 1.0`: `reset_ms = now_epoch_ms()` (next token already available).
- Else: `reset_ms = now_epoch_ms() + ceil((1.0 - t) / refill_rate * 1000.0) as u64`.
- On rejection (0 rows, tokens effectively 0): `reset_ms = now_epoch_ms() + ceil(1.0 / refill_rate * 1000.0) as u64`.

From the in-process fallback path:
- Same formula using `bucket.tokens` (pre-consumption value after refill computation).

---

## Wave: DESIGN / [REF] `RateLimiter` API Contracts

### Constructor signatures

```
// Current (to be changed):
RateLimiter::new(capacity: f64, refill_rate: f64) -> Arc<Self>

// New:
RateLimiter::new(capacity: f64, refill_rate: f64, pg_pool: Option<Arc<PgPool>>) -> Arc<Self>

// Unchanged:
RateLimiter::disabled() -> Arc<Self>
```

When `pg_pool = None`: in-process fallback mode only (used by all existing test constructors — backward compatible with US-14 test suite).

When `pg_pool = Some(pool)`: attempts Postgres path first on every `check()` call.

### `check()` decision tree

```
async fn check(&self, project_id: &str) -> Result<RateLimitInfo, RateLimitInfo>
```

1. If `!self.enabled` → `Ok(RateLimitInfo { remaining: self.capacity, limit: self.capacity, reset_ms: 0 })`
2. If `self.pg_pool.is_some()`:
   a. `tokio::time::timeout(Duration::from_millis(20), self.check_pg(project_id)).await`
   b. On `Ok(result)` → return `result` (distributed path used)
   c. On `Err(_)` (timeout elapsed) → `metrics::counter!("rate_limit_pg_timeout_total").increment(1)` → fall through to step 3
   d. On `Ok(Err(pg_error))` (Postgres hard error, resolved within 20ms) → log warn → fall through to step 3
3. Acquire `self.buckets.lock()` → get or insert `TokenBucket::new(self.capacity, self.refill_rate)` for `project_id` → call `try_consume()` → return `Ok(info)` or `Err(info)` with `RateLimitInfo` computed from bucket state

Note: step 2d distinguishes timeout from hard error. Only timeout increments `rate_limit_pg_timeout_total`. Hard Postgres errors are logged at WARN and fall back silently.

### `check_pg()` internal method

```
async fn check_pg(&self, project_id: &str) -> Result<Result<RateLimitInfo, RateLimitInfo>, sqlx::Error>
```

Executes the atomic UPDATE SQL (see migration section). Returns:
- `Ok(Ok(info))` — 1 row returned; request allowed
- `Ok(Err(info))` — 0 rows returned; request rate limited
- `Err(e)` — Postgres error; caller falls back to in-process bucket

---

## Wave: DESIGN / [REF] Atomic UPDATE SQL

```sql
-- Atomically consume 1 token from the rate bucket for the given project.
-- Returns the updated token count if allowed; 0 rows if rate limited.
UPDATE rate_buckets
SET tokens = LEAST($1::float8,
                   tokens + EXTRACT(EPOCH FROM (now() - last_refill)) * $2::float8
             ) - 1.0,
    last_refill = now()
WHERE project_id = $3
  AND tokens + EXTRACT(EPOCH FROM (now() - last_refill)) * $2::float8 >= 1.0
RETURNING tokens;
```

Parameters (bound in order):
- `$1` — `capacity` (f64): EMBYR_RATE_LIMIT_RPS; caps the refilled total via LEAST
- `$2` — `refill_rate` (f64): tokens per second (== capacity for a 1-second window)
- `$3` — `project_id` (VARCHAR)

Design notes:
- `LEAST(capacity, ...)` prevents unbounded token accumulation during idle periods.
- The `WHERE ... >= 1.0` predicate atomically gates deduction — no separate SELECT or advisory lock needed.
- Row lock scope: `UPDATE WHERE project_id = $3` acquires a row-level lock on the PK; no cross-project contention.
- `RETURNING tokens` carries the post-deduction value, which is used to populate `RateLimitInfo.remaining` and compute `reset_ms`.
- **Postgres UPDATE latency budget:** Expected latency under normal conditions: < 5ms (row-level lock on a small table, indexed PK). If the UPDATE consistently exceeds 10ms, investigate: long-running transactions holding row locks, connection pool exhaustion, Postgres autovacuum interference, or disk I/O saturation. The 20ms timeout is the hard safety bound; consistent latency > 10ms indicates infrastructure degradation requiring operator intervention, not an application bug.

---

## Wave: DESIGN / [REF] gRPC Trailing Metadata Contract

tonic version confirmed: **0.12** (root `Cargo.toml` line 16).

**tonic 0.12 metadata placement:**
- `Response<T>::metadata_mut()` for **unary** handlers → trailing metadata (HTTP/2 HEADERS frame with END_STREAM, after the response body). Correct placement for gRPC custom metadata per RFC.
- `Response<BoxStream>::metadata_mut()` for **streaming** handlers → initial metadata (first HEADERS frame). Headers are visible to gRPC clients. Semantic distinction from trailing metadata is not observable by the Firebase SDK for these headers.
- `Status::metadata_mut()` for **any rejection** → trailing metadata on the error HEADERS frame. Works identically for unary and streaming handlers.

**`attach_rate_limit_headers(md: &mut MetadataMap, info: &RateLimitInfo)` — free function in `rate_limit.rs`:**

Sets on every response (success and rejection):
- `x-ratelimit-limit` → `info.limit as u64` formatted as decimal ASCII
- `x-ratelimit-remaining` → `info.remaining.max(0.0).floor() as u64` formatted as decimal ASCII
- `x-ratelimit-reset` → `info.reset_ms` formatted as decimal ASCII (epoch milliseconds)

Sets additionally on rejection only:
- `retry-after-ms` → `ceil(max(0.0, 1.0 - info.remaining) / refill_rate * 1000.0) as u64` formatted as decimal ASCII

**Success path (7 unary handlers):**
```
// After authenticate() succeeds and before handler body:
match self.rate_limiter.check(&project_id).await {
    Ok(info) => {
        // proceed; attach info to response before returning
        let mut response = Response::new(payload);
        attach_rate_limit_headers(response.metadata_mut(), &info);
        Ok(response)
    }
    Err(info) => {
        let mut status = Status::resource_exhausted("rate limit exceeded");
        attach_rate_limit_headers(status.metadata_mut(), &info);
        attach_retry_after(status.metadata_mut(), &info, self.rate_limiter.refill_rate);
        return Err(status);
    }
}
```

**Success path (`run_query`, `listen` — streaming handlers):**
Rate-limit check occurs before the stream is created. On rejection, `Err(Status)` with metadata is returned identically to unary handlers. On success, headers are attached to `Response::metadata_mut()` (initial metadata of the streaming response):
```
let mut response = Response::new(stream);
attach_rate_limit_headers(response.metadata_mut(), &info);
Ok(response)
```

**Implementation note for `listen`:** The `listen` handler currently wraps the rate-limit check before spawning the stream task. The pattern remains: check → attach to `Response::new(stream)` metadata → return. The long-lived bidirectional stream does not re-check rate limits on subsequent messages from the same stream (per D6 scope).

---

## Wave: DESIGN / [REF] EMBYR_RATE_LIMIT_RPS Environment Variable

Read once at startup in `crates/embyr-server/src/lib.rs` (or `main.rs`):

```
let capacity: f64 = std::env::var("EMBYR_RATE_LIMIT_RPS")
    .ok()
    .and_then(|s| s.parse::<f64>().ok())
    .filter(|v| v.is_finite() && *v > 0.0)
    .unwrap_or(1000.0);
let refill_rate = capacity;  // token bucket: refill rate == capacity for 1-RPS-second window
```

Replaces hardcoded `1000.0` at three call sites in `lib.rs` (lines 174, 231, 280).

`start_test_server_with_rate_limit()` (line 325) is preserved unchanged — it passes `pg_pool = None` to `RateLimiter::new()`, keeping the in-process-only behaviour required by the existing US-14 acceptance test suite.

A new test constructor `start_test_server_with_distributed_rate_limit(system_db, capacity, pg_pool)` is added for DRL acceptance tests that exercise the Postgres path. This constructor is specified here; DELIVER creates it.

---

## Wave: DESIGN / [REF] Provisioning Integration — Transaction Wrapping

**New concern (not pre-resolved in DISCUSS):** The current `provision.rs` executes each backend mode branch as separate autocommit `sqlx::query(...).execute(pool)` calls — no explicit transaction. Adding the `rate_buckets` INSERT atomically requires wrapping both INSERTs in a `sqlx::Transaction`.

**Pattern for each backend mode branch:**
```
let mut tx = state.system_db.pool().begin().await?;
// ... INSERT INTO projects (...) on &mut tx ...
// INSERT INTO rate_buckets:
sqlx::query("INSERT INTO rate_buckets (project_id, tokens, last_refill) VALUES ($1, $2, now())")
    .bind(&req.project_id)
    .bind(state.rate_limit_capacity)
    .execute(&mut *tx)
    .await?;
tx.commit().await?;
```

If `rate_buckets` INSERT fails (e.g., FK violation because projects INSERT was rolled back), the transaction rolls back entirely. No orphaned project row is possible.

**`OperatorState` change:** Add `rate_limit_capacity: f64`. Populated at composition root from the same `capacity` variable used to construct `RateLimiter`. No new env var read in the handler.

---

## Wave: DESIGN / [REF] Migration `0018_rate_buckets.sql`

```sql
CREATE TABLE rate_buckets (
    project_id  VARCHAR(63)      NOT NULL PRIMARY KEY
                                 REFERENCES projects(id) ON DELETE CASCADE,
    tokens      DOUBLE PRECISION NOT NULL,
    last_refill TIMESTAMPTZ      NOT NULL
);

-- Backfill rate_buckets for active projects that existed before this migration.
-- Uses 1000.0 as the default initial token count (matches the prior hardcoded default).
-- Operators running EMBYR_RATE_LIMIT_RPS != 1000 should update these rows:
--   UPDATE rate_buckets SET tokens = <new_capacity> WHERE true;
INSERT INTO rate_buckets (project_id, tokens, last_refill)
SELECT id, 1000.0, now()
FROM   projects
WHERE  status IN ('active', 'suspended')
ON CONFLICT DO NOTHING;
```

**Design notes:**
- `ON DELETE CASCADE` ensures project hard-deletion (sweeper after 168h) automatically removes the rate bucket row with no additional sweeper code.
- `DOUBLE PRECISION` matches the Rust `f64` type used in `TokenBucket.tokens` — no type conversion at the sqlx boundary.
- No `CHECK (tokens >= 0)` constraint: the atomic UPDATE enforces non-negativity through the `>= 1.0` WHERE predicate; a CHECK would add write overhead on every UPDATE.
- Backfill scope: `status IN ('active', 'suspended')` — `deleted` projects will be swept by the existing sweeper and need no rate bucket.
- **Backfill ambiguity resolved:** Without the backfill, a `check_pg()` call on an existing project would return 0 rows, which is indistinguishable from "rate limited." The backfill removes this ambiguity without requiring a second SELECT query on every check.

---

## Wave: DESIGN / [REF] Earned Trust — `RateLimiter` Postgres Dependency

`RateLimiter` is a concrete struct, not a driven adapter behind a port. Principle 12 still applies to its Postgres dependency.

**Startup probe:** The existing system DB startup probe (`db.Ping()` within 2s + `sqlx-migrate` schema migration run) already validates Postgres connectivity and confirms `rate_buckets` table presence before any listener opens. No additional startup probe step is required for `RateLimiter` specifically.

**Three Earned Trust layers:**

| Layer | Mechanism | What it checks |
|-------|-----------|----------------|
| Compile-time | `sqlx::query!()` macro (if used) validates SQL against the live DB at compile time; type-checks `$1::float8`, `$3 VARCHAR` parameters. | SQL correctness and parameter types. |
| Structural (pre-commit) | Existing CI `cargo check` step covers `rate_limit.rs` changes; no new hook required for this module since it is not a driven adapter struct. | Compilation succeeds. |
| Behavioral (CI gold-test) | `tests/acceptance/us_drl_03_graceful_fallback.rs` injects a 500ms Postgres delay, asserts the 20ms timeout fires, verifies fallback activates. | Timeout detection and fallback path under real substrate pressure. |

**Metric as Earned Trust signal:** `rate_limit_pg_timeout_total` provides operators real-time observability of Postgres pressure on the rate limiter path. A spiking counter during a `VACUUM FULL` is the empirical signal that the degradation mode is active.

---

## Wave: DESIGN / [REF] Metrics

Uses the existing `metrics` crate (0.22.x) already wired to the Prometheus exporter on `GET /metrics` (admin port).

New metric added by this feature:

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `rate_limit_pg_timeout_total` | Counter | none | Increments each time the 20ms Postgres timeout fires and the fallback activates. Does NOT increment on hard Postgres errors (those are logged at WARN level, no counter). |

No new metric for hard errors at this time — the WARN log is sufficient for V1. A `rate_limit_pg_error_total` counter can be added in a future ADR if observability gaps are identified in production.

---

## Wave: DESIGN / [REF] ADRs

- `docs/product/architecture/adr-015-distributed-rate-limiter-postgres.md` — primary decision for distributed token bucket + fallback design

---

## Wave: DESIGN / [REF] Brief Update

`docs/product/architecture/brief.md` updated:
- System Constraints bullet for SD-04 annotated as superseded by ADR-015.
- `### Wave: DESIGN / [REF] Rate Limiting` subsection added under Application Architecture.
- SD-04 row in Application-Level Decisions Table updated with supersession note.

---

## Wave: DESIGN / [REF] Slice Map (design-level notes added)

| Slice | Stories | Design Notes |
|-------|---------|--------------|
| DRL-01 | US-DRL-04 (partial) | Add `migrations/0018_rate_buckets.sql`; add `pub mod rate_limit;` to `embyr-core/src/lib.rs`; define `RateLimitInfo` struct; read `EMBYR_RATE_LIMIT_RPS` in `lib.rs` |
| DRL-02 | US-DRL-01 | Extend `RateLimiter`: add `pg_pool`, `timeout_counter` fields; implement `check_pg()`; change `check()` return type; wire `system_db.pool()` in composition root; add `start_test_server_with_distributed_rate_limit()` |
| DRL-03 | US-DRL-03 | Add `tokio::time::timeout(20ms, ...)` wrapper in `check()`; increment `rate_limit_pg_timeout_total` on timeout; log WARN on hard errors |
| DRL-04 | US-DRL-02 | Add `attach_rate_limit_headers()` and `attach_retry_after()` free functions; update 9 call sites in `handler.rs` |
| DRL-05 | US-DRL-04 (complete) | Wrap provision INSERTs in `sqlx::Transaction`; add `rate_buckets` INSERT; add `rate_limit_capacity: f64` to `OperatorState` and wire in composition root |

---

## Wave: DISTILL / [REF] Scenario List

Date: 2026-08-07
Reconciliation: 0 contradictions across DISCUSS / DESIGN / DEVOPS (feature-delta.md is the combined SSOT).

| Test File | Test Name | User Story | Tags | Ignore |
|-----------|-----------|------------|------|--------|
| b12_postgres_rate_limit | `distributed_rate_limit_rejects_when_bucket_exhausted` | US-12 | @walking_skeleton @driving_port @real-io | NO — walking skeleton |
| b12_postgres_rate_limit | `rate_limit_enforced_at_configured_rps` | US-12 | @driving_port @real-io | YES |
| b12_postgres_rate_limit | `rate_bucket_tokens_decrease_atomically` | US-12 | @driving_port @real-io | YES |
| b12_postgres_rate_limit | `tokens_refill_after_one_second` | US-12 | @driving_port @real-io | YES |
| b11_provision_initializes_bucket | `provision_creates_rate_bucket_row` | US-11 | @driving_port @real-io @adapter-integration | YES |
| b11_provision_initializes_bucket | `provision_rate_bucket_is_transactional` | US-11 | @driving_port @real-io @adapter-integration | YES |
| b13_fallback_on_pg_failure | `fallback_allows_requests_when_pg_unavailable` | US-13 | @driving_port @real-io @infrastructure-failure | YES |
| b13_fallback_on_pg_failure | `fallback_capped_at_configured_limit` | US-13 | @driving_port @real-io @infrastructure-failure | YES |
| b13_fallback_on_pg_failure | `fallback_resolves_within_20ms` | US-13 | @driving_port @real-io @infrastructure-failure | YES |
| b14_ratelimit_headers | `all_grpc_handlers_return_ratelimit_headers` | US-14 | @driving_port @real-io | YES |
| b14_ratelimit_headers | `ratelimit_remaining_decrements_per_request` | US-14 | @driving_port @real-io | YES |
| b14_ratelimit_headers | `rate_limited_response_includes_retry_after_ms` | US-14 | @driving_port @real-io | YES |

Total scenarios: 12. Error/edge ratio: 8/12 = 67% (>40% target). Walking skeleton: 1 (NOT ignored).

---

## Wave: DISTILL / [REF] Walking Skeleton Strategy

**Strategy B — brownfield replacement.** No new user-visible product feature; existing `us_14_rate_limiting.rs` proves the old per-instance path works.

Walking skeleton (`distributed_rate_limit_rejects_when_bucket_exhausted` in b12) answers:
> "Can a token bucket row at 0.0 tokens cause the gRPC server to return RESOURCE_EXHAUSTED via the Postgres path?"

This is the minimal observable outcome that proves the Postgres-backed path exists end-to-end:
testcontainers Postgres → `rate_buckets` table → `check_pg()` → `Status::resource_exhausted` → tonic client receives it.

Litmus test (non-technical stakeholder): "Yes, when the shared bucket is empty, the API rejects the request" — satisfies D1 (Postgres is the enforcement point) and D7 (both arms of `check()` carry `RateLimitInfo`).

---

## Wave: DISTILL / [REF] Adapter Coverage Table

| Driven Adapter | Infrastructure Policy | Mechanism |
|----------------|----------------------|-----------|
| System Postgres (rate_buckets) | `docs/architecture/atdd-infrastructure-policy.md` | testcontainers-rs `Postgres::default().with_tag("15-alpine")` |
| gRPC driving port :8080 | `docs/architecture/atdd-infrastructure-policy.md` | tonic `FirestoreClient` → ephemeral port via `start_distributed_grpc_server()` |
| Admin HTTP :9090 | `docs/architecture/atdd-infrastructure-policy.md` | reqwest::Client (b11 provisioning tests only) |
| Postgres unavailability (b13) | N/A — infrastructure failure path | `// TODO: implement pg pause` (DRL-03 DELIVER decides mechanism) |

No Tier B (state-machine PBT) warranted: journey is 3 chained scenarios but input space is not domain-rich (token count is a single f64 with deterministic depletion). Per Mandate 10, Tier B is OPTIONAL and omitted.

---

## Wave: DISTILL / [REF] Scaffolds Created

| File | Status | Classification |
|------|--------|----------------|
| `tests/distributed_rate_limiting/mod.rs` | Created | Documentation marker only |
| `tests/distributed_rate_limiting/common/mod.rs` | Created | LIVE Postgres setup; RED gRPC server (todo!) |
| `tests/distributed_rate_limiting/acceptance/b11_provision_initializes_bucket.rs` | Created | RED (todo! + `#[ignore]`) |
| `tests/distributed_rate_limiting/acceptance/b12_postgres_rate_limit.rs` | Created | Walking skeleton: RED (todo!, NOT `#[ignore]`); 3 others: RED + `#[ignore]` |
| `tests/distributed_rate_limiting/acceptance/b13_fallback_on_pg_failure.rs` | Created | RED (todo! + `#[ignore]`) |
| `tests/distributed_rate_limiting/acceptance/b14_ratelimit_headers.rs` | Created | RED (todo! + `#[ignore]`) |
| `crates/embyr-server/Cargo.toml` | Updated | 4 `[[test]]` entries added for drl_b11–drl_b14 |

`cargo check --tests` result: 0 errors, 0 DRL-specific warnings (only pre-existing warnings from other test files).

---

## Wave: DISTILL / [REF] Test Placement

Tests live at `tests/distributed_rate_limiting/` — parallel to `tests/admin_api_v2/` and `tests/acceptance/`.

Rationale:
- `tests/acceptance/` holds user-story-level tests for the original API feature set (US-01 through US-14). Adding DRL tests there would mix the brownfield feature's new infrastructure tests with the existing protocol tests.
- A dedicated `tests/distributed_rate_limiting/` directory makes the acceptance scope of each DISTILL-wave feature independently discoverable and mirrors the `admin_api_v2` precedent.
- `[[test]]` entries in `crates/embyr-server/Cargo.toml` follow the existing `drl_b<NN>_<name>` naming convention.

---

## Wave: DISTILL / [REF] Pre-requisites and Implementation Order

DELIVER must implement slices in this order (dependency graph):

1. **DRL-01** (schema + env var) — creates `migrations/0018_rate_buckets.sql` and `embyr_core::rate_limit::RateLimitInfo`. Without this, `common::DrlTestContext::new()` runs migrations that don't include `rate_buckets`, causing b12–b14 runtime panics.
2. **DRL-02** (Postgres UPDATE path) — implements `start_test_server_with_distributed_rate_limit`; unskips b12 walking skeleton. Gate: walking skeleton goes GREEN.
3. **DRL-05** (provisioning transaction) — unskips b11. Gate: both b11 tests go GREEN.
4. **DRL-03** (20ms timeout + fallback) — unskips b13. Gate: b13 tests go GREEN.
5. **DRL-04** (response headers) — unskips b14. Gate: b14 tests go GREEN.

Existing `tests/acceptance/us_14_rate_limiting.rs` must remain GREEN throughout (backward-compat: `RateLimiter::new(..., None)` preserves the existing per-instance behavior).
