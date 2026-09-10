# Feature Delta: occ-precondition-validation

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/production-readiness-audit-2026-09-08.md` — read in full. Finding #9 confirmed
verbatim: *"Client-controlled OCC precondition timestamp panics the write path —
`Precondition.update_time.{seconds,nanos}` flows unvalidated from the client into a
`.expect("valid timestamp")` on out-of-range/malformed input, dropping the RPC ungracefully instead
of a clean `invalid_argument`."* Category: Rust / Correctness. Severity: **High**. Status (before
this DISCUSS): "Not started."

✓ `crates/embyr-pg-storage/src/backend_adapter.rs:208-213` read in full:
```rust
fn to_datetime(seconds: i64, nanos: i32) -> DateTime<Utc> {
    Utc.timestamp_opt(seconds, nanos as u32)
        .single()
        .expect("valid timestamp")
}
```
Confirmed the panic mechanics directly, not assumed: `Utc.timestamp_opt` returns `LocalResult::None`
whenever `nanos as u32 >= 1_000_000_000` (`nanos` is `i32`; a negative value also wraps to a huge
`u32` via the `as` cast, which is likewise rejected) or whenever `seconds` falls outside chrono's own
representable range (~262,000 years either side of the epoch). `LocalResult::Ambiguous` is confirmed
**unreachable for `Utc`** — chrono's own docs state ambiguity/DST gaps are a property of
offset-aware/local timezones only; `Utc` has no DST, so `.single()` only ever sees `None` or a
concrete value here. The `.expect("valid timestamp")` panics on that `None` case — confirmed the
sole failure mode, not a hypothetical one.

✓ Both call sites read in full:
- `crates/embyr-pg-storage/src/backend_adapter.rs:396-398` (`Some(WritePrecondition::UpdateTime(s,
  n))` arm inside the single-document update path) — `to_datetime` is called and its result bound
  directly into a `sqlx` query parameter; the panic fires **before** any SQL executes (no partial
  write, no rows touched).
- `crates/embyr-pg-storage/src/backend_adapter.rs:1035-1051` (the multi-write OCC-verification loop
  inside `commit_transaction`, used by Commit/Write/BatchWrite's own transactional apply path) — same
  call, same pre-SQL-execution timing, but here it fires **after** `pg_txn = self.pool.begin()...`
  has already opened a real Postgres transaction on a pooled connection. A panic while a
  `sqlx::Transaction` guard is alive on the stack runs that guard's `Drop` during unwind, which
  triggers `sqlx`'s own best-effort background rollback — confirmed no different in outcome from any
  other early-`?`-return failure in this same loop (the transaction is never committed), just a less
  graceful path to the same "OCC verification failed, nothing was written" end state.

✓ `crates/embyr-server/src/grpc/handler.rs:519-531` (`convert_precondition`) read in full — confirmed
zero validation, a direct pass-through: `ConditionType::UpdateTime(ts) =>
Some(WritePrecondition::UpdateTime(ts.seconds, ts.nanos))`. Called at 3 sites
(`handler.rs:1088,1822,2048` — Write, Commit, and the transactional-commit path respectively), so
all 3 client-facing write RPCs that accept a `Precondition` route through this same unvalidated
conversion.

✓ `crates/embyr-agent/src/server.rs:104-113` (`parse_precondition`) — read while investigating
blast radius (not originally named in the audit finding's own location list). Confirmed
**structurally identical, independent duplicate** of `convert_precondition`: same zero-validation
`ConditionType::UpdateTime(ts) => Some(WritePrecondition::UpdateTime(ts.seconds, ts.nanos))`
pass-through, in a *different* crate (`embyr-agent`, not `embyr-server`). See § Investigation
Finding 2 below — this matters for where the fix must live.

✓ Panic blast-radius investigation (task-required, confirmed directly, not assumed):
`crates/embyr-server/src/main.rs`/`lib.rs` and `crates/embyr-agent/src/main.rs` grepped for
`catch_unwind`/`panic::set_hook` — **zero matches in either binary** (same finding already
established once this session for the malformed-filter-shape feature, re-confirmed here
independently since this is a different code path). Tokio's own default per-spawned-task panic
isolation applies: a panic inside one gRPC request-handling task aborts **only that task** — tonic
surfaces it to the same caller as a transport-level error/reset, not a crash of the server process,
and never touches another tenant's concurrent request. This is a real, ungraceful RPC failure (the
audit's own framing), not a process-crash or data-corruption risk — confirmed, not merely asserted.

✓ Grepped the entire workspace for `timestamp_opt` (`Utc.timestamp_opt`/`TimeZone::timestamp_opt`) —
**exactly one call site exists**, the one at `backend_adapter.rs:210` this feature fixes. No
existing fallible-timestamp-conversion helper exists anywhere else to reuse (see § Investigation
Finding 1).

✓ Read `docs/feature/firestore-range-operator-value-type-support/feature-delta.md` § Resolution 2-5
to confirm how `RunQuery`'s own cursor/filter `Timestamp` values are handled, per the task's own
suggestion to check for a reusable pattern. Confirmed: cursor/filter-side `Timestamp` values are
**never converted to `chrono::DateTime` at all** — they are compared directly in SQL via Postgres
`ROW(seconds, nanos)` tuple comparison against the stored `(seconds, nanos)` columns, entirely
sidestepping chrono's own range constraints. This is why cursors/filters were never at risk of this
class of panic — not because a validation helper exists there, but because that code path uses a
structurally different (chrono-free) representation. Confirmed this is **not** a reusable
"validate-then-convert" pattern for the OCC path, which genuinely needs a real
`chrono::DateTime<Utc>` (bound as a `timestamptz` SQL parameter against the `documents.update_time`
column, itself `chrono`-typed) — see § Investigation Finding 1.

✓ `crates/embyr-core/src/domain/query.rs:16-24` (`validate_field_path`) read in full — confirmed the
exact reusable **pattern** (not helper) this feature should mirror: a pure, IO-free function living
in `embyr-core` (zero `tokio`/`sqlx`/`tonic` imports, per this repo's own `embyr-core` IO-freedom
constraint), returning `Result<(), CoreError>`, using the already-existing `CoreError::InvalidArgument
(String)` variant — the same variant this feature's own fix needs, requiring **zero new `CoreError`
variant**.

✓ `crates/embyr-core/src/error.rs` read in full — confirmed `CoreError::InvalidArgument(String)`
already exists (line 22-23).

✓ `crates/embyr-server/src/grpc/handler.rs:4148-4162` (`core_error_to_status`) and
`crates/embyr-agent/src/server.rs:87-102` (its own independent `core_error_to_status`) both read in
full — confirmed **both** already map `CoreError::InvalidArgument(_) => Status::invalid_argument(...)`
correctly, today, with no change needed. Whatever layer raises the new error, the wire-level
`INVALID_ARGUMENT` status is already wired in both binaries.

✓ `crates/embyr-agent/Cargo.toml` read in full — confirmed `embyr-agent` depends directly on
`embyr-pg-storage` (line 18). See § Investigation Finding 2.

✓ `docs/product/jobs.yaml` read in full (JOB-01 through JOB-20, including every NOTE). See
§ Persona & Job.

## Wave: DISCUSS / [REF] Investigation Findings

### Finding 1 — No existing fallible-timestamp-conversion helper exists anywhere; this is the only
unsafe conversion site in the workspace

The task's own premise (check whether `RunQuery`'s cursor/`startAt`/`endAt` handling already
validates timestamps safely, as a reuse candidate) does not hold the way it was framed: that code
path is safe not because of a validation helper, but because it never constructs a
`chrono::DateTime` from client seconds/nanos at all (it stays in raw `(i64, i32)` form and compares
via SQL `ROW(...)`). The OCC precondition path is structurally different — it binds a real
`DateTime<Utc>` against a real `timestamptz` column — so it cannot borrow that chrono-free
technique without a much larger change to the already-shipped `UPDATE ... WHERE update_time = $5`
mechanism (out of scope for this fix — see § Out of Scope). The fix must therefore make
`to_datetime` itself fallible. `validate_field_path` (`embyr-core/src/domain/query.rs`) is the
right **pattern** to mirror (pure, `embyr-core`-resident, `Result<(), CoreError>`,
`CoreError::InvalidArgument`), not a function to call.

### Finding 2 — Validating only in `embyr-server`'s `convert_precondition` would leave a second,
independent panic site completely unfixed: `embyr-agent`'s own `parse_precondition`

This is the single most important finding of this DISCUSS, and it changes the answer to the task's
own "where's the right layer to validate" question.

`embyr-agent` depends directly on `embyr-pg-storage` (confirmed, `Cargo.toml:18`) and instantiates
`PostgresBackendAdapter` itself, against the **customer's own local Postgres**, for every write the
agent's `StorageAgent` gRPC service (`:9191`, mTLS-gated) receives. `embyr-agent/src/server.rs`'s own
`parse_precondition` (lines 104-113) is a **separate, independently-written, zero-validation
pass-through** — structurally identical to `embyr-server`'s `convert_precondition`, but a different
function in a different crate — that flows straight into the SAME `to_datetime` panic in the SAME
shared `embyr-pg-storage` crate.

Two distinct request paths reach `to_datetime` today:
1. SDK client → `embyr-server` (`convert_precondition`) → direct-mode backends (`direct_pg`,
   `aws_secret`, `gcp_secret`) call `PostgresBackendAdapter::to_datetime` directly, **or** →
   `AgentBackendAdapter` forwards to the agent over mTLS (re-serializing a `Precondition` onto the
   internal `StorageAgent` proto).
2. Any mTLS-authenticated caller of the agent's own `StorageAgent` service directly (normally only
   `embyr-server`, but per audit finding #11's own established framing for this exact trust boundary
   — "mTLS-gated, so not remote-unauthenticated," a distinct caller from the public-facing SDK
   client) → `embyr-agent`'s own `parse_precondition` → the SAME `to_datetime`.

Fixing validation **only** at `embyr-server`'s `convert_precondition` closes path 1 completely (and
incidentally protects the agent too, *as long as* `embyr-server` is the only caller that ever reaches
the agent's `StorageAgent` service — an assumption, not a structural guarantee) but does **nothing**
for path 2: a caller hitting the agent's own RPC surface directly still hits the identical, still-live
panic in the agent's own `parse_precondition` → `to_datetime` chain, since that is genuinely separate
source code from `embyr-server`'s `convert_precondition`.

Fixing `to_datetime` itself — the one function both crates' independent precondition-parsing
functions both eventually call — closes **both** paths by construction, with a single change, in a
single crate (`embyr-pg-storage`), touching only the 2 call sites the audit already named. This also
means **no `backend_mode` scoping or exclusion is needed** (unlike most other JOB-01 features this
session, which routinely defer `backend_mode=agent` as separate follow-up scope) — the fix is
automatically uniform across every backend mode because `embyr-agent` reuses the identical shared
helper, not a copy of it.

**Resolution (answers the task's own open question)**: validate inside `to_datetime` itself
(changing its signature to return `Result<DateTime<Utc>, CoreError>`, propagated via `?` at both
existing call sites) as the **required, sufficient fix**. Additionally and optionally, DESIGN may
choose to *also* add a mirrored fail-fast check in `convert_precondition` (`embyr-server`) and/or
`parse_precondition` (`embyr-agent`) purely as a latency/UX optimization (reject before opening a
Postgres transaction, matching this session's own "reject before any handler logic runs" convention
from the Stripe-webhook and rate-limiter fixes) — but this is explicitly **not** a substitute for the
`to_datetime` fix, only a layered addition on top of it, since only the `to_datetime` fix actually
closes both request paths. Left as a named DESIGN choice below, not locked here.

### Finding 3 — No real Firestore SDK can construct the malformed input this feature guards against

Real Firestore SDKs' own `Timestamp` types (JS, Python, Java, Go, Admin SDKs) validate `seconds`
(roughly the `0001-01-01` to `9999-12-31` canonical range) and `nanoseconds` (`[0, 999999999]`)
client-side at construction time, before ever serializing to the wire. A `Precondition.update_time`
value in normal use is always either read back verbatim from a document snapshot the server itself
previously returned (already valid) or built via the SDK's own `Timestamp.now()`/`fromDate()`
(always valid). A real, well-behaved Alex-shaped app can never produce an out-of-range value through
any documented SDK API — triggering this bug requires constructing a raw gRPC `Precondition` no real
Firestore SDK's own type system permits. This is the identical trigger-shape test
`firestore-malformed-filter-shape-validation` already established and locked as its own Resolution 1
criterion for JOB-11-not-JOB-01 reuse (see § Persona & Job).

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend** — a small, defensive input-validation fix confined to one existing
  function plus its 2 call sites.
- JTBD: **reuse JOB-11, NOT JOB-01** (existing job) — see § Persona & Job; mirrors the
  `firestore-malformed-filter-shape-validation` precedent exactly (§ Investigation Finding 3).
- Walking Skeleton: **Yes** — a single, real write RPC with a malformed `update_time` precondition,
  proven to return a clean `INVALID_ARGUMENT` instead of a transport-level panic.
- UX Research Depth: **Lightweight** — a narrow defensive fix, no new emotional arc, no new journey
  artifact warranted (mirrors the malformed-filter-shape and embyr-agent-release-pipeline precedents).

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P2 Sam Chen (Service Operator / Platform Engineer)** — NOT P1 Alex. Per
§ Investigation Finding 3, Alex's real app can never construct the malformed input this feature
guards against; Sam is the one who cares that the service stays well-behaved (clean, named errors;
no raw panics in logs/traces) under any single caller's malformed or adversarial input, exactly
matching JOB-11's own already-established "one bad actor shouldn't produce ugly, unexplained failure
modes" framing (previously extended for the malformed-filter-shape feature).

**Job**: **JOB-11 `fair-multitenancy`**, reused, EXTENDED (not replaced) to also cover: a malformed
`Precondition.update_time` (out-of-range `seconds` and/or `nanos`) on any write RPC produces a
clean, named `INVALID_ARGUMENT` — never a raw panic/transport-reset — across **both** independent
precondition-parsing code paths this codebase has (`embyr-server`'s `convert_precondition` and
`embyr-agent`'s own `parse_precondition`), since both funnel into the same shared
`embyr-pg-storage::to_datetime` this feature fixes.

**Candidate considered and rejected**: **JOB-01 (`sdk-compat`, P1 Alex)** — rejected for the same
reason `firestore-malformed-filter-shape-validation` rejected it: no real Alex-shaped SDK client can
ever trigger this feature's own trigger cases (§ Investigation Finding 3), so this feature does
nothing to advance JOB-01's own "Alex's real app behaves identically to real Firestore" goal.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (1). >3 bounded contexts/modules? No — one function
(`to_datetime`, `crates/embyr-pg-storage/src/backend_adapter.rs`) plus its 2 existing call sites;
zero new crate, zero new domain type, zero new `CoreError` variant (reuses
`CoreError::InvalidArgument`, already wired to `Status::invalid_argument` in both
`core_error_to_status` implementations). Walking skeleton >5 integration points? No (1: a real write
RPC with a malformed `update_time` precondition, proven to return a clean error instead of a
transport-level panic). Estimated effort >2 weeks? No — well under a day (a signature change on one
private helper function, `Result`-propagation at 2 call sites, `if`-checks for the 2 malformed-input
shapes). Multiple independent user outcomes? No — a single outcome ("malformed OCC precondition input
never panics the write path") proven across the write path's 2 existing call sites.

**Scope Assessment: PASS** (0 oversizing signals fired) — comparable in size to
`firestore-malformed-filter-shape-validation`, the smallest sibling feature this session.

## Wave: DISCUSS / [REF] System Constraints

- `embyr-core` remains IO-free — if the validation logic is extracted into `embyr-core` (mirroring
  `validate_field_path`'s own location, per § Investigation Finding 1), it must be a pure function
  with zero `tokio`/`sqlx`/`tonic`/`chrono`-IO imports; the actual `chrono::DateTime` construction
  (which is pure computation, not IO) can still happen in `embyr-pg-storage` after validation passes,
  or the bounds check can be duplicated as two plain integer-range comparisons in `embyr-core` — the
  exact split is DESIGN's call.
- `to_datetime`'s new signature (`Result<DateTime<Utc>, CoreError>` instead of `DateTime<Utc>`)
  requires only `?`-propagation at its 2 existing call sites, both of which are already inside
  `async fn`s returning `Result<_, CoreError>` — zero further signature changes ripple outward.
- Zero new `CoreError` variant — `CoreError::InvalidArgument(String)` already exists and is already
  correctly mapped to `Status::invalid_argument` in **both** `core_error_to_status` implementations
  (`embyr-server/src/grpc/handler.rs:4148-4162` and `embyr-agent/src/server.rs:87-102`).
- `ConditionType::Exists(true)`/`Exists(false)` precondition handling (`WritePrecondition::MustExist`/
  `MustNotExist`) is architecturally untouched by this fix — those arms never call `to_datetime` at
  all (confirmed by direct reading of `convert_precondition`/`parse_precondition`); no behavior
  change, no test needed beyond a regression guard confirming this remains true.
- The 2 existing `to_datetime` call sites both compute the (fallible) conversion **before** any SQL
  executes or, in the transactional call site, before any row is locked — a validation failure there
  means zero partial writes and zero orphaned `FOR UPDATE` locks, confirmed by direct reading of the
  surrounding control flow (§ Reading Confirmation).
- No `backend_mode` scoping or exclusion needed — the fix is automatically uniform across
  `direct_pg`/`aws_secret`/`gcp_secret`/`agent` because `embyr-agent` shares the same
  `embyr-pg-storage::to_datetime` function via its own direct crate dependency (§ Investigation
  Finding 2) — not a copy of it.
- Whether to *additionally* fail fast in `convert_precondition` (`embyr-server`) and/or
  `parse_precondition` (`embyr-agent`) as defense-in-depth latency optimization, on top of the
  required `to_datetime` fix, is an open DESIGN choice (§ Investigation Finding 2) — not required to
  close the panic, since `to_datetime` alone is sufficient and covers both request paths.

## Wave: DISCUSS / [REF] User Stories

### US-01: A Malformed OCC Precondition Timestamp Gets a Clean Rejection Instead of a Panic

**job_id**: JOB-11 | **Release**: 1 | **Persona**: P2 Sam Chen

#### Elevator Pitch
**Before**: a caller sends a write RPC (`Commit`, `Write`, or a transactional commit via
`runTransaction`) whose `Precondition.update_time` carries an out-of-range `seconds` and/or `nanos`
value — no real Firestore SDK can construct this through its own documented API, but a raw gRPC
caller can. Today this panics the specific request-handling task, surfacing to that same caller as an
abrupt transport-level reset and showing up in Sam's own operational logs/traces as an alarming, raw
Rust panic string, indistinguishable at a glance from a genuine internal server defect.
**After**: run the identical malformed write RPC → the caller sees a clean, named
`INVALID_ARGUMENT` gRPC status describing exactly which field (`seconds` or `nanos`) was out of
range and why — no panic, no transport reset, no alarming log line.
**Decision enabled**: Sam Chen can immediately distinguish "a caller sent malformed input" from "the
server has an internal defect" when scanning operational logs/traces, and can confidently rule out a
genuine service-health incident for this specific error shape without paging anyone.

#### Who
- Sam Chen (P2) | Service Operator / Platform Engineer who owns this codebase's operational
  health/logging surface (same persona JOB-11 already names) | Needs every caller-triggerable failure
  mode to surface as a clean, named error — not a raw panic — so operational logs stay a reliable
  signal of genuine internal defects.
- Riley Nakamura (P4, DevSecOps Lead) | Downstream beneficiary for `backend_mode=agent` customers |
  Needs the identical guarantee to hold on the agent's own `StorageAgent` RPC surface (`:9191`,
  mTLS-gated), since her own deployed agent process independently reaches the same shared
  `to_datetime` helper (§ Investigation Finding 2) — not merely inheriting the fix incidentally
  through `embyr-server`.

#### Solution
Change `crates/embyr-pg-storage/src/backend_adapter.rs`'s private `to_datetime(seconds: i64, nanos:
i32) -> DateTime<Utc>` helper to return `Result<DateTime<Utc>, CoreError>` — returning
`Err(CoreError::InvalidArgument(...))`, naming the specific out-of-range field, instead of panicking
— and propagate that `Result` via `?` at both existing call sites (`:398` and `:1051`). Reuses the
already-existing `CoreError::InvalidArgument` variant, already correctly wired to
`Status::invalid_argument` in both `embyr-server`'s and `embyr-agent`'s own `core_error_to_status`.
Whether to additionally add a mirrored fail-fast check in `convert_precondition`
(`embyr-server/src/grpc/handler.rs`) and/or `parse_precondition` (`embyr-agent/src/server.rs`) for
latency (reject before opening a Postgres transaction) is left to DESIGN (§ Investigation Finding 2)
— not required for correctness.

#### Domain Examples

**Example 1 (Happy Path — regression guard, valid precondition continues to work exactly as
before)**: Alex's Firestore app calls
`updateDoc(orderRef, {status: "shipped"}, {lastUpdateTime: snapshot.updateTime})` where
`snapshot.updateTime` is the real, server-issued timestamp `{seconds: 1799942400, nanos: 123456789}`
(read back from a prior `getDoc()` on `projects/acme-corp/databases/(default)/documents/orders/order
-4471`). The write succeeds exactly as it does today — new `version`, new `update_time` returned —
because this precondition value is already within `chrono`'s valid range and was never at risk of the
panic in the first place.

**Example 2 (Edge Case — malformed `nanos`)**: Priya Kapoor, a security engineer running a pre-launch
fuzz-testing pass against the embyr `:8080` endpoint, sends a raw `Commit` RPC for
`projects/acme-corp/databases/(default)/documents/orders/order-4471` with
`Precondition.update_time = {seconds: 1799942400, nanos: 2147483647}` (`i32::MAX`, far outside the
valid `[0, 999999999]` range). Before this fix: the request task panics, Priya's tool sees a raw
transport error. After this fix: Priya's tool sees a clean `INVALID_ARGUMENT` naming the `nanos`
field and its valid range.

**Example 3 (Error/Boundary — malformed `seconds`)**: The same fuzz pass sends
`Precondition.update_time = {seconds: -9223372036854775808, nanos: 0}` (`i64::MIN`, far outside
chrono's ~262,000-year representable range around the epoch). Before this fix: panics identically to
Example 2, at the same `to_datetime` call site. After this fix: a clean `INVALID_ARGUMENT` naming the
`seconds` field.

#### UAT Scenarios (BDD)

```gherkin
Scenario: A malformed nanos value in an update_time precondition is cleanly rejected
  Given a document exists at "projects/acme-corp/databases/(default)/documents/orders/order-4471"
    with update_time {seconds: 1799942400, nanos: 500000000}
  When a Commit RPC updates that document with Precondition.update_time
    = {seconds: 1799942400, nanos: 2147483647}
  Then the RPC returns INVALID_ARGUMENT naming the "nanos" field as out of range
  And no document field is modified
  And other concurrent requests to the same server are unaffected

