# Slice 04: Alex's Whitelist Clause Parses and Enforces on Writes

**Story**: US-04 | **Release**: 2 | **Estimate**: 1 day

## Goal
Add list literals and the `in` membership operator to the grammar, wired into real write-path
enforcement.

## IN Scope
- Tokenizer: `[`, `]`, `,` as new structural tokens.
- New `Operand::ListLiteral(Vec<Operand>)` — a bracketed, comma-separated list of string and/or
  numeric literals.
- New `Condition::In(Operand, Operand)` (or equivalent) grammar production: `<operand> in
  <list literal>`, distinct from `Compare` (membership, not equality/relational comparison).
- `evaluate()`: membership check — the left operand's resolved `FieldValue` compared for equality
  against every element of the list literal.
- `in` against a non-list-literal RHS (e.g. a map-valued field) is rejected as a NAMED unsupported
  construct (AC-CEG-12) — never silently misevaluated as always-false.
- Real `CreateDocument`/`UpdateDocument` enforcement proof.

## OUT Scope
- Read-path/simulation parity (Slice 05).
- Map literals, map-key membership (locked out of this feature's scope entirely — Resolution 2).
- Timestamp/duration (Release 3).

## Learning Hypothesis
Disproves: `in` + list-literal support cannot be expressed as ONE new `Operand::ListLiteral` + ONE
new `Condition::In` variant without a second, parallel comparison mechanism alongside the existing
`Condition::Compare`.

## Acceptance Criteria
AC-CEG-09 through AC-CEG-12 (see `feature-delta.md` § User Stories, US-04).

## Dependencies
None beyond Release 1's own grammar architecture (reuses the same `Operand`/`Condition` shape, but
introduces no dependency on Release 1's specific numeric literals — string-literal list membership
works independently).

## Effort Estimate
1 day.

## Reference Class
Nearest reference class: `AuthTokenClaim`'s own "one new operand family" precedent (4a's
custom-claims epic) — a genuinely new grammar production, not a mechanical extension of an existing
one.

## Pre-Slice SPIKE
Not required.
