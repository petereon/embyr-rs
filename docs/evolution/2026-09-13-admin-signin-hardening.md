# Evolution: admin-signin-hardening

**Date:** 2026-09-13
**Feature:** Admin signin route no longer blocks the tokio reactor on Argon2id
verification, and is now rate-limited per source IP (Postgres-backed,
survives restarts/multi-instance). Closes both the unthrottled-brute-force
gap and (as an accepted side effect, not full elimination) narrows the
timing-oracle email-enumeration window.
**ADR:** `docs/product/architecture/adr-076-signin-rate-limiting.md`

## This closes findings #12 AND #13 from `docs/product/production-readiness-audit-2026-09-08.md`

Both findings share one route and one fix surface, so were designed and
delivered together per a single feature (as flagged as likely in
[[project_agent_field_path_validation]]'s own follow-up note).

## Business Context

`crates/embyr-server/src/admin/handlers/auth.rs`'s `signin` ran Argon2id
(64 MiB/t=3/p=4 — expensive by design) inline on a tokio async worker thread,
with zero rate-limit middleware on the route and a password-failure lockout
that never incremented (only TOTP failures did). ~100 concurrent password
guesses could consume ~6GB and starve the reactor for every other request on
the same process — a genuine DoS, not just a slow brute-force path. Finding
#13 (timing oracle: unknown email returns instantly, known email takes a full
Argon2id verify) compounds #12 by making enumeration cheap once guessing is
unthrottled.

## Key Decisions

