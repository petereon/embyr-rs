# Slice 01: A Listen Subscriber's Live Delivery Never Crosses Into Another Collection

**Story**: US-01 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1.5 days

## Goal
Fix `Listen`'s currently-shipped, project-wide cross-collection leak: today, `ListenRegistry::fan_out()` delivers every document-change event on a project's Postgres NOTIFY channel to every subscriber on that channel, regardless of which collection each subscriber's `AddTarget` actually named. This slice makes live delivery structurally scoped to the subscriber's own collection — the single highest-consequence, most severe finding of this feature's DISCUSS, independent of whether any access rule exists.

## IN Scope
- A collection-match check comparing each delivered `ListenEvent`'s own `collection_path` against the subscriber's own subscribed `collection.collection_path`, applied before forwarding to the client.
- Applies to BOTH `ListenEvent::Changed` and `ListenEvent::Removed`.
- Applies universally — regardless of whether either collection involved has any access rule defined (rule-independent, structural fix).
- Two subscribers to the SAME collection both continue to correctly receive that collection's own events (the fix narrows delivery correctly, it does not over-narrow).

## OUT Scope
- Any change to `ListenRegistry`'s own channel-keying scheme (still one NOTIFY channel per project) — the filter is applied at delivery time, not by redesigning channel topology (see OQ-SRRT-05).
- Access-control enforcement of any kind (Slices 03–05).
- Query-filter honoring (Slice 02).
- `TargetType::Documents` or collection-group Listen targets (out of scope for this feature entirely — Findings 1/3).

## Learning Hypothesis
**Disproves if it fails**: A subscriber's live delivery cannot be scoped to their own subscribed collection without either introducing per-subscriber (not per-project) NOTIFY channels — a much larger BC-3 redesign — or requiring an expensive per-event DB round-trip beyond what `fetch_event()` already performs.
**Confirms if it succeeds**: An in-memory comparison of already-available collection-path values, applied before forwarding, is sufficient — no channel-topology change, no additional I/O.

## Acceptance Criteria
- AC-17-105: A Listen subscriber's live stream never receives a `DocumentChange` event whose own `collection_path` differs from the subscriber's own subscribed collection.
- AC-17-106: A Listen subscriber's live stream never receives a `DocumentDelete` event whose own `collection_path` differs from the subscriber's own subscribed collection.
- AC-17-107: Two subscribers to the SAME collection both correctly receive that collection's own events.
- AC-17-108: The guarantee holds identically regardless of whether either collection has any access rule defined.

## Production-Data Taste Test
Real Postgres NOTIFY events across ≥2 distinct collections in the same real project, real Maria session subscribed to only one of them, real concurrent writes to the other.

## Dependencies
None — this is the first slice in the Walking Skeleton, and every later slice's own reasoning about "does this event belong to this subscriber" depends on this fix existing first.

## Reference Class
Mirrors `security-rules`'s own AC-17-14/15/16 "structural, not conventional" regression-guardrail discipline, applied here to a genuinely new BC-3-internal mechanism rather than an existing BC-4 short-circuit.
