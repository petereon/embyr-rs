# Slice 04: Reject Out-of-v1-Scope Imports Atomically

**Story**: US-04 | **Release**: 1 | **Estimate**: 1.5 days

## Goal
An import containing any construct outside this feature's locked v1 shape (nested/multi-segment collection paths, recursive wildcards, multiple wildcards per block, custom `function` calls, `get()`/`exists()` calls, or a condition referencing an undefined path-variable name) is rejected in full — zero partial application — naming every offending block individually.

## IN Scope
- Whole-file validation pass BEFORE any existing rule is touched (validate-then-apply-atomically, never apply-as-you-go).
- Per-block, per-construct-named rejection reasons, distinguishable from a plain grammar syntax error (mirrors `security-rules`'s own AC-17-03/04 precedent).
- Detection of a condition referencing a path-variable name not captured by its own enclosing block (a new, block-scoped validation this feature's context-dependent parsing introduces).
- Multiple offending blocks in one file are all named in a single response, not just the first found.

## OUT Scope
- Actually supporting any of the rejected constructs (Epics 4b/4d/4e).
- Any change to how an already-*stored* rule is rejected/validated (unchanged, existing behavior for the JSON API).

## Learning Hypothesis
Disproves: "An import containing a mix of in-scope and out-of-v1-scope `match` blocks cannot be rejected as a single atomic unit, naming every offending block individually, without either a partial-apply hazard or an unhelpfully generic rejection."

## Acceptance Criteria
AC-17-188, AC-17-189, AC-17-190, AC-17-191, AC-17-192, AC-17-193 (see feature-delta.md § User Stories, US-04).

## Dependencies
- Slice 01 (the parser this slice's validation pass extends).

## Effort Estimate
1.5 days. Reference class: `security-rules`'s own Slice 01 rejection-taxonomy work, extended from one condition to a whole file's worth of blocks (more enumeration, same mechanism class).

## Pre-Slice SPIKE
Not required — every rejected construct is drawn directly from real Firestore's own published rules-language reference; no ambiguity in what must be detected.
