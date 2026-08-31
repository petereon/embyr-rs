# Slice 02: Alex Discovers What Collections Exist Under a Path

**Feature**: firestore-list-rpcs | **Story**: US-02 | **Release**: 2
**Estimate**: 1 day | **job_id**: JOB-01

## Goal
Implement `ListCollectionIds` end-to-end: a new unary handler exposing a genuinely new query primitive (distinct immediate-child `collection_path` names under a parent prefix), reusing Slice 01's own proven `page_token` mechanism.

## IN Scope
- New proto: `ListCollectionIdsRequest`/`ListCollectionIdsResponse` messages + `rpc ListCollectionIds(...) returns (...)` declaration in `firestore.proto`.
- New unary handler `handle_list_collection_ids`, same auth/rate-limit/suspension sequence as Slice 01.
- New query primitive: distinct `collection_path` values that are exactly one segment longer than the parent prefix (root parent = no `/` at all; nested parent = prefix matches, no further `/` after it). Recommend a new `BackendAdapter` trait method with a default-error body for backends not yet implementing it (mirrors `run_aggregation_query`'s ADR-041 precedent) — exact signature is DESIGN's call.
- Reuse Slice 01's own hex-offset `page_token` scheme unchanged.
- Empty-parent validation (`InvalidArgument`), matching every other RPC.
- Backend modes: `direct_pg`, `aws_secret`, `gcp_secret` only.

## OUT Scope
- Agent-mode (`backend_mode=agent`) — `StorageAgent`'s own proto has no `ListCollectionIds` RPC at all, a hard proto-surface wall. Deferred, mirroring `Write`'s own original ADR-047 precedent. Default-error trait body so `AgentBackendAdapter` compiles unmodified and rejects at runtime.
- `ListDocuments` (Slice 01, must ship first — this slice depends on its token scheme being stable).

## Learning Hypothesis
Disproves: the "distinct immediate children of a path prefix" query cannot be built on the existing flat `collection_path` column without either double-counting nested grandchildren or missing root-level collections.
Confirms (if it succeeds): a single prefix-`LIKE` + segment-count constraint, reusing the existing collection-group `LIKE` technique already in `run_query`, is sufficient — no new column, no path-normalization migration needed.

## Acceptance Criteria
- [ ] AC-02-01: A `ListCollectionIdsRequest` with an explicit `page_size` returns at most that many distinct collection IDs per page, with `next_page_token` present if and only if more remain.
- [ ] AC-02-02: Requesting the next page with a previously-returned `next_page_token` returns the remaining collection IDs, never repeating or skipping any.
- [ ] AC-02-03: A root-level `parent` (database root, no document path) returns only top-level collections, never a nested subcollection.
- [ ] AC-02-04: A document with no subcollections returns an empty `collection_ids` array and an empty `next_page_token`, with no error.
- [ ] AC-02-05: Each distinct collection name is returned exactly once regardless of how many documents it contains.
- [ ] AC-02-06: An empty `parent` is rejected with `InvalidArgument`.

## Production-Data Taste Test
Real Postgres, `users/maria-santos-a1b2` with 3 real subcollections (`trip_entries`, `payment_methods`, `support_notes`), a real `ListCollectionIds` call with `page_size=2` — asserting page 1 returns exactly 2 distinct collection IDs + a non-empty `next_page_token`, page 2 returns the remaining 1 + an empty `next_page_token`; a separate root-level call returns top-level collection IDs only, never a nested one. No mocked adapter.

## Dependencies
- Slice 01 (US-01) — shared `page_token` encode/decode scheme must exist first.
- `crates/embyr-pg-storage/src/backend_adapter.rs::run_query`'s own collection-group `LIKE` technique (shipped, pattern to reuse, lines 544-554).

## Reference Class
The one genuinely new query primitive in this feature; no existing trait method or SQL query in this codebase enumerates path children today (confirmed against `access_rules.rs`, which is single-segment/bare-collection-id only, unrelated).
