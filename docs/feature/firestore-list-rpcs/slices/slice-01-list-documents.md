# Slice 01: Alex Lists the Documents in a Collection Without Writing a Query

**Feature**: firestore-list-rpcs | **Story**: US-01 | **Release**: 1 (Walking Skeleton)
**Estimate**: 1.5 days | **job_id**: JOB-01

## Goal
Implement `ListDocuments` end-to-end: a new unary handler built on the already-existing `BackendAdapter::run_query`, using the already-proven hex-offset `page_token` pagination scheme from `crates/embyr-agent/src/server.rs::list_documents`.

## IN Scope
- New proto: `ListDocumentsRequest`/`ListDocumentsResponse` messages + `rpc ListDocuments(...) returns (...)` declaration in `firestore.proto` (mirror the field shape already declared in `proto/embyr/agent/v1/storage_agent.proto` lines 359-371).
- New unary handler `handle_list_documents` in `crates/embyr-server/src/grpc/handler.rs`, mirroring `handle_get_document`'s own auth/rate-limit/suspension sequence.
- `collection_id` set: maps to a single `CollectionPath`, calls `run_query` with `limit: page_size+1, offset` (fetch-one-extra-to-detect-more-pages).
- `collection_id` empty: returns documents from every collection directly under `parent`.
- `page_token` encode/decode (hex offset) — extract from the agent binary or duplicate; DESIGN's call.
- Empty-parent validation (`InvalidArgument`), suspended-project rejection (`permission_denied`), matching every other RPC.
- Backend modes: `direct_pg`, `aws_secret`, `gcp_secret`. Agent-mode routing is escalated to DESIGN (see feature-delta.md § Handoff Package) — build so either resolution (reuse `run_query` uniformly, or special-case the agent's own dedicated RPC) is a small follow-up, not a rewrite.

## OUT Scope
- `ListCollectionIds` (Slice 02).
- Composite filtering/ordering — `RunQuery` already covers this; `ListDocuments` is the no-query convenience path only.
- Agent-mode routing decision itself (escalated, not decided here) — build the walking skeleton against non-agent backend modes; do not block on the escalation resolving.

## Learning Hypothesis
Disproves: the already-proven `run_query`+hex-offset-token pattern (confirmed feasible by direct code reading against the agent binary's own working implementation) cannot actually be reused unchanged for a new direct-mode unary handler without requiring a new `BackendAdapter` trait method after all.
Confirms (if it succeeds): zero new trait surface is needed; `run_query` alone is sufficient for both single-collection and all-collections-under-parent listing, paginated correctly.

## Acceptance Criteria
- [ ] AC-01-01: A `ListDocumentsRequest` with an explicit `page_size` returns at most that many documents per page, with `next_page_token` present if and only if more documents remain.
- [ ] AC-01-02: Requesting the next page with a previously-returned `next_page_token` returns the remaining documents, never repeating or skipping any.
- [ ] AC-01-03: An empty `collection_id` returns documents from every collection directly under `parent`, not just one.
- [ ] AC-01-04: A collection with zero documents returns an empty `documents` array and an empty `next_page_token`, with no error.
- [ ] AC-01-05: An empty `parent` is rejected with `InvalidArgument`.
- [ ] AC-01-06: A suspended project's `ListDocumentsRequest` is rejected with `permission_denied` before any query runs.

## Production-Data Taste Test
Real Postgres, a real `trip_entries` collection under `users/maria-santos-a1b2` with 3 well-formed documents (`yosemite-2024`, `banff-2024`, `patagonia-2025`), a real `ListDocuments` call with `page_size=2` — asserting page 1 returns 2 documents + a non-empty `next_page_token`, page 2 returns the remaining 1 document + an empty `next_page_token`. No mocked adapter.

## Dependencies
- `BackendAdapter::run_query` (shipped, `crates/embyr-core/src/storage/backend_adapter.rs`).
- `security-rules-query-path` (shipped, ADR-031).
- `crates/embyr-agent/src/server.rs::list_documents`'s own token technique (shipped, pattern to generalize).

## Reference Class
Structurally mirrors `handle_get_document`'s own auth sequence (unary, non-streaming); pagination mechanics mirror the agent binary's own already-shipped `list_documents` implementation.
