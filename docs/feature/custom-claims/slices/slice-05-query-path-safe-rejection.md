# Slice 05: A Claim-Referencing Rule Fails Safely, Not Silently, Against Query-Path Surfaces

**Story**: US-05 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1 day

## Goal
Prove that `RunQuery` (non-group and group) and a `Listen` subscription's subscribe-time compliance check both reject a claim-referencing rule outright (`RejectedUnsupportedRuleShape`) — safely, not silently — while GetDocument/writes/Listen's per-event recheck remain fully functional against the identical rule.

## IN Scope
- Acceptance-test proof that `check_query_compliance()`'s existing `decompose_decidable()` catch-all (`_ => Err(Undecidable)`) correctly rejects a `Condition` tree containing `Operand::AuthTokenClaim` for `RunQuery`, collection-group `RunQuery`, and `Listen`'s subscribe-time gate.
- Cross-check that GetDocument/writes/Listen-per-event (all `evaluate()`-based) remain unaffected by this limitation.

## OUT Scope
- Any new `embyr_core::access_control` code — if this slice requires new code to pass, Resolution 4's safety claim is disproven and DESIGN must add an explicit rejection path.
- Extending `check_query_compliance()`'s own decidable-atom set to actually SUPPORT claim-based query compliance — that is a named, deferred follow-up (§ Out of Scope), not this slice's job.

## Learning Hypothesis
**Disproves if it fails**: that a claim-referencing rule is automatically, safely rejected by query-path surfaces via the ALREADY-EXISTING catch-all, without any new code. If `check_query_compliance()` instead crashes, silently admits, or silently mis-filters, this is the single highest-consequence finding in this feature and must block release until fixed.

**Confirms if it succeeds**: this feature's own documented limitation (§ Out of Scope) is a real safety property, not a hopeful assumption.

## Acceptance Criteria
- AC-17-148: A `RunQuery` (non-group and group) against a claim-referencing rule is rejected outright.
- AC-17-149: A `Listen` subscription's subscribe-time compliance check against a claim-referencing rule is likewise rejected outright.
- AC-17-150: GetDocument, write-path, and Listen's per-event recheck remain unaffected by this limitation.

## Dependencies
Depends on Slice 02 (needs a real claim-referencing rule to test against); read-only interaction with `check_query_compliance()` (no code change expected).

## Production-Data Taste Test
Real `flagged_content` rule referencing the claim; real `RunQuery`/collection-group/Listen-subscribe attempts against it; real assertion of outright rejection; zero new `embyr_core::access_control` code touched (verified via diff, not just test pass).

## Effort Estimate
1 day. Reference class: `security-rules-realtime`'s own US-06/US-07 proof-obligation slices (regression + structural-independence proof).

## Pre-Slice SPIKE
Not needed — the safety mechanism (the existing catch-all) is confirmed by direct code read before this slice begins; the slice exists to prove it under real conditions, not to discover whether it exists.
