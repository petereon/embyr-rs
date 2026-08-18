# Slice 05: Undecidable Rule Shape Rejects Every Query Outright (Walking Skeleton)

**Story**: US-05 | **Release**: 1 | **Estimate**: 1.5 days

## Goal
A `RunQuery` against a collection whose read rule uses a shape this
feature's mechanism cannot prove compliant — `Condition::Or`,
`Condition::Not`, or a reference to the write-only `RequestResourceField`
operand — is rejected outright, for every query against that collection,
regardless of filter shape. This is the single highest-consequence branch
in the whole feature: a missing or wrong default here is a false-allow.

## IN Scope
- The default/fallthrough arm of the compliance function for any
  `Condition` variant not explicitly matched by Slices 01/03/04's decidable
  branches.
- New Trailmark domain example: `trip_comments` (rule combining ownership
  and public-visibility with `||`).
- A rejection reason distinguishable from Slice 02's "missing required
  filter" rejection — "rule shape not supported for query enforcement."
- Never-crash guarantee for a degenerate read rule referencing
  `request.resource.data.<field>`.

## OUT Scope
- Any attempt to make Or/Not decidable (explicitly out of scope per
  Resolution 1, Option A rejection).

## Learning Hypothesis
**Disproves**: "A rule shape this feature cannot prove compliant cannot be
safely refused outright for every query, without either crashing or
silently defaulting to allow." Confirmed false if an OR-shaped rule
rejects every query — including one that would satisfy one disjunct if
evaluated — and a degenerate `RequestResourceField` reference denies rather
than panics.

## Acceptance Criteria
- AC-17-65: A query against a rule containing `Condition::Or` anywhere is
  rejected, regardless of filter shape.
- AC-17-66: A query against a rule containing `Condition::Not` anywhere is
  rejected.
- AC-17-67: A query against a rule referencing `RequestResourceField` is
  rejected, never crashes.
- AC-17-68: The "unsupported rule shape" rejection is distinguishable from
  the "missing required filter" rejection (Slice 02).

## Dependencies
- Slice 01 (compliance function scaffold — this slice is its default arm).

## Reference Class
Mirrors `security-rules`'s AC-17-09 (fail-closed-on-missing-field, "never
crashes, always denies") in spirit — the equivalent fail-closed-by-default
discipline applied to rule SHAPE rather than field PRESENCE.

## Pre-Slice SPIKE
Not required — decidability was determined directly from `QueryFilter`'s
confirmed AND-only shape during this DISCUSS.
