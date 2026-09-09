# Evolution: rate-limiter-project-id-validation

**Date:** 2026-09-09
**Feature:** Rate-limiter's Prometheus metric no longer labels with raw, unauthenticated
`project_id` input — bounds an unauthenticated remote memory-exhaustion (label-cardinality DoS)
vector.
**Job:** JOB-12 (`observability`) — reused, persona P2 Sam Chen.
**ADRs:** ADR-069 (new) — rate-limit metric label cardinality bounding. ADR-016 amended (note only).

## This closes finding #2 from `docs/product/production-readiness-audit-2026-09-08.md`

## Business Context

`embyr_rate_limit_requests_total` used the caller-supplied `project_id` string directly as a
Prometheus label value, on a pre-authentication code path. Prometheus label cardinality is
effectively unbounded per unique string, so an unauthenticated attacker sending N distinct,
never-provisioned `project_id` values could grow the metric's cardinality by N — an unauthenticated
remote memory-exhaustion vector against the metrics process.

## Key Decisions

| Decision | Verdict |
|---|---|
| D1 | Reuse the rate limiter's own pre-existing existence signal (`known_existing`) rather than invent new validation — real projects get their `rate_buckets` row inserted atomically at provisioning time, strictly before any traffic arrives, so there's no cold-start gap |
| D2 | Non-existent/unconfirmed `project_id` values are labeled with a constant sentinel (`"unconfirmed"`), never the raw input — bounds cardinality to +1 regardless of attacker input volume |
| D3 | `check_inner`/`check_pg`/`check_in_process` all return `(Result<RateLimitInfo, RateLimitInfo>, bool)` — the existence signal travels alongside the normal result rather than being recomputed separately |

## Steps Completed

Full nWave subagent pipeline (DISCUSS=`nw-product-owner`, DESIGN=`nw-solution-architect` with its
own peer review, DISTILL=`nw-acceptance-designer`, DELIVER=`nw-software-crafter`):

1. **DISCUSS**: locked scope on JOB-12 (observability), 5 ACs (AC-RLV-01 through 05).
2. **DESIGN**: peer-reviewed (0 critical/high findings). Resolved the real architectural tension —
   a format check alone is insufficient (attacker can pass a well-formed but never-provisioned
   ID), and post-auth confirmation is impossible (this is a pre-auth code path) — by reusing the
   rate limiter's own existing row/map-entry lookup as the trust signal instead of adding new
   validation. Wrote ADR-069.
3. **DISTILL**: wrote 2 new acceptance tests — `b15_project_id_metric_label_cardinality` (25
   distinct never-provisioned IDs → at most 1 new label value; confirmed RED before the fix) and
   `b16_malformed_project_id_no_panic` (malformed-but-non-empty IDs don't crash the server;
   already GREEN, regression guard).
4. **DELIVER**: implemented exactly as designed. Full regression run caught one new,
   previously-unseen failure (`oauth_providers_op02_maria_signs_in_with_google`) alongside the
   already-known `drl_b12_postgres_rate_limit` flake — triaged per this session's own established
   "read the actual failure before dismissing as flaky" discipline: confirmed `PortNotExposed`
   (the same Docker-contention class seen elsewhere this session), then confirmed via isolated
   rerun that the whole binary passes cleanly. Unrelated to this diff.
5. **QUALITY_GATE**: mutation run (`ps -p`-verified clean before touching the file, per the
   never-touch-a-live-mutation-file discipline). 10 mutants: 2 caught, 7 unviable
   (`Default::default()` substitutions on non-`Default` types), 1 missed — investigated and
   confirmed a genuine, pre-existing gap in `check_pg`'s migration-0018-compat fallback
   (`remaining: capacity - 1.0`), unrelated to this feature's own new logic and out of scope for
   this fix. Documented rather than silently accepted.

## Lessons Learned

1. **Reuse an existing state signal instead of inventing new validation, when one is already
   computed for a different purpose on the same code path.** The rate limiter already had to know
   whether a `project_id` corresponded to an existing row/map-entry as part of its own core
   token-bucket logic — exposing that signal cost nothing new and closed the vulnerability with
   zero new IO or lookups. A broadly reusable "look before you write" pattern for any future
   pre-auth-input-as-label (or pre-auth-input-as-anything-unbounded) situation in this codebase.
2. **A pre-existing test gap surfaced by mutation testing on code you only lightly touched (wrapped
   in a new return shape, didn't rewrite) is legitimately out of scope** — fixing it would expand
   the diff beyond the vulnerability being closed. The right move is honest documentation in the
   mutation report (this session's established convention), not silent acceptance and not
   scope-creep into an unrelated fix.
3. **`PortNotExposed` Docker-contention flakes have now hit at least 5 distinct tests this
   session.** Always triage via isolated rerun before dismissing — reused directly from
   `feedback_triage_before_dismissing_as_flaky.md`.

## Key Files

- `crates/embyr-server/src/middleware/rate_limit.rs` — the only production file changed;
  `UNCONFIRMED_PROJECT_LABEL` sentinel, `known_existing` threaded through `check`/`check_inner`/
  `check_pg`/`check_in_process`.
- `docs/product/architecture/adr-069-rate-limit-metric-label-cardinality-bounding.md` (new).
- `docs/product/architecture/adr-016-prometheus-metrics.md` — amendment note.
- `tests/distributed_rate_limiting/acceptance/b15_project_id_metric_label_cardinality.rs` (new).
- `tests/distributed_rate_limiting/acceptance/b16_malformed_project_id_no_panic.rs` (new).
- `tests/distributed_rate_limiting/common/mod.rs` — `get_metrics()`,
  `rate_limit_metric_project_id_labels()` helpers.
- `docs/feature/rate-limiter-project-id-validation/deliver/mutation/mutation-report.md` — full
  account including the pre-existing-gap investigation.

## Follow-Up Work

Findings #3-#8 from the same audit remain. Finding #3 is on the SAME webhook route as
already-closed finding #1 (unbounded request-body buffering) — next target, warm context. Finding
#14 (unrelated, same `rate_limit.rs` file) was explicitly named as related-but-separate and not
touched by this fix. The migration-0018-compat `remaining` computation gap found during this
feature's own QUALITY_GATE is undocumented elsewhere in the audit — worth a low-severity addendum
if a future feature touches that code path.
