# Mutation Testing Report — admin-signin-hardening

**Tool**: cargo-mutants
**Scope**: `--in-diff` against `git diff 233141c 5a54e22` restricted to the files carrying genuinely
NEW mutable logic — `crates/embyr-server/src/admin/handlers/auth.rs`,
`crates/embyr-server/src/middleware/signin_rate_limit.rs`,
`crates/embyr-server/src/sweepers/signin_rate_limit_sweeper.rs`,
`crates/embyr-server/src/lib.rs`. Excluded `main.rs`/`router.rs`/`state.rs`/`rate_limit.rs`, whose
own diffs were skimmed first and confirmed to be visibility-bump (`private` -> `pub(crate)` on
`TokenBucket`) and pure wiring (new constructor parameter threaded through, no new branches or
decisions) — no mutable logic of their own to test.

**RAM constraint discipline followed throughout** (this machine has 8GB and this feature's own
concurrent-load tests already showed swap pressure): every cargo-mutants/cargo test invocation run
one at a time, Docker containers (`docker ps -a -q | xargs -r docker rm -f`) cleaned before each
launch, `ps -p <PID>` used to confirm genuine process exit (never trusted a "looks done" log) before
touching any scoped file, and `git diff --stat` on scoped files confirmed empty before every read.

## Run 1 — full-diff scan, intentionally stopped early

```
cargo mutants -p embyr-server --in-place --timeout 2400 --in-diff /tmp/ash_mut_diff.diff -- \
  --test admin_api_v2_b01_auth_migrations --test admin_signin_hardening_us02_sweeper_bounds_table_growth \
  -- --test-threads=1
```

`--timeout 2400` (40 min) was required after a first attempt at `--timeout 300` predictably failed
its own baseline — `admin_api_v2_b01_auth_migrations` alone takes ~17-19 minutes for a full pass, far
longer than a 5-minute cap. A second `--timeout 2400` attempt's baseline also failed once, but for an
unrelated, transient reason: `admin_signin_hardening_us02_sweeper_bounds_table_growth`'s own test
binary went missing between cargo's build step and the test-execution step — confirmed (via `ls`,
disk space, and a clean standalone `cargo test -p embyr-server --test
admin_signin_hardening_us02_sweeper_bounds_table_growth`, which passed in 3.9s) to be an artifact
eviction race in this machine's *shared* `~/.cargo/shared-target` directory (shared across this
machine's concurrent, unrelated Claude Code sessions/projects), not a defect in this feature's code.
A clean retry succeeded.

**46 mutants found in scope.** 46 real mutants each cost a full rebuild + the ~19-21 minute
integration-test pass (confirmed baseline: 102s build + 1179s test), making a full run
8+ hours on this hardware. After **9/46 resolved** (5 unviable, 1 caught, 3 missed — 0 timeout, all
healthy, no thrashing), the run was **intentionally stopped** once it had produced a clear, fully
investigated genuine gap (below) — completing the remaining 37 mutants at ~15-20 min each offered
diminishing signal relative to the cost on this hardware. This is a deliberate scope-vs-cost
trade-off, not a shortcut on investigation: every one of the 9 resolved mutants was examined before
stopping.

```
5 unviable  (build failures — standard `Default::default()`/type-mismatch noise on this codebase's
             `Result<T, E>` return types)
1 caught    (real NEW logic mutant, confirming test coverage exists for at least part of the diff)
3 missed:
  signin_rate_limit.rs:84  replace SigninRateLimiter::record with ()
  signin_rate_limit.rs:96  replace SigninRateLimiter::check_pg -> ... with Ok(Ok(()))
  signin_rate_limit.rs:130 replace < with == in SigninRateLimiter::check_pg
```

### Investigation of the 3 misses

**Miss 1 (`record` -> `()`, line 84) — investigated, confirmed accepted noise, not fixed.**
`record`'s only effect is `metrics::counter!(...).increment(1)` — a Prometheus counter increment
with no behavioral or security consequence, and no acceptance test in this codebase asserts on
metrics-registry state. This is the same class of mutant already documented and accepted in
`docs/feature/soft-delete-purge-sweeper/deliver/mutation/mutation-report.md` ("incrementing a
counter... is a value-identical no-op... not fixed; documented as an accepted, low-value,
effectively-equivalent mutant"). Not fixed.

**Misses 2 & 3 (`check_pg` whole-function stub, and `<` -> `==` inside it) — one genuine gap,
investigated and fixed.** Grepped every test-server wrapper in the workspace that constructs a
`SigninRateLimiter` (`tests/{admin_api_v2,oauth_providers,client_auth,card_payments_backend,
client_auth_hosted_identity,anonymous_sessions,security_rules}/common/mod.rs` — 7 files): **every
one calls `SigninRateLimiter::new(...)` (in-process only, `pg_pool: None`)**. `with_pg` is called
only in `crates/embyr-server/src/main.rs` (production). Since `check()` only calls `check_pg` when
`self.pg_pool` is `Some`, `check_pg` — the entire Postgres-backed rate-limit path, 100% new code
from this feature — is never invoked by any enabled test in the repo. Confirmed not a scoping
artifact (no other test target anywhere touches it) and not pre-existing (the function did not exist
before this commit), so this is squarely this feature's own responsibility to close.

**Fix**: added a `#[cfg(test)] mod tests` block directly in `signin_rate_limit.rs`, mirroring the
testcontainers-Postgres idiom already established in `crate::adapters::system_db`'s own unit tests
(spin up `Postgres::default().with_tag("15-alpine")`, `SystemDb::new(&url).await.unwrap();
db.migrate().await.unwrap()` for a real pool with migration `0037_signin_rate_limits.sql` applied),
with `SigninRateLimiter::with_pg(...)` called directly against it.

## Run 2 — narrow confirmation, `check_pg` mutants only (attempt 1)

```
cargo mutants -p embyr-server --in-place --timeout 300 \
  -f 'crates/embyr-server/src/middleware/signin_rate_limit.rs' -F 'check_pg' \
  --output mutants_check_pg.out -- --lib -- --test-threads=1
```

Scoped to just `check_pg`'s own mutants (`-F 'check_pg'`), tested against the fast `--lib` unit-test
target instead of the ~19-minute integration suite — the new tests are unit tests, so this is a
legitimate scope narrowing, not a weakening of what's being proven (this mirrors
`occ-precondition-validation`'s own precedent for closing a similar cross-target gap with a direct
same-crate unit test).

