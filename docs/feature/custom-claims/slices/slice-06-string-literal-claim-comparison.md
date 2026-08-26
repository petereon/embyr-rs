# Slice 06: Alex Compares a Claim to a String Literal

**Story**: US-06 | **Release**: 2 | **Walking Skeleton**: No | **Estimate**: 1.5 days

## Goal
A rule can compare a claim (or any operand) to a quoted string literal, e.g. `request.auth.token.department == "billing"` — resolving OQ-SR-04 for the whole grammar, scoped narrowly to strings only.

## IN Scope
- New `tokenize()` branch recognizing quoted string literals (`'...'` or `"..."` — DESIGN's call on which quote style, or both).
- New `Operand::StringLiteral(String)` variant.
- Comparison semantics for `StringLiteral` against `AuthTokenClaim`/`ResourceField`/`RequestResourceField` (falls through to `FieldValue::PartialEq`, mirroring existing literal-comparison patterns).
- Malformed/unterminated string literal → plain `SyntaxError`, distinguishable from `UnsupportedConstruct`.

## OUT Scope
- Numeric-literal support (explicitly out of scope, Resolution 3's own narrow framing).
- Escape-sequence support inside string literals beyond what's minimally needed for the domain examples (e.g. no need for `\"` escaping unless a real domain example requires it — flag as a further narrowing if DESIGN finds it adds meaningful scope).
- Claim-aware query-shape compliance (Slice 05's own boundary is unaffected by this slice).

## Learning Hypothesis
**Disproves if it fails**: that a rule can compare an operand to a string literal without a genuinely new tokenizer branch — if the existing `Word` token type could somehow be reused instead, this slice's own scope is smaller than estimated (a positive surprise, not a blocker).

**Confirms if it succeeds**: OQ-SR-04 is resolved for the domain examples this feature actually needs (role-based string comparison), without widening the grammar beyond that.

## Acceptance Criteria
- AC-17-151: A claim compared to a string literal parses and evaluates correctly.
- AC-17-152: A resource field compared to a string literal also now works (proving the fix is general, not claims-specific).
- AC-17-153: An unterminated/malformed string literal is rejected as a plain syntax error.

## Dependencies
Depends on Slice 02 (the `Operand`/parser structure this slice extends).

## Production-Data Taste Test
Real `support_tickets` rule `request.auth.token.department == "billing"`, real Jordan Lee session, real string-literal parse/evaluate round-trip against real System DB rule state.

## Effort Estimate
1.5 days. New tokenizer work is the largest single unit of new code in this feature.

## Pre-Slice SPIKE
Not needed — string-literal tokenizing is a well-understood parsing task; no unresolved unknowns identified during DISCUSS.
