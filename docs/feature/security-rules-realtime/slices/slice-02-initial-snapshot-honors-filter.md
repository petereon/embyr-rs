# Slice 02: A Rule-Protected Collection's Initial Snapshot Honors the Caller's Own Query Filter

**Story**: US-02 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1.5 days

## Goal
Fix `Listen`'s currently-shipped filter-drop bug: `handle_add_target`'s own `domain_query.filter` is hardcoded `None` — the client's `StructuredQuery.where_` clause is never extracted or consulted. Every Listen subscription today returns the entire, unfiltered collection in its initial snapshot. This slice makes the initial snapshot honor the caller's own filter, reusing `translate_filter()` (the exact function `RunQuery` already uses) unchanged — a genuine prerequisite for Slice 03's own subscribe-time compliance check to mean anything.

## IN Scope
- Extracting the full `StructuredQuery` (not merely `sq.from[0].collection_id`) from the `AddTarget`'s `QueryTarget`.
- Calling `translate_filter()` to build `domain_query.filter`, replacing the current hardcoded `None`.
- Single-field-equality and composite AND-filter shapes, matching `RunQuery`'s own already-supported filter grammar.
- An unfiltered subscription to an unruled collection continues to return the full, unfiltered collection (this is correct — an absent filter is not itself a bug).

## OUT Scope
- Any change to `translate_filter()` itself — reused unchanged.
- Compliance checking (Slice 03) — this slice ONLY makes a real filter available to check against; it does not itself gate anything.
- Collection-group filter shapes (`all_descendants` — out of scope for this feature, Finding 3).

## Learning Hypothesis
**Disproves if it fails**: `Listen`'s initial snapshot cannot honor the client's own query filter by reusing `translate_filter()` unchanged, without either duplicating filter-translation logic or requiring changes to the proto-to-domain translation layer itself.
**Confirms if it succeeds**: The SAME free function `RunQuery` already calls builds an identical `QueryFilter` from an identical `where_` clause, regardless of which RPC's `StructuredQuery` it came from.

## Acceptance Criteria
- AC-17-109: A Listen subscription's initial snapshot narrows by every filter specified in the client's `StructuredQuery.where_`, matching `RunQuery`'s own filter-honoring behavior for an identical filter shape.
- AC-17-110: Composite AND filters are honored in full, not partially.
- AC-17-111: A subscription specifying no filter at all continues to return the full collection, unchanged from pre-feature behavior.
- AC-17-112: Filter translation reuses `translate_filter()` — no second, independently-maintained filter-translation path.

## Production-Data Taste Test
Real filtered `AddTarget` requests against a real multi-document collection, asserting the initial snapshot excludes non-matching documents.

## Dependencies
None structurally, but sequenced second because it is a genuine prerequisite for Slice 03 — `check_query_compliance()` cannot decide anything meaningful against a filter that is always `None`.

## Reference Class
Mirrors `security-rules-query-path`'s own confirmation that `translate_filter`'s existing behavior is reused unchanged, not re-derived, for a new call site.
