# agent-mode-write-streaming — Feature Delta

**Wave**: DISCUSS | **Agent**: Luna (nw-product-owner) | **Date**: 2026-08-31
**Status**: Ready for DESIGN handoff — two escalated open questions (bidi-streaming RPC shape on the agent proto; cross-version graceful-degradation UX). See § Handoff Package.
**Upstream**: No DISCOVER/DIVERGE wave ran — commissioned directly by the orchestrator as one of 4 confirmed `backend_mode=agent` parity gaps remaining after every other RPC-level gap closed for `direct_pg`/`aws_secret`/`gcp_secret`. `firestore-write-streaming`'s own DISCUSS (2026-08-30) investigated and confirmed this exact gap (its own Resolution 4, § Reading Confirmation below) and named it as a candidate follow-up feature rather than silently dropping it.

---

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `proto/embyr/agent/v1/storage_agent.proto` (full, 372 lines) — confirmed `service StorageAgent` declares 15 RPCs: 11 unary, `RunQuery`/`Subscribe` server-streaming, **zero client-streaming or bidi-streaming RPC of any kind**. `Write` (the message type, not an RPC) exists only as a per-item entry inside `CommitRequest.writes` — structurally unrelated to a `Write` streaming RPC.
✓ `crates/embyr-agent/src/server.rs` (full, 860 lines) — confirmed the only streaming precedent is `subscribe()` (server-streaming: one request, a channel-backed `ReceiverStream` of `DocChange` events) and `run_query()` (same shape). Neither is bidi — both take exactly one client message and stream server→client only. There is no existing code path in this binary that reads a `tonic::Streaming<T>` request (client→server streaming input), so a genuinely new streaming-input handling pattern is required, not an extension of an existing one.
✓ `crates/embyr-server/src/adapters/agent_backend.rs` (full, 627 lines) — confirmed `AgentBackendAdapter` implements `BackendAdapter` via unary/server-streaming gRPC calls only (`client.get_document(req).await`, `client.run_query(req).await.into_inner()` then consumed as a stream). No client-streaming call pattern exists in this file today — a new `BackendAdapter` port method (or new dedicated non-`BackendAdapter` streaming entry point, per `firestore-write-streaming`'s own precedent of NOT routing `Write` through `BackendAdapter` at all — see below) would be net-new.
✓ `docs/feature/firestore-write-streaming/feature-delta.md` (targeted: § Reading Confirmation line 14, § Resolution 4 line 60, § System Constraints line 167, § DESIGN Escalation 2 lines 595-604) — confirmed the non-agent version of this exact RPC (`Write`, bidirectional streaming, `stream_token`/`stream_id` handshake mechanics) shipped for `direct_pg`/`aws_secret`/`gcp_secret` in that feature, with agent-mode explicitly deferred (its own Resolution 4) and independently re-confirmed by that feature's own DESIGN wave (Morgan, Escalation 2, 2026-08-30): "`storage_agent.proto` (372 lines, zero bidi/client-streaming RPC)... a materially larger unit of work than any prior agent-mode slice this codebase has shipped." This feature inherits that feature's own already-decided wire-contract SHAPE (stream handshake semantics, `stream_token` mismatch handling) as a design precedent to mirror on the agent proto — not a decision to re-litigate here in DISCUSS.
✓ `docs/SPEC.md` § Agent gRPC Protocol (lines 493-503) and § embyr Agent (lines 467-517) — confirmed the documented (not implemented) protocol summary already lists "Transaction lifecycle (Begin, GetForTransaction, Commit, Rollback, Sweep)" and does NOT separately list a streaming `Write` RPC — `Write` is absent even from SPEC.md's own aspirational protocol description, meaning this gap was never previously scoped into the documented agent contract at all, unlike `ListCollectionIds` and `Sweep` (§ sibling features' own Reading Confirmations). Line 503: **"The protocol is internal and versioned. The agent binary version must match the embyr SaaS major version."** — a documented deployment-discipline POLICY, not a runtime-enforced mechanism: `PingRequest{}`/`PingResponse{server_time}` (the only health-check RPC) carries no version field today. This is the single most important cross-feature finding — see § Handoff Package Escalation 2.
✓ `docs/product/jobs.yaml` lines 14-26 (JOB-01) — confirmed JOB-01's own functional dimension ("all SDK calls succeed unchanged") and emotional dimension ("feel confident the migration won't break production... feel in control of data residency") directly motivate closing this gap: `backend_mode=agent` customers are, by construction, the ones who chose embyr specifically FOR data-residency control — the exact JOB-01 emotional dimension — yet are the ONLY segment for whom the Firebase SDK's default offline-write-durability behavior (persistent `Write` stream) does not work.