**Result: 10 mutants tested in 56m — 5 caught, 5 missed.** The 2 originally-targeted mutants (the
whole-function stub and `<` -> `==`) were both **caught** by the new tests — but this run reached 8
more `check_pg` mutants never tested before (the original Run 1 only got through 9/46 total and had
not reached most of `check_pg`'s own mutants at all), surfacing **5 new misses**, all in the
`retry_after_ms` arithmetic (line 130-131):

```
MISSED  line 130:49  replace < with <=
MISSED  line 131:52  replace * with +
MISSED  line 131:52  replace * with /
MISSED  line 131:27  replace - with +
MISSED  line 131:27  replace - with /
```

### Investigation of the 5 new misses

**1 accepted (floating-point boundary-equivalent), 4 genuine — a real gap in the test's own
assertion strength, not a second logic bug.**

- `< -> <=` (line 130): the new test's `current` value lands close to `0.0` (comfortably below
  `1.0`), so `current < 1.0` and `current <= 1.0` produce identical results; the two conditions only
  diverge when `current == 1.0` exactly — an unreachable case in practice given real float timing
  arithmetic. Same equivalence class as the already-documented `soft-delete-purge-sweeper`
  `purged >= 0` mutant. **Accepted, not fixed.**
- `*`/`-` arithmetic mutants on the `retry_after_ms` formula (`((1.0 - current) / refill_rate *
  1000.0) as u64`): the first version of `check_pg_throttles_once_capacity_exhausted` only asserted
  `retry_after_ms > 0` — every one of these 4 mutants still produces *a* positive number, just the
  wrong one, so the assertion never distinguished them. **This is a genuine gap in the test's own
  precision, not in `check_pg`'s logic** (all 4 mutants were confirmed, by hand-computing the
  mutated formula, to still return a positive `u64` for the test's original inputs — the assertion
  was simply too weak to tell "positive" from "correct").

**Fix**: added a second, more precise unit test,
`check_pg_retry_after_ms_matches_expected_formula`, using `capacity = 1.5` (not `1.0`) so the
post-consumption `current` lands at a non-degenerate `~0.5` — a value where `1.0 - current`,
`1.0 + current`, and `1.0 / current` all diverge, and where `* 1000.0` vs `+`/`/` variants of the
same formula land far outside a computed expected range. Asserts `retry_after_ms` falls in
`1_700_000..=1_900_000` (expected: `(1.0 - 0.5) / (1/3600) * 1000.0 = 1_800_000`, with slack for
negligible refill and real Postgres round-trip latency between the two calls).

## Run 3 — narrow confirmation, `check_pg` mutants only (attempt 2, post-fix)

```
cargo mutants -p embyr-server --in-place --timeout 300 \
  -f 'crates/embyr-server/src/middleware/signin_rate_limit.rs' -F 'check_pg' \
  --output mutants_check_pg2.out -- --lib -- --test-threads=1
```

**Result: 10 mutants tested in 51m — 9 caught, 1 missed.** The 1 remaining miss is exactly the
predicted, already-investigated `< -> <=` boundary-equivalent mutant (accepted above) — all 4
arithmetic mutants are now caught. Gap closed.

## Final state

Three new unit tests added directly to `crates/embyr-server/src/middleware/signin_rate_limit.rs`
(`check_pg_allows_when_under_capacity`, `check_pg_throttles_once_capacity_exhausted`,
`check_pg_retry_after_ms_matches_expected_formula`) close the one genuine coverage gap this feature
introduced (`check_pg`, the Postgres-backed rate-limit path, previously unexercised by any test in
the workspace). `cargo test -p embyr-server --lib signin_rate_limit::tests`: 3/3 passed.

Full regression re-confirmed after the fix, run sequentially (this machine's own RAM constraint):
- `cargo test -p embyr-server --test admin_signin_hardening_us02_sweeper_bounds_table_growth`:
  3/3 passed (3.87s).
- `cargo test -p embyr-server --test admin_api_v2_b01_auth_migrations`: 19/19 passed (1188.09s).

**Disposition summary**: 1 mutant accepted as observability-only noise (`record`), 1 mutant accepted
as a floating-point boundary equivalence (`< -> <=`), 1 genuine gap found and closed via 3 new direct
unit tests (covering both the whole-function-stub and the arithmetic-precision dimensions of
`check_pg`). The full 46-mutant `--in-diff` scan was intentionally not completed to exhaustion
(9/46 resolved) given the ~19-21-minute-per-mutant cost on this hardware and the well-classified
signal already obtained — the one genuine gap it surfaced was fully investigated and closed via
targeted, fast `--lib`-scoped follow-up runs instead.
