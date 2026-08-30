# Slice 01: Batch Document Resolution (Walking Skeleton)

## Goal
Complete the already-declared `BatchGetDocuments` gRPC handler so a caller can resolve N specific documents by name in one round trip, with the identical per-document access-control guarantee `GetDocument` already provides — for both backend-mode families, with zero new adapter or agent-binary work.

## IN Scope
- Replace `handle_batch_get_documents`'s stub (`crates/embyr-server/src/grpc/handler.rs:1074-1078`) with a real implementation.
- Per-call: `extract_project_id`, `extract_api_key`, `rate_limiter.check`, `authenticate` (+ suspension check), `attach_client_identity_if_present` — each called exactly once, mirroring `handle_run_query`'s own multi-result-RPC granularity.
- Per requested document name: `parse_document_path`, `get_access_rule` (collection-scoped — a batch may span multiple collections), `adapter.get_document`, and — when a rule is defined — `parse_condition` + `evaluate()`, mirroring `handle_get_document`'s own sequence exactly.
- `found`/`missing` streamed response construction, mirroring `handle_run_query`'s own `Box::pin(tokio_stream::iter(responses))` pattern.
- `Deny` on any single requested document rejects the whole batch as `Status::permission_denied` (Resolution 5 of `feature-delta.md`).
- Validation: reject an empty `documents` list; reject document names spanning more than one project/database. Both before any per-document work begins.
- `metrics_adapter.record_read(project_id, documents.len())` — usage metering reflects N documents read, not 1 call.

## OUT Scope
- `DocumentMask` field-projection — inherited gap, `GetDocument` doesn't honor it either.
- `consistency_selector` (`transaction`/`new_transaction`/`read_time`) functional honoring — inherited gap, `GetDocument` doesn't honor it either. Accepted on the wire, not functionally applied.
- Response ordering guarantees — not required per `docs/SPEC.md §BatchGetDocuments`.
- Plain-REST JSON gateway support — pre-existing gap shared by every other multi-result RPC.
- Any change to `access_rules`, `get_access_rule`, `parse_condition`, `evaluate()`, `handle_get_document`, `BackendAdapter`, `PostgresBackendAdapter`, or `AgentBackendAdapter` — this slice is a pure new consumer of already-shipped mechanisms.
- Any `storage_agent.proto` or `embyr-agent` binary change — `BackendAdapter::get_document` already works correctly for `backend_mode=agent` today; no new adapter code needed (confirmed, Resolution 3).

## Learning Hypothesis
**Disproves if it fails**: `handle_get_document`'s own per-document resolve-and-authorize sequence cannot be applied N times, across documents spanning different collections with different (or absent) rules, without requiring new access-control logic beyond a simple per-document loop — OR the already-shipped `BackendAdapter::get_document` cannot serve both backend-mode families in this new batched call shape without new adapter code.
**Confirms if it succeeds**: this codebase's own access-control mechanism (ADR-027/029) is cleanly reusable at "apply N times" granularity with zero new domain logic, and `BackendAdapter::get_document`'s existing polymorphism is sufficient for a new caller shape with zero new adapter work — direct evidence for how future N-document-touching RPCs in this codebase should be built.

## Acceptance Criteria
- [ ] AC-01-01: A `BatchGetDocuments` call naming documents the caller is authorized to read, spanning one or more collections, returns a `found` result for every document that exists, in a single round trip.
- [ ] AC-01-02: A document named in the batch that does not exist returns a `missing` result alongside `found` results for the rest of the batch — never an error.
- [ ] AC-01-03: A batch containing at least one document the caller is not authorized to read is rejected as a permission denial for the whole request, mirroring `GetDocument`'s own outcome for the same scenario.
- [ ] AC-01-04: A collection with no access rule defined resolves every requested document from that collection unrestricted — zero behavior change from pre-feature `GetDocument`.
- [ ] AC-01-05: A request naming zero documents is rejected as invalid before any access-control evaluation or document fetch occurs.
- [ ] AC-01-06: A request naming documents that resolve to more than one distinct project/database is rejected as invalid before any access-control evaluation or document fetch occurs.

## Dependencies
- `security-rules` (`get_access_rule`, `parse_condition`, `evaluate()`) — shipped.
- `BackendAdapter::get_document`, both `PostgresBackendAdapter` and `AgentBackendAdapter` — shipped, already proven via `GetDocument`'s own production usage.
- No dependency on `aggregation-queries` (independent, parallel feature).

## Effort Estimate
1 day. Reference class: `handle_get_document` (existing, ~130 lines) applied in a loop, plus `handle_run_query`'s own stream-construction pattern (existing) — composition of two already-shipped patterns, not new design.

## Pre-Slice SPIKE
Not needed — zero escalated open questions (see `feature-delta.md` § Job Discovery Framing Resolution); both reused mechanisms are already read in full and confirmed directly reusable.
