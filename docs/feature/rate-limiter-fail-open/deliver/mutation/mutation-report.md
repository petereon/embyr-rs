# Mutation Testing Report — rate-limiter-fail-open

**Tool**: cargo-mutants 27.0.0
**Scope**: `crates/embyr-server/src/middleware/rate_limit.rs`, `--in-diff` scoped to the DELIVER commit
(`git diff 430dae1 0a88292975e3c2f4e82589defd4a1d04f4a2614e -- crates/embyr-server/src/middleware/rate_limit.rs`).

Closes Medium finding #20 (Security) from `docs/product/production-readiness-audit-2026-09-08.md`.

## Command

```
cargo mutants -p embyr-server --in-place --in-diff rate-limiter-fail-open.diff \
  --timeout 120 -- --test drl_b19_fail_open_on_pg_error -- --include-ignored --test-threads=1
```

Baseline (unmutated) measured first: `cargo test -p embyr-server --test drl_b19_fail_open_on_pg_error
-- --test-threads=1 --include-ignored` → 8 passed, 0 failed, **30.1s wall**. `--timeout 120` set with
~4x margin.

## Result: 4 candidate mutants — 0 caught, 0 missed, 4 unviable, 0 timeout

```
Found 4 mutants to test
ok       Unmutated baseline in 241s build + 32s test
4 mutants tested in 5m: 4 unviable
```

| Mutant | Location | Outcome |
|---|---|---|
| `check_inner` → `(Ok(Default::default()), true)` | `rate_limit.rs:187:9` | **Unviable** (build fail) |
| `check_inner` → `(Ok(Default::default()), false)` | `rate_limit.rs:187:9` | **Unviable** (build fail) |
| `check_pg` → `Ok((Ok(Default::default()), true))` | `rate_limit.rs:242:9` | **Unviable** (build fail) |
| `check_pg` → `Ok((Ok(Default::default()), false))` | `rate_limit.rs:242:9` | **Unviable** (build fail) |

## Interpretation

All 4 candidates are cargo-mutants' generic whole-function-stub genre (`FnValue`), applied to the
two touched function signatures (`check_inner`, `check_pg`). Both stubs require
`RateLimitInfo: Default`, which the type does not implement:

```
error[E0277]: the trait bound `RateLimitInfo: Default` is not satisfied
   --> crates/embyr-server/src/middleware/rate_limit.rs:187:13
```

Confirmed genuine (not an environment/scratch-dir artifact) by inspecting
`mutants.out/log/*_line_187_col_9.log` and `*_line_242_col_9.log` directly — real `E0277` compile
errors, consistent across both function sites. Same "whole-function-stub mutation-noise" pattern
seen on prior small diffs this session (`deployment-release-process`, `sanitize-backend-error-messages`).

This diff's own shape — a signature change (`(T, bool)` → `Result<(T, bool), sqlx::Error>`), two
`?`-propagation sites replacing `.ok().flatten()`/`.unwrap_or(false)`, and one new `Ok(Err(pg_error))
=>` match arm — introduces no new arithmetic, comparison, or boolean-logic expression for
cargo-mutants to mutate individually at the statement level; the only candidates it could construct
were the two whole-function stubs, both structurally unviable against this crate's own types.

The behavioral coverage this diff is actually meant to prove — that a `check_pg` Postgres error now
falls through to `check_in_process` instead of returning a synthetic full-capacity allow — is
exercised directly by `atomic_update_pg_error_falls_back_to_per_instance_bucket` and
`exists_check_pg_error_falls_back_to_per_instance_bucket` in the acceptance suite (8/8 passing,
unmutated), which assert on the fallback's actual bucket state, not on cargo-mutants' synthetic
stub outcomes.

No genuine gap found. No test-only fix required. "0 viable mutants" is a valid QUALITY_GATE result
here, consistent with this session's own precedent (`agent-field-path-validation`,
`healthz-dependency-checks`, `deployment-release-process`).

## Final state

8/8 `drl_b19_fail_open_on_pg_error` tests pass (30.1s). 0/0 viable mutants (4/4 unviable, all
confirmed genuine build failures against `RateLimitInfo`'s missing `Default` impl). Docker/testcontainers:
none spawned by this test target or this run — nothing to clean up. `mutants.out/` removed after the
run (gitignored scratch output).