No contradictions found between this feature's scope and prior evidence.

---

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

| # | Decision | Value |
|---|---|---|
| 1 | Feature Type | Backend — closes an SDK-facing RPC-level parity gap (`Write` bidi streaming) for `backend_mode=agent`, mirroring `firestore-write-streaming`'s own feature type for the other three backend modes |
| 2 | Walking Skeleton | YES — a real walking skeleton exists (US-01: single stream, single write, happy path, one representative agent-mode project) — see § WS Strategy |
| 3 | UX Research Depth | Lightweight, SDK-facing — Alex never sees a TUI; the "experience" is the Firebase SDK's own offline-write-durability behavior working transparently against an agent-mode project |
| 4 | JTBD Analysis | Confirmed by investigation: **extends JOB-01 (sdk-compat)**, same persona P1 Alex, same "make it real"/close-the-remaining-gap pattern as `firestore-write-streaming` itself — see § Persona & Job |

---

## Wave: DISCUSS / [REF] Pre-requisites

- `firestore-write-streaming` (shipped, non-agent modes) — the wire-contract SHAPE (stream handshake, `stream_token` mismatch/expiry semantics, `WriteResponse` per-write acknowledgment) this feature mirrors on the agent proto side. Not literally reusable code across the crate boundary (client-facing `google.firestore.v1` proto vs. agent's own independent `embyr.agent.v1` proto, per that proto file's own doc comment: "defines its own message types... to avoid cross-proto package dependencies") — a design precedent to follow, not a dependency to import.
- `PostgresBackendAdapter::commit_transaction` (`crates/embyr-pg-storage/src/backend_adapter.rs`) — the underlying write-application primitive both `embyr-server`'s non-agent path and `embyr-agent`'s own `commit()` handler already share; the new streaming RPC's per-write application logic can reuse this unchanged (writes arrive one at a time over the stream, each applied via the same primitive `Commit` already uses).
- No dependency on `agent-mode-list-collection-ids`, `agent-mode-field-transforms`, or `agent-mode-transaction-purge` (sibling split features, this session) — investigated and confirmed independent (§ Handoff Package framing note on the bundle-vs-split decision, recorded once in this feature as the "first" of the four).

---

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P1 — Alex (SDK Developer)**, existing persona, JOB-01's own — this time at **Meridian Health** (`meridian-health-prod`, `backend_mode=agent`), a healthcare SaaS company running its own PostgreSQL inside its own VPC specifically to keep patient data under HIPAA-scoped infrastructure control — the concrete, domain-real reason a company chooses `backend_mode=agent` over `direct_pg`/`aws_secret`/`gcp_secret` in the first place, and a direct realization of JOB-01's own emotional dimension ("feel in control of data residency").

**job_id decision**: `JOB-01` (`sdk-compat`), extended not new — same pattern as every prior "make it real" RPC-completion feature this session (aggregation-queries, batch-get-documents, `firestore-write-streaming` itself for the other three backend modes, firestore-batch-write, firestore-list-rpcs, firestore-field-transforms).

---

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

