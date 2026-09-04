# Slice 03: Two References to the Same Document Are Fetched Once

**Story**: US-03 | **Release**: 1 | **Estimate**: 1 day

## Goal
Prove, via a direct adapter call-count assertion (not merely a correct outcome), that
`exists()` and `get()` referencing the identical concrete path within one condition result in
exactly one real backend fetch.

## IN Scope
- A real acceptance test combining `exists(<path>)` and `get(<path>).data.<field>` on the SAME
  concrete path in one `&&`-composed condition, asserting the underlying adapter's own
  document-fetch call count is exactly 1 for that path.
- A second real acceptance test proving TWO DIFFERENT concrete paths each get their own real
  fetch (dedup is per-path, never a blanket single-fetch-per-evaluation cap).

## OUT Scope
- Any new production code — if Slice 01's own path-discovery function already returns a SET
  (not a list) of distinct paths, and the fetch step iterates that set once, dedup should already
  hold structurally. This slice's own job is to PROVE it, and to fix it if the proof fails.

## Learning Hypothesis
Disproves: two `get()`/`exists()` operand instances referencing the identical concrete path
cannot be deduplicated to one real fetch without either a cache keyed wrong or a correctness
regression.

## Acceptance Criteria
AC-CDR-09, AC-CDR-10 (see `feature-delta.md` § User Stories, US-03).

## Dependencies
Slice 01, Slice 02.

## Effort Estimate
1 day.

## Reference Class
New primitive (a path→result dedup map), nearest reference class: `resolve_access_rule_pattern`'s
own "cheap on the hot path, extended" discipline (ADR-064 § Decision Driver 2).

## Pre-Slice SPIKE
Not required.
