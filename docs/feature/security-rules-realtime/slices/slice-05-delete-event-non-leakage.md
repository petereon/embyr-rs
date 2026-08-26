# Slice 05: A Delete Event for a Document the Subscriber's Rule Would Deny Is Not Delivered

**Story**: US-05 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1.5 days

## Goal
Extend existence non-leakage (`security-rules`/`security-rules-write-path`'s own established discipline) to `Listen`'s own delete-event delivery. A document the subscriber's rule would have denied access to must not have its removal delivered either — the subscriber never learns a document existed in the first place, so its later deletion reveals nothing new about it that a correctly-enforced system should ever expose.

## IN Scope
- A compliance decision for each `ListenEvent::Removed` event, using pre-deletion field data (available via the `documents` table's own soft-delete mechanism — `deleted = true`, fields preserved) for content-referencing rules.
- Content-blind rules (auth-presence-only, public) decided without needing any field data at all — `evaluate()` doesn't need `resource.data` for those shapes.
- A document that became non-compliant for a subscriber BEFORE deletion (Slice 04's own scenario) does not deliver its subsequent deletion to that subscriber either.

## OUT Scope
- The exact fetch mechanism (querying without `fetch_event`'s current `AND NOT deleted` filter, vs. some other approach) — DESIGN's call (OQ-SRRT-02); this slice locks the observable requirement only.
- Any change to the `documents` table's own soft-delete schema.

## Learning Hypothesis
**Disproves if it fails**: A delete event for a document the caller's rule would deny access to cannot be withheld without either leaking the document's prior existence or requiring a new non-leakage mechanism beyond the one `security-rules`/`security-rules-write-path` already established.
**Confirms if it succeeds**: The soft-delete mechanism (Finding 7) makes pre-deletion field data genuinely fetchable, so the identical `evaluate()`-based decision used for `Changed` events extends cleanly to `Removed` events.

## Acceptance Criteria
- AC-17-122: A delete of a document the subscriber's own rule would have admitted is delivered as a `DocumentDelete` event.
- AC-17-123: A delete of a document the subscriber's own rule would have denied is not delivered.
- AC-17-124: A document that became non-compliant for a subscriber before deletion does not deliver its subsequent deletion to that subscriber either.
- AC-17-125: A content-blind rule's delete events are decided and delivered correctly without requiring the deleted document's own field data.

## Production-Data Taste Test
Real delete of a document the subscriber's rule denies, real assertion no removal event reaches that subscriber.

## Dependencies
Slice 04 (extends the same per-event mechanism to the `Removed` case).

## Reference Class
Mirrors `security-rules`'s own AC-17-10 and `security-rules-write-path`'s own AC-17-34/38 existence non-leakage precedent, extended to a fan-out/delivery context none of the 4 priors had.
