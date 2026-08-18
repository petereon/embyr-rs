# Slice 07: Simulate a Candidate Query Before Shipping Client Code

**Story**: US-07 | **Release**: 2 | **Estimate**: 1 day

## Goal
Alex can call the existing rule-simulation admin action, extended to accept
a candidate query filter shape, and see whether that exact query shape
would be admitted or rejected against a candidate or published rule —
without issuing any real query — catching a missing-filter or
unsupported-rule-shape bug in his own testing before it reaches a real end
user.

## IN Scope
- Extend `simulate_access_rule`'s request body with an optional candidate
  `QueryFilter` shape.
- The extended handler calls the SAME compliance function Slices 01–05 use
  for real enforcement — no second implementation.
- Zero effect on live/published `RunQuery` traffic.

## OUT Scope
- Any new endpoint (extends the existing simulate action, per US-07's own
  steer, mirroring `security-rules-write-path`'s own US-07 extension of the
  same handler).

## Learning Hypothesis
**Disproves**: "A query-shape simulation cannot share the exact same
compliance function real enforcement uses without duplicating (and risking
drift in) the decidable-shape logic." Confirmed false if simulating a
candidate filter against a published rule produces the identical admit/
reject outcome a real `RunQuery` would.

## Acceptance Criteria
- AC-17-73: Simulating a candidate query filter against a candidate rule
  returns the same admit/reject outcome real enforcement would produce.
- AC-17-74: Simulation correctly reports "missing required filter" for
  candidate filters lacking a required conjunct.
- AC-17-75: Simulation correctly reports "rule shape not supported" for
  candidate rules outside the decidable set.
- AC-17-76: Simulating a query shape has zero effect on live/published
  `RunQuery` traffic.

## Dependencies
- Slices 01–05 (the compliance function this slice wraps, not duplicates).
- `security-rules`'s/`security-rules-write-path`'s existing
  `simulate_access_rule` handler (extended a third time).

## Reference Class
Mirrors `security-rules`'s US-05 and `security-rules-write-path`'s US-07 —
both prior epics' own Release-2 simulation-extension pattern, applied a
third time to the same handler.

## Pre-Slice SPIKE
Not required.
