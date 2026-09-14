# RED Classification — preauth-db-amplification

Pre-DELIVER fail-for-the-right-reason gate (`nw-distill` skill). Run against
the CURRENT, unfixed `crates/embyr-server/src/grpc/handler.rs` and
`crates/embyr-server/src/middleware/rate_limit.rs` (DESIGN's guard-clause fix
not yet applied — that is DELIVER's job).

Commands:
- `cargo test -p embyr-server --test drl_b17_preauth_project_id_amplification --no-fail-fast -- --test-threads=1`
  → `5 passed; 3 failed`
- `cargo test -p embyr-server --test drl_b18_preauth_project_id_amplification_rest --no-fail-fast -- --test-threads=1`
  → `4 passed; 2 failed`

All 5 failures below are the RED-required part of this feature (the DB
round-trip elimination itself). Zero unrelated regressions; zero stray
Docker containers left behind (testcontainers auto-cleanup on `Drop`,
confirmed via `docker ps -a` post-run).

| Test | File | Classification | Empirically observed pre-fix behavior |
|---|---|---|---|
| `garbage_project_id_flood_never_invokes_rate_limiter_check_while_known_project_unaffected` (walking skeleton) | `b17_preauth_project_id_amplification.rs` | MISSING_FUNCTIONALITY (correct RED) | `embyr_rate_limit_requests_total{project_id="unconfirmed",outcome="allowed"}` moved from 0 to 5 across a 5-request garbage flood — i.e. `RateLimiter::check()` (and therefore `check_pg`'s 3-round-trip path) ran once per garbage `project_id`, exactly the amplification finding #14 describes. Perfect 1:1 correlation (5 requests -> delta of exactly 5), confirming the counter is a precise, non-flaky signal. |
| `single_garbage_project_id_get_document_never_invokes_rate_limiter_check` | same | MISSING_FUNCTIONALITY (correct RED) | Counter moved by exactly 1 for one garbage request (`before=7, after=8` — 7 carried over from the prior test in the same process/global metrics recorder; the delta, not the absolute value, is the signal). |
| `listen_rpc_with_garbage_project_id_never_invokes_rate_limiter_check` | same | MISSING_FUNCTIONALITY (correct RED) | Counter moved by 1 for a garbage `project_id` sent via the `Listen` RPC — confirms `extract_project_id_from_listen_request` (handler.rs:3900), the ONE call site that does not funnel through `extract_project_id`, independently needs its own guard. |
| `garbage_project_id_sign_in_with_custom_token_returns_malformed_token_with_no_db_row` | `b18_preauth_project_id_amplification_rest.rs` | MISSING_FUNCTIONALITY (correct RED) | Counter moved 0 -> 1: `rest_rate_limit_middleware` has no charset guard today, so `rate_limiter.check()` runs before `signInWithCustomToken`'s own handler ever sees the request. Response shape (400 MALFORMED_TOKEN) is ALREADY correct today (regression-guard assertion passes) — only the round-trip count is wrong. |
| `garbage_project_id_sign_up_returns_invalid_api_key_with_no_db_row` | same | MISSING_FUNCTIONALITY (correct RED) | Counter moved 1 -> 2 (2nd test in the same process). Response shape (401 INVALID_API_KEY) is ALREADY correct today — only the round-trip count is wrong. Confirms the middleware gap applies uniformly across `accounts:<verb>` actions, not just `signInWithCustomToken`. |

## Not RED (confirmed correct today AND must stay correct after the fix — regression guards)

| Test | Reason |
|---|---|
| `garbage_project_id_get_document_returns_same_invalid_argument_status_as_before_the_fix` (AC-PDA-03, gRPC) | `authenticate()` already applies `ProjectId::new(...).map_err(\|e\| Status::invalid_argument(e.to_string()))?` verbatim AFTER the 3 wasted round trips — so the returned `Status` is already byte-identical to what DESIGN's fix will produce earlier. Locks the contract so DELIVER's diff cannot accidentally change it. |
| `well_formed_but_unprovisioned_project_id_still_invokes_rate_limiter_check` (AC-PDA-05, gRPC) | A charset-VALID but never-provisioned `project_id` ("acme-corp-2026") is unaffected by this fix either way — the guard is a pure charset check and cannot distinguish "well-formed and real" from "well-formed and fake". Protects against DELIVER over-implementing OQ-PDA-02 (explicitly out of scope). |
| `empty_project_id_is_rejected_before_reaching_rate_limiter_exactly_as_today` (Example 4) | `extract_project_id`'s pre-existing non-empty check already rejects `projects//...` before any DB call, today and after the fix. |
| `known_provisioned_project_id_rest_request_is_unaffected_by_the_guard` (AC-PDA-02, REST) | A well-formed, already-bucketed `project_id`'s token count decrements normally either way — verified via `query_rate_bucket` token delta, not the counter (a real `projects` row exists here, so `rate_buckets`'s FK is satisfied and row/token state is a valid, more direct signal than the counter for this case). |
| `well_formed_but_unprovisioned_project_id_still_reaches_shared_database_via_rest` (AC-PDA-05, REST) | REST-side mirror of the gRPC residual test above — same reasoning. |

## Mechanism note (why the counter, not row-existence or pg_stat)

Two alternative DB-round-trip observability mechanisms were tried and
rejected before landing on the `embyr_rate_limit_requests_total{project_id=
"unconfirmed",outcome="allowed"}` counter delta (see
`tests/distributed_rate_limiting/common/mod.rs::metric_value` doc comment
for the full writeup):

1. **`pg_stat_user_tables` scan/insert counters** — empirically measured
   (this session) to lag real commits by Postgres's own internal
   `PGSTAT_MIN_INTERVAL` (~1s) stats flush. Produced false-zero deltas for
   fast, sequential single-request tests — caught by this exact RED-verify
   pass (two spurious failures with `left: 0, right: 2`-shaped output,
   diagnosed and replaced before this file was written).
2. **`rate_buckets` row EXISTENCE** — invalid specifically for
   garbage/never-provisioned `project_id`s: the table has a FK to
   `projects(id)` (migration 0018), so `check_pg`'s own
   `INSERT ... ON CONFLICT DO NOTHING` silently fails (swallowed by
   `let _ =` in production code) whenever no `projects` row exists. Row
   count stayed 0 whether or not the 3-round-trip path ran — a real,
   reproducible false negative, also caught during this RED-verify pass.

The counter is call-site-scoped (increments inside `RateLimiter::check()`
itself, after `check_inner()` returns) and is therefore unaffected by
whatever happens to the `INSERT` inside `check_pg`.

## Verdict

All 5 RED-required tests fail for the **right reason**: the assertion fires
because `RateLimiter::check()` (and therefore `check_pg`'s 3-round-trip
path) genuinely runs for a `project_id` that should have been rejected
earlier — never because of an import error, fixture bug, or setup failure.
Handoff to DELIVER is unblocked.
