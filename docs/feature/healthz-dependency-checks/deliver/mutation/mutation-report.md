# Mutation Testing Report — healthz-dependency-checks

**Tool**: cargo-mutants
**Scope**: `--in-diff` against `git diff cd7f2ff..b07ab57` restricted to the three files carrying
the new mutable logic — `crates/embyr-server/src/grpc/healthz.rs` (`livez_handler` rewrite,
`healthz_handler` rewrite with `system_db.probe()` + 3s `tokio::time::timeout`),
`crates/embyr-server/src/lib.rs` (`spawn_all_servers`, `start_test_server_with_tls` —
`healthz_app` sub-router wiring), and `crates/embyr-server/src/main.rs` (admin router
`healthz_app` merge). Excluded the test file itself (`pr10_healthz_dependency_checks.rs`),
`tests/production_readiness/mod.rs` registration, and docs, per the task's own scope.

**RAM constraint discipline followed** (8GB machine, Docker VM reserves 4GB fixed): single
`cargo-mutants` process, `--in-place` (inherently serial — the flag itself refuses `--jobs`, so
there is no way to violate the no-parallel-testcontainers rule even by mistake), `docker ps -a`
checked clean before and after (zero containers both times — the pr10 tests use a fixed-host-port
Postgres testcontainer, `stop()`/`start()`-cycled in place rather than leaked), `--test-threads=1`
on the underlying test binary.

## Command

```
cargo mutants -p embyr-server --in-place --timeout 300 \
  --in-diff healthz.diff \
  -- --test production_readiness pr10_ \
  -- --include-ignored --test-threads=1
```

`--timeout 300` was set from a real measurement, not a guess: `cargo test -p embyr-server --test
production_readiness pr10_ -- --include-ignored --test-threads=1` run by hand first — all 4 pr10
tests, single-threaded, real Postgres testcontainer start/stop/restart included: **32.7s total**.
300s is a >9x margin over that per-mutant test baseline. cargo-mutants' own measured unmutated
baseline came in at **267s build + 36s test**, confirming the 300s ceiling comfortably covers the
test phase even under this machine's `jobs = 2` cap (build time is governed separately by
`--build-timeout`, left at cargo-mutants' own default, and every mutant here built well within it
except the 5 that failed to compile at all — see below).

## Result: 7 mutants — 2 caught, 5 unviable, 0 missed, 0 timeout

```
Found 7 mutants to test
ok       Unmutated baseline in 267s build + 36s test
7 mutants tested in 15m: 2 caught, 5 unviable
```

**Caught (2)**:
```
crates/embyr-server/src/main.rs:74:5      replace main with ()
crates/embyr-server/src/grpc/healthz.rs:31:9  delete match arm Ok(Ok(())) in healthz_handler
```

**Unviable (5) — compiler-rejected, not a test-suite signal either way**:
```
crates/embyr-server/src/lib.rs:345:5  replace spawn_all_servers -> tokio::task::JoinHandle<()> with JoinHandle::new()
crates/embyr-server/src/lib.rs:345:5  replace spawn_all_servers -> tokio::task::JoinHandle<()> with JoinHandle::from_iter([()])
crates/embyr-server/src/lib.rs:345:5  replace spawn_all_servers -> tokio::task::JoinHandle<()> with JoinHandle::new(())
crates/embyr-server/src/lib.rs:345:5  replace spawn_all_servers -> tokio::task::JoinHandle<()> with JoinHandle::from(())
crates/embyr-server/src/lib.rs:963:5  replace start_test_server_with_tls -> TestServer with Default::default()
```

**Missed**: none. **Timeout**: none.

## Interpretation

Only two mutants in the entire `--in-diff` scope were even syntactically viable to compile against
this diff's actual return types — and both were caught:

- **`healthz.rs:31:9` — deleting the `Ok(Ok(())) => 200` match arm inside `healthz_handler`.** This
  is the one mutant that touches the diff's real new behavior (the `system_db.probe()` +
  `tokio::time::timeout` readiness check). Verified directly against the log
  (`mutants.out/log/crates__embyr-server__src__grpc__healthz.rs_line_31_col_9.log`): 3 of the 4 pr10
  tests failed — `readiness_and_liveness_across_a_real_postgres_outage_on_both_mounts`,
  `healthz_stays_healthy_when_only_a_tenant_customer_database_is_down`, and
  `healthz_503_body_never_leaks_postgres_driver_or_schema_error_text` all panicked; only the
  structural `livez_handler_body_has_zero_postgres_or_external_calls` test (which asserts on
  `livez_handler`'s source text, unrelated to this arm) still passed. This is exactly the coverage
  the walking-skeleton test was designed to prove: removing the success path collapses `/healthz`
  to permanently-unhealthy-shaped behavior, and three independent tests each catch it from a
  different angle (baseline-200 assertion, tenant-scope assertion, 503-body assertion).
- **`main.rs:74:5` — replacing the whole `main` function with `()`.** A textbook whole-function-stub
  mutant (this session's own established "whole-function-stub mutation-noise" pattern from
  `sanitize-backend-error-messages`) — `main` was only touched by this diff's `healthz_app`
  wiring/merge, and stubbing the entire function naturally breaks server startup, so every pr10
  test that depends on `ServerProcess::start()` succeeding fails. Confirmed via the same log
  inspection: identical 3-failed/1-passed shape. Caught, as expected, but not a targeted signal on
  the diff's own new logic specifically — the whole binary not starting is a very coarse mutant.

The 5 unviable mutants are all cargo-mutants' generic return-type substitution strategy failing to
find a real value of the right type and guessing wrong, confirmed by reading the compiler output in
each log file rather than assumed:

- The four `JoinHandle::new()/from_iter([()])/new(())/from(())` mutants on `spawn_all_servers` all
  fail with `error[E0433]: cannot find type 'JoinHandle' in this scope` (`lib.rs:345:5` logs) —
  `tokio::task::JoinHandle` isn't imported unqualified at that scope, so cargo-mutants' generated
  replacement text doesn't even resolve as a type. This is a candidate-generation artifact of the
  mutation tool, not a coverage gap: there is no way for *any* test to "catch" code that never
  compiles.
- `Default::default()` on `start_test_server_with_tls -> TestServer` fails with
  `error[E0277]: the trait bound 'TestServer: Default' is not satisfied` (`lib.rs:963:5` log) —
  `TestServer` (the struct bundling this test harness's ports/handles) simply has no `Default` impl.
  Same category: a mutant cargo-mutants could never have compiled, regardless of the diff's actual
  logic quality.

`livez_handler` itself produced **zero mutants** — its body is a single unconditional
`(StatusCode::OK, Json(json!({"status": "ok"})))` return with no branch, no dependency call, and no
value cargo-mutants' AST-level mutators can meaningfully substitute (matches this session's
established "0 viable mutants is a valid QUALITY_GATE result" precedent from
`agent-field-path-validation` — correctness here is proven by the AC-HDC-02 structural test and the
walking-skeleton's liveness-stays-200-through-the-outage assertions, not by mutation coverage on a
body with no logic to mutate).

## Final state

No test or production changes made — the mutation run found no gap. Both mutants that could even
compile against this diff's return types were caught; the 5 that could not compile are confirmed
(via direct compiler-error inspection, not assumption) to be cargo-mutants candidate-generation
noise unrelated to test quality. Docker: 0 containers before and after the run (`docker ps -a`
clean both times, no fixed-host-port Postgres testcontainer left running).

**Disposition summary**: 2 caught (100% of viable mutants), 0 missed, 5 unviable (100%
compiler-rejected, source-verified), 0 timeout. Finding #15 closed with no residual
mutation-testing gap.
