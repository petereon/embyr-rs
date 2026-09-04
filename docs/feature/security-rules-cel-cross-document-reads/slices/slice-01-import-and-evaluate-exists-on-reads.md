# Slice 01: Alex's Role-Lookup `exists()` Clause Parses and Enforces on Reads (Walking Skeleton)

**Story**: US-01 | **Release**: 1 | **Estimate**: 1.5 days

## Goal
Build the two-phase evaluation mechanism (path-discovery → fetch → evaluate) end-to-end for the
simplest case: a single `exists()` check gating a real `GetDocument` call.

## IN Scope
- New `Operand::CrossDocumentExists(PathTemplate)` where `PathTemplate` is a sequence of literal
  segments interspersed with `$(request.auth.uid)`/`$(request.path.<var>)` substitutions.
- Tokenizer/parser recognition of `exists(/databases/$(database)/documents/<segments>)` — the
  literal `$(database)` segment is always present and always resolves to the current
  `project_id` (mirrors real Firestore's own required-but-fixed `$(database)` convention).
- A new PURE `embyr-core` function: given a parsed `Condition` tree and the already-known bindings
  (auth uid, path variable value, ancestor bindings), returns the SET of distinct concrete
  document paths the condition's own `CrossDocumentExists`/`CrossDocumentGet` operands need.
- A new `embyr-server`-side fetch step: given that set, issue one real `BC-2` read per distinct
  path, build a `path → Option<FirestoreDocument>` map.
- `evaluate()`'s new 8th parameter: the pre-fetched map. `resolve_field_value`'s new
  `CrossDocumentExists` arm resolves to `FieldValue::Boolean(map.get(path).is_some())`.
- Wired into `handle_get_document` only this slice (write-path is Slice 04).
- A candidate condition using ANY substitution shape beyond `$(request.auth.uid)`/
  `$(request.path.<var>)` is a NAMED `UNSUPPORTED_EXPRESSION_GRAMMAR` rejection.

## OUT Scope
- `get()`'s own `.data.<field>` access (Slice 02).
- Deduplication proof (Slice 03) — dedup should already fall out of the path-discovery function
  returning a SET (not a list), but the explicit call-count proof is Slice 03's own concern.
- Write-path, simulation (Slices 04–05).
- Chaining, any other substitution shape (locked out of this feature's own scope entirely).

## Learning Hypothesis
Disproves: a two-phase (path-discovery → fetch → evaluate) mechanism cannot be built while keeping
`embyr-core` genuinely IO-free, without either loosening the `deny.toml` boundary or duplicating
the fetch logic per call site.

## Acceptance Criteria
AC-CDR-01 through AC-CDR-04 (see `feature-delta.md` § User Stories, US-01).

## Dependencies
None — first slice.

## Effort Estimate
1.5 days.

## Reference Class
New mechanism, nearest reference class: `resolve_access_rule_pattern`'s own "one indexed lookup
on the hot miss path" precedent (ADR-063), generalized from a fixed lookup to a data-dependent one.

## Pre-Slice SPIKE
Recommended, low-cost: confirm the exact `SharedBackendAdapter`/`get_document`-equivalent call
shape available for fetching an ARBITRARY document path (not just the request's own current
document) — verify before locking the fetch step's own exact signature in DESIGN.
