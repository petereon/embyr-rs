# Slice 02: The Same Expanded Condition Gates Writes — Zero New Production Code

**Story**: US-02 | **Release**: 1 | **Estimate**: 0.5 day (confirmatory)

## Goal
Prove real `CreateDocument`/`UpdateDocument` enforcement against a function-authored rule, with
zero production code beyond Slice 01's own import-time expansion.

## IN Scope
- Acceptance test only: import a function-call-bearing rule (Slice 01's own mechanism), issue a
  real `CreateDocument`/`UpdateDocument` call, assert correct allow/deny.

## OUT Scope
- Any production code change — if this slice needs one, ADR-067's own central claim (Resolution 5)
  is wrong and must be revisited before proceeding.

## Learning Hypothesis
Disproves: write-path enforcement needs ANY production code change beyond Slice 01's own
import-time expansion.

## Acceptance Criteria
AC-CF-05 (see `feature-delta.md` § User Stories, US-02).

## Dependencies
Slice 01.

## Effort Estimate
0.5 day.

## Reference Class
Mirrors `security-rules-cel-expression-grammar`'s own Slices 02/03 "confirmatory, zero production
code" precedent — the strongest form yet (Slice 01's own mechanism needs no per-surface wiring at
all, unlike 4c's own per-`Operand`-type wiring).

## Pre-Slice SPIKE
Not required.
