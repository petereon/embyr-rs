# ADR-048: `BatchWrite` Per-Write Transaction Loop, Wire Contract, and Translate-and-Catch Design

## Status

Accepted

## Context

`firestore-batch-write` (JOB-01) adds the `BatchWrite` unary RPC — the third
and last undeclared write RPC on the client-facing `google.firestore.v1.Firestore`
service (`docs/feature/firestore-batch-write/feature-delta.md` § Reading
Confirmation: no `BatchWriteRequest`/`BatchWriteResponse` message exists in the
vendored proto tree today). `docs/SPEC.md` §BatchWrite (lines 658-666) already
documents the full wire contract: `{writes}` in, `{write_results, status}` out,
each write applying in its **own** transaction, no top-level RPC error ever
returned. DISCUSS's own ground-truth reading of
`crates/embyr-pg-storage/src/backend_adapter.rs` already established the
central architectural conclusion this ADR locks: `commit_transaction` is
all-or-nothing per call, so `BatchWrite` must synthesize one
`begin_transaction()`+`commit_transaction()` pair PER WRITE, not once per
call — the identical synthesis pattern `firestore-write-streaming`'s own
ADR-046 already proved correct, applied here at a finer (per-write, not
per-`WriteRequest`-batch) granularity.

DESIGN's own re-reading of the current code, at today's line numbers (not
trusted from DISCUSS's own citation alone, though it turned out unchanged),
confirms and extends three points:

1. **`commit_transaction` (`crates/embyr-pg-storage/src/backend_adapter.rs:851-1065`)
   is confirmed all-or-nothing per call, re-read in full.** It opens exactly
   one Postgres transaction (`pg_txn = self.pool.begin()`, line 889), runs
   OCC/precondition checks (`UpdateTime` lines 895-932, `MustExist`/
   `MustNotExist` lines 934-987, version-based OCC line 990) for every write
   in the `writes` argument inside that ONE `pg_txn`, and any single write's
   precondition failure returns `Err(CoreError::TransactionAborted)` before
   any write is applied — `pg_txn` is dropped un-committed, rolling back every
   write in the call, including ones with no precondition problem of their
   own. `begin_transaction` (lines 834-849) is confirmed a plain `INSERT ...
   VALUES ($1, $2)` against the `transactions` table (implicit
   `status = 'active'` default), no Postgres transaction of its own.
2. **A failed `commit_transaction` call leaves its `transactions` row at
   `status = 'active'` indefinitely.** The row's `status` is only ever
   updated to `'committed'` (line 1051-1057) INSIDE the same `pg_txn` that a
   precondition failure rolls back before reaching that line — so on failure,
   the UPDATE never runs, `pg_txn` never commits, and the row is left exactly
   as `begin_transaction` created it: `active`. The 60-second expiry check
   (lines 877-886) only fires on a SUBSEQUENT `commit_transaction` call
   presenting the SAME `transaction_id` — which never happens for a synthetic,
   per-write transaction ID that is generated fresh, used once, and never
   returned to any caller (`BatchWrite`'s own wire contract has no
   client-visible transaction field, identical in this respect to `Write`'s
   own synthetic per-`WriteRequest` transactions, ADR-046 § Context finding
   1). **This is a pre-existing gap, not introduced by this feature** — it is
   the identical failure-path behavior `Commit`'s own single client-supplied
   transaction already has today, and `Write`'s own per-`WriteRequest`
   synthetic transaction inherited unchanged (ADR-046 does not mention it,
   but the same code path applies). `BatchWrite`'s own finer per-write
   granularity means a batch with K failing writes can leave up to K orphaned
   `active` rows per call, instead of at most 1 for `Commit`/`Write`. Named
   explicitly below (§ Decision 6, § Consequences), not silently inherited.
3. **`translate_writes_for_commit` (`handler.rs:659-747`) is confirmed to call
   `evaluate_write_rule_for_commit` (write-path access-rule evaluation) once
   per write already**, via the `security-rules-write-path` bug fix dated
   2026-08-30 (`handler.rs:1470-1478` comment, inside `handle_commit`) — this
   is a materially DIFFERENT state than `firestore-write-streaming`'s own
   ADR-046 found (at that time, `handle_commit` had zero write-rule
   enforcement). DISCUSS's own § System Constraints already assumed this
   current, correct state ("Write-path access-rule evaluation is per-write...
   mirrors `handle_create_document`/... a rule denial for one write must
   become that write's own `status[i]` entry") — confirmed accurate by this
   re-read, not a correction needed here, unlike ADR-046's own equivalent
   finding.

## Decision Drivers

1. **Reuse over invention (standing session practice)** — `BatchWrite`'s own
   value proposition is a new per-write CONTROL-FLOW shape (catch instead of
   short-circuit) around already-shipped translation and apply logic, not new
   write semantics. Any new composition must reuse existing port methods and
   existing per-write translation logic, not duplicate them.
2. **Positional alignment is a hard invariant, not a convenience** —
   `docs/SPEC.md`'s own explicit contract: `write_results[i]`/`status[i]`
   always present, at the request's own length, regardless of success or
   failure. The per-write loop must never skip a position or reorder.
3. **No top-level RPC error, ever, once past pre-loop validation** — this is
   `BatchWrite`'s single most distinguishing invariant versus every sibling
   write RPC in this codebase (`Commit` and `Write` both terminate/fail the
   whole call on a single bad write; `BatchWrite` never does, per-write
   failure or not). The per-write loop body must therefore never propagate a
   `?`-style early return for anything write-specific — only pre-loop
   call-level rejections (auth, rate limit, suspension, malformed request
   shape) may produce a top-level `Err`.
4. **Status-code family and structure consistency** — `core_error_to_status`
   (`handler.rs:2588-2602`) is reused unchanged for every per-write
   `CoreError`; no new error taxonomy is invented for `BatchWrite` alone.
5. **Wire-shape correctness over SPEC.md's own descriptive shorthand** —
   `docs/SPEC.md` describes `status[i] = null` for success, but
   `google.rpc.Status` (`proto/google/rpc/status.proto`) is a `message`, and a
   `repeated google.rpc.Status` field cannot carry a `null` entry at the wire
   level (proto3 message fields are always present, never optional-and-absent,
   inside a `repeated` list). "`null`" is SPEC.md's description of what an SDK
   surfaces to application code after decoding a `Status{code: 0}` (`OK`), not
   a literal wire instruction. This ADR resolves the ambiguity explicitly (§
   Decision 5) so DELIVER does not have to guess or invent a sentinel.

## Decision

### 1. Proto additions

`proto/google/firestore/v1/write.proto` (alongside `CommitRequest`, mirroring
`firestore-write-streaming`'s own placement convention for `WriteRequest`):

```protobuf
// A request for [Firestore.BatchWrite][google.firestore.v1.Firestore.BatchWrite].
message BatchWriteRequest {
  // Required. The database name. In the format:
  // `projects/{project_id}/databases/{database_id}`.
  string database = 1;

  // The writes to apply. Unlike `Commit`, this is NOT applied atomically —
  // each write applies (or fails) independently (docs/SPEC.md §BatchWrite).
  repeated Write writes = 2;
}

// The response for [Firestore.BatchWrite][google.firestore.v1.Firestore.BatchWrite].
message BatchWriteResponse {
  // The result of applying each write. Always the same length as the
  // request's own `writes`, positionally aligned. Entry i is always present
  // (an empty WriteResult on failure), never absent.
  repeated WriteResult write_results = 1;

  // The status of applying each write. Always the same length and position
  // as `write_results`. A `Status{code: 0}` (OK) entry means that write
  // succeeded; any other code means it failed. There is no separate
  // top-level RPC error for a per-write failure — see § Decision Driver 3.
  repeated google.rpc.Status status = 2;
}
```

`proto/google/firestore/v1/firestore.proto`, `service Firestore`, added after
`Write` (mirrors this codebase's own existing grouping of mutation-shaped RPCs
together, before the query-shaped ones):

```protobuf
  // Applies a batch of writes, each independently, without an all-or-nothing
  // transaction. No top-level error is ever returned — all failures are
  // reported per-write in the response's own `status` array.
  rpc BatchWrite(BatchWriteRequest) returns (BatchWriteResponse);
```

`import "google/rpc/status.proto";` is already present in `firestore.proto`
(used today by `TargetChange.cause`, line 12/390) — zero new import required,
confirmed by DISCUSS's own Reading Confirmation and re-confirmed here.

**Residual, non-blocking** (same treatment as ADR-038/046's own residuals):
field numbers above match this architect's moderate-to-high-confidence
recollection of real Firestore's own public `BatchWriteRequest`/
`BatchWriteResponse` proto shape. `labels` (a `map<string, string>` field
present on real Firestore's `BatchWriteRequest`) is deliberately NOT included
— `docs/SPEC.md` does not document it, mirroring `Write`'s own identical
`labels` omission decision (ADR-046 § Decision 1, same YAGNI reasoning). Add
it if a real-client integration test surfaces a need, not preemptively.

### 2. Per-call sequence (`handle_batch_write`, once per call — unary, not a session)

Mirrors `handle_commit`'s own auth/rate-limit/suspension sequence exactly
(`handler.rs:1447-1465`): extract `project_id`/`api_key` → `rate_limiter.check`
→ `authenticate` (+ suspension check) → `attach_client_identity_if_present`.

**New validation, before any write-level work begins** (mirrors
`handle_batch_get_documents`'s own `DDD-BGD-14` precedent,
`handler.rs:1283-1289`, which bounds `documents.len()` to 1000 for the
identical reason — a single rate-limit token must not purchase unbounded
per-call resource consumption):

```
if req.writes.len() > 500 {
    return Err(Status::invalid_argument("writes must not exceed 500 per call"));
}
```

500, not 1000 (`BatchGetDocuments`' own cap), because it matches real
Firestore's own documented per-call write limit for `BatchWrite` (identical to
`Commit`'s own limit) — moderate-to-high confidence recollection, same
residual treatment as § Decision 1's field numbers. Applied UNIFORMLY to every
`backend_mode`, including `agent` — see ADR-049 for the reasoning that this
general cap, not a backend-mode-specific threshold, is this feature's sole
latency mitigation for agent-mode.

**Empty batch** (AC-01-04): `req.writes.is_empty()` short-circuits to an
immediate `BatchWriteResponse{write_results: vec![], status: vec![]}`, before
the 500-cap check even matters (an empty batch trivially satisfies it, but the
early return keeps the happy path for the walking skeleton's simplest case
free of any per-write machinery).

### 3. Per-write loop body (the call's own loop, not a spawned task — unary RPC)

For each `Write` in `req.writes`, in request order:

1. **Translate and evaluate**: call the new `translate_writes_catching`
   (§ Decision 4) once for the WHOLE batch up front, returning
   `Vec<Result<DomainWrite, Status>>` — one entry per input write, positionally
   aligned, never short-circuiting (this differs from `handle_commit`'s own
   single `?`-propagating call to `translate_writes_for_commit`, by design).
2. **For each `Ok(domain_write)`**: call
   `adapter.begin_transaction(&project_id, TransactionOptions::ReadWrite)`,
   then, on success, `adapter.commit_transaction(&project_id, &txn_id,
   vec![domain_write])` — a single-element `writes` vec, per write, per §
   Context finding 1. Both calls reuse the ALREADY-SHIPPED, UNCHANGED
   `BackendAdapter` port methods; no new trait method.
3. **On success** (`commit_transaction` returns `Ok(vec![write_result])`):
   push the translated `WriteResult` at this write's own position in
   `write_results`, push `Status{code: 0, message: "", details: vec![]}` (OK)
   at the same position in `status`.
4. **On any failure** — from translation (`Err(Status)` at step 1), from
   `begin_transaction` (`Err(CoreError)`), or from `commit_transaction`
   (`Err(CoreError)`, including a precondition violation,
   `CoreError::TransactionAborted`/`FailedPrecondition`): push an EMPTY
   `WriteResult{update_time: None, transform_results: vec![]}` at this write's
   own position, and push `core_error_to_status(e)` (or the translation's own
   already-a-`Status` value, used as-is) converted to `embyr_proto::rpc::Status`
   via a new small helper, `status_to_proto` (§ Decision 5), at the same
   position. **The loop CONTINUES to the next write — no `?`, no early
   `return`, anywhere inside this loop body** (§ Decision Driver 3).
5. **After the loop**: build `BatchWriteResponse{write_results, status}` —
   always exactly `req.writes.len()` entries in both arrays, positionally
   aligned to the request (AC-01-01, AC-02-05). Return `Ok(Response::new(...))`
   — the RPC call itself ALWAYS succeeds once past pre-loop validation, even
   if every write in the batch failed (AC-02-03).

### 4. Translate-and-catch variant — shared-helper extraction, not a duplicate copy

`translate_writes_for_commit` (`handler.rs:659-747`) is refactored, NOT
rewritten: its existing per-write match body (the `Update`/`Delete`/
`Transform` arms, each ending in `evaluate_write_rule_for_commit(...).await?`)
is extracted verbatim into a new private helper:

```rust
async fn translate_one_write_for_commit(
    system_db: &SystemDb,
    adapter: &SharedBackendAdapter,
    project_id_str: &str,
    verified_identity: Option<&embyr_core::client_identity::VerifiedEndUserIdentity>,
    proto_write: &embyr_proto::firestore::Write,
) -> Result<DomainWrite, Status>
```

`translate_writes_for_commit` becomes a thin loop calling the helper with `?`
per write — **its own external signature and behavior are completely
unchanged**, so `handle_commit` and `write_stream.rs` (both existing callers)
require zero call-site changes:

```rust
pub(crate) async fn translate_writes_for_commit(...) -> Result<Vec<DomainWrite>, Status> {
    let mut domain_writes = Vec::with_capacity(proto_writes.len());
    for proto_write in proto_writes {
        domain_writes.push(
            Self::translate_one_write_for_commit(system_db, adapter, project_id_str, verified_identity, proto_write).await?,
        );
    }
    Ok(domain_writes)
}
```

A new, additive sibling function serves `BatchWrite` alone:

```rust
pub(crate) async fn translate_writes_catching(
    system_db: &SystemDb,
    adapter: &SharedBackendAdapter,
    project_id_str: &str,
    verified_identity: Option<&embyr_core::client_identity::VerifiedEndUserIdentity>,
    proto_writes: &[embyr_proto::firestore::Write],
) -> Vec<Result<DomainWrite, Status>> {
    let mut results = Vec::with_capacity(proto_writes.len());
    for proto_write in proto_writes {
        results.push(
            Self::translate_one_write_for_commit(system_db, adapter, project_id_str, verified_identity, proto_write).await,
        );
    }
    results
}
```

Zero write-semantics logic exists in two places — a future change to how a
`Transform` write is translated, for instance, edits `translate_one_write_for_commit`
once and is correct for `Commit`, `Write`, and `BatchWrite` alike.

### 5. Status wire representation — `Status{code: 0}`, not an absent entry

`status[i]` for a successful write is `google.rpc.Status{code: 0, message:
"", details: vec![]}` (`code: 0` is `google.rpc.Code.OK`), not an omitted or
`null` protobuf value (§ Decision Driver 5 — proto3 cannot represent that for
a `repeated message` field). `docs/SPEC.md`'s own "`status[i] = null`" wording
describes the SDK-level projection after decode (a real Firebase SDK maps
`code: 0` to a `null`/no-error result for that write), not the wire encoding.
A new small helper converts any `tonic::Status` to the wire type:

```rust
fn status_to_proto(status: Status) -> embyr_proto::rpc::Status {
    embyr_proto::rpc::Status {
        code: status.code() as i32,
        message: status.message().to_string(),
        details: vec![],
    }
}
```

This is the first real producer of a populated `embyr_proto::rpc::Status`
anywhere in this codebase — `TargetChange.cause` (the only other call site of
this type) is declared but never populated today (confirmed by grep, zero
assignment sites found).

### 6. Transaction-row hygiene — named, not fixed

A per-write `commit_transaction` failure leaves that write's own synthetic
`transactions` row at `status = 'active'` forever (§ Context finding 2). This
ADR does **not** add cleanup logic (no new `rollback_transaction` call on
failure, no new sweeper) — doing so would be new, unscoped, unrequested
behavior extending past a pre-existing, inherited gap `Commit`/`Write` already
have. It is named explicitly here, and in § Consequences, as a candidate
follow-up (a `transactions`-table sweeper, mirroring the project soft-delete
sweeper convention this codebase already has for a different table,
`docs/product/architecture/brief.md`/CLAUDE.md "Soft-delete: projects →
deleted_at, sweeper after 168h") — not this feature's own job, and not a
blocker, since these synthetic per-write transaction IDs are never returned to
any caller and are therefore never looked up again by anyone.

## Alternatives Considered

### Per-write transaction loop shape

**A. Single `commit_transaction` call for the whole batch.** Rejected:
directly contradicted by § Context finding 1 (ground-truth confirmed
all-or-nothing) — would silently give `BatchWrite` `Commit`'s own semantics,
failing the RPC's entire reason for existing.

**B. Parallel/concurrent `begin_transaction`+`commit_transaction` pairs
(e.g., `futures::future::join_all` over the batch) instead of a sequential
loop.** Considered, for latency. Rejected for v1: no evidence this codebase's
Postgres connection pool is sized to safely support up to 500 concurrent
transactions from a single call without contention or pool exhaustion — a new
capacity question with no benchmark to answer it, and inventing a "safe"
concurrency limit would repeat the exact unevidenced-number problem this
session's own agent-mode escalation was explicitly told to avoid (ADR-049).
The walking-skeleton discipline (Slice 01/02) favors the simplest loop shape
proven correct first; named as a candidate DEVOPS/performance follow-up once
the sequential version's actual latency is measured, not designed blind here.

**C. Sequential per-write loop (chosen).** Matches ADR-046's own proven
`begin_transaction`-before-`commit_transaction` synthesis pattern, applied at
finer (per-write) granularity; zero new port method; zero new concurrency-safety
question to answer without evidence; correctness-first, matching this
feature's own two-slice sequencing (prove it works, then prove isolation holds
under failure).

### Translate-and-catch shape

**A. Duplicate the whole per-write match/evaluate body into two independent
functions** (one short-circuiting via `?`, one catching every `Result`).
Rejected: violates this session's own standing Reuse Analysis discipline
(share logic, do not duplicate it); a future change to write-translation
semantics (e.g., wiring the OCC `version` field, a named pre-existing gap)
would require two synchronized edits forever, an ongoing maintenance tax for
zero benefit.

**B. Extract a shared per-write helper, called by two thin wrappers (chosen).**
Zero duplicated write-semantics logic. `translate_writes_for_commit`'s own
external signature and behavior, for its EXISTING callers
(`handle_commit`, `write_stream.rs`), are completely unchanged — this is a
pure additive refactor, not a breaking one. Mirrors the exact extraction
discipline this codebase already applied once before (`translate_writes_for_commit`
was itself extracted from `handle_commit`'s own inline body during
`firestore-write-streaming` so `Write` could share it, ADR-046 § Decision 3) —
a continuation of an established pattern, not a new architectural style.

**C. Add a `catch: bool` parameter to `translate_writes_for_commit` itself,
branching its own control flow internally.** Rejected: conflates two
genuinely different CONTRACTS (`Result<Vec<T>, Status>` for the short-circuit
case vs. `Vec<Result<T, Status>>` for the catching case) into one signature —
every existing caller (`handle_commit`, `write_stream.rs`) would need to
either pass a literal `false` forever or be touched by this feature for no
functional reason. Two thin wrappers sharing one helper achieve the identical
code-sharing goal with a correct type for each caller's own actual need.

### Status wire representation for a successful write

**A. Omit `status[i]` entirely for a successful write, relying on `prost`'s
default-value elision on the wire.** Rejected: `repeated` message fields do
not support a sparse/index-skipping representation in protobuf — every
successful write would need EITHER a genuinely missing array entry (breaking
the positional-alignment invariant, § Decision Driver 2, since a consumer
could no longer index `status[i]` reliably against `write_results[i]`) or the
same `Status{code: 0}` this ADR already chooses; there is no third wire-legal
option that preserves alignment.

**B. `Status{code: 0, message: "", details: vec![]}` for success (chosen).**
Wire-legal, preserves positional alignment exactly, matches proto3's own
actual constraints (§ Decision Driver 5), and is the literal encoding a real
Firestore server almost certainly uses for the identical reason (a `repeated
message` field cannot carry a sparse `null`).

## Consequences

**Positive**: correctness-first design, provably composed of already-shipped,
already-proven primitives (`BackendAdapter::begin_transaction`/
`commit_transaction`, unchanged; `evaluate_write_rule_for_commit`, unchanged;
`core_error_to_status`, unchanged). Zero new `BackendAdapter` trait method,
zero new `CoreError` variant. Shared per-write translation logic
(`translate_one_write_for_commit`) keeps `Commit`/`Write`/`BatchWrite`'s
write-semantics identical by construction — a future semantic change updates
one function, not three. `docs/SPEC.md`'s own "`status[i] = null`" shorthand
is disambiguated into an exact, unambiguous wire encoding before DELIVER has
to guess.

**Negative**: `BatchWrite` issues N sequential Postgres round trips per call
(one `begin_transaction` + one `commit_transaction` pair per write) instead of
`Commit`'s single `pg_txn`. A batch at the 500-write cap issues 1000 round
trips in one call — a real, accepted latency cost, and matching real
Firestore's own actual documented server-side behavior (`docs/SPEC.md`
§BatchWrite: "each write runs in its own transaction" — applies identically
server-side for real Firestore too), not a defect unique to this
implementation.

**Negative, named explicitly**: a per-write `commit_transaction` failure
leaves its synthetic `transactions` row at `status = 'active'` indefinitely
(§ Decision 6) — a pre-existing gap `Commit`/`Write` already have, now
exercised at up to Nx frequency (up to 500x, at the cap) per `BatchWrite`
call instead of at most 1x per `Commit`/`Write` call. Named as a candidate
follow-up (a `transactions`-table sweeper), not fixed here, not blocking (the
rows are never looked up again by any caller).

**Residual, non-blocking**: the 500-write cap and the exact
`BatchWriteRequest`/`BatchWriteResponse` field numbers (§ Decision 1) are
moderate-to-high-confidence recollection of real Firestore's own public proto
shape and documented limits, not verified against a live system in this
sandbox — same treatment as ADR-038/046's own residuals.
