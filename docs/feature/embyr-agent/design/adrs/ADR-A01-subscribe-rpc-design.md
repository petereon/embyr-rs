# ADR-A01: Subscribe RPC Design — Server-Streaming vs Bidirectional vs Polling

## Status
Accepted

## Context

US-A05 requires that `onSnapshot` events reach embyr SaaS within 2 seconds of a committed write on agent-backed projects. The agent holds the Postgres LISTEN/NOTIFY connection. embyr SaaS must receive change events from the agent and fan them out to active Listen-stream clients.

The current `storage_agent.proto` has no `Subscribe` RPC. Three design options were considered.

The `AgentClient` trait in `embyr-core::storage` already defines `subscribe_changes(&self, project_id: ProjectId) -> Result<Receiver<DocChange>, AgentError>`, confirming the SaaS side expects a streaming receiver model.

## Decision

**Option A1 — server-streaming `Subscribe(SubscribeRequest) returns (stream DocChange)`** is adopted.

The agent is the event producer. embyr SaaS is the consumer. There is no semantic content that the SaaS needs to send to the agent after the subscription is established; connection liveness is managed by the gRPC keep-alive mechanism on the mTLS channel. Server-streaming maps exactly to this one-directional producer pattern.

## Alternatives Considered

**Option A2 — bidirectional streaming with heartbeat.** The SaaS would periodically send a `Heartbeat` message; the agent would stream DocChange events. This adds protocol complexity (message types for heartbeat, ack) with no benefit: the gRPC layer already provides TCP keep-alive semantics. The agent does not need client-side control signals to maintain its Postgres LISTEN connection. Rejected: complexity without benefit.

**Option A3 — polling (no Subscribe RPC; SaaS polls agent GetDocument on a timer).** Polling interval must be ≤ 2s to meet the KPI, which means the SaaS would fire ≥ 0.5 rps per active document per second. For a project with 1,000 active onSnapshot listeners, this is ≥ 500 Postgres SELECTs/sec — a factor of 500x amplification over NOTIFY, which fires once per write. Polling is not bounded by write rate; NOTIFY is. Rejected: violates performance quality attribute and 2s KPI at non-trivial listener counts.

## Consequences

**Positive:**
- Single unidirectional streaming RPC is the simplest gRPC primitive for this use case.
- Reconnection is handled by the existing exponential backoff logic in `AgentBackendAdapter` (1s → 30s).
- SaaS side maps directly to `Receiver<DocChange>` semantics: tonic streaming response is consumed in a tokio task, buffered in a channel (capacity 64), and dispatched to the `ListenRegistry`.
- On stream disconnect, the SaaS triggers RESET to all active Listen clients for the project — identical behaviour to the direct-mode NOTIFY reconnect path.

**Negative:**
- The proto file must be extended before Slice S05A can begin; this is a planned proto extension (D5 locked decision from DISCUSS).
- Server-streaming cannot carry backpressure signals to the agent. If the SaaS consumer is slow, the tonic stream buffer will fill; the agent send will eventually block (tonic default bounded buffer). This is acceptable because the channel capacity 64 overflow → RESET mechanism already handles slow consumers at the SaaS level.

## Enforcement

The proto extension is the only mechanism that makes this binding. The `DocChange` message definition in the proto is the contract between agent and SaaS. Any schema change must update the proto version (reserved fields for backward compatibility per proto3 rules) and regenerate `embyr-proto`.
