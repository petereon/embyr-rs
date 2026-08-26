# Slice 08: Alex Can Pre-Check Whether a Candidate Listen Subscription Would Be Admitted

**Story**: US-08 | **Release**: 2 | **Walking Skeleton**: No | **Estimate**: 1 day

## Goal
Extend `security-rules-query-path`'s own `simulate_query_compliance` admin action (unchanged) to be documented as also applicable to a candidate Listen subscription's own initial-snapshot filter shape — Alex can validate his `onSnapshot()` query code, and confirm a collection's rule will admit it, before shipping.

## IN Scope
- Documentation/framing that `simulate_query_compliance` (already built by `security-rules-query-path`) applies identically to a candidate Listen subscription's filter shape, since Listen's own subscribe-time gate reuses `check_query_compliance()` identically to `RunQuery`'s own.
- A candidate collection id with no rule reports "admitted" (content unrestricted), matching Slice 06's own unruled default — distinguishing "admitted because compliant" from "admitted because unruled" in the response.

## OUT Scope
- Any new response contract or new admin handler — reuses `simulate_query_compliance` completely unmodified.
- Simulating the per-event re-check (Slice 04's own mechanism) — subscribe-time simulation only, matching the one-time nature of admission.

## Learning Hypothesis
**Disproves if it fails**: A Listen-subscription simulation cannot share the exact same `check_query_compliance()` function real subscribe-time enforcement uses without duplicating the compliance logic a fifth time.
**Confirms if it succeeds**: Zero new mechanism is needed — this is purely a framing/documentation addition.

## Acceptance Criteria
- AC-17-134: Simulating a candidate query filter against a candidate rule, framed as a Listen subscription, returns the same admit/reject outcome real subscribe-time enforcement would produce.
- AC-17-135: Simulation correctly reports "missing required filter" for candidate filters lacking a required conjunct.
- AC-17-136: Simulation correctly reports "admitted" for a candidate collection id with no rule.
- AC-17-137: Simulating a Listen-subscription shape has zero effect on live/open Listen traffic.

## Production-Data Taste Test
Real candidate rules + real candidate query filters checked against the real compliance function, framed as a candidate Listen subscription.

## Dependencies
Slices 02–03 (the compliance mechanism this wraps).

## Reference Class
Mirrors all 4 priors' own Release-2 simulation story precedent (`security-rules` US-05, `security-rules-write-path` US-07, `security-rules-query-path` US-07, `security-rules-collection-group-rules` US-07).
