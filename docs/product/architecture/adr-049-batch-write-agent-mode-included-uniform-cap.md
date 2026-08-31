# ADR-049: Agent-Mode `BatchWrite` Is Included in v1, No Backend-Mode-Specific Threshold (Resolves DISCUSS Resolution 2)

## Status

Accepted

## Context

DISCUSS's own Resolution 2 (`docs/feature/firestore-batch-write/feature-delta.md`)
escalated, not resolved, whether `backend_mode=agent` `BatchWrite` ships in
v1: include it (accepting the latency profile), defer it as a named follow-up
(mirroring `Write`'s own ADR-047 deferral), or scope a smaller,
agent-mode-specific batch-size threshold. Unlike `Write`'s own agent-mode gap
(ADR-047 — a hard capability wall: zero bidirectional/client-streaming RPC of
any kind on `StorageAgent`'s proto), DISCUSS's own ground-truth reading
already established `BatchWrite`'s own per-write mechanism
(`begin_transaction`+`commit_transaction`, both unary) is **structurally
feasible today** — `StorageAgent`'s proto already declares both, and
`AgentBackendAdapter` already implements both, unchanged, with the identical
trait signature `PostgresBackendAdapter` has. The open question is a genuine
latency trade-off, not a capability gap: `N` writes against `backend_mode=agent`
means `N` sequential `(begin_transaction, commit_transaction)` round-trip
pairs over the already-open mTLS channel to the customer-VPC agent, instead of
`N` local Postgres transactions.

## Verification (ground-truth, not trusted from DISCUSS's own claim alone)

Re-read directly:
- `crates/embyr-server/src/adapters/agent_backend.rs` — `begin_transaction`
  (line 510) and `commit_transaction` (line 527) both exist, both proxy
  `StorageAgent`'s own `BeginTransaction`/`Commit` RPCs over the already-open
  mTLS channel, both already implement the `BackendAdapter` trait identically
  to `PostgresBackendAdapter`. No streaming client method is required or
  missing — confirmed a materially different gap shape than `Write`'s own
  (ADR-047).
- `crates/embyr-core/src/storage/backend_adapter.rs` — `BackendAdapter` trait
  (full, read in § ADR-048 Context) declares `begin_transaction`/
  `commit_transaction` as ordinary trait methods with no `backend_mode`
  parameter or variant anywhere in the port signature — the port itself is
  already backend-agnostic by construction; `handle_batch_write` (ADR-048 §
  Decision 2-3) calls only these trait methods, dispatched to whichever
  concrete adapter `authenticate()`'s own existing backend-mode-resolution
  mechanism already selected, identical to every other RPC in this codebase.
- `docs/product/architecture/adr-041-agent-mode-aggregation-scope.md` (§
  Alternatives Considered, Alternative 3) — this codebase already has a
  documented, rejected precedent for handler-level backend-mode branching
  ("Detect... in the handler itself, before calling the adapter at all...
  Rejected: would require the handler to know backend-mode-specific operator
  support (a leaky abstraction across the `BackendAdapter` port boundary)").
  The identical objection applies to a handler-level `if backend_mode ==
  "agent" { smaller_cap } else { 500 }` check this ADR considers and rejects
  below (§ Alternatives Considered, Alternative B).

**Confirmed: DISCUSS's own claim (structurally feasible, latency-only
question) holds under direct verification.**

## Decision Drivers

1. **Reuse over invention** — the SAME `handle_batch_write` path (ADR-048)
   already works, unmodified, for `backend_mode=agent`; no new code is
   required to "include" it. Excluding it would require NEW code (a rejection
   branch) to carve out an exception, the inverse of this session's own
   standing reuse-first practice.
2. **No unevidenced numbers** (DISCUSS's own explicit instruction for this
   escalation) — this DISCUSS pass has no latency benchmark or customer SLA
   data. Any backend-mode-specific threshold this ADR might invent (e.g., "50
   writes for agent-mode") would be exactly the kind of guess DISCUSS was
   told not to make, now made one level later at DESIGN instead.
3. **Established anti-leaky-abstraction precedent** — ADR-041's own
   Alternative 3 already rejected handler-level backend-mode branching in an
   analogous situation (agent-specific SUM/AVG support); this ADR applies the
   SAME precedent rather than re-litigating it from scratch.
4. **Real Firestore's own documented contract is backend-topology-agnostic**
   — `docs/SPEC.md` §BatchWrite states "each write runs in its own
   transaction," with no backend/deployment-dependent variance documented
   anywhere. A backend-mode-specific behavior difference (rejection or silent
   shrink) would itself be an unevidenced wire/contract deviation.
5. **Named trade-offs over hidden ones** (this session's own established
   practice — ADR-046's access-control-gap Consequence, ADR-047's
   unserved-segment Consequence) — the latency cost is real; naming it
   explicitly as an accepted Consequence is preferred over either silently
   ignoring it or over-engineering an unevidenced mitigation.

## Decision

**Agent-mode `BatchWrite` is included in v1, unmodified, reusing the exact
same `handle_batch_write` path (ADR-048) built for `direct_pg`/`aws_secret`/
`gcp_secret`.** No handler-level `backend_mode` branching is added anywhere in
`handle_batch_write`. The sole per-call resource-consumption mitigation is the
general 500-write cap (ADR-048 § Decision 2), applied UNIFORMLY to every
`backend_mode` — no smaller, agent-specific threshold.

This means: a `BatchWriteRequest` against a `backend_mode=agent` project can
issue up to 1000 sequential mTLS round trips (500 `begin_transaction` + 500
`commit_transaction` calls, at the cap) to the customer-VPC agent in a single
RPC call — bounded, not unbounded, but materially slower than the identical
call against any other `backend_mode`. This is an accepted, named trade-off
(§ Consequences), not a defect.

## Alternatives Considered

**A. Defer agent-mode `BatchWrite` entirely, mirroring `Write`'s own ADR-047
deferral.** Rejected: ADR-047's deferral was justified by a genuine capability
gap — zero streaming RPC existed anywhere on `StorageAgent`'s proto, requiring
new proto authoring, a new agent-binary handler, and a new client stub before
`Write` could even be attempted for `backend_mode=agent`. `BatchWrite` has no
such gap (§ Verification) — deferring here would exclude a fully-working
capability for a PERFORMANCE concern alone, with zero data establishing the
performance is actually unacceptable for any real customer. This is the
inverse of evidence-based scoping, and it would leave JOB-04/JOB-09's
credential-isolation segment needlessly unserved for a second RPC in a row
(after `Write`), when this one has no structural reason to be.

**B. Agent-mode-specific batch-size threshold** (e.g., a smaller max-batch-size
applied only when `backend_mode=agent`). Rejected on two independent grounds:
(1) it requires `handle_batch_write` to know and branch on `backend_mode`
BEFORE dispatching to the adapter — the exact leaky-abstraction shape this
codebase's own ADR-041 (Alternative 3) already rejected in an analogous
decision, now proposed to be repeated here; (2) sizing that threshold with no
benchmark or SLA evidence (DISCUSS's own explicit finding: none exists) would
require inventing a number this ADR has no principled basis for — "smaller
than 500" is not itself a number a crafter can implement without DESIGN
supplying one, and supplying an unevidenced one directly contradicts the
DISCUSS-level instruction this escalation was raised under.

**C. Include unconditionally, relying on the uniform 500-write cap as the
sole mitigation (chosen).** Zero new code, zero new abstraction leak at the
handler/port boundary, zero invented number — the SAME cap real Firestore
itself documents (ADR-048 § Decision 2) already bounds agent-mode's own worst
case to a known, finite ceiling. Consistent with this codebase's own
established port/adapter discipline (backend-specific behavior differences
live behind the `BackendAdapter` trait, never in front of it as handler-level
branching).

## Consequences

**Positive**: zero additional code beyond `handle_batch_write`'s own generic
composition (ADR-048) — `AgentBackendAdapter` is reused completely unchanged,
exactly as `PostgresBackendAdapter`/the AWS/GCP-secret adapters are.
JOB-04/JOB-09's credential-isolation customer segment gets full `BatchWrite`
parity in v1, unlike `Write`'s own still-unserved gap for that same segment
(ADR-047) — this feature closes a capability gap for that segment that
`firestore-write-streaming` could not. No new abstraction leak is introduced
at the handler/port boundary; ADR-041's own precedent is reinforced, not
re-litigated with a different outcome for a similar-shaped question.

**Negative, named explicitly**: a `BatchWriteRequest` at or near the 500-write
cap against `backend_mode=agent` incurs up to 1000 sequential mTLS round trips
in a single call — a materially different (likely much slower) latency
profile than the identical call against any other `backend_mode`, and
different from every other RPC's own agent-mode story in this codebase (all
of which are "one call in, one call out," 1:1). This is accepted, not
mitigated, for lack of evidence that a mitigation is even needed for any real
customer's actual usage pattern. Flagged as a concrete, evidence-gated
follow-up candidate: once real agent-mode `BatchWrite` latency is observed
(DEVOPS/production telemetry, post-ship), a data-justified backend-mode-aware
threshold, a parallel-fan-out optimization for agent-mode specifically, or
simply confirming no customer sends large batches in practice, all become
legitimate, evidence-based decisions — inventing any of them today, with zero
data, would be exactly the premature optimization DISCUSS's own DoR note
(feature-delta.md, "no numeric latency target... DESIGN/DEVOPS may add one if
evidence justifies it") already warns against.

**Residual, non-blocking**: no benchmark or customer SLA data confirms 1000
sequential mTLS round trips is actually tolerable for any real customer —
recommended as a DEVOPS/production-readiness measurement candidate once this
feature ships, not a DESIGN-time blocker (same non-blocking treatment as
ADR-048's own two residuals).
