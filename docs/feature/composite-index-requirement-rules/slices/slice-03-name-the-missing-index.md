# Slice 03: The Rejection Names the Specific Missing Index (LAST slice)

**Story**: US-03 | **Release**: 2 | **Estimate**: 0.5-1 day

## Goal
The `FAILED_PRECONDITION` rejection names the specific `{collection_path, fields}` the missing
index needs, reusing `IndexFieldSpec`/`IndexFieldOrder` from `firestore-composite-indexes-admin
-api` — closing the loop that feature opened.

## IN Scope
- A pure function computing the required `Vec<IndexFieldSpec>` from a `StructuredQuery` that
  `requires_composite_index` has already flagged (derived from the SAME filter/orderBy fields
  Slices 01-02 already inspect — no new query analysis, just formatting what's already known).
- The rejection message includes this shape (real-Firestore-adjacent, not a transliterated proto —
  matches this codebase's own established convention, ADR-068).
- Real end-to-end proof: each of the 3 detected trigger shapes (Slices 01-02) produces a rejection
  naming the correct fields.

## OUT Scope
- Any change to `CreateIndex`/`ListIndexes`/`DeleteIndex` themselves (firestore-composite-indexes
  -admin-api, unchanged).

## Learning Hypothesis
Disproves: naming the specific missing index in the rejection message needs new production
infrastructure beyond reusing `firestore-composite-indexes-admin-api`'s own already-shipped types.

## Acceptance Criteria
AC-CIR-07 (see `feature-delta.md` § User Stories, US-03).

## Dependencies
Slices 01-02 (needs to know WHICH trigger fired to name the correct missing fields).

## Effort Estimate
0.5-1 day.

## Reference Class
Mirrors `security-rules-cel-functions`'s own "reuse an existing type, zero new type" discipline.

## Pre-Slice SPIKE
Not required.
