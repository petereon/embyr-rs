# ADR-046: `Write` Stream Wire Contract, Session Lifecycle, and Error Model

## Status

Accepted

## Context

`firestore-write-streaming` (JOB-01) adds the `Write` bidirectional-streaming RPC
to the client-facing `google.firestore.v1.Firestore` service — genuinely new
proto surface (`docs/feature/firestore-write-streaming/feature-delta.md` §
Reading Confirmation: no `WriteRequest`/`WriteResponse` message exists in the
vendored proto tree today). `docs/SPEC.md` §Write Stream (lines 705-724)
already documents the high-level 3-step protocol (handshake, `stream_id`/
`stream_token` issuance, write loop) and the 3-way termination taxonomy
(`io.EOF` clean, `codes.Canceled` clean, any other error propagates). DISCUSS
escalated one open question (Resolution 3 / US-03: what happens on a
`stream_token` mismatch) and left one ambiguity unresolved in US-01's own
Domain Example 3 (does a single write's precondition violation terminate the
stream or behave as a recoverable per-message rejection).

DESIGN's own ground-truth reading of `crates/embyr-server/src/grpc/handler.rs`
and `crates/embyr-pg-storage/src/backend_adapter.rs` surfaces two further
findings not captured by DISCUSS's own Reading Confirmation, both load-bearing
for Slice 01's implementability:

1. **`commit_transaction` requires a pre-existing, `active`-status transaction
   row.** `PostgresBackendAdapter::commit_transaction`
   (`crates/embyr-pg-storage/src/backend_adapter.rs:851-1010`) begins by
   parsing `transaction_id.0` as a 16-byte UUID (`uuid_from_bytes`, hard error
   on any other length) and looking up a matching row in the `transactions`
   table with `status = 'active'`, erroring `CoreError::TransactionNotFound`
   if absent. There is no "ad-hoc, transactionless atomic batch" path in this
   codebase today. `handle_commit`'s own reuse of this primitive works today
   only because a real client always supplies a `transaction` field sourced
   from a prior `BeginTransaction` call (or, in the all-empty-bytes case, would
   itself hit this same `TransactionNotFound` — an existing, unexercised edge
   `Commit`'s own test suite evidently does not cover, out of scope to fix
   here). `Write`'s own wire contract has no client-visible transaction field
   at all — DISCUSS's Resolution 3 established each `WriteRequest` batch is
   its OWN independent atomic apply, not a step in a client-visible
   transaction. This means the per-message loop must synthesize an ad-hoc,
   client-invisible transaction via the ALREADY-SHIPPED
   `BackendAdapter::begin_transaction` port method immediately before each
   `commit_transaction` call — a new CALLER of an existing, unchanged port
   method, not new domain logic, but a genuine refinement of DISCUSS's own
   "reused unchanged" framing that Slice 01's implementer needs to know
   explicitly, not rediscover via a failing test.
