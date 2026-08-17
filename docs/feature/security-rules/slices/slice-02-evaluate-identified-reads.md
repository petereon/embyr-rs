# Slice 02: Evaluate Rules for Identified Callers on Reads (Walking Skeleton)

**Story**: US-02 | **Release**: 1 | **Estimate**: 2 days | **job_id**: JOB-17

## Goal
A signed-in end user's `GetDocument` read is allowed or denied based on whether their verified identity and the target document satisfy the collection's rule.

## IN Scope
- Rule-evaluation function: given `request.auth` (`Some(VerifiedEndUserIdentity)` or `None`) and `resource.data` (the target document's fields), evaluate the v1-grammar condition to allow/deny.
- Wiring into `handle_get_document` — consuming the existing (currently-discarded) `_verified_identity` value from `attach_client_identity_if_present()`, not re-deriving it.
- Fail-closed behavior when a referenced field is absent from the document.
- Existence non-leakage: a denied read's response is identical regardless of whether the target document exists.
- PermissionDenied response, distinguishable from project-suspension and other existing rejection causes.

## OUT Scope
- Anonymous-session handling specifics (Slice 03 — this slice's evaluator must accept `None`, but the anonymous-specific scenarios are Slice 03's).
- No-rule-defined guardrail proof (Slice 04).
- Any RPC other than `GetDocument` (`RunQuery`, writes, `Listen`) — deferred, see feature-delta.md § Out of Scope.

## Learning Hypothesis
**Disproves if it fails**: a rule condition over `request.auth`/`resource.data` cannot be evaluated against a real `GetDocument` call without either re-verifying identity through a second, drift-prone code path, or requiring more expression-language investment than the constrained v1 grammar (Resolution 1, Option C) actually provides.
**Confirms if it succeeds**: the v1 grammar is expressive enough for the concrete owner-equality case, and the existing `_verified_identity` value is a sufficient, unmodified input — no second identity-verification code path is needed.

## Acceptance Criteria
- AC-17-06: Own-document read (condition true) succeeds unchanged from pre-feature behavior.
- AC-17-07: Other-user's read (condition false) denied PermissionDenied, attributable to the rule.
- AC-17-08: A non-ownership rule (e.g. "any signed-in caller") allows non-owners — grammar not limited to owner-equality.
- AC-17-09: Missing referenced field fails closed, never a crash/500.
- AC-17-10: Denied-read response never leaks document existence.

## Dependencies
- Slice 01 (a rule must exist to evaluate against).
- `client-auth`'s `attach_client_identity_if_present()` / `VerifiedEndUserIdentity` (DONE, merged) — consumed unmodified.

## Effort Estimate
2 days. Reference class: `client-auth` US-02 (sign-in + AC-16-08 guardrail, 1.5 days) plus incremental cost of the evaluation function itself (new computation, not present anywhere in the codebase today).

## Pre-Slice SPIKE
Not required — grammar scope locked at DISCUSS; evaluation is a pure, deterministic boolean computation with no external unknowns (no network, no clock-skew handling beyond what already exists for JWT `exp`).
