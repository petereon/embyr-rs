# Slice 01: Single-Write Stream Against an Agent-Mode Project (Walking Skeleton)

**Story**: US-01 | **Release**: 1 | **Estimate**: 2 days

## Goal

Prove a bidi-streaming `Write` RPC can exist on `storage_agent.proto` and correctly apply + acknowledge exactly one write against a real agent-mode Postgres.

## IN Scope

- New bidi-streaming RPC + request/response messages on `storage_agent.proto` (shape decided by DESIGN, Escalation 1)
- `embyr-agent` binary handler: first client-streaming-input code path in this binary
- `embyr-server`-side streaming client call (new `AgentBackendAdapter` method or dedicated entry point, per DESIGN)
- Single write applied via existing `commit_transaction` primitive, acknowledged with `WriteResult`
- Precondition-failure and malformed-write scoped-error handling (does not close the stream)
- Clean stream close

## OUT Scope

- Multi-write sessions, disconnect/reconnect recovery (Slice 02)
- Version-negotiation/graceful-degradation mechanism (escalated to DESIGN, may land as a separate follow-up)

## Learning Hypothesis

Disproves: a bidi-streaming RPC cannot actually be added to the agent's own tonic-based gRPC surface without breaking the existing mTLS `ServerTlsConfig`/`add_service` wiring.
Confirms (if it succeeds): the mechanism class is a viable, drop-in-compatible addition to this binary.

## Acceptance Criteria

- [ ] A write sent over a newly-opened stream against an agent-mode project is persisted and acknowledged with a populated `updateTime`
- [ ] A precondition failure on one write returns a scoped error result without terminating the stream
- [ ] A malformed write returns a scoped `InvalidArgument` result without terminating the stream
- [ ] The client can close the stream cleanly
- [ ] Exercised against a real `embyr-agent` binary and real Postgres (no mocked transport)

## Dependencies

None (first slice).

## Effort Estimate

2 days.

## Reference Class

`firestore-write-streaming` Slice 01 (non-agent modes) — same shape of walking skeleton (single stream, single write, happy path), different proto family.

## Pre-Slice SPIKE

Not required — DESIGN wave resolves the message-shape question (Escalation 1) before implementation begins; no implementation-time uncertainty remains once DESIGN lands.