| Signal | Threshold | This feature | Fired? |
|---|---|---|---|
| User stories | >10 | 2 (US-01, US-02) | **NO** |
| Bounded contexts / modules | >3 | 1 — BC-2 Document Storage's own write path, agent-mode slice | **NO** |
| Walking Skeleton integration points | >5 | 4 — (1) new bidi-streaming RPC + messages on `storage_agent.proto`, (2) new `embyr-agent` binary handler (first client-streaming-input code path in this binary), (3) new streaming client call in `AgentBackendAdapter` (or a dedicated non-`BackendAdapter` streaming entry point, mirroring `firestore-write-streaming`'s own routing decision — DESIGN question, not decided here), (4) reuse of `commit_transaction`'s existing write-application primitive per stream item | **NO** |
| Estimated effort | >2 weeks | ~3-4 days across 2 slices | **NO** |
| Independent shippable outcomes | multiple unrelated | **NO** — one coherent outcome (Firebase SDK's offline-write-durability behavior works against agent-mode projects), split by scenario breadth (single-write happy path, then sustained multi-write/reconnect), not by unrelated feature | **NO** |

**0 of 5 signals fired. Verdict: PASS — single feature, right-sized as a 2-slice feature, no split needed.** This is, however, the largest and most novel of the 4 sibling `backend_mode=agent` gap features (§ Bundle-vs-Split Note below) — right-sized on its own, but not a candidate for bundling with the other 3.

### Bundle-vs-Split Note (recorded once, applies to all 4 sibling features)

Investigated rather than assumed, per the orchestrator's own explicit question: does this feature's own proto-versioning/streaming-mechanism work create shared infrastructure the other 3 gaps (`agent-mode-list-collection-ids`, `agent-mode-field-transforms`, `agent-mode-transaction-purge`) would benefit from, justifying one combined feature? **No — confirmed by evidence, not assumed.** The agent proto has no version-negotiation FIELD to design (§ Reading Confirmation, SPEC.md line 503 — the documented policy is operational lockstep, not a runtime handshake), so there is no shared MECHANISM being built here that the other three would consume. Each of the 4 gaps is additive to the proto independently (new RPC, new message, new oneof variant, or — for `agent-mode-transaction-purge` — zero proto change at all, purely agent-local). Complexity classes differ sharply: this feature is a net-new streaming mechanism (no bidi precedent anywhere in the agent binary); `agent-mode-list-collection-ids` is a small unary addition; `agent-mode-field-transforms` is wire-only (compute logic already shared and working); `agent-mode-transaction-purge` touches no wire protocol at all. Bundling would create >5 integration points, >3 bounded-context-equivalent surfaces, and 4 independently-shippable outcomes — all three Elephant Carpaccio oversized signals. **Decision: SPLIT into 4 independent features**, mirroring this session's own established precedent of one feature per RPC-completion gap (`firestore-write-streaming`, `firestore-list-rpcs`, `firestore-field-transforms` were separate features even though all three are "Firestore parity" gaps discovered in the same investigation). The one genuinely cross-cutting concern — graceful handling when `embyr-server` is upgraded ahead of a customer's deployed `embyr-agent` binary — is real and applies to this feature plus `agent-mode-list-collection-ids` and `agent-mode-field-transforms` (all three touch the wire protocol); it is escalated in full here (§ Handoff Package Escalation 2) and referenced, not re-litigated, in those two sibling features.

---

## Wave: DISCUSS / [REF] Journey (Lightweight, SDK-facing, per Decision 3)

**Alex's mental model**: Alex's app already uses the Firebase SDK's default persistent `Write` stream against every other embyr project Meridian Health runs (`backend_mode=direct_pg` staging environment). Alex expects the exact same behavior — writes queue locally, flush over a long-lived stream, survive brief disconnects — against `meridian-health-prod` (`backend_mode=agent`). Today, any SDK call that opens a `Write` stream against an agent-mode project fails outright (the RPC does not exist on the agent's own gRPC surface at all — `Unimplemented`), which Alex cannot distinguish from a misconfiguration without embyr-specific tribal knowledge.

**Emotional arc** (Confidence Building): **Start** — mild confusion; the SDK behaves identically everywhere else Alex has tested it, so an `Unimplemented` error specifically on the production (agent-mode) project reads as a bug in Alex's own setup, not a known gap. **Middle** — Alex opens a stream, sends a write, watches it acknowledge — the mechanism now works identically to every other backend mode Alex has used. **End** — confident; Alex stops treating `backend_mode=agent` as a second-class deployment target and trusts the SDK's offline-durability guarantees uniformly across Meridian Health's full environment fleet.

**Shared artifact**: the `stream_token`/acknowledgment mechanics `firestore-write-streaming` already established for non-agent modes — single source of truth for the SEMANTICS (not the wire bytes, which are proto-family-specific per § Pre-requisites) is that feature's own DESIGN decision, reused here without renegotiation.

**Failure modes** (feeds DISTILL scenario generation): the stream disconnects mid-session (network blip between embyr-server and the agent, inside the customer's VPC boundary) | a write inside the stream fails a precondition (document was deleted by a concurrent client) | the agent's own Postgres connection pool is exhausted mid-stream | Alex's deployed `embyr-agent` binary predates this feature (§ Handoff Package Escalation 2) | the stream is opened but no write is ever sent (idle timeout).

---

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

**Persona**: P1 Alex | **Goal**: The Firebase SDK's default `Write` stream works against a `backend_mode=agent` project exactly as it already does against `direct_pg`/`aws_secret`/`gcp_secret`.

### Backbone

| A. Open Write Stream | B. Send Writes Over the Stream | C. Stream Lifecycle |
|---|---|---|
| Handshake establishes a stream session against an agent-mode project **[WS]** | A single write is applied and acknowledged **[WS]** | Client closes the stream cleanly **[WS]** |
| — | Multiple writes in one session are each independently acknowledged | Stream survives a brief disconnect and resumes |
| — | A write that fails a precondition returns a scoped error, not a stream-ending one | An idle stream times out gracefully |

### Walking Skeleton

Open a `Write` stream against `meridian-health-prod` (agent-mode) → handshake completes → send one write (document update) → receive one acknowledgment with the applied write's result → close the stream cleanly. This is Slice 01, US-01.

### Release 1 — Single-Write Streaming Works End-to-End (Slice 01, US-01)

Outcome: the net-new bidi-streaming mechanism exists and correctly applies and acknowledges exactly one write per stream against a real agent-mode Postgres — proves the mechanism class, not yet the full durability contract.

### Release 2 — Sustained Multi-Write Sessions Match SDK Offline-Durability Behavior (Slice 02, US-02)

Outcome: a stream carries multiple writes across its lifetime, each acknowledged independently, and survives the failure modes the Firebase SDK's own offline queue actually exercises (brief disconnect, a single write's precondition failure not aborting the whole session).

---

## Wave: DISCUSS / [REF] WS Strategy

**Strategy: B (Real, Narrow Slice)** — the walking skeleton (US-01) runs against a real agent-mode deployment (real `embyr-agent` binary, real customer-VPC-style Postgres, real mTLS channel) for the single-write happy path only, mirroring `firestore-write-streaming`'s own WS Strategy B for the non-agent modes and `customer-db-transaction-sweeper`'s own precedent of "real Postgres, narrow scenario" for this codebase's background/infrastructure work. Not Strategy C (mocked) — a bidi-streaming mechanism's correctness is exactly the kind of thing a mock would falsely validate. Not Strategy D — no environment-conditional behavior exists to switch between.

---

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| Slice | Story | Release | Estimate | Learning Hypothesis (disproves) | Production-data taste test |
|---|---|---|---|---|---|
| 01 (WS) | US-01 | 1 | 2 days | A bidi-streaming RPC cannot actually be added to the agent's own tonic-based gRPC surface without breaking the existing mTLS `ServerTlsConfig`/`add_service` wiring — i.e., the mechanism class itself is not a drop-in addition to this binary's current shape | Real `embyr-agent` binary (test-harness `serve()` entry point, mirroring existing integration test patterns), real Postgres, one real bidi stream carrying one real write, asserting the write is actually persisted and the acknowledgment carries the correct `WriteResult` — no mocked transport |
| 02 | US-02 | 2 | 1.5 days | A stream carrying multiple writes, or surviving a mid-session disconnect, silently drops or double-applies a write — i.e., the mechanism only works for the single-shot case and does not actually deliver the offline-durability contract the SDK depends on | Real stream, 3+ real writes in one session including one deliberately-failing precondition, plus one deliberate disconnect-and-reconnect, asserting exactly the expected writes landed exactly once each |

**Total estimate: ~3.5 days across 2 slices.**

**Taste tests applied**: 4+ components/slice — Slice 01 introduces 3 new components (proto RPC/messages, agent binary handler, client-side streaming call) — PASS. New-abstraction-first — the one new abstraction (the streaming RPC itself) ships in Slice 01, Slice 02 adds no new abstraction — PASS. Falsifiable hypothesis per slice — PASS (table above). Production data only — PASS (both slices use real Postgres, no synthetic-only scenario). No identical-except-scale slices — PASS (Slice 01 is single-write, Slice 02 is a materially different durability-contract scenario, not the same test at larger N).

---

## Wave: DISCUSS / [REF] Prioritization

| Priority | Slice | Target Outcome | Rationale |
|---|---|---|---|
| 1 | Slice 01 (WS) | The bidi-streaming mechanism class exists and works end-to-end for one write | Highest-uncertainty piece (no bidi precedent in this binary) — validate the mechanism before layering durability-contract scenarios on top |
| 2 | Slice 02 | Multi-write sessions and disconnect recovery match SDK expectations | Depends on Slice 01's own mechanism existing; delivers the actual value SDK developers depend on (queued offline writes surviving a real network blip) |

---

## Wave: DISCUSS / [REF] System Constraints

- **No existing bidi-streaming or client-streaming-input RPC exists anywhere in `embyr-agent`'s current binary** (§ Reading Confirmation) — `Subscribe`/`RunQuery` are server-streaming only. This is a genuinely new mechanism class for this binary, not an extension of an existing pattern.
- **Reuse `commit_transaction`'s existing write-application primitive** for each write received over the stream — no new Postgres write-application logic; the new work is entirely at the streaming-transport and per-item-acknowledgment layer.
- **The agent proto's own message shapes are independent of the client-facing `google.firestore.v1` proto** (proto file's own doc comment, § Reading Confirmation) — new messages for this RPC must be authored fresh in `embyr.agent.v1`, mirroring `firestore-write-streaming`'s own semantics but not literally importing its message types.
- **Version-compatibility is a genuinely open, first-of-its-kind question for this codebase** (§ Handoff Package Escalation 2) — flagged for DESIGN, not decided in DISCUSS.

