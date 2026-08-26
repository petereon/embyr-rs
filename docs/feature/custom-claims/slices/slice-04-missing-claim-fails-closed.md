# Slice 04: A Missing Claim Fails Closed

**Story**: US-04 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 0.5 day

## Goal
A rule referencing a claim absent from the caller's verified identity (or an anonymous/no-identity session) evaluates to Deny, never crashes, and never silently allows.

## IN Scope
- Reuse of the existing `FieldMissing` short-circuit mechanism (`eval_bool`'s `?` propagation) for `AuthTokenClaim` resolution against an absent key in `AuthContext.claims`.
- Anonymous-session (`auth: None`) behavior against a claim-referencing rule — reuses the existing `auth.is_none()` fail-closed path.
- Type-mismatch behavior (claim present but wrong `FieldValue` variant) — resolves via ordinary `PartialEq`, denies without special-casing.

## OUT Scope
- Any new error type or rejection class for "claim missing" distinct from the existing `FieldMissing`/`Deny` path.
- Query-path safety (US-05) — a separate, distinct concern.

## Learning Hypothesis
**Disproves if it fails**: that a missing claim can be made to fail closed using the exact same `FieldMissing` mechanism already proven for resource fields, without a new special case. If a new case is needed, ADR-027's own "no per-operator null-propagation semantics beyond plain short-circuit" discipline may need reconsideration for claims.

**Confirms if it succeeds**: claims integrate into the existing fail-closed model with zero new semantics.

## Acceptance Criteria
- AC-17-146: A condition referencing a claim absent from the caller's verified identity evaluates to Deny, never crashes.
- AC-17-147: An anonymous caller against a claim-referencing rule is denied, identically to how an anonymous caller is denied against a uid-referencing rule.

## Dependencies
Depends on Slice 02 (reuses its operand/resolution code).

## Production-Data Taste Test
Real session with a token minted WITHOUT the referenced claim; real anonymous (no client-identity header) session; both against a real claim-gated collection.

## Effort Estimate
0.5 day. Pure proof-obligation slice, mirrors Slice 03's own shape.

## Pre-Slice SPIKE
Not needed.
