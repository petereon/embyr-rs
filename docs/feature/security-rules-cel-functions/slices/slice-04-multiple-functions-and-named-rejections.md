# Slice 04: Multiple Functions, Multiple Call Sites, Every Scoped-Out Construct Named (LAST slice)

**Story**: US-04 | **Release**: 2 | **Estimate**: 1 day

## Goal
Prove the mechanism generalizes beyond the single-function walking skeleton, and lock in
distinguishable named rejections for every scoped-out construct (Resolutions 2–4).

## IN Scope
- A file with 2+ function definitions, each called from a different `match` block (AC-CF-07).
- The SAME function called from 2 different `match` blocks (AC-CF-08).
- `FUNCTION_PARAMETERS_UNSUPPORTED`: a function definition with a non-empty parameter list, AND
  separately a call site with non-empty argument text (AC-CF-09).
- `DUPLICATE_FUNCTION`: two function definitions sharing the same name (AC-CF-10).
- Nesting rejection: a function whose own body calls another function — proven to reuse the
  pre-existing, unmodified `CUSTOM_FUNCTION` construct tag (AC-CF-11), not a new tag.

## OUT Scope
- Actually building parameters, nesting, or `let` bindings — this slice proves REJECTION, never
  builds the capability (Resolutions 2–4, locked out of v1 entirely).

## Learning Hypothesis
Disproves: the substitution mechanism only works for the single-function walking-skeleton shape
and cannot generalize to multiple definitions/call sites, or cannot correctly and distinguishably
reject every scoped-out construct in one pass.

## Acceptance Criteria
AC-CF-07 through AC-CF-11 (see `feature-delta.md` § User Stories, US-04).

## Dependencies
Slice 01.

## Effort Estimate
1 day.

## Reference Class
Mirrors `decompose`'s own existing "collect every offending block, never fail on just the first"
discipline, applied here to definition-time validation.

## Pre-Slice SPIKE
Not required.