---

## Wave: DISCUSS / [REF] Driving Ports

| Port | Trigger | Notes |
|---|---|---|
| Firebase SDK `Write(stream WriteRequest) returns (stream WriteResponse)` | Client SDK's own default offline-write-durability behavior | Already shipped for non-agent modes (`firestore-write-streaming`); this feature closes the identical entry point for `backend_mode=agent` |

---

## Wave: DISCUSS / [REF] User Stories

<!-- markdownlint-disable MD024 -->

### US-01: Alex's App Opens a Write Stream Against an Agent-Mode Project and a Write Lands

**job_id**: JOB-01 (sdk-compat) — extends, "make it real" pattern (§ Persona & Job)

#### Problem

Alex's Firebase SDK app works identically against every embyr project Meridian Health runs — except `meridian-health-prod` (`backend_mode=agent`), where any SDK call that opens the default persistent `Write` stream fails immediately with an unrecognized-RPC error. Alex cannot tell whether this is a Meridian Health configuration mistake or a real embyr limitation, and cannot safely promote the exact same client code from staging (`direct_pg`) to production (`agent`) without special-casing it.

#### Who

- Alex, SDK Developer at Meridian Health | Building a HIPAA-scoped healthcare app whose production Postgres lives entirely inside Meridian Health's own VPC (`backend_mode=agent`) | Motivated to trust the Firebase SDK's own offline-write-durability contract uniformly across every environment, without per-backend-mode special-casing in application code

