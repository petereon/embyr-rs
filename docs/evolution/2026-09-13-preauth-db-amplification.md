# Evolution: preauth-db-amplification

**Date:** 2026-09-13
**Feature:** Structurally-invalid `project_id`s are rejected before the rate limiter's
own 3-round-trip Postgres check runs, eliminating pre-authentication DB amplification
against the shared system pool. No new ADR — a call-order fix reusing an existing check.

## This closes finding #14 from `docs/product/production-readiness-audit-2026-09-08.md`

## Business Context

`crates/embyr-server/src/middleware/rate_limit.rs`'s rate limiter ran BEFORE real
authentication, on every request — including ones carrying a garbage/unknown
`project_id` that could never authenticate. `extract_project_id` only checked
non-empty, so any such request still cost 3 Postgres round trips (`check_pg`'s
UPDATE → SELECT EXISTS → INSERT) against the shared system DB pool that auth/sessions/
admin API all depend on — a pre-auth, zero-authentication-required 3x amplification an
attacker could trigger indefinitely.

## Key Decisions

| Decision | Verdict |
|---|---|
| Fix | Reuse the EXISTING `embyr_core::domain::project::ProjectId::new` charset guard (already used later in `authenticate()`) — move/duplicate it earlier, before `rate_limiter.check()` is ever invoked. No new validation logic. |
| Call-site wiring (OQ-PDA-01) | 3 choke points, not 15: `extract_project_id` (14/15 gRPC sites), `extract_project_id_from_listen_request` (the `Listen` RPC's own separate path), `rest_rate_limit_middleware` (REST) |
| REST response asymmetry | `signInWithCustomToken` with invalid project_id → 400 `malformed_response()`; every other REST action → 401 `invalid_api_key()` — found and required by DESIGN's own peer review (iteration 1 caught a High: a naive single-shape guard would have broken `signInWithCustomToken`'s existing distinct rejection shape) |
| Well-formed-but-unprovisioned residual (OQ-PDA-02) | Explicitly NOT closed — a project_id passing the charset but never provisioned still costs the old 3-round-trip path. Left cheaply open for a future cache/bloom-filter feature (the guard is structurally independent of `check_pg`) |
| Finding #20 (fail-open-on-DB-error, same file) | Explicitly out of scope, confirmed untouched by construction (zero diff lines inside `check_pg`/`check_inner`/`RateLimiter`) |
| ADR | None — zero new component/type/CoreError variant/dependency; a pure call-order fix reusing pre-existing, already-tested code. Matches the precedent set by `rate-limiter-project-id-validation`, `agent-field-path-validation`, `stripe-webhook-secret-required` |

## Steps Completed

Full nWave subagent pipeline (DISCUSS=`nw-product-owner`, DESIGN=`nw-solution-architect` with peer review, DISTILL=`nw-acceptance-designer` with peer review, DELIVER=`nw-software-crafter`):

1. **DISCUSS**: traced the exact call path (`handler.rs`, ~15 call sites, all pre-auth), confirmed finding #20 as a separate, untouched concern. DoR 9/9.
2. **DESIGN**: found the fix collapses to 3 edit points because 14/15 gRPC call sites already funnel through 2 shared functions. Peer review iteration 1 found 1 High (naive guard would break `signInWithCustomToken`'s response shape) — fixed, iteration 2 approved 0 crit/high. No new ADR needed.
3. **DISTILL**: 10 acceptance scenarios (6 gRPC + 4 REST). Chose a Prometheus-metric-delta assertion for "zero DB round trips" after empirically rejecting two more fragile alternatives mid-session: `pg_stat_user_tables` counters (lag real commits by Postgres's own ~1s stats-flush interval, causing false-zero-deltas) and `rate_buckets` row existence (invalid for never-provisioned IDs, since the table's FK to `projects` makes the INSERT itself silently no-op). RED-verified: 5/10 failed for the right reason pre-fix.
4. **DELIVER**: implemented all 4 edits exactly as designed (commit `8abd284`). 14/14 tests green. `git diff` grep-confirmed zero touches inside `check_pg`/`check_inner`/`RateLimiter`.
5. **Independent verification**: fresh subagent re-confirmed all 7 code-level claims and re-ran the 2 scoped test targets clean (14/14) — see resource note below.
6. **QUALITY_GATE**: 12 mutants (`--in-diff`), 8 caught (all new logic), 4 missed — all 4 landed on a PRE-EXISTING guard swept in by diff-hunk proximity, not new code. Each verified individually: 1 is a genuine equivalent mutant (`ProjectId::new`'s own regex already rejects empty strings under the same error code), 3 are on the `Listen` RPC path which the acceptance test deliberately treats as out-of-scope (`let _ = result`) since it only asserts the DB-round-trip metric, which holds under every guard permutation. No fix needed — closes clean. Commit `f7abf84`.

## Lessons Learned

1. **Mid-session machine resource incident, fixed structurally, not per-feature.** Repeated full-workspace `cargo build/test --workspace` dispatches (a habit carried over from every prior feature's "full regression after DELIVER" step) reliably strained/killed this machine — it has only 8GB RAM, and Docker Desktop's own VM reserves 4GB fixed, leaving ~4GB for cargo. Root-caused mid-feature and fixed with `jobs = 2` in `~/.cargo/config.toml` (global, caps concurrent rustc processes) plus a hard policy change: never run workspace-wide cargo commands again on this machine, always scope to the touched crate. See [[feedback_machine_resource_constraints]]. This feature's own DELIVER/verification/QUALITY_GATE work was completed AFTER this fix, scoped to `-p embyr-server` only, and ran cleanly and quickly (single-crate build ~2m39s vs. the machine-killing full-workspace runs before).
2. **`--in-diff` mutation scoping sweeps in nearby pre-existing code, not just new lines** — a recurring, now well-documented cargo-mutants quirk (git diff hunks include context lines). All 4 misses this feature were on a pre-existing guard adjacent to the new code, not the new code itself. The discipline that keeps this honest: verify each miss against the SOURCE, don't assume "in the diff = new logic."
3. **A 12-mutant, single-crate diff is a useful contrast against `admin-signin-hardening`'s 46-mutant/19-min-suite scale** — same-day, same audit list, opposite ends of mutation-testing cost. Neither needed a shortcut; the small one just didn't have one to take.

## Key Files

- `crates/embyr-server/src/grpc/handler.rs` — `extract_project_id`, `extract_project_id_from_listen_request`.
- `crates/embyr-server/src/middleware/rate_limit.rs` — `rest_rate_limit_middleware`.
- `crates/embyr-server/src/rest/sign_in.rs` — `malformed_response` visibility bump.
- `tests/distributed_rate_limiting/acceptance/b17_preauth_project_id_amplification.rs`, `b18_..._rest.rs` (new).
- `docs/feature/preauth-db-amplification/deliver/mutation/mutation-report.md`.
- `~/.cargo/config.toml` — `jobs = 2` (machine-wide fix, not feature-scoped, but landed during this feature's work).

## Follow-Up Work

- Finding #15+ (High) remain — next in audit order.
- Finding #20 (Medium, fail-open-on-DB-error, same file) — explicitly untouched, still open.
- OQ-PDA-02 (well-formed-but-unprovisioned residual) — left cheaply extensible, not scheduled.
