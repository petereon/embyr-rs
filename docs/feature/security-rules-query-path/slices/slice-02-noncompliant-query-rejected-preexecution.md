# Slice 02: Non-Compliant Query Rejected Before Execution (Walking Skeleton)

**Story**: US-02 | **Release**: 1 | **Estimate**: 1 day

## Goal
A `RunQuery` call missing the required matching filter — no filter at all,
an unrelated filter, or the wrong operator — is rejected outright, before
any row is fetched from Postgres, with a reason naming the missing
constraint and distinguishable from other rejection classes.

## IN Scope
- The "absent/non-matching required conjunct" branch of Slice 01's
  compliance function.
- A distinguishable rejection response (exact status/reason-code shape is
  DESIGN's call) naming that the query is missing the rule's required
  equality filter.
- Confirmation the rejection occurs strictly before `adapter.run_query()` —
  zero Postgres I/O for a rejected query.

## OUT Scope
- The compliance function's core matching logic itself (Slice 01, reused).
- Auth-presence-only / undecidable-shape rejections (Slices 03/05, distinct
  branches).

## Learning Hypothesis
**Disproves**: "A non-compliant query cannot be rejected before touching
Postgres, with a specific and distinguishable reason, using only the
already-translated `StructuredQuery`." Confirmed false if an unfiltered or
wrongly-filtered `RunQuery` is refused with zero document-row fetches and a
reason distinct from `authenticate()`-level or composite-index rejections.

## Acceptance Criteria
- AC-17-53: A `RunQuery` with no filter, against a rule requiring a matching
  conjunct, is rejected before any document is fetched.
- AC-17-54: A `RunQuery` with unrelated filters but no matching conjunct is
  rejected.
- AC-17-55: A different operator (e.g. `!=`) on the rule's referenced field
  does not satisfy an equality rule.
- AC-17-56: The rejection response names the specific missing constraint,
  distinguishable from `authenticate()`-level and composite-index
  rejections.

## Dependencies
- Slice 01 (compliance function, happy-path branch).

## Reference Class
Mirrors `security-rules`' US-02's denial scenario (own-doc allow / other-doc
deny, same story) and `security-rules-write-path`'s create-denial scenario
(US-02) — a single story combining admit + reject branches of the SAME
mechanism.

## Pre-Slice SPIKE
Not required — depends directly on Slice 01's mechanism.