#### Solution

A new bidirectional-streaming RPC on `storage_agent.proto`, mirroring `firestore-write-streaming`'s own already-shipped wire-contract semantics for non-agent modes, lets Alex's SDK open a persistent `Write` stream against an agent-mode project, send a write, and receive an acknowledgment carrying the applied write's result.

#### Elevator Pitch

Before: Alex's SDK app calling `Write()` against `meridian-health-prod` (`backend_mode=agent`) receives an `Unimplemented` gRPC error on the very first stream-open attempt — no write is ever sent or acknowledged.
After: Alex's app calls the Firebase SDK's default `db.doc('patients/p_204').set({...})` (which the SDK internally routes through its persistent `Write` stream) → sees the write acknowledged with a normal Firestore `WriteResult` (`updateTime` populated), identical to the same call against a `direct_pg` project.
Decision enabled: Alex decides Meridian Health's production (agent-mode) deployment is safe to treat identically to staging in application code — no backend-mode-specific write-path branching needed.

#### Domain Examples

##### 1: Happy Path — Alex updates a patient record via the default SDK write path

Alex's app calls `db.doc('patients/p_204').update({ status: 'discharged' })` against `meridian-health-prod`. The SDK opens a `Write` stream, sends the update, and the stream acknowledges it with `updateTime` set. The document's `generation` increments by 1 in the agent's local Postgres.

##### 2: Edge Case — A precondition fails mid-stream, the stream itself stays open

Alex's app sends a write to `patients/p_205` with a `MustNotExist` precondition, but the document already exists (created by a concurrent request). The stream returns a scoped `FailedPrecondition` result for that specific write; the stream itself remains open and Alex's next write on the same stream succeeds normally.

##### 3: Error/Boundary — Alex's deployed agent binary predates this feature

Meridian Health's ops team has not yet redeployed `embyr-agent` past the version that adds this RPC. Alex's SDK call opens a `Write` stream and receives `Unimplemented` — the SAME failure mode as today, not a new or more confusing one (§ Handoff Package Escalation 2 — whether this should be surfaced with a clearer message is a DESIGN question, not resolved here).

#### UAT Scenarios (BDD)

##### Scenario: A single write over a new stream is applied and acknowledged
```gherkin
Given meridian-health-prod is backend_mode=agent with a reachable embyr-agent
And no document exists yet at patients/p_204
When Alex's app opens a Write stream and sends an update to patients/p_204 setting status to "admitted"
Then the stream acknowledges the write with a populated updateTime
And the document patients/p_204 exists in the agent's local Postgres with status "admitted"
```

##### Scenario: A precondition failure on one write does not close the stream
```gherkin
Given meridian-health-prod has a document at patients/p_205 that already exists
When Alex's app sends a write to patients/p_205 over an open stream with a MustNotExist precondition
Then the stream returns a FailedPrecondition result scoped to that write only
And the stream remains open for subsequent writes
```

