# Slice 02: OR-Query Security Compliance (LAST slice, the security-critical half)

**Goal**: an OR query against an ownership-equality-protected collection is admitted ONLY if EVERY
disjunct branch independently proves the ownership constraint — never silently admitted based on
just one branch, which would be an access-control bypass.

## IN scope
- `filter_binds_field_to_uid` (`crates/embyr-core/src/access_control/mod.rs`): new
  `QueryFilter::CompositeOr` match arm using `.all()` (every branch must independently satisfy the
  binding) — `Composite`'s own existing `.any()` arm is untouched.

## OUT scope
- Any change to `check_query_compliance`'s own outer logic, `decompose_decidable`, or the
  `Condition`/`Atom` types — this slice is a single, precise fix inside the ONE function that
  recurses into the query's own filter tree.

## Learning hypothesis
**Disproves** (if it fails): the `.any()` → `.all()` fix, applied ONLY to the new `CompositeOr`
arm, is sufficient to close the access-control gap without any other change. If additional changes
elsewhere turn out to be needed, the "one centralized function, one precise fix" premise was wrong.
**Confirms** (if it succeeds): `filter_binds_field_to_uid` really is the single point all 7
`check_query_compliance()` callers route through — fixing it once here closes the gap for
`RunQuery`, `RunAggregationQuery`, both admin `simulate` endpoints, and realtime `Listen`
simultaneously, with zero duplicated logic.

## Acceptance criteria
AC-OR-04, AC-OR-05, AC-OR-06 (feature-delta.md § US-02).

## Dependencies
Slice 01 (needs `CompositeOr` to exist as a domain variant before this slice can add a match arm
for it).

## Effort estimate
≤1 day.

## Reference class
ADR-031's own `filter_binds_field_to_uid` — this slice is a direct, minimal extension of that
already-shipped, already-security-reviewed mechanism.

## Production-data acceptance criterion
Real `RunQuery` calls against a real running server, a real security-rules-protected collection,
and 2 real user identities: an OR query where one branch doesn't prove ownership must be REJECTED
(proven by a real attempt, not a unit-test-only assertion); an OR query where every branch DOES
prove ownership must be ADMITTED and return the correctly-scoped result set.