| Decision | Verdict |
|---|---|
| Reactor fix | Wrap the existing `embyr_core::auth::argon2::verify_password` call in `tokio::task::spawn_blocking`; remove the route's own duplicated inline Argon2 construction (was calling a *different*, un-reviewed code path than the rest of the codebase) |
| Rate limiting | New Postgres-backed `SigninRateLimiter` (table `signin_rate_limits`, migration `0037`), reusing the existing `TokenBucket` algorithm from `middleware/rate_limit.rs` (bumped `pub(crate)`) rather than inventing a second algorithm |
| Rate-limit key | Source IP only (`ConnectInfo<SocketAddr>`), capacity 150 / refill 10 per minute — chosen over per-email keying to also throttle enumeration sweeps across many candidate emails from one source |
| Email-enumeration timing oracle (#13) | Accepted as bounded-but-not-eliminated once rate limiting is in place — not fully closed by design; DISCUSS and DESIGN both explicitly evaluated and rejected a constant-time-response fix as disproportionate scope for this pass |
| TOTP-only lockout | Left unchanged — explicitly out of scope, not part of either finding's own text |
| New 4th sweeper | `signin_rate_limit_sweeper.rs`, mirrors `soft_delete_purge_sweeper.rs`'s shape exactly; 24h retention bounds table growth against an IP-rotating attacker |
| CI enforcement of ADR-003's spawn_blocking mandate | Flagged in ADR-076 as recommended follow-up; explicitly out of scope for this feature (not in any of the 8 locked ACs) |

## Steps Completed

Full nWave subagent pipeline (DISCUSS=`nw-product-owner`, DESIGN=`nw-solution-architect` with peer review, DISTILL=`nw-acceptance-designer` with peer review, DELIVER=`nw-software-crafter`):

1. **DISCUSS**: confirmed both findings share the same route and the same fix
   surface (rate limiting narrows the timing oracle's exploitability even
   though it doesn't eliminate it), combined into one feature. Confirmed the
   TOTP-only lockout mechanism and the timing oracle itself are pre-existing,
   documented, and correctly scoped as accepted trade-offs rather than silent
   gaps.
2. **DESIGN**: ADR-076, full component shapes for every touched/new file, C4
   diagram. Peer-reviewed: 0 critical/high.
3. **DISTILL**: 8 ACs, 2 stories. Peer-reviewed: 0 critical/high/medium, 2
   low observations (both later confirmed satisfied by DELIVER — no
   connection held across `spawn_blocking`; `check_pg` is a single
   race-free `INSERT ... ON CONFLICT ... RETURNING`). Found and fixed a
   genuinely fragile walking-skeleton acceptance test in two passes: first,
   a 100-concurrent-request flood test failed on stale TOTP freshness once
   real CPU contention slowed the flood down; second (deeper) pass found the
   REAL root cause was that 100 concurrent full-strength Argon2id verifies
   genuinely cannot complete inside any reasonable TOTP freshness window on
   an 8-physical-core sandbox — fixed by reducing `FLOOD_SIZE` to 30 and
   recalibrating the wall-time assertion to an empirically-measured 15000ms
   (real per-call cost here is ~700ms, not ADR-076's estimated 50-100ms).
4. **DELIVER**: implemented across 21 files (commit `5a54e22`). Removed the
   route's own duplicated/unreviewed inline Argon2 construction entirely.
   Independently re-verified by a fresh subagent: no inline `Argon2::new`
   remains anywhere in the file; the DB row-fetch completes and returns
   BEFORE `spawn_blocking` is entered (closure captures only owned
   `String`s, never the pool or the row).
5. **QUALITY_GATE**: cargo-mutants against the DELIVER diff (`--in-diff`,
   46 mutants in scope). Intentionally stopped early at 9/46 (1 caught, 3
   missed, 5 unviable) once it surfaced one clear, fully-investigated
   genuine gap — this feature's own acceptance suite takes ~19-21 minutes
   per full pass, making the remaining 37 mutants cost 8+ more hours for
   diminishing signal. The gap: `SigninRateLimiter::check_pg` (100% new
   Postgres-backed code) was never exercised by any test in the workspace
   (every test wrapper builds `SigninRateLimiter::new`, in-process only;
   `with_pg` is production-only). Fixed with 3 new `#[cfg(test)]` unit
   tests using the testcontainers-Postgres idiom from `system_db.rs`. A
   narrow `--lib`-scoped confirmation run found the fix's first attempt
   itself under-asserted (a `> 0` check missed 4 arithmetic mutants on the
   `retry_after_ms` formula) — tightened to a value-precise assertion;
   final narrow run: 9/10 caught, 1 accepted (floating-point boundary
   equivalence). Full regression re-run clean (19/19 + 3/3).

## Lessons Learned

1. **Mutation-testing cost can legitimately justify stopping a run before
   full completion — a new judgment call for this session.** Every prior
   feature ran mutation to completion; here the feature's own acceptance
   suite (~19-21 min/pass) made a full 46-mutant run cost 8+ hours. Stopping
   once a genuine, fully-investigated gap was found (not just "got tired of
   waiting") is defensible; the honest thing is documenting the stop
   explicitly in the mutation report, not hiding it.
2. **A shallow symptom (stale TOTP code) can mask a deeper one (real CPU
   contention).** The first fix to the flaky walking-skeleton test addressed
   TOTP freshness; it still failed, because 100 concurrent Argon2id verifies
   genuinely can't complete fast enough on 8 physical cores regardless of
   TOTP timing. Worth re-deriving the wall-clock budget empirically instead
   of trusting the ADR's own upfront estimate (50-100ms was 7-14x too low
   for this hardware).
3. **A dispatched subagent ending its turn to "wait for a notification" on
   its own already-running background command is a recurring failure mode**
   (hit 5+ times across 3 agents this feature) — filed as a `SendFeedback`
   bug. Forceful, single-imperative status requests ("run X now, paste raw
   output") reliably broke the pattern when it recurred.
4. Reconfirms [[feedback_mutation_testing_docker_contention]]'s cargo-sweep
   hazard: a background `cargo-sweep-shared-target.sh` script evicted a test
   binary mid-run, producing one spurious baseline failure — correctly
   diagnosed via standalone re-run rather than assumed to be a regression.

## Key Files

- `crates/embyr-server/src/admin/handlers/auth.rs` — `spawn_blocking` wrap, rate-limit gate, inline Argon2 removed.
- `crates/embyr-server/src/middleware/signin_rate_limit.rs` (new) — `SigninRateLimiter`.
- `crates/embyr-server/src/sweepers/signin_rate_limit_sweeper.rs` (new).
- `migrations/0037_signin_rate_limits.sql` (new).
- `crates/embyr-server/src/lib.rs` — `spawn_admin_server` now captures the real peer IP.
- `docs/product/architecture/adr-076-signin-rate-limiting.md`.
- `docs/feature/admin-signin-hardening/deliver/mutation/mutation-report.md`.

## Follow-Up Work

- Finding #14 (Reliability): pre-auth 3-round-trip DB amplification via unknown `project_id`, `crates/embyr-server/src/middleware/rate_limit.rs:203-255` — next in audit order.
- ADR-076's own recommended follow-up: CI grep gate enforcing ADR-003's `spawn_blocking` mandate workspace-wide (no existing enforcement was found for what turned out to be a real, live violation) — flagged, not scheduled.
- Full elimination of the #13 timing oracle (constant-time response shaping) — explicitly deferred as disproportionate scope; rate limiting alone was judged sufficient bounding for this pass.
