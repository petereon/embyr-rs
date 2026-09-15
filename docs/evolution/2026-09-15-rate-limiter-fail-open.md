# Evolution: rate-limiter-fail-open

**Date**: 2026-09-15
**Closes**: Medium finding #20 (Security), `docs/product/production-readiness-audit-2026-09-08.md`

## Business Context

The Postgres-backed rate limiter discarded DB errors as a silent "allowed" — `.ok().flatten()` on
the atomic UPDATE and `.unwrap_or(false)` on the row-exists check both fail open. Combined with
finding #14 (pre-auth DB amplification), an attacker who degrades the system DB turns off rate
limiting globally as a side effect, on every project at once, for the duration of the outage.

## Key Decisions

| Decision | Rationale |
|---|---|
| Bounded fail-open via existing `check_in_process` reuse | On a genuine Postgres error, `check_pg` now returns `Err(sqlx::Error)` and `check_inner` routes it to the same per-instance in-memory bucket already used for the timeout case — no new fallback path, no new component. |
| No new ADR | ADR-015 already specified this exact behavior (bounded, per-instance fallback on DB error). The 2026-08-08 implementation never built it — this is a conformance fix against the ADR's own original spec, not a new design decision. |
| ADR-015 addendum, not a new ADR | Noted as a closed gap against the existing spec rather than opening a new decision record. |

## Lessons

- ADR-015 already specified bounded per-instance fallback on Postgres error; the original
  2026-08-08 implementation silently substituted unconditional allow instead. Audit-driven
  conformance check caught the drift between spec and code.
- A pre-existing acceptance-test fixture bug was found and fixed along the way: an unresolvable
  DSN meant the Argon2id cost in the auth path was paid *after* the rate-limit token had already
  been decremented, masking the exhaustion assertions the fail-open tests depend on.
- 0 viable mutants (4/4 unviable, confirmed genuine `RateLimitInfo: Default` build failures, not
  an environment artifact) is consistent with this session's "0 viable mutants is a valid
  QUALITY_GATE result" precedent for small, surgical diffs.

## Key Files

- `crates/embyr-server/src/middleware/rate_limit.rs` — `check_pg` signature change to
  `Result<(Result<RateLimitInfo, RateLimitInfo>, bool), sqlx::Error>`; `check_inner` routes
  `Ok(Err(pg_error))` to `check_in_process` instead of returning a synthetic full-capacity allow.
- `tests/distributed_rate_limiting/acceptance/b19_fail_open_on_pg_error.rs` — acceptance coverage
  (8 tests).
- `docs/feature/rate-limiter-fail-open/deliver/mutation/mutation-report.md` — QUALITY_GATE report.

## Follow-Up

Next in audit order: finding #21 (Medium, Security) — no CORS layer on :8081 despite serving
gRPC-Web/BrowserChannel; fails closed today but no origin policy is written or reviewed.
