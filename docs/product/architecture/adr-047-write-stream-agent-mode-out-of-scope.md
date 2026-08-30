# ADR-047: Agent-Mode `Write` Is Out of v1 Scope (Confirms DISCUSS Resolution 4)

## Status

Accepted

## Context

DISCUSS's own Resolution 4 (`docs/feature/firestore-write-streaming/feature-delta.md`)
deferred agent-mode (`backend_mode=agent`) `Write` out of this feature's v1
scope, reasoning that `StorageAgent`'s own proto surface has zero
bidirectional-streaming RPC of any kind today — structurally larger than
`aggregation-queries`' own agent-mode slice, which extended an already-unary
RPC shape (`RunAggregationQuery` existed unary on both sides before that
feature). DISCUSS explicitly escalated this for DESIGN's own confirmation
rather than treating it as settled, naming the consequence: JOB-04/JOB-09's
own credential-isolation customer segment remains unserved by `Write` until a
dedicated follow-up feature ships.

## Verification (ground-truth, not trusted from DISCUSS's own claim)

Read in full:
- `proto/embyr/agent/v1/storage_agent.proto` (372 lines) — `service
  StorageAgent` declares exactly 11 RPCs: `GetDocument`, `CreateDocument`,
  `UpdateDocument`, `DeleteDocument` (all unary), `RunQuery` (server-streaming),
  `BeginTransaction`, `Commit`, `Rollback`, `Ping` (all unary),
  `RunAggregationQuery` (unary — not streaming, unlike the client-facing
  `Firestore.RunAggregationQuery`), `ListDocuments` (unary), `Subscribe`
  (server-streaming). **Zero client-streaming or bidirectional-streaming RPC
  exists anywhere in this proto.** `Commit`'s own agent-side shape
  (`CommitRequest{database, writes, transaction}` → `CommitResponse`) is
  unary, mirroring the client-facing `Commit`, not `Write`.
- `crates/embyr-server/src/adapters/agent_backend.rs` (targeted:
  `commit_transaction`, lines 527-580, and the `begin_transaction`/service
  client wiring) — confirms `AgentBackendAdapter` proxies `BackendAdapter`
  trait calls to `StorageAgent`'s existing unary `Commit`/`BeginTransaction`
  RPCs over mTLS. No streaming client method exists to reuse; a `Write`-shaped
  agent-mode implementation would need (a) a net-new `rpc WriteStream(stream
  WriteRequest) returns (stream WriteResponse);` declaration on
  `storage_agent.proto` itself, (b) a new generated client stub, (c) a new
  `AgentBackendAdapter` method bridging the client-facing session/loop shape
  onto that new RPC, and (d) a corresponding new `StorageAgentService` method
  in `crates/embyr-agent/src/server.rs` implementing the receive-and-reply
  loop a SECOND time, independently, inside the agent binary — a materially
  larger and structurally different unit of work than any other agent-mode
  slice this codebase has shipped (`aggregation-queries`' Slice 02 added a
  new unary RPC arm to an already-unary-capable proto; this would add an
  entirely new streaming MECHANISM CLASS to a proto and binary that has never
  had one).

**Confirmed: DISCUSS's own claim holds under direct verification, not merely
plausible.**

## Decision

**Agent-mode `Write` is deferred, explicitly and by name, out of this
feature's v1 scope.** `crates/embyr-server/src/adapters/agent_backend.rs` is
untouched by this feature. `proto/embyr/agent/v1/storage_agent.proto` is
untouched by this feature. `crates/embyr-agent/` is untouched by this feature.
The client-facing `Write` RPC (ADR-046) is built and reachable only for
`backend_mode` in `{direct_pg, aws_secret, gcp_secret}` — a project configured
with `backend_mode=agent` receives no `Write` handler dispatch differentiation
in v1 (the RPC itself is service-wide; DELIVER's own implementation is
responsible for what a `backend_mode=agent` project's `Write` handshake
actually returns — this ADR does not prescribe a specific rejection status,
since no code path routes an agent-mode project through the new handler at
all unless `AgentBackendAdapter` is selected upstream by the SAME
`authenticate()`/backend-mode-resolution mechanism every other RPC already
uses; if that resolution selects `AgentBackendAdapter`, `begin_transaction`/
`commit_transaction` calls against it will surface whatever `CoreError`
`AgentBackendAdapter` already produces for an unimplemented capability today
— DELIVER should verify this produces a caller-legible status, not a panic,
as a cheap pre-existing-behavior check, not new design).

**Named, unserved trade-off** (DISCUSS's own framing, restated and locked, not
softened): JOB-04/JOB-09's credential-isolation customers — those running
`backend_mode=agent` specifically so embyr's own SaaS control plane never
holds direct Postgres credentials — do not get the SDK's own offline-write-
durability guarantee in v1. This is a real, customer-visible capability gap
for that segment, not a hidden one. It is named here as a candidate follow-up
feature (`agent-mode-write-streaming` or similar), requiring its own DISCUSS
pass to size the genuinely-new streaming-mechanism work identified above
(proto authoring, new agent binary RPC, new client stub, new
`AgentBackendAdapter` method) — not assumed to be a small extension of this
feature's own v1 delivery.

## Alternatives Considered

**A. Build agent-mode `Write` in this same feature (Slice 05).** Rejected:
the verification above shows this is not a "mirror the existing agent-mode
slice pattern" extension (as `aggregation-queries` was) but a genuinely new
streaming mechanism class on a SEPARATE deployment artifact (the customer-VPC
agent binary) that has never shipped one — violates walking-skeleton
discipline (DISCUSS § WS Strategy) and would roughly double this feature's own
already-larger-than-usual scope (4 slices, ~4.5 days) for a segment DISCUSS
itself estimates as smaller than the primary `direct_pg`/`aws_secret`/
`gcp_secret` segment this feature targets.

**B. Silently drop agent-mode `Write` with no ADR, treating it as an obvious,
unremarkable exclusion.** Rejected: JOB-04/JOB-09's own credential-isolation
customers are a named, real persona segment (confirmed present in
`docs/product/jobs.yaml`) — an unserved capability gap for a named customer
segment must be a locked, visible decision, not an implicit one a future
reader has to reverse-engineer from an absent code path.

**C. Defer explicitly, named trade-off, candidate follow-up feature (chosen).**
Matches this codebase's own established convention for scoping deferred work
(mirrors ADR-041's own "zero agent-binary changes" discipline, applied here to
an entire RPC rather than one operator).

## Consequences

**Positive**: this feature's own v1 scope stays walking-skeleton-sized (4
slices, DISCUSS's own estimate unchanged by this ADR). Zero risk of a rushed,
under-designed second streaming-loop implementation inside the agent binary.
The unserved segment is named, not hidden, giving product/roadmap owners a
real basis to prioritize the follow-up.

**Negative**: `backend_mode=agent` customers remain unable to rely on the
Firebase SDK's own default offline-write-durability behavior until a
dedicated follow-up feature ships — an explicit, accepted, customer-visible
gap for JOB-04/JOB-09's segment specifically.
