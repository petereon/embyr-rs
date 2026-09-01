# Slice 01: Import and Decompose a Real `.rules` File

**Story**: US-01 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 2 days

## Goal
Alex submits a real, simple `.rules` file (single top-level collections, at most one leaf-level wildcard per `match` block) and every in-scope block is decomposed into a call to the existing, unmodified `upsert_access_rule`/`upsert_write_access_rule` admin actions.

## IN Scope
- Parse the outer `service cloud.firestore { match /databases/{database}/documents { ... } }` wrapper.
- Parse `match /<single-collection>/{<optional-doc-id-var>} { allow <verbs>: if <condition>; }` blocks.
- Conditions use only the already-locked v1 grammar (comparison, `&&`/`||`/`!`, `resource.data`/`request.resource.data`/`request.auth.token.<claim>`, string/bool literals) — no new expressiveness beyond the leaf-level path variable (Slice 02's own concern, not this slice's).
- Decompose each block into the existing `upsert_access_rule` (read verbs) / `upsert_write_access_rule` (write verbs) calls, unmodified.
- Idempotent re-import (identical file twice → no duplicate state).

## OUT Scope
- Path-variable *evaluation* (this slice stores the block; Slice 02 makes the captured variable resolvable) — a block using a wildcard imports successfully here but is not yet enforced correctly until Slice 02 ships.
- Rejection of out-of-scope constructs (Slice 04).
- Non-interference proof (Slice 05).
- Any new storage table or schema change.

## Learning Hypothesis
Disproves: "A real `.rules` file's outer `service`/`match` syntax cannot be parsed and decomposed into the existing per-collection admin upsert calls without inventing a new storage shape or a new evaluation mechanism."
Confirms (if it holds): the outer syntax layer is genuinely just a translation step, not a new bounded-context concern.

## Acceptance Criteria
AC-17-174, AC-17-175, AC-17-176, AC-17-177, AC-17-178 (see feature-delta.md § User Stories, US-01).

## Dependencies
- `security-rules`'s existing `upsert_access_rule`/`upsert_write_access_rule` (DONE, shipped).
- None upstream within this feature.

## Effort Estimate
2 days. Reference class: `security-rules`'s own Slice 01 (1.5 days, single-condition define/redefine) — this slice is larger because it parses a whole file's worth of blocks, not one condition.

## Pre-Slice SPIKE
Not required — the decomposition target (existing upsert calls) is fully proven; only the outer-syntax parser is new, and its grammar is small and well-documented (real Firestore's own published rules-language reference).
