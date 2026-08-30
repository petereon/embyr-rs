# Slice 04 — AVG Aggregation, Postgres-Family Backend Modes

**Goal**: `RunAggregationQuery` with an AVG aggregation on a numeric field works end-to-end for `backend_mode` = `direct_pg`/`aws_secret`/`gcp_secret`, correctly distinguishing "no matching documents" (null average) from "average happens to be zero."

**Feature**: aggregation-queries
**Story**: US-04
**Estimated effort**: 0.75 day
**Sequence**: 4 of 4 (Release 2; requires Slice 03)

---

## IN Scope

- Extend `PostgresBackendAdapter::run_aggregation_query` to support `AVG(field)` — reuses Slice 03's own numeric-field-extraction/exclusion mechanism for both numerator and denominator.
- Zero-matching-documents case returns an absent/null `average` field in `aggregate_fields`, never `0`, never a divide-by-zero error.
- Extend the wire-level `Aggregation` enum handling in `grpc/handler.rs::handle_run_aggregation_query` to accept `AVG`.

## OUT Scope

- `backend_mode=agent` AVG — named follow-up, same reasoning as Slice 03.
- Multiple aggregations per request.

---

## Learning Hypothesis

**Disproves**: "AVG cannot be derived from the SAME numeric-field-extraction mechanism Slice 03 built (would require an independently-designed averaging path), OR the zero-matching-documents case cannot be distinguished cleanly from a real average of 0."

**Confirms if successful**: AVG is a thin composition on top of SUM's own exclusion semantics, and the null-vs-zero distinction is representable cleanly in the existing `aggregate_fields: {alias -> Value}` response shape (an absent map entry, not a `0` value).

---

## Acceptance Criteria

- [ ] AC-01-17: An AVG aggregation on a numeric field, filtered to a caller's own documents, returns the correct average across all documents holding a valid numeric value for that field.
- [ ] AC-01-18: A document missing the averaged field, or holding a non-numeric value, is excluded from both the numerator and denominator — mirrors Slice 03's own exclusion rule (AC-01-12).
- [ ] AC-01-19: Averaging across zero matching documents returns an absent/null average — never a divide-by-zero error, never reported as `0`.
- [ ] AC-01-20: AVG aggregation is governed by the identical access-rule compliance mechanism as COUNT/SUM (Slices 01/03).
- [ ] AC-01-21: Existing COUNT (Slices 01-02) and SUM (Slice 03) aggregations are unaffected by AVG's addition — zero regression.

---

## Dependencies

- Slice 03: `required` (reuses its numeric-field-extraction mechanism directly).

---

## Note

The null-vs-zero distinction for an empty result set is this slice's single highest-consequence design risk (a wrong default of `0` would be silently misleading to any dashboard consuming it) — candidate designated correctness-testing surface for DELIVER (mirrors this codebase's own "designated mutation-testing surface" convention for other single-highest-consequence arms, e.g. ADR-031's US-05).
