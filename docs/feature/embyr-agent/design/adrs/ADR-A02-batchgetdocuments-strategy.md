# ADR-A02: BatchGetDocuments — N Parallel GetDocument RPCs vs Proto Extension

## Status
Accepted

## Context

`BatchGetDocuments` is a Firestore operation that fetches multiple documents by path in a single RPC. On the SaaS side, `BackendAdapter.batch_get()` must be implemented. The current `storage_agent.proto` has no `BatchGetDocuments` RPC (proto gap from DISCUSS D6).

The agent is single-project and executes each document fetch as a `SELECT ... WHERE project_id = $1 AND collection_path = $2 AND document_id = $3`. A batch of N documents requires N independent SELECTs; Postgres does not provide a "multi-row fetch by (path, id) list" that performs better than N parallel SELECTs on the documents schema (no composite index designed for multi-path batch retrieval).

## Decision

**Option B2 — `AgentBackendAdapter.batch_get()` on the SaaS side fans out N parallel `GetDocument` RPCs** over the existing mTLS channel. No proto change. No new message types.

`tonic` channels are multiplexed over a single HTTP/2 connection; multiple concurrent RPCs share one TLS connection. Parallelism is provided by spawning N concurrent `get_document` calls and awaiting them with `tokio::join_all`.

## Alternatives Considered

**Option B1 — Extend proto with `BatchGetDocuments(BatchGetDocumentsRequest) returns (stream BatchGetDocumentsResponse)`.** This would require: new message types, proto regeneration, agent-side implementation, SaaS-side implementation. The agent implementation would issue N independent SELECTs — the same SQL it would issue for N parallel `GetDocument` calls. The proto extension adds wire-framing overhead (one streaming RPC vs N unary RPCs) but does not reduce total SQL work, because the documents table has no bulk-by-path-list index. The primary benefit of a dedicated `BatchGetDocuments` RPC is a reduction in per-RPC overhead; at typical batch sizes (≤ 100 documents per Firestore batch), this overhead is negligible (each tonic unary RPC adds ~0.1ms over gRPC-multiplexed HTTP/2). Rejected: proto surface expansion with no performance benefit at design-target scale; violates simplest-solution principle.

## Consequences

**Positive:**
- No proto change required for batch operations. Proto stays smaller and more stable.
- SaaS adapter layer owns batching logic; agent remains a thin executor of single-document operations.
- HTTP/2 multiplexing means N parallel `GetDocument` calls share one TLS connection; overhead is identical to one BatchGetDocuments RPC for practical batch sizes.
- Simpler agent binary: agent handles only single-document operations, reducing code surface and test surface.

**Negative:**
- Each `GetDocument` call is a separate gRPC request/response round-trip. For very large batches (> 500 documents) over a high-latency WAN link between SaaS and agent VPC, this could add latency relative to a single BatchGetDocuments RPC. This is an acceptable trade-off for V1; a BatchGetDocuments proto extension can be added in a future slice if profiling demonstrates WAN latency amplification.
- `tokio::join_all` for large batches consumes proportionally more memory. The Firestore protocol caps `batchGetDocuments` at 100 documents per call; this bounds the concurrency.

## Enforcement

The `BackendAdapter.batch_get()` trait method is the contract. `AgentBackendAdapter` implements it by spawning N `get_document` futures. This is the only place where N-parallel behaviour is wired; the agent binary has no knowledge of batching.