##### Scenario: The stream closes cleanly when the client finishes
```gherkin
Given an open Write stream against meridian-health-prod with one acknowledged write
When Alex's app closes the stream
Then the agent releases the stream's server-side resources
And no further writes are accepted on that closed stream
```

##### Scenario: An unrecognized document path within the stream returns a scoped error
```gherkin
Given an open Write stream against meridian-health-prod
When Alex's app sends a write with a malformed document name
Then the stream returns an InvalidArgument result scoped to that write only
And the stream remains open
```

#### Acceptance Criteria

- [ ] A write sent over a newly-opened stream against an agent-mode project is persisted and acknowledged with a populated `updateTime`
- [ ] A precondition failure on one write returns a scoped error result without terminating the stream
- [ ] A malformed write (invalid document path) returns a scoped `InvalidArgument` result without terminating the stream
- [ ] The client can close the stream cleanly at any point after zero or more writes
- [ ] The mechanism is exercised against a real `embyr-agent` binary and real Postgres in the walking-skeleton test (no mocked transport)

#### Outcome KPIs

- **Who**: Alex (SDK Developer, agent-mode customers)
- **Does what**: Completes a Firebase SDK `Write` stream call against a `backend_mode=agent` project
- **By how much**: 0% → 100% of well-formed single-write stream sessions succeed (currently 100% fail with `Unimplemented`)
- **Measured by**: Count of successful stream-acknowledged writes against the reference test suite
- **Baseline**: 0 — the RPC does not exist on the agent proto today

#### Technical Notes (Optional)

- Reuses `commit_transaction`'s existing write-application primitive per stream item — no new Postgres write logic.
- Mirrors `firestore-write-streaming`'s own wire-contract semantics (stream handshake, per-write acknowledgment shape) without importing its message types (independent proto family, § System Constraints).
- Depends on DESIGN resolving Escalation 1 (bidi RPC/message shape) before implementation.

---

### US-02: Alex's App Sustains a Multi-Write Session and Recovers From a Brief Disconnect

**job_id**: JOB-01 (sdk-compat) — extends, same as US-01

#### Problem

Even after US-01 ships, Alex's app has only been proven to work for a single write per stream. The Firebase SDK's real offline-durability value comes from queuing MULTIPLE writes locally and flushing them over one persistent session, and from that session surviving the kind of brief network blip that's common on real infrastructure (including inside a customer's own VPC, between `embyr-server` and `embyr-agent`). Without this, Alex's production app would silently lose the durability guarantee the SDK's own documentation promises.

#### Who

- Alex, SDK Developer at Meridian Health | Same persona and deployment as US-01 | Needs the SDK's offline-queue behavior — not just a single write — to actually work in production

#### Solution

The same streaming mechanism from US-01 correctly handles multiple sequential writes in one session, each independently acknowledged, and survives a stream disconnect/reconnect without losing or double-applying any write.

#### Elevator Pitch

Before: Alex's app can only be trusted to send one write per stream session; a burst of offline-queued writes flushing together, or a brief VPC network blip, has unverified behavior against `meridian-health-prod`.
After: Alex's app queues 5 writes offline (e.g., discharge summary fields) and flushes them in one `Write` stream session on reconnect → sees all 5 writes acknowledged independently, each with its own `updateTime`.
Decision enabled: Alex decides it's safe to let the Firebase SDK's own offline queue behave exactly as documented in production, without a manual "flush one write at a time" workaround.

#### Domain Examples

##### 1: Happy Path — Alex's app flushes 5 queued writes in one stream session

After being offline briefly, Alex's app reconnects and its SDK-internal queue flushes 5 pending writes to `patients/p_206` field updates over one `Write` stream. Each of the 5 is acknowledged independently, in order.

##### 2: Edge Case — The stream disconnects after 2 of 5 writes are acknowledged

