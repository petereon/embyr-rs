# Slice 04: Each Individually-Delivered Live Change Event Is Re-Checked Against the Rule Using the Already-Fetched Document

**Story**: US-04 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 2 days

## Goal
Extend `evaluate()` (ADR-027/030, unchanged) to `Listen`'s own ongoing, per-event delivery decision — the same mechanism `GetDocument`/writes already use, now applied continuously across a subscription's entire lifetime rather than once. This closes the task's own "owner field changes after admission" hypothesis: a document whose content no longer satisfies the subscriber's rule has its NEXT live event withheld, not delivered. Costs zero additional I/O — `fetch_event()` already fetches the document before fan-out, for every NOTIFY, regardless of security rules.

## IN Scope
- A per-event `evaluate()` call inside `listen_handler.rs`'s own event-consumption loop (`ListenEvent::Changed` arm), consuming the SAME already-fetched `FirestoreDocument`.
- `request_resource_fields` passed as an empty map (no "proposed new document" concept exists for a Listen delivery — mirrors `handle_get_document`'s own identical convention).
- Fail-closed on a missing rule-referenced field — never crash, never deliver (reuses ADR-027's mechanism verbatim).
- Applies only when the subscribed collection has an active rule (gated behind Slice 03's own rule lookup — no additional lookup needed, the row is already resolved).

## OUT Scope
- Any modification to `evaluate()` itself, or any new `Operand`/`Condition` shape — reused completely unmodified.
- `ListenEvent::Removed` handling (Slice 05) — this slice covers ONLY `Changed` events.
- A second document fetch — the existing `fetch_event()` fetch is reused, never duplicated.

## Learning Hypothesis
**Disproves if it fails**: An individually-delivered live change event cannot be re-checked against the caller's rule using the document `fetch_event()` already fetches, without either re-fetching a second time or requiring a new evaluation mechanism beyond `evaluate()`.
**Confirms if it succeeds**: The identical, already-in-memory `FirestoreDocument.fields` feeds directly into `evaluate()`'s existing `resource_fields` parameter with zero additional I/O.

## Acceptance Criteria
- AC-17-117: A live change event for a document that still satisfies the subscriber's own rule is delivered.
- AC-17-118: A live change event for a document that no longer satisfies the subscriber's own rule is withheld, not delivered.
- AC-17-119: A live change event referencing a document with a missing rule-referenced field fails closed, never crashes.
- AC-17-120: `evaluate()` is reused completely unmodified, called with `request_resource_fields` as an empty map.
- AC-17-121: The per-event re-check adds zero additional I/O beyond `fetch_event()`'s own existing fetch.

## Production-Data Taste Test
Real NOTIFY-triggered write that changes a delivered document's `owner_id` mid-session, real assertion the next event for that document is withheld from the original subscriber.

## Dependencies
Slice 01 (collection scoping must hold first — a per-event check on an event from the WRONG collection is meaningless) and Slice 03 (the rule row must already be resolved).

## Reference Class
This feature's single highest-consequence arm alongside Slice 01 — designated mutation-testing surface (per-feature strategy, CLAUDE.md). Structurally the INVERSE of `security-rules-query-path`'s own Resolution 2 (`evaluate()` rejected for RunQuery, no document exists at query time) — here a document genuinely, cheaply exists at delivery time.