Scenario: A malformed seconds value in an update_time precondition is cleanly rejected
  Given a document exists at "projects/acme-corp/databases/(default)/documents/orders/order-4471"
    with update_time {seconds: 1799942400, nanos: 500000000}
  When a Commit RPC updates that document with Precondition.update_time
    = {seconds: -9223372036854775808, nanos: 0}
  Then the RPC returns INVALID_ARGUMENT naming the "seconds" field as out of range
  And no document field is modified

Scenario: A malformed update_time inside a transactional commit's OCC verification is cleanly
  rejected, not just the single-document update path
  Given a transaction has read document "projects/acme-corp/databases/(default)/documents/orders
    /order-4471" via runTransaction
  And the transaction's own write list includes an Update with Precondition.update_time
    = {seconds: 1799942400, nanos: 3000000000}
  When CommitTransaction is called
  Then the RPC returns INVALID_ARGUMENT naming the malformed field
  And the transaction is not partially applied — zero writes from the same transaction are committed

Scenario: A valid update_time precondition that matches the current document continues to succeed
  (regression guard)
  Given a document exists at "projects/acme-corp/databases/(default)/documents/orders/order-4471"
    with update_time {seconds: 1799942400, nanos: 123456789}
  When a Commit RPC updates that document with Precondition.update_time
    = {seconds: 1799942400, nanos: 123456789}
  Then the write succeeds
  And the response returns the new version and new update_time exactly as it did before this feature