2. **`handle_commit` performs zero write-path access-rule evaluation.**
   `handle_commit` (`handler.rs:1295-1379`, read in full) never calls
   `get_write_access_rule`/`access_control::evaluate()` — confirmed by direct
   grep across `handler.rs` for both symbols, which only match inside
   `handle_create_document`/`handle_update_document`/`handle_delete_document`
   (ADR-030, `security-rules-write-path`). This means DISCUSS's Resolution 2
   ("access-control evaluation... mirrors `handle_create_document`/
   `handle_update_document`/`handle_delete_document`'s own per-write
   evaluation") named the wrong reuse target for the mechanism it describes —
   the mechanism it's describing lives in the three single-document handlers,
   not in `handle_commit`, which is `Write`'s own actual, DISCUSS-directed
   reuse target for translation/apply. `Commit` and the three single-document
   handlers already diverge on this exact axis today, independent of this
   feature.

## Decision Drivers

1. **Reuse over invention (standing session practice)** — `Write`'s own value
   proposition is a new *transport* for already-shipped mutation-apply logic
   (DISCUSS § Journey, "Shared artifact"). Any new composition this ADR adds
   must be composition of EXISTING port methods, not new domain logic.
2. **`docs/SPEC.md`'s own termination taxonomy is exhaustive, not
   illustrative** — three cases only (`io.EOF`, `codes.Canceled`, any other
   error terminates). No fourth "recoverable per-message rejection, stream
   stays open" shape is documented for `Write` anywhere in the vendored spec.
   That shape belongs exclusively to `BatchWrite` (`docs/SPEC.md` §Errors,
   already confirmed out of scope, DISCUSS § Reading Confirmation) — a
   separate, still-undeclared RPC with its own per-write non-terminating
   `status` array. Conflating the two would import a documented-elsewhere
   mechanism into an RPC whose own spec explicitly does not have it.
3. **Status-code family consistency with this codebase's own established
   convention** — `core_error_to_status` (`handler.rs:2362-2376`) already maps
   `CoreError::OccConflict`/`CoreError::TransactionAborted` (optimistic
   sequencing/version-check failures) to `Status::aborted`, and
   `CoreError::FailedPrecondition` (state-precondition violations) to
   `Status::failed_precondition`. Any new rejection this feature introduces
   should slot into this existing taxonomy by matching the SEMANTIC family of
   the failure, not invent a fourth, uncatalogued code.
4. **Minimal new state** — Slice 03's own learning hypothesis names the
   confirms-if-succeeds outcome explicitly: `stream_token` continuity should
   be "a purely in-session, in-memory concern — no new storage or cross-request
   coordination." Any design requiring a shared registry keyed by `stream_id`
   is over-built relative to what the SPEC documents.
5. **Do not silently widen enforcement beyond what this feature's own reuse
   target already does** — `Write` reuses `handle_commit`'s translation/apply
   primitive by DISCUSS's own explicit direction; enforcing write-path access
   rules on `Write` when `Commit` itself does not would make `Write` *stricter*
   than the very RPC it is modeled on, letting a client trivially bypass any
   new enforcement by using `Commit` instead — a real security-equivalence
   gap this feature must not silently paper over by inventing enforcement
   `Commit` itself lacks.

## Decision

### 1. Proto additions

`proto/google/firestore/v1/write.proto` (alongside `CommitRequest`, per
DISCUSS's own Technical Notes placement):

```protobuf
// A request for [Firestore.Write][google.firestore.v1.Firestore.Write].
message WriteRequest {
  // Required. The database name. Present on every message (handshake and
  // subsequent), matching real Firestore's own wire shape.
  string database = 1;

  // The ID of the write stream, issued by the first WriteResponse. Empty on
  // the first (handshake) message; must echo the issued stream_id on every
  // subsequent message.
  string stream_id = 2;

  // The writes to apply. Empty on the first (handshake) message — a non-empty
  // `writes` on the handshake message is rejected (AC-01-04).
  repeated Write writes = 3;

  // A token from the most recent WriteResponse, used to ensure ordered
  // request delivery. Empty on the first (handshake) message; must echo the
  // most-recently-issued stream_token on every subsequent message (Slice 03).
  bytes stream_token = 4;
}

// The response for [Firestore.Write][google.firestore.v1.Firestore.Write].
message WriteResponse {
  // The ID of the write stream, issued once at handshake and echoed
  // unchanged on every subsequent response.
  string stream_id = 1;

  // A token to use on the next WriteRequest. Rotates on every response.
  bytes stream_token = 2;

  // The result of applying the writes. Empty on the handshake response (no
  // writes were submitted).
  repeated WriteResult write_results = 3;

  // The time at which the commit occurred. Set on every response, including
  // the handshake response (docs/SPEC.md §Write Stream, step 2).
  google.protobuf.Timestamp commit_time = 4;
}
```

`proto/google/firestore/v1/firestore.proto`, `service Firestore`, added after
`Rollback` (matching this codebase's existing grouping of mutation-shaped
RPCs before the query-shaped ones):

```protobuf
  // Streams batches of document writes, in order, applying each batch
  // atomically and replying once per batch.
  rpc Write(stream WriteRequest) returns (stream WriteResponse);
```

**Residual, non-blocking** (same treatment as ADR-038's own field-number
residual): field numbers above are chosen to match this architect's
moderate-to-high-confidence recollection of real Firestore's own public
`WriteRequest`/`WriteResponse` proto shape. `labels` (a `map<string, string>`
field present on real Firestore's `WriteRequest`) is deliberately NOT
included — `docs/SPEC.md` does not document it, no evidence any client
requires it for this codebase's own SDK-parity goal, and proto3 tolerates a
real SDK sending an undeclared field silently (unknown-field skip on decode).
Add it if a real-client integration test surfaces a need (YAGNI), not
preemptively.

### 2. Session composition (handshake, once per stream)

`handle_write` mirrors `handle_listen`'s own scaffold exactly
(`handler.rs:1913-2042`): `request.get_mut().next()` to peek the handshake
message → reject if `writes` or `stream_id` is non-empty (AC-01-04) →
`rate_limiter.check` → `authenticate` (+ suspension check, AC-01-06) →
`attach_client_identity_if_present` (each exactly once, AC-01-05) →
`request.into_inner()` + `tokio::spawn` a task owning the `Streaming<WriteRequest>`
body and an `mpsc::Sender<Result<WriteResponse, Status>>`, wrapped as
`ReceiverStream` for the response.

**Access-control correction (supersedes DISCUSS Resolution 2's own reuse-target
premise, not its intent):** no `get_write_access_rule`/`evaluate()` call is
added to `Write`, at handshake or per-message. `Write`'s own actual, DISCUSS-
directed reuse target (`handle_commit`) has zero write-rule enforcement today
— confirmed by ground-truth read, § Context above. Adding enforcement here
that `Commit` itself lacks would (a) be new write-semantics code, contradicting
this feature's own "zero new write-semantics code" framing, and (b) create a
false sense of protection trivially bypassed via `Commit`. This is named as a
THIRD item in the existing "inherited, not introduced" gap family alongside
OCC `version`/`DocumentTransform.field_transforms` (DISCUSS § System
Constraints) — a cross-cutting follow-up shared by `Commit` and `Write` alike
(extend `security-rules-write-path`'s own enforcement to `Commit`, which then
flows to `Write` for free), not this feature's own job.

### 3. Per-message write loop (the spawned task's own loop body)

New file `crates/embyr-server/src/grpc/write_stream.rs` (sibling to
`handler.rs`, not under `realtime/` — `Write` is BC-2's own mutation-apply
mechanism; `realtime/` is BC-3's Listen-specific module, reused here only for
its scaffold SHAPE, not as a shared domain module, per DISCUSS § Reading
Confirmation's own explicit distinction).

For each `WriteRequest` received:

1. **Token check first, before any translation or apply** (Slice 03): compare
   `msg.stream_token` byte-for-byte against the current token held in a local
   `let mut` binding owned by the spawned task itself — NOT a shared registry.
   No `stream_id`-keyed `HashMap` anywhere in this codebase for this feature;
   `stream_id` is an opaque echo value with no server-side lookup use (Decision
   Driver 4). On mismatch: build `Status::aborted("stream_token mismatch: a
   WriteRequest must present the most recently issued stream_token")`, send via
   the `mpsc::Sender`, break the loop (§ Decision 4 below — same exit path as
   every other terminal error).
2. **Translate**: reuse `handle_commit`'s own per-`Write`-message translation
   logic (`handler.rs:1319-1351`) unchanged, extracted into a shared, callable
   function reachable from both `handle_commit` and this new loop (exact
   extraction mechanics — free fn vs. `pub(crate)` associated fn — is DELIVER's
   own call, not prescribed here).
3. **Begin an ad-hoc transaction** (§ Context finding 1): call
   `adapter.begin_transaction(&project_id, TransactionOptions::ReadWrite)` to
   obtain a synthetic, client-invisible `TransactionId` — a new CALLER of the
   already-shipped port method, immediately consumed by the next step, never
   exposed on the wire (`Write`'s own proto has no transaction field).
4. **Apply**: call `adapter.commit_transaction(&project_id, &txn_id,
   domain_writes)`, identical to `handle_commit`'s own call shape.
5. **On success**: build `WriteResponse{stream_id, stream_token: <rotated>,
   write_results, commit_time}`, rotate the locally-held token, send via
   `mpsc::Sender`, continue the loop.
6. **On failure** (`CoreError` from step 4, including
   `CoreError::FailedPrecondition` for a precondition-violating write): map via
   the EXISTING, unchanged `core_error_to_status` (`handler.rs:2362-2376`) —
   `Status::failed_precondition` for a precondition violation, reused verbatim
   from `Commit`'s own established mapping (US-01 Domain Example 3's own
   "same mechanism `Commit` already uses" framing, honored literally). Send via
   `mpsc::Sender`, break the loop (§ Decision 4).

### 4. Termination and error model — one exit mechanism, three triggers

**Both open questions (Escalation 1 and US-01's own precondition-termination
ambiguity) resolve to the SAME answer, by the SAME mechanism, for the SAME
reason: `docs/SPEC.md`'s own termination taxonomy has no documented
"recoverable, stream-stays-open" shape for `Write` (Decision Driver 2). A
`stream_token` mismatch and a precondition violation are therefore both "any
other error" under that taxonomy — both TERMINATE the whole stream.**

There is exactly one error-exit path in the spawned task's loop: build a
`Status`, `tx.send(Err(status)).await`, then `return`/`break` — ending the
task, which drops `tx`, which closes the response stream to the client. Three
triggers reach this ONE path:

- `stream_token` mismatch → `Status::aborted` (§ Decision 3 below).
- A write's precondition violation, or any other `commit_transaction` failure
  → whatever `core_error_to_status` already produces, unchanged (typically
  `Status::failed_precondition` for preconditions, `Status::internal` for a
  lost DB connection, etc.).
- `Streaming::next()` yields `Some(Err(status))` with any code other than
  `Cancelled` → the received `status` is propagated as-is.

Two triggers instead return cleanly (task ends, no `Err` sent, `tx` simply
drops — closing the stream with no error, matching real gRPC clean-close
semantics):

- `Streaming::next()` yields `None` (client `io.EOF`).
- `Streaming::next()` yields `Some(Err(status))` with `status.code() ==
  Cancelled`.

**Slice ordering consequence, made explicit**: Slice 03's own "compare-and-
reject" is a small addition to the loop's entry point that reuses the EXACT
error-exit mechanism Slice 04 builds for "genuine server-side apply error" —
it does not need its own bespoke termination handling. This makes Slice 04
landing before Slice 03 (already DISCUSS's own prioritization, § Prioritization)
a structural dependency, not just a value-priority preference: Slice 03's own
implementation is most simply expressed as one new `if` at the top of a loop
whose error-exit path Slice 04 already proved correct.

**No stream-count registry** (AC-04-04): `Write`, unlike `Listen`, needs no
shared cross-stream state (no Postgres NOTIFY fan-out, no `active_listeners`-
style map) — every session's state is 100% local to its own spawned task.
"No leaked resources" reduces to "the loop actually returns in every
condition above," verified by an integration test asserting the `tokio::spawn`
handle completes (or the mpsc receiver closes) after each of US-04's three
scenarios — no new bookkeeping component is introduced to satisfy this AC
(ponytail: a counter nobody's specified a consumer for is speculative scope).

### `stream_id`/`stream_token` generation (Slice 01, informative format from
`docs/SPEC.md`, DESIGN's own exact mechanism)

- `stream_id`: `format!("{:016x}", <unix nanoseconds>)` at handshake, held
  constant for the session's lifetime, echoed unchanged on every response.
  Never used as a lookup key (Decision Driver 4) — a same-nanosecond collision
  across two concurrent handshakes has zero behavioral consequence.
- `stream_token`: `Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true)`
  (RFC3339Nano, matching `docs/SPEC.md`), UTF-8 bytes into the `bytes
  stream_token` field — opaque to any real client, exactly like `Listen`'s own
  `resume_token` (also `bytes`-typed despite carrying a custom internal
  structure). Regenerated on every response, including the handshake response.

## Alternatives Considered

### Escalation 1 — `stream_token` mismatch mechanism

**A. Per-message rejection, stream stays open (chosen candidate shape from
Slice 03's own framing, option (a)).** Rejected: requires inventing a fourth,
undocumented termination shape for `Write` not present anywhere in
`docs/SPEC.md`; the only RPC in this codebase's own spec with that shape
(`BatchWrite`) is a structurally different, explicitly out-of-scope RPC.
Importing its behavior into `Write` would create a real behavioral
inconsistency between what this codebase documents for `Write` and what it
implements.

**B. Whole-stream termination, `Status::failed_precondition`.** Considered:
`FailedPrecondition` is this codebase's own convention for state-precondition
violations, and a token mismatch is arguably "the session is not in the state
this request assumes." Rejected in favor of C: a `stream_token` mismatch is
structurally a SEQUENCER check failure (the token rotates once per response,
functioning as an implicit request-ordering counter) — gRPC's own canonical
`ABORTED` semantics ("typically due to a concurrency issue such as a sequencer
check failure") describes this exact shape more precisely than
`FAILED_PRECONDITION`'s ("system state precondition not met, do not retry
blindly"), and `ABORTED`'s documented client action ("retry the entire
higher-level operation") maps directly onto this feature's own intended
recovery path — open a fresh handshake and resend, not "fix some external
state and resend the identical request" (`FAILED_PRECONDITION`'s implied
action). Reusing `FAILED_PRECONDITION` here would also collide, at the
STATUS-CODE level, with the ALREADY-CHOSEN mapping for precondition-violating
writes inside the same stream (§ Decision 3) — a client's error-handling logic
could no longer distinguish "my document write violated a precondition" from
"my session token was stale" without inspecting the message string, discarding
a real, useful distinction gRPC status codes exist to carry.

**C. Whole-stream termination, `Status::aborted` (chosen).** Matches the
sequencer-check-failure semantic family exactly; reuses this codebase's own
established status-code convention for that family
(`CoreError::OccConflict`/`TransactionAborted` → `Status::aborted`) WITHOUT
reusing or conflating the underlying OCC MECHANISM itself (DISCUSS Resolution
3's own non-conflation requirement — this is a status-code-family reuse, not a
`version`-column/`BeginTransaction` reuse); gives the client a status
distinguishable from a precondition violation; requires zero new
`CoreError` variant (the rejection is constructed directly as a `Status` inside
`write_stream.rs`, never passing through `CoreError` at all, since it is
detected before any port call is made).

### US-01 precondition-violation termination question

**A. Recoverable per-message rejection (stream stays open).** Rejected for
the identical reason as Escalation 1's Alternative A — no such shape is
documented for `Write`; it belongs to `BatchWrite` alone.

**B. Whole-stream termination, reusing `core_error_to_status` unchanged
(chosen).** Directly satisfies US-01 Domain Example 3's own framing ("the same
mechanism `Commit` already uses for precondition violations") literally — the
SAME function, not a re-derived mapping — while correctly generalizing
`Commit`'s own unary-RPC "the call fails" into streaming's own equivalent,
"the stream terminates with that status."

## Consequences

**Positive**: one unified error-exit mechanism serves three distinct
triggers (token mismatch, precondition violation, generic apply failure),
minimizing new code and giving Slice 04 (built first, per DISCUSS's own
prioritization) a genuine structural reason to land before Slice 03, not just
a value-priority one. Zero new `CoreError` variants. Zero new cross-request
shared state. `stream_token` mismatch and precondition violations are
status-code-distinguishable from each other. The `begin_transaction`-before-
`commit_transaction` finding is surfaced explicitly, preventing a
`TransactionNotFound` surprise mid-Slice-01 implementation.

**Negative**: every non-EOF/Cancel error, including a single mismatched token
on an otherwise-healthy session, tears down the whole stream — a real,
accepted cost matching `docs/SPEC.md`'s own documented contract (the SDK's own
reconnect logic is expected to open a fresh handshake, per this feature's own
Domain Examples and Emotional Arc framing), not a design shortfall. A future
`BatchWrite` implementation would need its own, structurally different,
per-write non-terminating error path — explicitly not reusable from this
design, and not attempted here.

**Negative, named explicitly**: `Write` inherits `Commit`'s own pre-existing
absence of write-path access-rule enforcement (§ Decision 2). A collection
with a write rule defined is enforced for `CreateDocument`/`UpdateDocument`/
`DeleteDocument` but not for `Commit` or (after this feature ships) `Write` —
an existing inconsistency this feature does not introduce but does propagate
to a second RPC. Flagged as a cross-cutting follow-up candidate, not this
feature's own scope.

**Residual, non-blocking**: exact real-Firestore field numbers for
`WriteRequest`/`WriteResponse` (§ Decision 1) are moderate-to-high confidence,
not verified against a live system in this sandbox — same treatment as
ADR-038's own residual.