A network blip drops the stream after writes 1-2 are acknowledged but before writes 3-5 are sent. The SDK reopens a new stream and resends writes 3-5 (SDK's own retry responsibility); embyr-agent applies them without re-applying 1-2.

##### 3: Boundary — Two writes in the same session target the same document

Alex's app sends two sequential updates to `patients/p_207` in the same stream session (a rapid double-tap in the UI). Both are applied in order; the document's final state reflects the second write, and `generation` increments by 2.

#### UAT Scenarios (BDD)

##### Scenario: Multiple writes in one stream session are each acknowledged independently
```gherkin
Given an open Write stream against meridian-health-prod
When Alex's app sends 3 sequential writes to 3 different documents over the same stream
Then all 3 writes are acknowledged, each with its own updateTime
And all 3 documents exist in the agent's local Postgres with the sent field values
```

##### Scenario: A stream disconnect does not lose already-acknowledged writes
```gherkin
Given an open Write stream against meridian-health-prod with 2 writes already acknowledged
When the stream disconnects unexpectedly
Then the 2 already-acknowledged writes remain persisted in the agent's local Postgres
```

##### Scenario: A reopened stream after disconnect accepts new writes normally
```gherkin
Given a Write stream against meridian-health-prod disconnected after 2 acknowledged writes
When Alex's app opens a new Write stream and sends a 3rd write
Then the 3rd write is acknowledged normally
And no duplicate write for documents already acknowledged in the prior stream occurs
```

##### Scenario: Two sequential writes to the same document apply in order
```gherkin
Given an open Write stream against meridian-health-prod
When Alex's app sends two sequential updates to patients/p_207 setting status first to "in_review" then to "discharged"
Then patients/p_207 has status "discharged"
And the document's generation has incremented by 2
```

##### Scenario: An idle stream with no writes eventually times out without error noise
```gherkin
Given an open Write stream against meridian-health-prod with no writes sent
When the stream remains idle past the configured idle timeout
Then the stream closes without an error surfaced to Alex's app for the idle period itself
```

#### Acceptance Criteria

- [ ] 3+ sequential writes in one stream session are each independently acknowledged with correct `updateTime` values
- [ ] A stream disconnect does not roll back or lose writes already acknowledged before the disconnect
- [ ] A new stream opened after a disconnect accepts further writes normally, with no duplicate application of previously-acknowledged writes
- [ ] Two sequential writes to the same document within one session apply in order, with `generation` incrementing once per write
- [ ] An idle stream times out without surfacing a client-facing error for the idle period alone

#### Outcome KPIs

- **Who**: Alex (SDK Developer, agent-mode customers)
- **Does what**: Trusts the Firebase SDK's offline-write-queue behavior in production against an agent-mode project
- **By how much**: 0% → 100% of multi-write sessions (up to the SDK's own typical batch size) complete without loss or duplication
- **Measured by**: Count of correctly-acknowledged multi-write sessions, including one disconnect/reconnect scenario, against the reference test suite
- **Baseline**: 0 — no multi-write session has ever succeeded against an agent-mode project (US-01 not yet shipped)

#### Technical Notes (Optional)

- Depends on US-01's own streaming mechanism shipping first.
- Reconnection/resend semantics are the SDK's own client-side responsibility (documented Firebase behavior) — embyr-agent's own obligation is idempotent-safe acknowledgment per write it actually receives, not de-duplication of client-side resends.

---

## Wave: DISCUSS / [REF] Outcome KPIs (Feature-Level Summary)

### Feature: agent-mode-write-streaming

### Objective

Alex can trust the Firebase SDK's default offline-write-durability behavior uniformly across every embyr backend mode Meridian Health runs, including `backend_mode=agent`, with zero backend-mode-specific application code.

### Outcome KPIs

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|-----|-----------|-------------|----------|-------------|------|
| 1 | Alex | Completes a single-write `Write` stream session against an agent-mode project | 0% → 100% of well-formed sessions succeed | 0% (RPC does not exist) | Reference test suite | North Star |
| 2 | Alex | Completes a multi-write, disconnect-tolerant `Write` stream session against an agent-mode project | 0% → 100% of well-formed multi-write sessions succeed with no loss/duplication | 0% | Reference test suite | Leading |

### Metric Hierarchy

- **North Star**: KPI #1 — the mechanism exists and works at all
- **Leading Indicators**: KPI #2 — the durability contract Alex actually depends on
- **Guardrail Metrics**: no write is ever double-applied across a reconnect; no already-acknowledged write is ever lost

### Measurement Plan

| KPI | Data Source | Collection Method | Frequency | Owner |
|-----|------------|-------------------|-----------|-------|
| Single-write success rate | Reference test suite | Automated assertion | Per CI run | embyr-agent + embyr-server (DELIVER) |
| Multi-write durability | Reference test suite | Automated assertion, including disconnect scenario | Per CI run | embyr-agent + embyr-server (DELIVER) |

### Hypothesis

We believe that adding a bidi-streaming `Write` RPC to `storage_agent.proto`, mirroring `firestore-write-streaming`'s own already-shipped wire-contract semantics, will make the Firebase SDK's default write-durability behavior work identically against `backend_mode=agent` projects.
We will know this is true when Alex's app can complete both a single-write and a multi-write, disconnect-tolerant stream session against a real agent-mode deployment with zero backend-mode-specific application code.

---

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Story: US-01

| DoR Item | Status | Evidence/Issue |
|----------|--------|----------------|
| Problem statement clear | PASS | Alex's SDK app fails outright on the default `Write` stream against agent-mode, with no distinguishable-from-misconfiguration error |
| User/persona identified | PASS | Alex, SDK Developer, Meridian Health, agent-mode, HIPAA data-residency motivation |
| 3+ domain examples | PASS | Happy path, precondition-failure edge case, predates-feature error boundary |
| UAT scenarios (3-7) | PASS | 4 scenarios |
| AC derived from UAT | PASS | 5 AC items map directly to the 4 scenarios plus the real-transport constraint |
| Right-sized | PASS | 2 days, 4 scenarios |
| Technical notes | PASS | Reuse of `commit_transaction`, dependency on Escalation 1 resolution |
| Dependencies tracked | PASS | Depends on `firestore-write-streaming`'s own already-shipped wire-contract precedent (informational, not a blocking code dependency) |
| Outcome KPIs | PASS | KPI #1, numeric target and measurement method defined |

### Story: US-02

| DoR Item | Status | Evidence/Issue |
|----------|--------|----------------|
| Problem statement clear | PASS | Single-write proof (US-01) does not validate the SDK's real multi-write, disconnect-tolerant offline-queue contract |
| User/persona identified | PASS | Same as US-01 |
| 3+ domain examples | PASS | Happy path, disconnect edge case, same-document-double-write boundary |
| UAT scenarios (3-7) | PASS | 5 scenarios |
| AC derived from UAT | PASS | 5 AC items map directly |
| Right-sized | PASS | 1.5 days, 5 scenarios |
| Technical notes | PASS | Depends on US-01; SDK-side resend is out of this story's own obligation |
| Dependencies tracked | PASS | Depends on US-01 shipping first (explicit) |
| Outcome KPIs | PASS | KPI #2, numeric target and measurement method defined |

### DoR Status: PASSED (both stories)

---

## Wave: DISCUSS / [REF] Handoff Package

### Escalation 1 — Bidi-streaming RPC and message shape on the agent proto

No existing bidi-streaming or client-streaming-input precedent exists anywhere in `embyr-agent`'s current binary (§ Reading Confirmation) — `Subscribe`/`RunQuery` are server-streaming only. DESIGN must decide: the exact `AgentWriteRequest`/`AgentWriteResponse` (or equivalently-named) message shapes, whether the handshake mirrors `firestore-write-streaming`'s own `stream_token` mechanism exactly or adapts it for the agent's own independent proto family, and whether this routes through `BackendAdapter` as a new trait method or through a dedicated non-`BackendAdapter` entry point (mirroring `firestore-write-streaming`'s own routing decision for non-agent modes, § Pre-requisites).

### Escalation 2 — Cross-version graceful degradation (applies to this feature + 2 siblings)

**This is the single most important finding across all 4 `backend_mode=agent` parity-gap features investigated this session.** `docs/SPEC.md` line 503 documents a POLICY ("the agent binary version must match the embyr SaaS major version") but there is no RUNTIME mechanism enforcing or even checking it — `PingRequest{}`/`PingResponse{server_time}` carries no version field. Today, if `embyr-server` is upgraded to expect this new `Write` RPC (or `agent-mode-list-collection-ids`'s new RPC, or `agent-mode-field-transforms`'s new `Write` oneof variant) but a customer's deployed `embyr-agent` binary predates it, the observed failure is a clean, non-corrupting `Unimplemented` (or, for the transform oneof-variant case, a clean `invalid_argument` — § agent-mode-field-transforms's own server.rs fallback already handles an unrecognized `Write.operation` safely) — NOT a crash or silent data corruption. **Open question for DESIGN**: is today's implicit "clean gRPC error bubbles to the SDK caller as an opaque error" acceptable, or should `embyr-server` proactively detect an old agent (e.g., via a `protocol_version` field added to `Ping`) and surface a clearer, customer-facing "upgrade your embyr-agent binary" message instead of letting the SDK caller see a raw `Unimplemented`? This is a genuine, first-of-its-kind product decision for this codebase (every other feature this session ships `embyr-server` and its dependents as one deployed unit) — flagged for DESIGN to resolve once, ideally in a way all 3 wire-touching sibling features (this one, `agent-mode-list-collection-ids`, `agent-mode-field-transforms`) can reuse identically rather than inventing 3 separate answers.

### Handoff Confirmation

Next step (NOT performed by this agent): orchestrator dispatches `nw-solution-architect` for the DESIGN wave — full rigor with ADRs (at minimum: bidi RPC/message design per Escalation 1; the cross-version degradation mechanism per Escalation 2, ideally decided once and referenced by the 2 sibling features) and Reuse Analysis, per the standing session practice.