Scenario: A valid but stale update_time precondition still produces the existing OCC-conflict
  behavior, unchanged (regression guard)
  Given a document exists at "projects/acme-corp/databases/(default)/documents/orders/order-4471"
    with update_time {seconds: 1799942400, nanos: 123456789}
  When a Commit RPC updates that document with a well-formed but stale Precondition.update_time
    = {seconds: 1799942300, nanos: 0}
  Then the RPC returns the same optimistic-concurrency-conflict error it returned before this feature
  And no document field is modified

Scenario: Exists(true) and Exists(false) preconditions are entirely unaffected (regression guard)
  Given a document exists at "projects/acme-corp/databases/(default)/documents/orders/order-4471"
  When a Commit RPC updates that document with Precondition.exists = true
  Then the write succeeds exactly as it did before this feature
  And no timestamp parsing or validation occurs for this precondition type
```

#### Acceptance Criteria
- [ ] AC-OCC-01: a write RPC (`Commit` or `Write`) with `Precondition.update_time.nanos` outside
      `[0, 999999999]` returns a clean `INVALID_ARGUMENT` naming the `nanos` field — no panic, no
      transport reset, no document modified.
- [ ] AC-OCC-02: a write RPC with `Precondition.update_time.seconds` outside chrono's representable
      range returns a clean `INVALID_ARGUMENT` naming the `seconds` field — no panic.
- [ ] AC-OCC-03: the identical clean rejection (no panic) applies inside the transactional
      multi-write OCC-verification loop (`commit_transaction`, the second existing panic call site
      at `backend_adapter.rs:1051`), not only the single-document update path — and the transaction
      is not partially applied.
- [ ] AC-OCC-04 (regression guard): a write RPC with a VALID `update_time` precondition that matches
      the current document's stored `update_time` continues to succeed exactly as before this
      feature.
- [ ] AC-OCC-05 (regression guard): a write RPC with a VALID but stale (non-matching) `update_time`
      precondition continues to produce the same optimistic-concurrency-conflict error it produced
      before this feature — this fix adds no new restriction to any well-formed precondition.
- [ ] AC-OCC-06 (regression guard): `Precondition.exists = true` and `Precondition.exists = false`
      continue to work exactly as before — this fix touches zero code on that path, since neither
      arm ever calls `to_datetime`.
- [ ] AC-OCC-07 (uniformity guard): the fix is confirmed uniform across `backend_mode` values —
      since `embyr-agent` shares the identical `embyr-pg-storage::to_datetime` function via a direct
      crate dependency (§ Investigation Finding 2), no `backend_mode`-specific exclusion or follow-up
      feature is required, unlike most other JOB-01 features this session.

#### Outcome KPIs
- **Who**: Sam Chen (P2, Service Operator / Platform Engineer), and Riley Nakamura (P4, DevSecOps
  Lead) as the downstream beneficiary for `backend_mode=agent` deployments.
- **Does what**: no longer sees a raw, uninformative Rust panic in operational logs/traces for a
  malformed `Precondition.update_time` on any write RPC, on either request path (`embyr-server`'s
  direct/agent-forwarding path, or a direct caller of `embyr-agent`'s own `StorageAgent` RPC).
- **By how much**: from 2 confirmed live panic sites (both call sites of `to_datetime`) to 0 — 100%
  of malformed `update_time` precondition inputs produce a clean, named `INVALID_ARGUMENT` instead.
- **Measured by**: AC-OCC-01 through AC-OCC-03 (direct positive proof), AC-OCC-04 through AC-OCC-06
  (regression proof — zero behavior change to any well-formed precondition), AC-OCC-07 (uniformity
  proof across backend modes).
- **Baseline**: 2 of 2 known call sites panic today on out-of-range input, confirmed by direct code
  reading (§ Reading Confirmation), not by reproduction alone.

#### Technical Notes
- `to_datetime`'s signature change (`DateTime<Utc>` → `Result<DateTime<Utc>, CoreError>`) is a
  private-function change with exactly 2 call sites, both already inside `async fn`s returning
  `Result<_, CoreError>` — no further signature ripple.
- Zero new `CoreError` variant; reuses `CoreError::InvalidArgument(String)`, already correctly wired
  to `Status::invalid_argument` in both `embyr-server::grpc::handler::core_error_to_status` and
  `embyr-agent::server::core_error_to_status`.
- DESIGN must decide the exact validation-location split (§ Investigation Finding 2): required —
  `to_datetime` itself validates (closes both request paths by construction); optional — a mirrored
  fail-fast check added to `convert_precondition` and/or `parse_precondition` for latency, and
  whether that shared bounds-check logic should be extracted into `embyr-core` (mirroring
  `validate_field_path`'s own location and shape) to avoid duplicating the two range checks by hand
  in up to 3 places.
- No new external dependency, no new port/adapter trait method, no change to any public trait
  signature (`BackendAdapter` itself is untouched — `to_datetime` is a private module function).

## Wave: DISCUSS / [REF] Out of Scope

- **Changing the OCC precondition mechanism to avoid `chrono::DateTime` entirely** (mirroring how
  `RunQuery`'s own cursor/filter `Timestamp` values are compared via raw `ROW(seconds, nanos)`
  tuples, per § Investigation Finding 1) — a real, structurally larger alternative that would touch
  the already-shipped `UPDATE ... WHERE update_time = $5` mechanism and the `documents.update_time`
  column's own type; out of scope for a defensive "make it not panic" fix. Named here so a future
  reader does not mistake this DISCUSS as having silently ruled it out by oversight.
- **A general audit of every other panicking `.expect()`/`.unwrap()` in `embyr-pg-storage` or
  elsewhere** — this feature closes exactly the 2 call sites the audit finding named; a broader sweep
  is unscoped, separate work with no current evidence of a comparable client-triggerable panic
  elsewhere (the workspace-wide `timestamp_opt` grep confirms this is the *only* site of this
  specific class).
- **Adding the optional fail-fast checks in `convert_precondition`/`parse_precondition`** — locked as
  a DESIGN-owned choice (§ Investigation Finding 2), not committed to as part of this DISCUSS's own
  scope; the `to_datetime` fix alone is sufficient to satisfy every AC above.
- **Reframing this as a security/DoS-mitigation feature** — the blast radius is confirmed
  self-contained to the offending caller's own single request (Tokio's own per-task panic isolation,
  no `catch_unwind` exists in either binary); this is operational-log clarity and RPC-contract
  correctness, not a security boundary, mirroring the same conclusion
  `firestore-malformed-filter-shape-validation` already reached for its own, structurally similar
  finding.

## Wave: DISCUSS / [REF] Walking Skeleton Strategy

**Strategy A** (real, minimal, end-to-end) — the single slice is a real write RPC
(`Commit`/`Write`/transactional-commit) with a genuinely malformed `Precondition.update_time`, proven
against a real running server and a real Postgres backend to return a clean `INVALID_ARGUMENT`
instead of a transport-level panic — not a unit-test-only or mocked proof.

## Wave: DISCUSS / [REF] Driving Ports

gRPC `:8080` `Commit`/`Write`/transactional-`Commit` (existing routes, zero new RPC) on
`embyr-server`; gRPC `:9191` `StorageAgent` `Commit`-equivalent (existing route, zero new RPC) on
`embyr-agent`, per § Investigation Finding 2's own confirmation that both binaries reach the shared
fix.

## Wave: DISCUSS / [REF] Pre-requisites

- None. `to_datetime`, both its call sites, `CoreError::InvalidArgument`, and both
  `core_error_to_status` implementations all already exist and are already correct for every other
  variant — this is a narrow, self-contained fallibility fix to one existing function.

## Wave: DISCUSS / [REF] Definition of Ready Validation

### DoR Checklist (9-item hard gate)
1. [x] Story traces to a job_id (JOB-11) — reused, not new, with the rejected alternative (JOB-01)
   explicitly reasoned against, directly mirroring the already-locked
   `firestore-malformed-filter-shape-validation` precedent (§ Investigation Finding 3).
2. [x] Story has a complete Elevator Pitch (Before / After / Decision enabled), naming a real
   user-invocable entry point (the `Commit`/`Write`/transactional-`Commit` gRPC RPCs).
3. [x] 3+ domain examples with real, concrete data (real numeric boundary values — `i32::MAX`,
   `i64::MIN` — and a realistic document path/persona, not generic placeholders).
4. [x] UAT scenarios in Given/When/Then (7 scenarios — within the 3-7 right-sized range).
5. [x] Acceptance criteria derived directly from the UAT scenarios (AC-OCC-01 through AC-OCC-07).
6. [x] Right-sized (well under 1 day, 7 scenarios, single demoable outcome) — Scope Assessment
   passed.
7. [x] Technical notes identify constraints (signature change scope, zero new `CoreError` variant,
   DESIGN-owned validation-location choice).
8. [x] Outcome KPIs have a numeric target (2 known panic sites → 0) and measurement methods (direct
   AC proof).
9. [x] Prior-wave artifacts read and reconciled (audit finding #9, both call sites, both
   precondition-parsing functions in both crates, `CoreError`, both `core_error_to_status`
   implementations, the sibling `firestore-malformed-filter-shape-validation` and
   `firestore-range-operator-value-type-support` precedents, and `jobs.yaml` JOB-11 all directly
   informed this feature's shape).

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Persona/job: **P2 Sam Chen / JOB-11 (`fair-multitenancy`)**, reused — not P1 Alex / JOB-01; no
  real Firestore SDK client can trigger this feature's own trigger cases (§ Investigation Finding 3),
  mirroring the already-locked `firestore-malformed-filter-shape-validation` precedent exactly.
- [D2] Fix location: **`to_datetime` itself must become fallible** (`Result<DateTime<Utc>, CoreError>`,
  reusing `CoreError::InvalidArgument`) — this is the required, sufficient fix, because it is the one
  function both `embyr-server`'s `convert_precondition` AND `embyr-agent`'s own independent
  `parse_precondition` both eventually call (§ Investigation Finding 2). Validating only in
  `embyr-server`'s `convert_precondition` (the task's own suggested "lean toward earliest" default)
  would leave the agent's own directly-reachable `StorageAgent` RPC surface unprotected — a genuine
  blocking reason against that shallower option, found by direct investigation rather than assumed
  away.
- [D3] Optional additional fail-fast checks in `convert_precondition`/`parse_precondition` (defense
  in depth, latency optimization) are named but explicitly left to DESIGN, not locked here.
- [D4] Zero `backend_mode` scoping/exclusion needed — uniform fix by construction, a genuine
  departure from this session's usual "defer `backend_mode=agent` as follow-up" pattern, made
  possible because the vulnerable function is already shared, not duplicated, between the two
  binaries' storage layers.

### Requirements Summary
- Primary need: 2 confirmed live panic sites on the write path, triggerable only by a non-SDK
  -conforming raw gRPC caller, must produce a clean, named `INVALID_ARGUMENT` instead.
- Walking skeleton scope: US-01, the entire feature — single story, 7 UAT scenarios, 7 ACs.
- Feature type: Backend correctness/hardening fix — Finding #9 (High) from the 2026-09-08
  production-readiness audit.

### Constraints Established
- `to_datetime` becomes fallible; both existing call sites propagate via `?`; zero new `CoreError`
  variant; zero new port/adapter trait method; zero behavior change to `Exists(true)`/`Exists(false)`
  preconditions or to any well-formed `update_time` precondition (match or stale-mismatch).
- No `backend_mode` exclusion — uniform across `direct_pg`/`aws_secret`/`gcp_secret`/`agent`.

### Upstream Changes
- **`docs/product/jobs.yaml`, JOB-11 entry**: append a dated NOTE (not applied by this DISCUSS —
  flagged for the FINALIZE step, matching this session's own established convention) — "JOB-11 now
  also covers a malformed `Precondition.update_time` (out-of-range `seconds`/`nanos`) on any write
  RPC producing a clean, named `INVALID_ARGUMENT` instead of a raw panic, across BOTH independent
  precondition-parsing code paths this codebase has (`embyr-server`'s `convert_precondition` and
  `embyr-agent`'s own separate `parse_precondition`), since both funnel into the same shared
  `embyr-pg-storage::to_datetime` — same job, same persona (P2 Sam Chen), not a new job, mirroring
  `firestore-malformed-filter-shape-validation`'s own precedent. Unlike that sibling feature, this fix
  requires zero `backend_mode` scoping: the vulnerable function is shared, not duplicated, between
  `embyr-server` and `embyr-agent`'s own storage layers. See
  docs/feature/occ-precondition-validation/feature-delta.md § Investigation Finding 2."

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 4 locked Decisions (D1-D4), 1-story walking-skeleton plan, 7
ACs (AC-OCC-01 through AC-OCC-07) to design executable scenarios against. DESIGN's own investigation
scope: (1) whether to extract the two range checks (`nanos` bounds, `seconds` bounds) into a small,
pure `embyr-core` function mirroring `validate_field_path`'s own location/shape, to avoid duplicating
raw bounds logic if the optional fail-fast checks in `convert_precondition`/`parse_precondition` are
also added; (2) whether to add those optional fail-fast checks at all, given `to_datetime`'s own fix
is already sufficient and covers both request paths (§ Investigation Finding 2); (3) the exact error
message wording for the two malformed-field cases (naming `seconds` vs `nanos` specifically, per
AC-OCC-01/02).

## Wave: DESIGN / [REF] Reading Confirmation

✓ `crates/embyr-pg-storage/src/backend_adapter.rs:208-218` re-read at DESIGN depth. Confirmed
`to_datetime` (209-213) is a bare module-private `fn` (no `pub`) — not reachable from any other crate
directly, only through `PostgresBackendAdapter`'s own `BackendAdapter` trait-impl methods. Confirmed
`from_datetime` (216-218) is a pure, infallible `(i64, i32)`-from-`DateTime<Utc>` projection with zero
client-input involvement.

✓ Workspace-wide grep for `to_datetime(` performed independently (not trusting DISCUSS's own count
without re-verification, per this session's own established blast-radius lesson): **exactly 3
matches**, all inside `crates/embyr-pg-storage/src/backend_adapter.rs` — the definition (line 209) and
its 2 call sites (line 398, line 1051). No third call site anywhere in the workspace, including
`embyr-agent`. Confirms DISCUSS's blast radius exactly; nothing missed.

✓ `crates/embyr-pg-storage/src/backend_adapter.rs:357-362` (`update_document`) and `:991-996`
(`commit_transaction`) read in full for their own `async fn` signatures. Both confirmed to already
return `Result<_, CoreError>` (`Result<WriteResult, CoreError>` and `Result<Vec<WriteResult>, CoreError>`
respectively) — a bare `?` after `to_datetime(...)` requires zero `.map_err()` wrapper at either site.

✓ `crates/embyr-agent/src/server.rs:104-114` (`parse_precondition`) re-read. Confirmed it builds
`Some(WritePrecondition::UpdateTime(ts.seconds, ts.nanos))` directly from the proto with no
intermediate `to_datetime` call of its own — structurally identical confirmation to
`embyr-server/src/grpc/handler.rs:519-531`'s own `convert_precondition`. Neither function can be given
early validation without changing its own return type from `Option<WritePrecondition>` to something
fallible (`Result<Option<WritePrecondition>, CoreError>` or `Result<_, Status>`), which would ripple to
every one of their own call sites (3 in `handler.rs`, plus `parse_precondition`'s own callers in
`embyr-agent/src/server.rs`) — confirmed a strictly larger, cross-cutting signature change versus the
single-function, already-`Result`-returning fix at `to_datetime` itself.

✓ `crates/embyr-core/src/error.rs:22-23` re-confirmed: `CoreError::InvalidArgument(String)` exists,
no new variant needed.

✓ Existing regression coverage confirmed by direct grep (`UpdateTime|precondition` across `tests/`):
`tests/acceptance/us_02_write_document.rs` (single-document `UpdateTime` precondition, match case) and
`tests/acceptance/us_06_transactions.rs` (transactional `UpdateTime` precondition — a stale-precondition
conflict test at ~line 509-520, plus `Exists(true)`/`Exists(false)` regression tests at ~553-726) already
exercise every VALID precondition shape this fix must leave unchanged (AC-OCC-04, AC-OCC-05, AC-OCC-06).
No malformed-input case exists yet in either file — DISTILL adds the new AC-OCC-01/02/03/07 scenarios;
DELIVER must not regress the existing valid-path tests in these two files.

## Wave: DESIGN / [REF] Signature Design Decision

**New signature** (locked):
```rust
fn to_datetime(seconds: i64, nanos: i32) -> Result<DateTime<Utc>, CoreError>
```

**`from_datetime` is NOT changed** (confirms the task's own suggested reasoning): it only converts an
already-valid, server-computed `DateTime<Utc>` — read back from the `documents.update_time`
`timestamptz` column or produced by `NOW()` — into wire-format `(i64, i32)` for a RESPONSE. It never
receives client input and cannot itself produce chrono's `LocalResult::None` case, because its input is
already a valid, constructed `DateTime<Utc>` by construction. Making it fallible would add a `Result`
that can never actually be `Err`, an unrequested abstraction (ponytail: no error type for a case that
cannot occur).

**Call-site propagation** (both already inside `Result<_, CoreError>`-returning `async fn`s — bare `?`,
no `.map_err()` needed):
- `backend_adapter.rs:398` (inside `update_document`, `Result<WriteResult, CoreError>`):
  `let precondition_dt = to_datetime(s, n)?;`
- `backend_adapter.rs:1051` (inside `commit_transaction`'s per-write OCC-verification loop,
  `Result<Vec<WriteResult>, CoreError>`): `let expected_dt = to_datetime(expected_secs, expected_nanos)?;`
  — the `?` here returns out of `commit_transaction` while `pg_txn` (the open `sqlx::Transaction` guard)
  is still on the stack, so `pg_txn`'s own `Drop` runs the same best-effort rollback it already runs
  today when the panic unwound through it (§ DISCUSS Reading Confirmation) — this fix makes that
  rollback a normal, graceful `?`-triggered `Drop`, not a panic-triggered one. Same outcome (transaction
  never committed), strictly better mechanism (no panic in the process at all).

**Implementation** (illustrative — DELIVER owns exact decomposition/naming):
```rust
fn to_datetime(seconds: i64, nanos: i32) -> Result<DateTime<Utc>, CoreError> {
    if !(0..=999_999_999).contains(&nanos) {
        return Err(CoreError::InvalidArgument(format!(
            "Precondition.update_time.nanos must be within [0, 999999999], got: {nanos}"
        )));
    }
    Utc.timestamp_opt(seconds, nanos as u32)
        .single()
        .ok_or_else(|| CoreError::InvalidArgument(format!(
            "Precondition.update_time.seconds is out of range for a valid timestamp, got: {seconds}"
        )))
}
```
`nanos` is range-checked FIRST, before the `nanos as u32` cast — this is load-bearing, not stylistic: a
negative `nanos` wraps to a huge `u32` under `as`, which `timestamp_opt` would also reject, but
attributing that rejection to `seconds` instead of `nanos` would violate AC-OCC-01's own field-naming
requirement. Checking `nanos`'s own signed range before the cast guarantees AC-OCC-01 (nanos) and
AC-OCC-02 (seconds) are never cross-attributed. Once `nanos` is confirmed in `[0, 999_999_999]`, the
only remaining way `timestamp_opt(...).single()` can return `None` is `seconds` outside chrono's
representable range — safe to attribute unconditionally to `seconds` at that point.

## Wave: DESIGN / [REF] Early-Validation Decision (resolves D3 / Investigation Finding 2's open question)

**Decision: do NOT add early fail-fast checks in `convert_precondition` (`embyr-server`) or
`parse_precondition` (`embyr-agent`). The `to_datetime` fix alone is sufficient, and is where the fix
stays.**

Reasoning:
- Both existing call sites already run `to_datetime` **before any SQL statement executes** (confirmed
  independently at DESIGN depth, § Reading Confirmation above and DISCUSS's own System Constraints):
  `update_document`'s single `UPDATE` statement is issued strictly after `to_datetime` returns;
  `commit_transaction`'s per-write loop calls `to_datetime` strictly before its own `SELECT ... FOR
  UPDATE`. There is no partial write and no held row lock to avoid in either path today — the only cost
  an early check could remove is `commit_transaction`'s own `pg_txn = self.pool.begin()` (a pooled
  connection checkout + `BEGIN`), and even that is a single, cheap, already-pooled round trip, not a
  materially costly operation.
- Adding the check earlier requires changing `convert_precondition`'s and `parse_precondition`'s own
  return types from `Option<WritePrecondition>` to something fallible, rippling to every one of their
  own call sites (3 in `handler.rs` alone, per DISCUSS's own count) — a strictly larger diff than the
  1-function, already-`Result`-typed change at `to_datetime`, for a latency benefit bounded to a single
  pooled-connection round trip on exactly one of the two request paths.
- Ponytail: don't validate twice when the single downstream check is already correct, fast (pure
  integer comparisons plus one `chrono` call, no IO), and safe (proven to run before any SQL). This
  mirrors DISCUSS's own framing of the choice exactly (§ Investigation Finding 2, "optional... not a
  substitute").

`convert_precondition` and `parse_precondition` are therefore **unchanged** by this feature.

## Wave: DESIGN / [REF] embyr-core Extraction Decision

**Decision: do NOT extract the two range checks into `embyr-core`. Keep them inline inside
`to_datetime`, in `embyr-pg-storage`.**

`validate_field_path` (`embyr-core/src/domain/query.rs:16-24`) was named by DISCUSS as the **pattern**
to mirror (pure function, `Result<_, CoreError>`, `CoreError::InvalidArgument`) — not a function to
relocate to. Because the Early-Validation Decision above keeps `to_datetime` as the ONLY place these two
checks are needed (no duplication across `convert_precondition`/`parse_precondition`, since neither
gets an early check), there is exactly one implementation and one call site for the bounds logic.
Extracting a single-use, single-call-site pair of integer-range checks into a different crate is an
unrequested abstraction (ponytail rung 1: does this need to exist at all — no, YAGNI) that would add a
public `embyr-core` API surface and a cross-crate call for zero duplication removed. If a future feature
needs the identical bounds check in a second location, that is the point to extract it — not before.

## Wave: DESIGN / [REF] Error Message Wording

Matches this codebase's own established `CoreError::InvalidArgument` phrasing convention (confirmed by
grepping existing usages — `embyr-core/src/domain/project.rs:13-15`: `"project id must match ^...$, got:
{s}"`; `embyr-core/src/domain/query.rs:20-22`: `"field path must match ^...$, got: {path}"`): name the
field, state the constraint, echo the offending value.

- **nanos**: `"Precondition.update_time.nanos must be within [0, 999999999], got: {nanos}"`
- **seconds**: `"Precondition.update_time.seconds is out of range for a valid timestamp, got:
  {seconds}"` (no fixed numeric range is quoted for `seconds` — chrono's own representable bound is an
  implementation detail of the `chrono` crate, not a stable contract worth hard-coding into a
  user-facing message; "out of range for a valid timestamp" is accurate without over-promising a range
  chrono does not itself publish as a stable API guarantee).

Both flow unchanged through the existing `CoreError::InvalidArgument(_) => Status::invalid_argument(...)`
arm in both `embyr-server`'s and `embyr-agent`'s own `core_error_to_status` — zero change needed there.

## Wave: DESIGN / [REF] ADR Decision

**No new ADR.** This is a narrow fallibility fix to one existing private helper's signature, reusing
100% pre-existing infrastructure (`CoreError::InvalidArgument`, both already-correct
`core_error_to_status` mappings) with zero new component, zero new port/adapter, zero new cross-cutting
architectural pattern, and zero rejected-but-plausible alternative left unrecorded (the two real
alternatives considered — early fail-fast validation in the conversion functions, and an `embyr-core`
extraction — are both fully reasoned through directly above, with no residual trade-off a future
maintainer would need an ADR to recover). Matches this session's own precedent: the structurally
identical `firestore-malformed-filter-shape-validation` fix recorded zero new ADR. Contrast
`rate-limiter-project-id-validation`, which DID warrant one (ADR-069) — that fix introduced a genuinely
new mechanism (reusing an existence-check as a metrics-label gate) with a real, non-obvious trade-off
surface; this fix has none.

## Wave: DESIGN / Handoff Package

**Files requiring a change:**
1. `crates/embyr-pg-storage/src/backend_adapter.rs` — the ONLY production-code file requiring a change.
   `to_datetime` (lines 209-213) — signature changes to `Result<DateTime<Utc>, CoreError>`, body adds
   the `nanos`-then-`seconds` validation shown above. Its 2 call sites (line 398 inside
   `update_document`, line 1051 inside `commit_transaction`) each gain a trailing `?`. `from_datetime`
   (lines 216-218) is unchanged.

**Files confirmed to need NO change (blast radius, grep-verified independently at DESIGN depth):**
- `crates/embyr-server/src/grpc/handler.rs` — `convert_precondition` (519-531) and its 3 call sites
  (1088, 1822, 2048) are unchanged (§ Early-Validation Decision). `core_error_to_status` (4148-4162)
  already maps `CoreError::InvalidArgument` correctly.
- `crates/embyr-agent/src/server.rs` — `parse_precondition` (104-114) is unchanged (§ Early-Validation
  Decision). `core_error_to_status` (87-102) already maps `CoreError::InvalidArgument` correctly.
- `crates/embyr-core/*` — `CoreError` unmodified (zero new variant); no extraction added (§ embyr-core
  Extraction Decision).

**Documentation changes made this wave:** none beyond this feature-delta.md. No new ADR (§ ADR
Decision). `docs/product/jobs.yaml` JOB-11 NOTE remains flagged for FINALIZE, per DISCUSS's own Upstream
Changes entry — not applied here.

**Regression guards DISTILL/DELIVER must run (grep-verified to exist and be real tests):**
- `tests/acceptance/us_02_write_document.rs` — exercises a valid, matching `UpdateTime` precondition on
  a single-document update (AC-OCC-04's regression guard).
- `tests/acceptance/us_06_transactions.rs` — exercises a stale `UpdateTime` precondition inside a
  transactional commit (AC-OCC-05's regression guard) and `Exists(true)`/`Exists(false)` preconditions
  (AC-OCC-06's regression guard).
- New DISTILL-authored scenarios cover AC-OCC-01 (malformed `nanos`), AC-OCC-02 (malformed `seconds`),
  AC-OCC-03 (malformed `update_time` inside `commit_transaction`'s OCC loop), and AC-OCC-07 (uniformity
  across `backend_mode` — a real write through `embyr-agent`'s own `:9191` `StorageAgent` surface with a
  malformed precondition, proving the shared `to_datetime` fix, not a mocked assertion).
- Full workspace `cargo test` (this feature's own regression floor) — run once, at the pre-commit gate,
  per this repo's own root `CLAUDE.md` test-run token-discipline rule.

**External integrations**: none. This feature touches no external API or third-party service — no
contract-testing annotation applies.

**Development paradigm reminder for DELIVER**: functional-where-practical Rust (repo `CLAUDE.md`) — the
new `to_datetime` body is a pure transformation (no shared mutable state, explicit `Result<T, CoreError>`
error type), consistent with the paradigm; no port/adapter trait signature changes.

**Contingency note (peer review, iteration 1)**: the Early-Validation and `embyr-core` Extraction
decisions above are linked — both are locked as "not needed" for the same reason (single call site, no
duplication). If DISTILL/DELIVER later reintroduces early fail-fast checks in
`convert_precondition`/`parse_precondition` for a reason not anticipated here, re-open the `embyr-core`
Extraction Decision at that point, since duplication across 3 call sites would then justify it. Not
expected — named for process visibility only.

## Wave: DESIGN / [REF] Peer Review

Reviewed by `nw-solution-architect-reviewer`, iteration 1. **approval_status: approved**,
critical_issues_count: 0, high_issues_count: 0, medium_issues_count: 1 (Early-Validation/embyr-core
Extraction decision contingency — addressed above), low_issues_count: 1 (process-documentation
durability note, no action needed). Verdict: "architecturally minimal and correctly justified"; nanos-
before-seconds ordering, blast radius (3 workspace matches, 2 call sites), and no-new-ADR reasoning all
independently confirmed sound.

## Wave: DISTILL / [REF] Prior Wave Consultation — Reading Confirmation

+ `docs/feature/occ-precondition-validation/feature-delta.md` (this file, DISCUSS + DESIGN sections) —
  read in full.
+ `docs/architecture/atdd-infrastructure-policy.md` — read; all ports needed (gRPC data port :8080,
  Agent gRPC :9191, System/Customer Postgres) already have recorded mechanisms. No new rows appended.
+ `tests/common/state_delta.rs` — read; Rust state-delta port already bootstrapped (prior feature). Not
  invoked by this feature's own new scenarios (see § Mandate 8 Applicability below).
+ `tests/acceptance/us_02_write_document.rs`, `tests/acceptance/us_06_transactions.rs` — read in full
  (both named by DESIGN's own Handoff Package as the existing regression-guard homes).
+ `tests/acceptance/embyr_agent/mod.rs`, `tests/acceptance/embyr_agent/us_a02_write_operations.rs`,
  `tests/acceptance/embyr_agent.rs` — read in full to confirm the `AgentHandle`/`start_test_agent` mTLS
  harness convention and the module-registration mechanism (`#[path = ...] mod ...;` in
  `embyr_agent.rs`, itself registered as `[[test]] name = "embyr_agent"` in
  `crates/embyr-agent/Cargo.toml`).
+ `crates/embyr-pg-storage/src/backend_adapter.rs` (`to_datetime`, both call sites) — re-confirmed
  UNFIXED at DISTILL time (line 212 still `.expect("valid timestamp")`) — DESIGN's locked signature
  change is DELIVER's job, not applied here.
+ `crates/embyr-server/src/grpc/handler.rs` (`core_error_to_status`, `handle_update_document`, `commit`)
  and `crates/embyr-agent/src/server.rs` (`core_error_to_status`, `parse_precondition`) — re-confirmed
  both already map `CoreError::InvalidArgument` to `Status::invalid_argument` correctly, and confirmed
  which RPC hits which `to_datetime` call site: `UpdateDocument` RPC → `update_document` (`:398`);
  `Commit`/`Write` RPC → `commit_transaction` (`:1051`).
- `docs/product/kpi-contracts.yaml` — not found; no `@kpi` scenario applicable (this feature adds no new
  KPI, per DISCUSS's own Outcome KPIs section — the metric is "2 confirmed panic sites → 0",
  measured directly by AC-OCC-01/02/03, not a new emitted-event contract).
- `docs/feature/occ-precondition-validation/discuss/wave-decisions.md`,
  `docs/feature/occ-precondition-validation/design/wave-decisions.md`,
  `docs/feature/occ-precondition-validation/devops/wave-decisions.md` — not found as separate files;
  DISCUSS/DESIGN decisions are inlined in this same `feature-delta.md` (this feature predates/does not
  use the split-file layout). No DEVOPS wave was run for this feature (small backend fix, no
  infrastructure change) — defaults apply (existing embyr-server/embyr-agent deployment topology,
  unchanged by this fix).

## Wave: DISTILL / Wave-Decision Reconciliation HARD GATE

DISCUSS and DESIGN sections of this same `feature-delta.md` were read in full above. Checked every
DISCUSS decision (D1-D4, § Wave Decisions Summary) against every DESIGN decision (Signature Design,
Early-Validation, embyr-core Extraction, Error Message Wording, ADR): **zero contradictions** — DESIGN
implements D2 exactly as scoped (`to_datetime` becomes the sole fix site), explicitly reasons through
D3's open question (declining the optional fail-fast checks) rather than silently dropping it, and
introduces no new persona/job/backend_mode claim that conflicts with DISCUSS's D1/D4. No DEVOPS section
exists to check for a third-way contradiction (no DEVOPS wave was run — narrow backend fix, no
infrastructure change per DISCUSS's own Feature Type classification).

**Reconciliation passed — 0 contradictions.**

## Wave: DISTILL / [REF] Scenario List

| # | Scenario | AC | Tags | File |
|---|----------|----|----|------|
| 1 | `update_document_with_malformed_nanos_precondition_returns_invalid_argument` | AC-OCC-01 | `@walking_skeleton @driving_port @real-io @error` | `tests/acceptance/us_02_write_document.rs` |
| 2 | `update_document_with_malformed_seconds_precondition_returns_invalid_argument` | AC-OCC-02 | `@driving_port @real-io @error` | `tests/acceptance/us_02_write_document.rs` |
| 3 | `commit_transaction_with_malformed_nanos_precondition_returns_invalid_argument` | AC-OCC-03 | `@driving_port @real-io @error` | `tests/acceptance/us_06_transactions.rs` |
| 4 | `updating_with_malformed_nanos_precondition_returns_invalid_argument` | AC-OCC-07 | `@driving_port @real-io @error` | `tests/acceptance/embyr_agent/us_a02_write_operations.rs` |

Scenario 1 is designated the walking skeleton (single real write RPC, real Postgres backend, malformed
`update_time`, proven to return a clean `INVALID_ARGUMENT` instead of a transport-level panic — matches
DISCUSS's own Walking Skeleton Strategy verbatim). All 4 scenarios are `@real-io` (real gRPC server,
real Postgres via testcontainers, real mTLS agent process) — no `@in-memory` variant exists or is
warranted for a 2-call-site signature-fallibility fix. Error-path ratio: 4/4 new scenarios = 100%
(pure defensive-input-rejection feature; DISCUSS's own AC-OCC-04/05/06 happy-path/regression
requirements are satisfied by PRE-EXISTING tests, not new ones — see § Regression Guard Mapping below).

**Skip/one-at-a-time staging**: NOT applied. This is a single private-function signature-fallibility
fix (DESIGN's own Scope Assessment: well under a day, 1 story, 2 call sites) — GREEN flips all 4
scenarios simultaneously with one code change, matching this session's own precedent for
structurally-identical sibling bug-fix features (`firestore-malformed-filter-shape-validation`,
`rate-limiter-project-id-validation`), neither of which used per-scenario skip staging. Applying
one-at-a-time `#[ignore]` gating here would create artificial staging for a change with no meaningful
intermediate state.

## Wave: DISTILL / [REF] Regression Guard Mapping (AC-OCC-04/05/06 — no new tests)

| AC | Guard | Existing test | File |
|----|-------|---------------|------|
| AC-OCC-04 (valid, matching precondition succeeds) | Pre-existing | `concurrent_writes_on_same_document_one_aborts_per_occ` (the one-succeeds branch) | `tests/acceptance/us_02_write_document.rs` |
| AC-OCC-05 (valid, stale precondition → existing OCC-conflict error, unchanged) | Pre-existing | `occ_conflict_causes_aborted_status` | `tests/acceptance/us_06_transactions.rs` |
| AC-OCC-06 (`Exists(true)`/`Exists(false)` unaffected) | Pre-existing | `commit_write_with_must_not_exist_precondition_is_rejected_when_document_already_exists`, `commit_write_with_must_exist_precondition_is_rejected_when_document_does_not_exist` | `tests/acceptance/us_06_transactions.rs` |

Per DESIGN's own Handoff Package ("DELIVER must not regress the existing valid-path tests in these two
files"), these three ACs are regression floors, not new scenarios — confirmed by direct reading of each
test body, not assumed from the file name.

## Wave: DISTILL / [REF] Adapter Coverage Table

| Adapter / Driving Port | `@real-io` scenario | Covered by |
|---|---|---|
| gRPC data port :8080 — `UpdateDocument` RPC | YES | Scenario 1, 2 (real tonic client, real testcontainers Postgres) |
| gRPC data port :8080 — `Commit` RPC (transactional) | YES | Scenario 3 |
| Agent gRPC :9191 (mTLS) — `StorageAgent.UpdateDocument` RPC | YES | Scenario 4 (real `rcgen` mTLS certs, real testcontainers Postgres, in-process `embyr-agent`) |
| `PostgresBackendAdapter::to_datetime` (both call sites) | YES (indirectly, through both RPC paths above) | Scenarios 1+2 exercise `:398`; Scenario 3 exercises `:1051` |

Zero "NO — MISSING" rows: this feature introduces zero new adapters (it is a validation fix inside an
already-adapter-tested function); both existing call sites are exercised by the new scenarios and both
existing `core_error_to_status` mappings are exercised transitively.

## Wave: DISTILL / [REF] Driving Adapter Verification

Both driving ports named in DISCUSS (§ Driving Ports) are exercised via their real protocol, not via a
service-function shortcut:
- `embyr-server` gRPC :8080 — Scenarios 1-3 use `FirestoreClient` over a real `tonic::transport::Channel`
  connected to `start_test_server`'s bound address; exit is a real `tonic::Status` with `.code()` and
  `.message()` asserted.
- `embyr-agent` gRPC :9191 (mTLS) — Scenario 4 uses `StorageAgentClient` over a real mTLS
  `tonic::transport::Channel` (client cert required, per the existing `agent_common`/`AgentHandle`
  harness), reusing `start_test_agent` rather than inventing new mTLS test setup (per the task's own
  instruction).

## Wave: DISTILL / [REF] Test Placement

Extended 3 pre-existing, already-`[[test]]`-registered files rather than creating new ones or new
`Cargo.toml` registrations — matching this repo's own established convention (grep for
"precondition"/"UpdateTime"/"occ" across `tests/` found `us_02_write_document.rs` and
`us_06_transactions.rs` already own this exact behavioral area; DESIGN's own Handoff Package named
both by path). No new `[[test]]` entry was needed in either `crates/embyr-server/Cargo.toml` or
`crates/embyr-agent/Cargo.toml` — all three files are already registered (`us_02_write_document`
line 90, `us_06_transactions` line 106 in `embyr-server/Cargo.toml`; `embyr_agent` line 42 in
`embyr-agent/Cargo.toml`, which itself registers `us_a02_write_operations.rs` as a submodule via
`#[path = ...] mod us_a02_write_operations;` in `tests/acceptance/embyr_agent.rs`).

## Wave: DISTILL / [REF] Scaffolds

None created. Mandate 7 (RED-ready scaffolding) is N/A for this feature: `to_datetime`, both its call
sites, `CoreError::InvalidArgument`, and both `core_error_to_status` implementations already exist in
production code today (confirmed by DISCUSS/DESIGN reading and re-confirmed at DISTILL time, see
§ Reading Confirmation above). This is a signature-fallibility fix to existing, already-imported
production code — no new module, no new import, nothing to stub.

## Wave: DISTILL / [REF] Mandate 8 Applicability (state-delta / Universe)

Per the Layered Test Discipline table (`nw-test-design-mandates`), all 4 new scenarios run at the
**Subprocess/FS acceptance** layer (real gRPC server + real Postgres via testcontainers) — layer 3.
Mandate 8's `assert_state_delta` is a layer 1-3 REQUIREMENT for state-MUTATING assertions; these 4
scenarios' regression assertions ("document unchanged") are already expressed as direct field-equality
checks against a `GetDocument` response (a port-exposed observable, not an internal field) — consistent
with the Universe discipline in spirit. `tests/common/state_delta.rs`'s `assert_state_delta` helper was
evaluated for use here and intentionally NOT invoked: this repo's own existing sibling tests in the same
two files (e.g. `commit_write_with_must_not_exist_precondition_is_rejected_when_document_already_exists`)
use direct `assert_eq!` on the fetched document field, not the `state_delta` port, for the identical
"write rejected, document unchanged" shape — matching the established local convention (Pillar 2 chained
narrative: new scenarios read like the existing sibling scenarios in the same file) rather than
introducing a second assertion idiom into files that don't otherwise use it.

## Wave: DISTILL / [REF] Pre-requisites

None. Per DISCUSS's own Pre-requisites section: `to_datetime`, both call sites, `CoreError`, and both
`core_error_to_status` mappings already exist. All test infrastructure (real server harness, real mTLS
agent harness, testcontainers Postgres) already exists and required zero new fixtures.

## Wave: DISTILL / [REF] Pre-DELIVER Fail-For-Right-Reason Gate

Executed against current (unfixed) code — see `docs/feature/occ-precondition-validation/distill/red-classification.md`
for full detail. Summary: all 4 new scenarios classified **MISSING_FUNCTIONALITY (correct RED)**. All 4
independently confirmed the SAME empirical failure mode: the server task panics at
`backend_adapter.rs:212:10` ("valid timestamp"), Tokio's per-task panic isolation contains the crash to
that one request (test harness process survives), and the client observes `tonic::Code::Cancelled`
("h2 protocol error: http2 error") — not `INVALID_ARGUMENT`, not a raw process crash. This empirically
confirms DISCUSS's own confirmed (not assumed) investigation of Tokio panic-isolation behavior. Zero
scenarios in the IMPORT_ERROR/FIXTURE_BROKEN/SETUP_FAILURE/WRONG_ASSERTION categories — all 3
target files compile clean (`cargo test --no-run` for all 3 test targets, 0 errors).

**Gate result: PASS. Cleared for DELIVER handoff.**

## Wave: DISTILL / [REF] Mandate Compliance Evidence

- **CM-A** (Mandate 1, hexagonal boundary): all 4 scenarios invoke the driving port exclusively
  (`FirestoreClient`/`StorageAgentClient` over a real `tonic::transport::Channel`) — zero direct
  imports of `PostgresBackendAdapter`, `to_datetime`, or any internal component in step bodies. Import
  listing: `us_02_write_document.rs`/`us_06_transactions.rs` import `embyr_proto::firestore::*` +
  `embyr_server::{adapters::system_db::SystemDb, start_test_server}` (composition-root entry, not an
  internal adapter call); `us_a02_write_operations.rs` imports `embyr_proto::agent::*` +
  `super::agent_common::{start_test_agent, AgentHandle}` (the mTLS composition-root entry).
- **CM-B** (Mandate 2, business language): scenario doc-comments use Given/When/Then in business terms
  ("a document exists with a valid update_time", "the RPC returns INVALID_ARGUMENT naming the X field");
  technical terms (gRPC RPC names, `tonic::Code`) appear only inside step BODIES (assertions), not in
  doc-comment titles — matching the established convention of every sibling scenario in these 3 files.
- **CM-C** (Mandate 3, user journey completeness): each scenario is trigger → business-rule enforcement
  → observable outcome (clean named error OR unchanged document) → business value (Sam Chen's log
  clarity goal, per DISCUSS's own Outcome KPIs). Walking skeleton (Scenario 1) is demo-able: "send this
  malformed write, see this clean error, not a crash."
- **CM-D** (Mandate 4, pure function extraction): N/A — no new business logic was extracted for this
  feature; DESIGN explicitly declined the `embyr-core` extraction (§ embyr-core Extraction Decision,
  YAGNI reasoning already recorded). `to_datetime`'s own new validation logic is DELIVER's
  implementation concern, not DISTILL's.

## Wave: DISTILL / [REF] Peer Review

Reviewed by `nw-acceptance-designer-reviewer`, iteration 1. **approval_status: approved**,
blocker_count: 0, high_count: 0, low_count: 0. All three mandates pass (CM-A hexagonal boundary,
CM-B business language, CM-C user journey completeness). Dimension scores 8-10/10 across happy-path
bias, GWT format, business language, coverage completeness, walking-skeleton user-centricity, priority
validation, observable-behavior assertions, traceability, and walking-skeleton boundary proof. Verdict:
"Tests correctly validate the feature against all 7 acceptance criteria." Handoff to DELIVER: cleared.

## Wave: DISTILL / Next Wave

**Handoff To**: nw-software-crafter (DELIVER wave).
**Deliverables**: 4 new RED acceptance scenarios across 3 already-registered test files (no new
`[[test]]` entries needed), `docs/feature/occ-precondition-validation/distill/red-classification.md`
(gate evidence), this DISTILL section. DELIVER's own scope per DESIGN's locked signature: change
`to_datetime`'s signature in `crates/embyr-pg-storage/src/backend_adapter.rs` to
`Result<DateTime<Utc>, CoreError>`, add the `nanos`-then-`seconds` validation exactly as DESIGN's own
Implementation snippet shows, add `?` at both call sites (`:398`, `:1051`). Zero other production files
require a change (§ DESIGN Handoff Package). Full-workspace `cargo test` reserved for DELIVER's own
pre-commit gate, per this repo's root `CLAUDE.md` test-run token-discipline rule — not run here.
