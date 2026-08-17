# Slice 03: Evaluate Rules for Anonymous (Never-Signed-In) Sessions (Walking Skeleton)

**Story**: US-03 | **Release**: 1 | **Estimate**: 1 day | **job_id**: JOB-17

## Goal
A session with no verified end-user identity is evaluated against a rule as `request.auth == null` — giving `client-auth`'s optional identity real behavioral consequence for the first time.

## IN Scope
- Treating `attach_client_identity_if_present()`'s `None` result (absent header) as `request.auth == null` in rule evaluation.
- Treating an invalid client-identity header (malformed/expired/wrong-project — also `None` per existing ADR-026 DDD-CA-5 semantics) identically to an absent header for rule-evaluation purposes — no new rejection class.
- Proving a rule requiring `request.auth != null` denies anonymous callers, and a rule allowing `if true` does not.

## OUT Scope
- Any change to `attach_client_identity_if_present()`'s own logic — this slice consumes its existing output unmodified.
- Identified-caller evaluation specifics (Slice 02).
- No-rule-defined guardrail (Slice 04).

## Learning Hypothesis
**Disproves if it fails**: rules cannot meaningfully consume `client-auth`'s "optional identity" without inventing a new, second rejection channel for invalid identity — i.e., that reusing ADR-026's existing "attach nothing" semantics unchanged is insufficient for rule purposes.
**Confirms if it succeeds**: `client-auth`'s existing `None`-on-invalid semantics is exactly the right, sufficient input for rules — zero changes needed to `client-auth`'s own code.

## Acceptance Criteria
- AC-17-11: Anonymous session denied by a rule requiring `request.auth != null`, attributable to the rule.
- AC-17-12: Anonymous session succeeds against a rule explicitly allowing unauthenticated access.
- AC-17-13: Invalid (malformed/expired/wrong-project) identity header evaluated identically to absent header — no new rejection class introduced.

## Dependencies
- Slice 02 (the evaluation function this slice exercises with `request.auth = None`).
- `client-auth`'s existing ADR-026 DDD-CA-5 semantics (DONE, merged, unmodified).

## Effort Estimate
1 day. Reference class: mostly test/proof work over Slice 02's evaluator — no new production abstraction, matching `client-auth`'s own pattern of a thin extension slice following its evaluation-routine slice.

## Pre-Slice SPIKE
Not required.
