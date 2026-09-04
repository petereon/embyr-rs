# Slice 06: Alex's Time-Window Clause Parses and Enforces on Writes

**Story**: US-06 | **Release**: 3 | **Estimate**: 1.5 days

## Goal
Add `request.time`, `duration.value(N, unit)`, and a narrowly-scoped `+`/`-` arithmetic operand
pairing (timestamp +/- duration only, non-nested), wired into real write-path enforcement.

## IN Scope
- New `Operand::RequestTime`, resolving to a caller-supplied "now" `FieldValue::Timestamp` threaded
  into `evaluate()` (new parameter, mirrors `path_variable_value`'s zero-new-I/O precedent — the
  value is already computed at every real write-handler call site).
- `duration.value(<integer literal>, '<unit>')` recognized via the existing call-syntax scan
  (`detect_unsupported_construct`'s identifier-then-`(` shape), parsed ONLY as the right-hand
  operand of a new `+`/`-` arithmetic pairing against a `Timestamp`-typed left operand — never a
  standalone operand. Units: `s`, `m`, `h`, `d` at minimum; unrecognized unit is a NAMED rejection
  (AC-CEG-18).
- New `Operand::Arithmetic` (or equivalent minimal AST node) evaluating `Timestamp +/- Duration ->
  Timestamp`, then participating in Slice 01's own relational-comparison grammar unchanged.
- `*`, `/`, `%`, and nested arithmetic (`a + b + c`, `(a+b)*c`) rejected as a NAMED unsupported
  construct (AC-CEG-19) — never a bare `SyntaxError`.
- Real `CreateDocument`/`UpdateDocument` enforcement proof.

## OUT Scope
- Read-path/simulation parity (Slice 07).
- Any arithmetic beyond the ONE evidenced timestamp+duration pairing (Integer/Double arithmetic
  stays unsupported — no domain example needs it).

## Learning Hypothesis
Disproves: `request.time` + `duration.value(...)` + scoped `+`/`-` arithmetic cannot be expressed
as an `Operand::RequestTime` + a narrowly-scoped `Operand::Arithmetic` AST node without either a
general-purpose expression-evaluator rewrite or silently unbounded arithmetic generality.

## Acceptance Criteria
AC-CEG-15 through AC-CEG-19 (see `feature-delta.md` § User Stories, US-06).

## Dependencies
Slice 01 (numeric literals — duration amounts are themselves numeric literals). Independent of
Release 2 (Slices 04-05).

## Effort Estimate
1.5 days — the highest-complexity slice in this feature (2 new operand families: `RequestTime` and
the scoped arithmetic pairing).

## Reference Class
Nearest reference class: 4b′'s own `bind_recursive_prefix` (a genuinely new pure primitive, built
once, reused at every call site) — the SAME "new primitive family, evidenced narrowly, designed to
widen additively later" shape.

## Pre-Slice SPIKE
Recommended, low-cost: confirm the exact `chrono`/timestamp arithmetic semantics available in
`embyr-core` (already zero-IO, likely already a dependency given `FieldValue::Timestamp` exists) —
verify before locking the exact `Operand::Arithmetic` AST shape in DESIGN.
