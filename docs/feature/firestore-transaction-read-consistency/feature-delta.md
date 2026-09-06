# Feature Delta: firestore-transaction-read-consistency

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/jobs.yaml`, JOB-01 (`sdk-compat`, persona P1 Alex) — read in full.
✓ `docs/SPEC.md` §Transaction, §Read-Within-Transaction, §CommitTransaction, §Transactions-table
schema, §Consistency/OCC error-mapping table — the wire contract for transactional reads is
ALREADY fully documented here (`GetDocumentForTransaction` records `(path, version)` in the
transaction's own read set; `CommitTransaction` re-validates every recorded read at commit,
aborting with `codes.Aborted` on any mismatch) — this is a documented-but-unbuilt gap, the SAME
pattern as `aggregation-queries`/`firestore-composite-indexes-admin-api`'s own JOB-01 NOTEs, not a
net-new design decision.
✓ `migrations/customer/0002_transactions.sql` — confirmed the `transactions` table has NO `reads`
column at all (only `transaction_id`/`project_id`/`status`/`started_at`) — SPEC.md's own documented
`reads map(string → int64)` field was never actually created.
✓ `migrations/customer/0001_documents.sql` — confirmed a `version BIGINT NOT NULL DEFAULT 1` column
already exists on `documents`, separate from `update_time`, already incremented on every write
(`crates/embyr-pg-storage/src/backend_adapter.rs`, every `UPDATE`/`INSERT ... ON CONFLICT` sets
`version = version + 1` or an explicit value) — this is the exact OCC token SPEC.md's `reads` map
is meant to store per path, already alive and correctly maintained, just never READ from the
transactional-read path today.
✓ `crates/embyr-pg-storage/src/transactions/occ.rs` (`verify_versions`) — confirmed an ALREADY
-EXISTING, ALREADY-WORKING version-based OCC check: for any `Write::Update`/`Write::Delete` that
carries an explicit `version: Some(v)`, `commit_transaction` locks the document row (`FOR UPDATE`)
and aborts (`CoreError::TransactionAborted`) if the current `version` column doesn't match `v`.
This is the write-attached-version half of OCC; this feature adds the sibling, read-derived half.
✓ `crates/embyr-core/src/storage/backend_adapter.rs` — confirmed `run_query` ALREADY has a
`transaction_id: Option<&TransactionId>` trait parameter (added by a prior feature), but
`crates/embyr-pg-storage/src/backend_adapter.rs`'s own `run_query` impl names it `_transaction_id`
and does nothing with it; `get_document` has NO such parameter at all on the trait. Both handler
call sites in `crates/embyr-server/src/grpc/handler.rs` (`handle_run_query`, lines 2323 and 3180)
hardcode `None` for this parameter — `consistency_selector` is never read off `RunQueryRequest` at
all. `GetDocumentRequest`'s own `consistency_selector.transaction` bytes field is likewise never
read anywhere in `handle_get_document`.
✓ `crates/embyr-server/src/adapters/agent_backend.rs` — confirmed `AgentBackendAdapter::run_query`
already carries the identical `_transaction_id`-ignored pattern (`backend_mode=agent` transactions
across the SaaS↔Agent boundary are a pre-existing, unaddressed gap — this feature does not change
that; see § Out of Scope).
✓ `docs/feature/firestore-write-streaming/feature-delta.md`, `docs/feature/firestore-batch-write/
feature-delta.md` — read for precedent on how this codebase has previously handled
`begin_transaction`/`commit_transaction` reuse and agent-mode deferral; both establish the pattern
this feature follows (extend the trait, real behavior in the primary backends, agent-mode deferred
as a separate candidate follow-up).

**Direct investigation, not inference — is this actually a JOB-01/Alex concern, or (mirroring the
last feature's own reassignment lesson) is it really someone else's?** Real Firestore's
`runTransaction(fn)` — used by every SDK for "read some documents, then write based on what you
read, atomically" — depends ENTIRELY on this mechanism for correctness: the callback's own
`transaction.get(docRef)` calls are what establish the read set the commit later validates. A real
Alex-shaped app doing e.g. `runTransaction(tx => { const doc = await tx.get(ref); tx.update(ref,
{count: doc.data().count + 1}) })` — the single most common transaction pattern in real Firestore
usage — gets ZERO protection today: two concurrent callers can both read the same counter, both
compute `count + 1` from the same stale value, and both commits succeed, silently dropping one
increment. This is Alex's OWN app's data getting corrupted, not an operator-visible log/trace
concern — squarely JOB-01 (`sdk-compat`), P1 Alex, no reassignment warranted. Confirmed by
investigation, not assumed by habit.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend** (Decision 1).
- JTBD: **JOB-01, P1 Alex** (Decision 4 = "Yes", existing job) — confirmed above, not assumed.
- Walking Skeleton: **Yes** (Decision 2) — Slice 01 proves the mechanism end-to-end for the single
  -document `GetDocument` case before extending to `RunQuery`/`BatchGetDocuments`.
- UX Research Depth: **Lightweight** (Decision 3) — a backend correctness fix; no new emotional arc
  beyond what JOB-01's own existing persona profile already documents.

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P1 Alex (SDK Developer)** — unchanged from JOB-01's existing profile.

**Job**: **JOB-01 `sdk-compat`**, reused, EXTENDED (not replaced) to also cover: a real
`runTransaction(fn)` callback's own reads participate in the SAME optimistic-concurrency guarantee
real Firestore gives them — a document read inside a transaction that changes before that
transaction commits aborts the transaction (`codes.Aborted`, matching real Firestore exactly),
instead of silently allowing a lost-update race.

## Wave: DISCUSS / [REF] Business Context — why this outranks the crash-elimination arc's own severity

The 3-feature crash-elimination arc (`firestore-query-filter-operator-support` →
`firestore-equal-notequal-value-type-support` → `firestore-range-operator-value-type-support`,
FINALIZED 2026-09-06) closed every remaining panic reachable by an ordinary, well-formed Firestore
SDK query. This gap is a DIFFERENT, arguably HIGHER-severity-in-effect category: a panic is loud —
the request fails visibly, the caller's own code sees an error and (if written correctly) retries.
A lost update from unenforced transactional-read OCC is **silent** — both concurrent commits report
success, and the data is simply wrong, discovered later if at all. This is the single
highest-real-world-impact Firestore-parity gap identified in this codebase's own most recent
production-readiness scan (2026-09-06), ranked above every other finding in that scan for exactly
this reason.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (3, one per RPC). >3 bounded contexts/modules? Touches 3
crates (`embyr-core` trait signature, `embyr-pg-storage` migration + implementation,
`embyr-server` handler call sites) — the SAME crate-count as `firestore-range-operator-value-type-
support`, which passed this gate with an identical 3-slice-shaped 2-crate touch (that feature was
2 crates; this one is 3, but each crate's own change is a single, mechanical, well-precedented
edit — no new bounded context, no new port, no new adapter). Walking skeleton >5 integration
points? No (1: `GetDocument` read-registration + `commit_transaction` read-set validation).
Estimated effort >2 weeks? No — each of the 3 slices mirrors an already-proven mechanism
(`verify_versions`'s own FOR-UPDATE-lock-and-compare shape) almost verbatim. Multiple independent
user outcomes? No — all 3 slices serve the exact same outcome (transactional reads are honored) for
3 different read RPCs.

**Scope Assessment: PASS** (0 oversizing signals fired, using the same reasoning precedent as
`firestore-range-operator-value-type-support`'s own multi-crate PASS).

## Wave: DISCUSS / [REF] Journey — Alex's "My Counter Increments Correctly Under Load" Arc

### Mental model

Alex's app runs `db.runTransaction(async tx => { const doc = await tx.get(counterRef); await
tx.update(counterRef, {count: doc.data().count + 1}); })` from many concurrent requests — the
textbook Firestore transaction pattern for a shared counter, inventory decrement, or seat
reservation. Alex expects (because this is real Firestore's own documented guarantee) that if two
of these transactions overlap and one commits first, the second one's read is now stale and its
commit is REJECTED (`ABORTED`), so the SDK's own automatic retry re-reads the fresh value and
correctly reapplies the increment. Today, embyr accepts BOTH commits unconditionally — Alex's
counter under-counts silently, with no error, no retry, no signal anything went wrong.

### Failure modes (feeds DELIVER test design)

- Two transactions both `tx.get()` the same document, one commits an update, the SAME document
  the second transaction read has now changed: today the second transaction's commit succeeds
  anyway (lost update); after this feature, it aborts with `ABORTED`, matching real Firestore.
- A transaction reads a document that does NOT exist, then (in the same transaction) another actor
  creates that exact document, then the original transaction commits: today succeeds regardless;
  after this feature, the "confirmed absent" read is also validated at commit and aborts if the
  document now exists — the create-under-you case, not just the update-under-you case.
- A transaction reads a document and NOTHING else changes it before commit: unaffected — commits
  succeed exactly as before (this feature adds a check, not a new restriction on the happy path).
- A transaction with an INVALID or EXPIRED transaction ID attempts a read: rejected with the SAME
  `CoreError::TransactionNotFound` `commit_transaction`/`rollback_transaction` already produce for
  this case today — no new error class.

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

### Backbone

Alex's app calls `runTransaction(fn)` → the SDK's `tx.get(docRef)` call today performs a plain,
unregistered read → `tx.update(...)`/commit succeeds regardless of what changed between read and
commit → **this feature**: every read performed with a transaction ID registers `(path, version)`
in that transaction's own read set → `CommitTransaction` re-validates every registered read
against the CURRENT document state, aborting on any mismatch, exactly as real Firestore does.

### Walking Skeleton

**Slice 01**: `GetDocument` read-registration + `commit_transaction` read-set validation, proven
end-to-end against a real two-actor lost-update race (one transaction's commit succeeds, the
other's aborts).

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| # | Story | Release | Estimate | Learning Hypothesis | Reference Class |
|---|---|---|---|---|---|
| 01 | US-01 (GetDocument, Walking Skeleton) | 1 | ≤1 day | Disproves: the read-set-validation mechanism can reuse `verify_versions`'s own FOR-UPDATE-lock-and-compare shape without inventing new OCC infrastructure | Mirrors `crates/embyr-pg-storage/src/transactions/occ.rs::verify_versions`, already shipped and proven |
| 02 | US-02 (RunQuery) | 1 | ≤0.5 day | Disproves: query-result read-registration is a trivial N-times application of Slice 01's own single-document mechanism, since `run_query`'s trait signature already carries `transaction_id` | Direct extension of Slice 01; the trait plumbing already exists, only the handler-side extraction + pg-storage-side registration loop are new |
| 03 | US-03 (BatchGetDocuments) | 1 | ≤0.5 day | Disproves: batch-get read-registration is the SAME N-times application already used, by this codebase's own established precedent, for `batch-get-documents`' access-control sequence | Mirrors `batch-get-documents`' own "N times, once per requested document, unchanged" reuse pattern (JOB-01 NOTE, 2026-08-30) |

## Wave: DISCUSS / [REF] Prioritization

Sequential 01 → 02 → 03: each slice is a strict superset of the previous slice's own mechanism
(register-a-read, validate-at-commit); Slice 01 carries all the real design risk (the FOR-UPDATE
locking + absence-vs-version-mismatch semantics), Slices 02/03 are mechanical extensions once
Slice 01 is proven — highest-uncertainty slice first, matching this session's own established
prioritization rationale (cf. `firestore-range-operator-value-type-support`'s Slice 01 before 02).

## Wave: DISCUSS / [REF] System Constraints

- `embyr-core` gains ONE trait-signature change: `get_document` gains a
  `transaction_id: Option<&TransactionId>` parameter, mirroring `run_query`'s own already-existing
  parameter shape exactly — no new port, no new adapter, no new `CoreError` variant (reuses
  `TransactionAborted`/`TransactionNotFound`, both already used by `commit_transaction`/
  `rollback_transaction` today).
- Blast radius of that signature change (confirmed by direct grep, not estimated): 8 call sites in
  `crates/embyr-server/src/grpc/handler.rs`, 4 in `crates/embyr-agent/src/server.rs`, 1 impl each in
  `crates/embyr-pg-storage/src/backend_adapter.rs` and `crates/embyr-server/src/adapters/
  agent_backend.rs`. Every call site EXCEPT the 3 read RPCs this feature targets
  (`handle_get_document`, and the `get_document` calls made ON BEHALF of those 3 RPCs) passes
  `None` — this feature does not change behavior for any non-transactional read path.
- `backend_mode=agent` is explicitly deferred (§ Out of Scope) — `AgentBackendAdapter::get_document`
  gains the parameter but ignores it (`_transaction_id`), mirroring its own already-existing
  `run_query`/`run_aggregation_query` pattern exactly; `crates/embyr-agent/src/server.rs`'s own
  internal calls to `self.storage.get_document(&path)` pass `None` (the agent's OWN internal
  `embyr.agent.v1.GetDocumentRequest` proto has no transaction field — extending it is out of scope
  here, matching the established "agent-mode deferred, separate proto surface" precedent from
  `firestore-write-streaming`/`firestore-list-rpcs`/`firestore-field-transforms`).
- Read-set entries are recorded via BOUND SQL parameters into `jsonb_build_object(...)`, never
  string-interpolated into the JSONB key — a locked, non-negotiable constraint (this session's own
  standing security posture: never skip input validation/injection-safety at a trust boundary,
  even for a value that today happens to come from an already-parsed resource path).
- `ReadTime` (point-in-time reads) and `BatchGetDocuments`' own `new_transaction`
  (auto-begin-a-transaction-inline) `ConsistencySelector` variants are explicitly OUT OF SCOPE —
  distinct Firestore features from "read within an ALREADY-BEGUN transaction," not silently folded
  in (§ Out of Scope).

## Wave: DISCUSS / [REF] User Stories

### US-01: A Transaction's `GetDocument` Read Participates in Optimistic Concurrency Control

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: `runTransaction(tx => tx.get(docRef))` performs a plain, unregistered read; the SAME
document changing before commit has NO effect on whether the transaction's commit succeeds.
After: run a real `GetDocument` with a `transaction` ID, then commit that transaction after the
SAME document was modified by another actor → the commit fails with `ABORTED`, matching real
Firestore exactly.
Decision enabled: Alex's own transactional read-modify-write code (counters, inventory,
reservations) is now safe under concurrent load — the SDK's own automatic transaction-retry logic
(built into every real Firestore SDK) now has a real signal to retry on.

#### Acceptance Criteria
- [ ] AC-TRC-01: `GetDocument` with `consistency_selector.transaction` set registers that
      document's current `(path, version)` (or confirmed-absence) in the transaction's own read
      set.
- [ ] AC-TRC-02: `Commit` with that same `transaction` ID, after the read document's version
      changed (a concurrent write committed in between), returns `ABORTED` — the transaction's OWN
      writes are NOT applied.
- [ ] AC-TRC-03: `Commit` with that same `transaction` ID, when NOTHING changed the read document,
      succeeds exactly as it does today (zero regression on the happy path).
- [ ] AC-TRC-04: reading a document that does NOT exist, then having another actor CREATE that
      exact document before commit, also aborts the transaction (`ABORTED`) — the absence-based
      read set entry is validated too, not just the exists-with-a-version case.
- [ ] AC-TRC-05: `GetDocument` with an invalid/expired `transaction` ID returns the SAME error
      (`NotFound`, per `CoreError::TransactionNotFound`'s existing mapping) `Commit`/`Rollback`
      already return for this case today — no new error class introduced.

### US-02: A Transaction's `RunQuery` Reads Participate in Optimistic Concurrency Control

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: `runTransaction(tx => tx.get(query))` — the query-based read form real SDKs also expose
inside transactions — performs a plain, unregistered read of every matching document.
After: run a real `RunQuery` with a `transaction` ID, then commit after ANY one of the returned
documents changed → the commit fails with `ABORTED`.
Decision enabled: Alex's own transactional code that reads a QUERY (not just a single document) —
e.g. "read all pending orders, mark the first one claimed" — gets the identical safety guarantee
US-01 gives single-document reads.

#### Acceptance Criteria
- [ ] AC-TRC-06: `RunQuery` with `consistency_selector.transaction` set registers `(path, version)`
      for EVERY document the query returns, using the identical mechanism as AC-TRC-01.
- [ ] AC-TRC-07: `Commit` with that transaction ID, after any ONE of the query's own returned
      documents changed, returns `ABORTED`.
- [ ] AC-TRC-08 (regression guard): a non-transactional `RunQuery` (`consistency_selector` unset)
      is completely unaffected — zero new registration, zero new behavior.

### US-03: A Transaction's `BatchGetDocuments` Reads Participate in Optimistic Concurrency Control

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: `db.getAll(ref1, ref2, ...)` inside a transaction performs N plain, unregistered reads.
After: run a real `BatchGetDocuments` with a `transaction` ID, then commit after ANY one of the
requested documents changed → the commit fails with `ABORTED`.
Decision enabled: Alex's own transactional multi-document reads (e.g. "read both accounts before
transferring funds between them") get the identical safety guarantee US-01/US-02 give single
-document and query reads.

#### Acceptance Criteria
- [ ] AC-TRC-09: `BatchGetDocuments` with `consistency_selector.transaction` set registers
      `(path, version)` for every requested document, reusing the identical per-document mechanism
      as AC-TRC-01 (mirrors `batch-get-documents`' own established N-times-reuse precedent).
- [ ] AC-TRC-10: `Commit` with that transaction ID, after any ONE of the batch's own requested
      documents changed, returns `ABORTED`.

## Wave: DISCUSS / [REF] Outcome KPIs

### Feature: firestore-transaction-read-consistency

### Objective
Close the highest-real-world-impact gap identified in this codebase's most recent
production-readiness scan: transactional reads across all 3 read RPCs now participate in the same
optimistic-concurrency guarantee real Firestore gives them, eliminating a silent lost-update race.

### Outcome KPIs
| KPI | Target | Measurement |
|---|---|---|
| Read RPCs whose transactional reads are OCC-protected | 3 of 3 (`GetDocument`/`RunQuery`/`BatchGetDocuments`) | AC-TRC-01 through AC-TRC-10 |
| Regression on non-transactional reads or already-passing transactional writes | 0 | AC-TRC-03, AC-TRC-08, existing `client_auth`/`transactions`-adjacent regression suites |
| Mutation-testing kill rate on the new read-set validation logic | 100% effective (this session's own established bar) | `cargo-mutants --in-diff`, `--lib`-scoped |

## Wave: DISCUSS / [REF] Out of Scope

- **`backend_mode=agent` transactional reads** — `AgentBackendAdapter`'s own internal wire protocol
  to the customer-VPC agent has no transaction-carrying field on `GetDocumentRequest` today;
  extending it is a separate, deferred candidate feature (candidate id
  `agent-mode-transaction-read-consistency`), mirroring the established
  `agent-mode-write-streaming`/`agent-mode-list-collection-ids`/`agent-mode-field-transforms`
  deferral pattern exactly.
- **`ReadTime` consistency selector** (point-in-time reads at a past timestamp) — a distinct
  Firestore feature from transactional reads; unimplemented today, unaffected by this feature,
  not folded in.
- **`BatchGetDocuments`' own `new_transaction` auto-begin variant** — a convenience that implicitly
  calls `BeginTransaction` as part of the same RPC; this feature only handles the
  ALREADY-BEGUN-transaction case (`consistency_selector.transaction`). Auto-begin is a smaller,
  separable enhancement, named as a candidate follow-up, not built here.
- **Increasing the 60-second transaction TTL or exposing it as configuration** — SPEC.md already
  documents a configurable `transactions.ttl`; whether this codebase's own hardcoded 60s
  (`crates/embyr-pg-storage/src/backend_adapter.rs`, `commit_transaction`) should become real
  config is an orthogonal, pre-existing gap, not introduced or worsened by this feature.

## Wave: DISCUSS / [REF] WS Strategy

**Strategy A** (real, minimal, end-to-end) — Slice 01 is a real two-actor lost-update race, proven
against a real running server and a real Postgres backend, not a mock.

## Wave: DISCUSS / [REF] Driving Ports

gRPC `:8080` `GetDocument`, `RunQuery`, `BatchGetDocuments`, `Commit` (all existing routes, zero new
RPC).

## Wave: DISCUSS / [REF] Pre-requisites

- `crates/embyr-pg-storage/src/transactions/occ.rs::verify_versions` (already shipped) — the
  mechanism this feature's own read-set validation directly mirrors.
- `begin_transaction`/`commit_transaction`/`rollback_transaction` (already shipped, unmodified in
  their own outer contract — this feature adds a validation step INSIDE `commit_transaction`, does
  not change its signature or its callers).
- No new external dependency, no new bounded context.

## Wave: DISCUSS / [REF] Handoff Package

Handed to `nw-solution-architect` (DESIGN): this feature-delta.md, the confirmed blast-radius list
(§ System Constraints), and the explicit instruction to design the read-set storage shape and the
new `verify_reads`-style validation function as a direct sibling to `verify_versions`, reusing its
own FOR-UPDATE-lock-and-compare primitive rather than inventing a new one.

## Wave: DISCUSS / [REF] SSOT Updates

`docs/product/jobs.yaml`, JOB-01 entry: append a new dated NOTE — "JOB-01 now also covers
transactional-read optimistic-concurrency control for `GetDocument`/`RunQuery`/
`BatchGetDocuments` — a real `runTransaction(fn)` callback's own reads now participate in the same
OCC guarantee real Firestore gives them (a document read inside a transaction that changes before
commit aborts the transaction), closing a documented-but-unbuilt SPEC.md gap (§Transactions'
`GetDocumentForTransaction`/`CommitTransaction` contract existed in SPEC.md but was never
implemented — the `transactions` table had no `reads` column at all). Same job, same persona, not
a new job. Reuses `crates/embyr-pg-storage/src/transactions/occ.rs::verify_versions`'s own
FOR-UPDATE-lock-and-compare mechanism as a direct sibling, zero new `CoreError` variant. Identified
as the highest-real-world-impact finding in a 2026-09-06 production-readiness scan, ranked above
the already-closed 3-feature crash-elimination arc because the failure mode is a SILENT lost
update, not a visible panic. See
docs/feature/firestore-transaction-read-consistency/feature-delta.md."

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Requirements Completeness Score: **0.97**

### DoR Checklist (9-item hard gate)
1. [x] Every story traces to a job_id (JOB-01)
2. [x] Story has a complete Elevator Pitch (all 3 stories)
3. [x] Every AC is testable without ambiguity
4. [x] Walking Skeleton identified (US-01)
5. [x] Scope Assessment passed
6. [x] No slice contains only `@infrastructure` stories
7. [x] Out of Scope explicitly named (4 items)
8. [x] Outcome KPIs have numeric targets and measurement methods
9. [x] Prior-wave artifacts read and reconciled (SPEC.md's own documented-but-unbuilt contract
   confirmed by direct code inspection, not assumed from the doc alone)

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Open Questions

None carried forward unresolved.

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] JOB-01/P1 Alex confirmed correct (not reassigned) — the failure mode corrupts Alex's OWN
  application data, not an operator-visible operational concern.
- [D2] Mechanism: extend `get_document`'s trait signature to match `run_query`'s own already
  -existing `transaction_id: Option<&TransactionId>` parameter; add a `reads` JSONB column to
  `transactions`; add a `verify_reads` sibling to the already-shipped `verify_versions`, called
  from `commit_transaction` in the same position.
- [D3] `backend_mode=agent`, `ReadTime`, and `BatchGetDocuments`' `new_transaction` auto-begin
  variant are explicitly deferred — this feature covers exactly the
  already-begun-transaction/direct-and-secret-backed-modes case.

### Requirements Summary
- Primary need: transactional reads across all 3 read RPCs must participate in real Firestore's
  own optimistic-concurrency guarantee — today they do not, causing silent lost updates.
- Walking skeleton scope: US-01 (`GetDocument`), extended by US-02/US-03 (`RunQuery`/
  `BatchGetDocuments`) using the identical mechanism.
- Feature type: Backend.

### Constraints Established
- Zero new `CoreError` variant (reuses `TransactionAborted`/`TransactionNotFound`).
- Read-set JSONB keys are bound parameters, never string-interpolated.
- `backend_mode=agent` unaffected (deferred, separate candidate feature).

### Upstream Changes
None — this DISCUSS confirms rather than contradicts JOB-01's existing scope; SPEC.md's own
documented contract is realized, not amended.

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 3 locked Decisions, 3-slice plan

---

## Wave: DESIGN / [REF] Prior Wave Consultation — Reading Confirmation

✓ This feature-delta.md's own DISCUSS sections in full, all 3 Decisions.
✓ `crates/embyr-pg-storage/src/transactions/occ.rs` — `verify_versions`'s exact SQL shape (FOR
UPDATE lock, compare, `TransactionAborted` on mismatch), confirmed as the direct template for the
new `verify_reads` function.
✓ `crates/embyr-pg-storage/src/backend_adapter.rs::commit_transaction` (lines 916-1060+) — the
EXACT insertion point for `verify_reads`'s own call (immediately after the existing
`verify_versions` call, inside the same already-open `pg_txn`).

## Wave: DESIGN / [REF] Data Model

`migrations/customer/0005_transaction_reads.sql` (new migration — 0001-0004 already exist; an
`ALTER TABLE`, not an edit to `0002_transactions.sql`):

```sql
ALTER TABLE transactions ADD COLUMN reads JSONB NOT NULL DEFAULT '{}';
```

Key: `"{collection_path}/{document_id}"` (flat string, matches this codebase's own existing
path-as-string convention). Value: the document's `version` (JSON number) if it existed at read
time, or JSON `null` if the read confirmed the document did NOT exist (the create-under-you case,
AC-TRC-04). Re-reading the same path within one transaction OVERWRITES its entry (last-observed
value wins) — matches SPEC.md's own literal "map(string → int64)" semantics (a map has one entry
per key, not a list), and matches this codebase's own established last-write-wins convention for
JSONB maps elsewhere (e.g. `push_value_equality`'s whole-value JSONB comparison approach).

## Wave: DESIGN / [REF] Architecture Design

### 1. `embyr-core`: trait signature (mirrors `run_query`'s own existing shape)

```rust
async fn get_document(
    &self,
    path: &DocumentPath,
    transaction_id: Option<&TransactionId>,
) -> Result<Option<FirestoreDocument>, CoreError>;
```

Every non-transactional call site (all EXCEPT the 3 read RPCs' own top-level handler, and any
INTERNAL `get_document` call made strictly for access-rule/existence-check purposes unrelated to
the client's own read — confirmed case-by-case during DELIVER, not assumed) passes `None`,
identical in shape to `run_query`'s own existing `None`-everywhere-except-the-2-RunQuery-call-sites
pattern.

### 2. `embyr-pg-storage`: read-registration (new, in `get_document` and `run_query`)

When `transaction_id` is `Some(id)`:
1. Parse `id` via the already-existing `uuid_from_bytes` helper — a malformed ID surfaces the SAME
   `CoreError::InvalidArgument` it already produces at `Commit`/`Rollback` today (AC-TRC-05 reuses
   this existing error, not a new one for the "malformed bytes" sub-case).
2. Confirm the transaction is `status = 'active'` and unexpired — the SAME check
   `commit_transaction` already performs; on failure, `CoreError::TransactionNotFound` (reused).
3. After the document read completes (found-with-version, or confirmed-absent), UPSERT one entry
   into that transaction row's `reads` JSONB map via a BOUND-PARAMETER query:
   `UPDATE transactions SET reads = reads || jsonb_build_object($3::text, $4) WHERE
   transaction_id = $1 AND project_id = $2` — `$3` bound as the `"{collection_path}/{document_id}"`
   key string, `$4` bound as the version (`i64`) or SQL `NULL` (which `jsonb_build_object` renders
   as JSON `null`). Never string-interpolated (§ System Constraints).

`run_query` performs step 3 once per document the query returns (AC-TRC-06); `BatchGetDocuments`
(a `handle_batch_get_documents`-level loop over the SAME per-document `get_document` call
-already-established-by-precedent) gets it for free once `get_document` itself is transaction-aware
(AC-TRC-09) — no separate pg-storage-level code path needed for the batch case.

### 3. `embyr-pg-storage`: read-set validation at commit (`verify_reads`, new sibling to
`verify_versions`)

`crates/embyr-pg-storage/src/transactions/occ.rs` gains:

```rust
/// Verify all recorded transactional reads inside an open Postgres transaction.
///
/// For each `(path, expected_version)` entry in `reads` (expected_version is
/// `None` for a confirmed-absent read), the document's CURRENT state must
/// still match: `Some(v)` requires the document to exist with `version = v`;
/// `None` requires the document to still NOT exist. Any mismatch returns
/// `CoreError::TransactionAborted` — the identical error `verify_versions`
/// already returns for a write-attached version mismatch.
pub async fn verify_reads(
    conn: &mut PgConnection,
    project_id: &str,
    reads: &serde_json::Value,
) -> Result<(), CoreError> { /* ... */ }
```

Called from `commit_transaction` immediately after the existing `verify_versions` call (same
already-open `pg_txn`, same position in the function), reading the `reads` column added to the
initial `SELECT status, started_at FROM transactions ...` query (extended to
`SELECT status, started_at, reads FROM transactions ...`).

### 4. `embyr-server`: handler-side extraction (3 call sites)

`handle_get_document`, `handle_run_query`, `handle_batch_get_documents` each gain: parse
`req.consistency_selector` for the `Transaction(bytes)` variant (ignore `ReadTime`,
`NewTransaction` — § Out of Scope), wrap in `TransactionId(bytes)`, pass `Some(&txn_id)` (or `None`
if unset) to the adapter call that already exists at each site — no new adapter call, no new RPC.

## Wave: DESIGN / [REF] Wave Decisions Summary

### Key Decisions
- [D1] `reads` is a NEW JSONB column via a NEW migration (`0003_transaction_reads.sql`), not an
  edit to the already-applied `0002_transactions.sql` — this codebase's own established
  migration-immutability convention (every other customer-DB schema change this session added a
  new numbered migration, never edited a shipped one).
- [D2] `verify_reads` is a NEW function, a sibling to `verify_versions`, not a generalization of
  it — the two check semantically different things (write-attached explicit version vs.
  read-derived expected version-or-absence) and forcing one shared implementation would obscure
  that distinction for a ~15-line function; ponytail's own "two rungs work, take the higher one"
  guidance does not apply here because the higher rung (shared abstraction) would COST clarity, not
  save real duplication.
- [D3] Read-set entries are always bound parameters into `jsonb_build_object`, never
  string-interpolated — locked, non-negotiable (§ System Constraints).

### Constraints Established
- No new `CoreError` variant.
- No new port/adapter trait beyond the one signature change to `get_document`.
- `backend_mode=agent` compiles unmodified (`_transaction_id`, ignored) — zero behavior change for
  agent-mode transactions, explicitly deferred.

## Wave: DESIGN / [REF] Next Wave

**Handoff To**: nw-software-crafter (DELIVER wave, per this project's own established convention)
**Deliverables**: this feature-delta.md's DESIGN section, 3 slice briefs
