# ADR-060: Agent-Mode `Write` Requires No New Agent Proto Surface — Corrects ADR-047's Capability-Gap Claim

## Status

Accepted

## Context

`agent-mode-write-streaming`'s own DISCUSS (`docs/feature/agent-mode-write-streaming/feature-delta.md`)
escalated Escalation 1 on the premise, inherited from ADR-047
(`firestore-write-streaming`'s own DESIGN wave, 2026-08-30), that closing this
gap requires "a net-new `rpc WriteStream(stream WriteRequest) returns (stream
WriteResponse);` declaration on `storage_agent.proto` itself... a new
generated client stub... a new `AgentBackendAdapter` method... and a
corresponding new `StorageAgentService` method in
`crates/embyr-agent/src/server.rs`." DISCUSS's own Slice 01 brief names this
as its own top learning hypothesis to disprove: "a bidi-streaming RPC cannot
actually be added to the agent's own tonic-based gRPC surface without
breaking the existing mTLS `ServerTlsConfig`/`add_service` wiring."

This ADR's own ground-truth re-verification (this feature's DESIGN wave, not
trusting ADR-047's claim unexamined — the same discipline ADR-046/047/049
themselves apply to DISCUSS) finds the premise does not hold. **No new RPC,
message, or agent-binary handler is needed.** The mechanism `firestore-write-streaming`
already built and shipped (ADR-046) is, by construction, already
backend-agnostic, and `firestore-batch-write`'s own ADR-049 already
established — for a structurally identical mechanism — that this exact
composition works unmodified for `backend_mode=agent`. ADR-047's own
conclusion did not connect these two facts; this ADR does.

**Numbering note**: originally drafted as `adr-056`/`adr-057`, renumbered to
`adr-060`/`adr-061` after a collision with two concurrently-running sibling
DESIGN waves (`agent-mode-list-collection-ids`, renumbered `056`→`059`;
`agent-mode-field-transforms`, landed at `057`) — same renumbering pattern
those two features' own ADRs already document. The stray, superseded
`adr-056`/`adr-057` files for THIS feature have been overwritten with a
short redirect stub, not left as silent duplicates.

## Verification (ground-truth, not trusted from ADR-047's own claim)

Read in full or targeted, directly, not re-derived from any prior wave's
summary:

1. **`crates/embyr-server/src/grpc/write_stream.rs`** (full, 212 lines,
   `firestore-write-streaming` Slice 01, ADR-046) — the spawned task's own
   per-`WriteRequest` loop calls exactly two `BackendAdapter` trait methods
   per message: `adapter.begin_transaction(&project_id,
   TransactionOptions::ReadWrite)` then `adapter.commit_transaction(&project_id,
   &txn_id, domain_writes)`, where `adapter: SharedBackendAdapter = Arc<dyn
   BackendAdapter + Send + Sync>` (dependency-inverted, resolved once at
   handshake by `authenticate()`, held by the spawned task for the session's
   lifetime). **No streaming-shaped port method is called anywhere in this
   file.** The client-facing bidi stream is entirely a property of
   `handle_write`'s own `tonic::Streaming<WriteRequest>`/`mpsc::Sender`
   scaffold (mirroring `handle_listen`, ADR-046 § Decision 2) — the loop
   *driving* that stream issues a sequence of ordinary unary port calls, one
   `begin_transaction`+`commit_transaction` pair per `WriteRequest`,
   regardless of which concrete adapter `authenticate()` selected.
2. **`crates/embyr-server/src/adapters/agent_backend.rs`, lines 512-591**
   (`begin_transaction`, `commit_transaction`) — both are fully implemented,
   not placeholders: `begin_transaction` proxies `StorageAgent`'s existing
   unary `BeginTransaction` RPC; `commit_transaction` translates
   `Vec<embyr_core::storage::backend_adapter::Write>` into `Vec<AgentWrite>`
   and proxies `StorageAgent`'s existing unary `Commit` RPC, translating the
   response back into `Vec<WriteResult>`. **ADR-047's own Verification
   section cites these exact same line numbers (527-580) and still concludes
   "no streaming client method exists to reuse" — true but irrelevant:
   `write_stream.rs` never needed a streaming client method. ADR-047 conflated
   "the SDK-facing RPC is bidi-streaming" with "therefore the backend-facing
   port call must also be streaming," a non sequitur ADR-046's own actual
   per-message-independent-atomic-apply design (DISCUSS's own Resolution 3)
   already disproves.**
3. **`crates/embyr-server/src/grpc/handler.rs::authenticate`, lines 196-361**
   — the SAME uniform backend-mode-resolution mechanism every RPC in this
   codebase uses, called once by `handle_write`'s own handshake (ADR-046 §
   Decision 2) exactly as by every other handler. For `row.backend_mode ==
   "agent"` (lines 296-331), it constructs `AgentBackendAdapter::new(...)` and
   returns it as the SAME `SharedBackendAdapter` trait object `write_stream.rs`
   already depends on — no `Write`-specific or RPC-specific branching exists
   in `authenticate()` anywhere. `handle_write` receiving an
   `AgentBackendAdapter`-backed adapter for an agent-mode project is not new
   code to write; it already happens today, unconditionally, the moment a
   `backend_mode=agent` project's API key hits `handle_write`.
4. **`docs/product/architecture/adr-049-batch-write-agent-mode-included-uniform-cap.md`**
   (full) — independently establishes, for `BatchWrite` (ADR-048's own
   `handle_batch_write`, a structurally identical "N independent
   `begin_transaction`+`commit_transaction` pairs, driven by a loop, dispatched
   through `SharedBackendAdapter`" mechanism), that agent-mode requires **zero
   new code**: "`StorageAgent`'s proto already declares both, and
   `AgentBackendAdapter` already implements both, unchanged, with the
   identical trait signature `PostgresBackendAdapter` has... No streaming
   client method is required or missing." ADR-049 explicitly contrasts itself
   against ADR-047's "hard capability wall" framing (§ Alternatives
   Considered, Alternative A) without noticing that `Write`'s own
   `write_stream.rs` loop is the SAME shape as `BatchWrite`'s own
   `handle_batch_write` loop — a for-loop over messages/writes, each
   independently issuing the identical `begin_transaction`+`commit_transaction`
   pair. The only actual difference between the two RPCs is WHERE the loop's
   iterations come from (one `BatchWriteRequest`'s own `Vec<Write>` for
   `BatchWrite`; a sequence of `WriteRequest`s arriving over a
   client-driven stream, for `Write`) — not whether the backend-facing
   mechanism is streaming. It never is, for either RPC.
5. **`crates/embyr-agent/src/server.rs::commit`, lines 598-629** — a
   **genuine, load-bearing bug**, found via this same ground-truth pass, that
   ADR-047/049 neither surfaced: the handler discards its own successful
   result — `Ok(_results) => Ok(Response::new(CommitResponse { commit_time:
   Some(...), ..Default::default() }))` — meaning `CommitResponse.write_results`
   is **always empty** (`Default::default()`), regardless of how many writes
   were actually committed. `AgentBackendAdapter::commit_transaction` (§
   above, item 2) reads `resp.into_inner().write_results` expecting it to be
   populated — it never is, today. This bug already silently affects the
   EXISTING, already-shipped unary `Commit` RPC for every `backend_mode=agent`
   project (every `write_results[i].update_time` a real SDK client receives
   from agent-mode `Commit` is `None` → mapped to `(0, 0)` by
   `AgentBackendAdapter`, an epoch timestamp, not the real commit time) — a
   pre-existing, orthogonal defect this feature's own AC-01 ("acknowledged
   with a populated `updateTime`") makes newly load-bearing, since `Write`
   reuses the identical `commit_transaction` port call.

**Confirmed, corrected: `agent-mode-write-streaming` requires zero new proto
surface, zero new agent-binary handler, and zero new `AgentBackendAdapter`
method. It requires one narrow, pre-existing bug fix in
`crates/embyr-agent/src/server.rs::commit`, without which Slice 01's own
walking-skeleton AC cannot pass — for either `Write` (new) or `Commit`
(already shipped).**

## Decision Drivers

1. **Ground-truth over inherited claims (this session's own established
   discipline)** — ADR-047's own claim was reasonable given the reasoning it
   applied, but that reasoning did not account for ADR-046's own actual
   per-message design (finalized, if not fully cross-referenced, in the SAME
   DESIGN wave) or ADR-049's own later, directly-analogous precedent. This
   ADR's job is to re-derive the answer from the CURRENT codebase, not to
   defer to a stale conclusion once evidence contradicts it — exactly the
   discipline ADR-046 itself applied to DISCUSS's own Resolution 2, and
   ADR-049 applied to ADR-047's own framing (partially — it corrected the
   BatchWrite-specific instance without generalizing the underlying insight
   back onto `Write`).
2. **Simplest solution first (standing methodology)** — an already-shipped,
   already-tested, backend-agnostic composition that already produces the
   required behavior is strictly simpler than authoring new proto messages, a
   new agent RPC, and a new client stub for a mechanism that structurally
   cannot differ from what already exists (`begin_transaction`+
   `commit_transaction`, unconditionally dependency-inverted).
3. **Fix the bug that blocks the AC, not the RPC that doesn't need building**
   — Slice 01's own AC-01 depends on `write_results` actually containing a
   populated `updateTime`. The blocking gap is a five-line handler bug, not an
   absent streaming mechanism. Solving the wrong problem (build a new RPC)
   would not even fix the actual blocker.
4. **Minimal new state (reused from ADR-046)** — `stream_id`/`stream_token`
   remain 100% embyr-server-local, in-memory, per-session concerns (ADR-046 §
   Decision Driver 4). Nothing about backend selection changes this; there is
   no reason to introduce agent-side session state for a mechanism that was
   deliberately designed to need none.

## Decision

### 1. No proto changes

`proto/embyr/agent/v1/storage_agent.proto` is untouched by this feature. The
existing `BeginTransaction`/`Commit` unary RPCs (already relied upon,
unmodified, by both `Commit` and `BatchWrite` for `backend_mode=agent`, per
ADR-049) are `Write`'s own entire backend-facing surface — already shipped,
already generating client stubs `AgentBackendAdapter` already uses.

### 2. No `embyr-agent` binary changes, except one bug fix

`crates/embyr-agent/src/server.rs::commit` is fixed to populate
`CommitResponse.write_results` from its own `commit_transaction` result,
instead of discarding it:

- Map each domain `WriteResult { update_time: (i64, i32), .. }` from
  `self.storage.commit_transaction(...)`'s `Ok(results)` arm into
  `embyr_proto::agent::WriteResult { update_time: Some(Timestamp { seconds,
  nanos }) }` — the SAME shape `handle_commit`/`write_stream.rs` already build
  on the embyr-server side (ADR-046, unchanged), and the SAME shape
  `AgentBackendAdapter::commit_transaction` already expects to receive.
- No other change to `commit`'s own signature, error handling, or the
  `TransactionNotFound`/`TransactionAborted` match arms (§ Consequences —
  this is a strict, additive correction, not a behavior change to any
  currently-correct path).
- No new RPC, no new message field, no `stream_id`/`stream_token` awareness
  added anywhere in `embyr-agent` — the agent remains, by design, unaware
  that a `Commit` it receives originated from a client-facing unary `Commit`
  call or from one iteration of a client-facing `Write` stream's own loop.
  This is the intended, load-bearing consequence of `Write`'s own
  per-`WriteRequest`-independent-atomic-apply contract (DISCUSS Resolution 3,
  ADR-046 § Context finding 1): from the agent's perspective, both RPCs
  produce an indistinguishable sequence of ordinary `BeginTransaction`/
  `Commit` pairs.

### 3. No `AgentBackendAdapter` changes

`crates/embyr-server/src/adapters/agent_backend.rs` is untouched.
`begin_transaction`/`commit_transaction` already implement `BackendAdapter`
identically to `PostgresBackendAdapter`; `write_stream.rs` already calls them
through the same `SharedBackendAdapter` trait object every other RPC uses.
Once the agent-side bug (§ Decision 2) is fixed, `AgentBackendAdapter::commit_transaction`'s
own existing translation (`resp.into_inner().write_results...map(|wr|
WriteResult { update_time: wr.update_time.map(...).unwrap_or((0,0)), .. })`)
begins receiving real data — no code change needed on this side for that to
happen.

### 4. `handle_write`/`write_stream.rs` — untouched

`crates/embyr-server/src/grpc/handler.rs::handle_write` and
`crates/embyr-server/src/grpc/write_stream.rs` (both `firestore-write-streaming`,
ADR-046) are untouched by this feature. They are already, unconditionally,
backend-agnostic. `stream_id`/`stream_token` generation (ADR-046's own
`generate_stream_id`/`generate_stream_token`) already runs identically
regardless of `backend_mode` — no new agent-mode-specific format or scheme is
introduced, because none is needed.

### 5. Write-path access-rule enforcement — already applies, unchanged

Ground-truth re-read of `handler.rs::translate_one_write_for_commit` (the
shared translation function both `handle_commit` and `write_stream.rs` call,
per its own doc comment) shows it now calls
`Self::evaluate_write_rule_for_commit` for every `Update`/`Delete`/`Transform`
write — added after ADR-046 was originally written (same-day
`security-rules-write-path` bug fix, per the comment at
`handler.rs::handle_commit`). **ADR-046 § Decision 2's own "not added" framing
is stale; current ground truth is that write-path access-rule enforcement
already applies uniformly to `Commit` and `Write` alike, for every
`backend_mode` including `agent`, via this shared function.** This is a
correction for accuracy, not a new decision this ADR makes — no code change is
needed or proposed here; `agent-mode-write-streaming` inherits this
enforcement automatically, with no security-equivalence gap between `Commit`
and `Write` for agent-mode.

### 6. Correcting DISCUSS's own AC framing for precondition/malformed-write handling

DISCUSS's own US-01 Domain Example 2, AC items, and Slice 01's own IN-Scope
bullet ("Precondition-failure and malformed-write scoped-error handling —
does not close the stream") describe a "scoped rejection, stream stays open"
behavior. **This contradicts ADR-046's own already-decided, already-shipped
mechanism** (§ Decision 4 there: every non-`EOF`/`Cancelled` error —
including a precondition violation and a malformed-write translation error —
terminates the WHOLE stream via one shared exit path; `docs/SPEC.md`'s own
termination taxonomy for `Write` is exhaustive, with no fourth
"reject-and-stay-open" shape). Because agent-mode `Write` reuses
`write_stream.rs` completely unchanged (§ Decision 4 above), it is
STRUCTURALLY INCAPABLE of behaving differently from non-agent `Write` on this
axis — both share the identical loop, the identical error-exit path, the
identical `core_error_to_status` mapping. **DELIVER must implement and test
against ADR-046's real behavior (whole-stream termination,
`Status::failed_precondition` for a precondition violation,
`Status::invalid_argument` for a malformed write, both via the SAME
`tx.send(Err(...)); return;` exit path), not DISCUSS's own "scoped, stream
stays open" AC wording.** This is the identical shape of correction ADR-046
itself applied to DISCUSS's own Resolution 2 for `firestore-write-streaming` —
an inherited-precedent misreading, not a new design tension.

### 7. Idle-stream-timeout AC — satisfied trivially, no new mechanism

DISCUSS's Slice 02 AC ("An idle stream times out without surfacing a
client-facing error for the idle period alone") describes a mechanism
`firestore-write-streaming` itself never built (grep of that feature's own
full `feature-delta.md` for "idle"/"timeout": zero matches). `write_stream.rs`
has no idle-triggered error path today, for any backend. The AC is satisfied
by this absence — there is no code anywhere in the reused loop that would
inject a spurious error purely from elapsed idle time — not by adding new
idle-reaping logic. If genuine idle-connection resource reclamation is later
needed, that is a transport/infrastructure-level concern (tonic/HTTP2
keepalive configuration), cross-cutting across every streaming RPC in this
codebase, not specific to `Write` or to `backend_mode=agent`, and out of this
feature's own scope to invent.

## Alternatives Considered

**A. Build the new proto RPC, agent handler, and `AgentBackendAdapter` method
as DISCUSS/ADR-047 originally framed (Slice 01's own stated IN-Scope list).**
Rejected: solves a capability gap that does not exist (§ Verification), adds
a second, redundant streaming-loop implementation inside `embyr-agent` for a
mechanism that is provably unreachable from that side (the agent never sees
more than one `BeginTransaction`+`Commit` pair per call, streaming or not),
and — critically — does not fix the actual defect (§ Verification item 5)
blocking Slice 01's own AC, since a brand-new agent RPC would need the exact
same "populate `write_results`" logic written correctly the first time, with
no existing bug to guide discovery of the requirement.

**B. Fix the `commit()` bug as a separate, standalone bugfix feature, keep
`agent-mode-write-streaming` scoped to "just wire dispatch, no proto
changes."** Considered: the bug is real and orthogonal to `Write` specifically
(it also affects today's agent-mode `Commit`). Rejected: Slice 01's own
walking-skeleton AC (AC-01 — "acknowledged with a populated `updateTime`")
cannot pass without this fix; shipping this feature without it would produce
a walking skeleton that is provably broken on its own primary happy path,
directly contradicting DISCUSS's own WS Strategy B ("real, narrow slice...
not mocked — a bidi-streaming mechanism's correctness is exactly the kind of
thing a mock would falsely validate"). The fix is five lines, in a file this
feature's own Slice 01 already must touch conceptually (verifying the
agent-side commit path); splitting it into a separate feature would delay
Slice 01 behind an artificial second DISCUSS/DESIGN pass for a fix DESIGN
already fully specifies here.

**C. No new proto/binary/adapter code; fix the one blocking bug in
`embyr-agent::commit`; correct DISCUSS's own AC framing to match ADR-046's
real behavior; the feature's own deliverable is test coverage proving the
already-existing composition end-to-end (chosen).** Zero new abstraction,
zero new port method, zero new `CoreError` variant, zero new wire surface —
this feature's entire code footprint is one bug fix plus new integration
tests. Matches Decision Driver 2 (simplest solution) and Decision Driver 3
(fix what actually blocks the AC) exactly.

## Consequences

**Positive**: `backend_mode=agent` customers get `Write` streaming parity with
zero new wire protocol to version or reason about for cross-version
compatibility (§ ADR-061) — a materially smaller, lower-risk feature than
DISCUSS/Slice-brief estimated (2 days for Slice 01, 1.5 for Slice 02),
predominantly test-writing effort rather than new production code. The
`commit()` bug fix additionally repairs the EXISTING, already-shipped
agent-mode `Commit` RPC's own `updateTime` reporting for real production
customers, as a byproduct — a genuine, unrelated defect this feature's own
ground-truth DESIGN pass surfaced and fixes, not introduced.

**Negative, named explicitly**: this feature does not build any new
observable "streaming" mechanism on the agent side — a future reader
comparing `storage_agent.proto`'s RPC count before and after this feature
ships will see zero net change, which could read as "nothing shipped" without
this ADR's own explanation. This ADR and the `feature-delta.md` DESIGN
section are the record of what was actually verified, fixed, and tested.

**Negative, inherited unchanged from ADR-046**: agent-mode `Write` inherits
the SAME latency profile ADR-049 already named for agent-mode `BatchWrite` —
each `WriteRequest` costs 2 sequential mTLS round trips to the customer-VPC
agent (`begin_transaction` + `commit_transaction`) instead of 2 local
Postgres round trips. Unlike `BatchWrite` (up to 500 writes bundled into one
call, up to 1000 round trips), `Write`'s own per-message-independent contract
means this cost is naturally amortized across the SDK's own write cadence,
not bunched into one blocking call — a materially better profile than
`BatchWrite`'s own named trade-off, not named as a new concern requiring
mitigation.

**Negative, inherited unchanged from ADR-052/053 (`firestore-field-transforms`)**:
`AgentBackendAdapter::commit_transaction` silently drops `Write::Transform`
writes (`Write::Transform { .. } => None`, filtered out of the request
entirely) — agent-mode has no transform wire representation, an
already-decided, already-accepted v1 gap (ADR-052 § Alternatives Considered).
`Write` against a `backend_mode=agent` project inherits this exact
limitation, unchanged, automatically, UNLESS/UNTIL `agent-mode-field-transforms`'s
own sibling feature (ADR-057, concurrent this session) ships its own wire
addition — at which point `Write` inherits the fix automatically too, for the
same "shared translation function" reason. Named here for completeness, not
a new finding.

**Residual, non-blocking**: no benchmark data confirms the 2-round-trip-per-message
latency is acceptable for any real customer's actual write cadence — same
non-blocking treatment as ADR-049's own residual; a DEVOPS/production-readiness
measurement candidate post-ship, not a DESIGN-time blocker.
