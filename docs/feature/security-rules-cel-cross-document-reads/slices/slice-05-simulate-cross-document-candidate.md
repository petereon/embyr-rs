# Slice 05: Alex Simulates a Cross-Document Candidate Rule Before Publishing It (LAST slice)

**Story**: US-05 | **Release**: 2 | **Estimate**: 1 day

## Goal
Extend `simulate_access_rule` to accept a caller-supplied SYNTHETIC set of referenced documents,
so a cross-document candidate condition can be tested with zero real backend fetch.

## IN Scope
- `SimulateAccessRuleBody` gains a new `cross_document_reads: BTreeMap<String, BTreeMap<String,
  serde_json::Value>>` field (concrete path → synthetic field map), translated via the EXISTING
  `json_value_to_field_value` helper — reused, not duplicated.
- `simulate_access_rule` resolves `CrossDocumentExists`/`CrossDocumentGet` operands against this
  synthetic map directly — no path-discovery/real-fetch step at all (mirrors `resource`/
  `request_resource`'s own existing synthetic-input discipline, and `request_time`'s own
  identical precedent from 4c's own Slice 07).
- A path referenced by the candidate condition but ABSENT from the synthetic map simulates
  identically to a real nonexistent document.

## OUT Scope
- Any change to real enforcement's own path-discovery/fetch mechanism (Slices 01-04) — this
  slice is additive to `simulate_access_rule` only.

## Learning Hypothesis
Disproves: `simulate_access_rule` cannot share the SAME two-phase mechanism real enforcement uses
without either a second, independently-maintained fetch path or a synthetic-document escape
hatch. (Unlike every prior CEL-parity epic's own simulation slice, which needed ZERO production
code — this is the first simulation slice in the whole initiative that genuinely needs new
production code, since real enforcement's own mechanism involves real I/O simulation cannot use.)

## Acceptance Criteria
AC-CDR-13, AC-CDR-14 (see `feature-delta.md` § User Stories, US-05).

## Dependencies
Slices 01-02 (the `Operand` shapes/resolution logic this slice reuses, minus the real-fetch step).

## Effort Estimate
1 day.

## Reference Class
New: simulation needs a CALLER-SUPPLIED synthetic document set (mirrors `request_time`'s own
synthetic-input precedent from 4c's own Slice 07), not a real fetch.

## Pre-Slice SPIKE
Not required.
