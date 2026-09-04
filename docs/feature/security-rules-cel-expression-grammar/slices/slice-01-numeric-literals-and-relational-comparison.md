# Slice 01: Alex's Numeric-Bound Clause Parses and Enforces on Reads (Walking Skeleton)

**Story**: US-01 | **Release**: 1 | **Estimate**: 1 day

## Goal
Add integer/double literals and relational comparison operators (`<`, `<=`, `>`, `>=`) to the
condition grammar, wired into real `GetDocument` read-path enforcement.

## IN Scope
- Tokenizer: digit-starting tokens produce a new numeric-literal token (integer, and a decimal
  variant for doubles); `-` immediately preceding a digit with no preceding operand is a negative
  literal.
- New `Operand::IntLiteral(i64)` / `Operand::DoubleLiteral(f64)`.
- New `CompareOp::Lt`/`Le`/`Gt`/`Ge`, parsed from `<`, `<=`, `>`, `>=` tokens.
- `evaluate()`'s comparison logic: numeric relational comparison between two numeric operands
  (`ResourceField` resolving to `FieldValue::Integer`/`Double`, or a numeric literal), well-defined
  `Deny` (never panic) on a type mismatch (numeric operand vs. a non-numeric resolved field).
- Real `GetDocument` enforcement proof (not just parse-and-discard).

## OUT Scope
- Write-path, simulation (US-02/US-03).
- `in`, list literals, timestamp/duration, any arithmetic operator (later slices/releases).
- Mixed Integer/Double comparison coercion beyond what `FieldValue::PartialEq`-adjacent comparison
  already needs — if it requires a new comparison helper, keep it minimal (numeric-vs-numeric only).

## Learning Hypothesis
Disproves: a numeric-literal + relational-comparison grammar extension cannot share the existing
`Operand`/`Condition`/tokenizer architecture without either a second, parallel parser or a
disruptive rewrite of the existing string/bool-literal handling.

## Acceptance Criteria
AC-CEG-01 through AC-CEG-05 (see `feature-delta.md` § User Stories, US-01).

## Dependencies
None — first slice.

## Effort Estimate
1 day.

## Reference Class
Mirrors 4a's own Slice 01 (first grammar-widening slice of a CEL-parity epic) and custom-claims'
own `StringLiteral`/`BoolLiteral` addition precedent (ADR-034) — one new literal-token family,
uniform propagation into `Operand`/tokenizer/parser.

## Pre-Slice SPIKE
Not required — direct code read (`feature-delta.md` § Reading Confirmation) already confirmed the
exact tokenizer/parser shape to extend.
