# Slice 02: A Boolean-Claim Rule Gates a GetDocument Read

**Story**: US-02 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1.5 days

## Goal
Alex defines a rule referencing `request.auth.token.<claim>` (boolean value, or a claim-to-resource-field comparison), and a `GetDocument` call is correctly allowed or denied based on it.

## IN Scope
- New `Operand::AuthTokenClaim(String)` variant in `embyr-core::access_control`.
- New `"request.auth.token."`-prefix branch in `word_to_operand()`, mirroring the existing `"resource.data."`/`"request.resource.data."` branches.
- New `AuthContext.claims: BTreeMap<String, FieldValue>` field.
- Resolution/comparison logic in `compare_operands`/`resolve_field_value` for `AuthTokenClaim`, including the claim-to-resource-field comparison case (falls through to existing `FieldValue::PartialEq`).
- Boolean-literal and null-literal comparisons against a claim (`== true`, `!= null`).
- Grammar composition: a claim check combined with existing ownership/auth-required conditions via `&&`/`||`.

## OUT Scope
- String-literal claim comparisons (US-06, Release 2) — OQ-SR-04 remains unresolved for this slice.
- Write-path wiring (US-03) — this slice proves GetDocument only.
- Missing-claim fail-closed proof as its own dedicated story (US-04) — this slice's own domain example 3 touches it but the dedicated proof is US-04's job.

## Learning Hypothesis
**Disproves if it fails**: that a rule referencing `request.auth.token.<claim>` can be parsed and evaluated within the existing `Condition`/`Operand`/`evaluate()` structure, mirroring `RequestResourceField`'s own precedent, without a new AST or a new evaluator function.

**Confirms if it succeeds**: Resolution 2's central claim that the grammar extends cleanly via the established `Operand`-addition playbook.

## Acceptance Criteria
- AC-17-141: A rule referencing `request.auth.token.<claim>` with a boolean claim value correctly allows/denies.
- AC-17-142: A rule combining a claim check with existing grammar (`&&`/`||`) evaluates correctly.
- AC-17-143: A claim-to-resource-field comparison correctly allows/denies via ordinary `FieldValue` equality.

## Dependencies
Depends on Slice 01 (claims must exist on `AuthContext` before a rule can reference one).

## Production-Data Taste Test
Real `flagged_content` rule defined via the existing admin API, real Priya Nair (`is_moderator: true`) and Maria/Dana (no such claim) sessions, real `GetDocument` gRPC calls against real System DB rule state.

## Effort Estimate
1.5 days. Reference class: `security-rules-write-path`'s own `RequestResourceField` addition (ADR-030) — comparable scope.

## Pre-Slice SPIKE
Not needed — the mechanism is directly precedented by `RequestResourceField`, confirmed by direct code read.
