# Mutation Testing Report — preauth-db-amplification

**Tool**: cargo-mutants
**Scope**: `--in-diff` against `git diff 0a63de5 8abd284` restricted to the two files carrying the
new mutable logic — `crates/embyr-server/src/grpc/handler.rs` (`extract_project_id`,
`extract_project_id_from_listen_request`) and `crates/embyr-server/src/middleware/rate_limit.rs`
(`rest_rate_limit_middleware`). Excluded `crates/embyr-server/Cargo.toml` (new `[[test]]` entries
only) and `crates/embyr-server/src/rest/sign_in.rs` (`fn` -> `pub(crate) fn` visibility bump only,
skimmed first and confirmed to carry zero mutable logic — cargo-mutants found nothing there, as
expected). Test files themselves not mutated.

**RAM constraint discipline followed** (8GB machine, Docker VM reserves 4GB fixed): single
`cargo-mutants` process, `--in-place` (inherently serial, no `-j`), `docker ps -a` checked clean
before and after (zero containers — these acceptance tests use the shared in-process
`DrlTestContext` Postgres container pattern, torn down per test), `--test-threads=1` on the
underlying test binary.

## Command

```
cargo mutants -p embyr-server --in-place --timeout 300 \
  --in-diff pda_mut_diff.diff \
  -- --test drl_b17_preauth_project_id_amplification --test drl_b18_preauth_project_id_amplification_rest \
  -- --include-ignored --test-threads=1
```

`--timeout 300` was set from a real measurement, not a guess: `cargo test -p embyr-server --test
drl_b17_preauth_project_id_amplification --test drl_b18_preauth_project_id_amplification_rest --
--include-ignored --test-threads=1` run twice by hand — once warm (19.2s total) and once after
touching `handler.rs` to force a rebuild (27.6s total, ~10.8s + 6.6s test time on top of an
incremental rebuild). 300s is a >10x margin over that per-mutant baseline, generous enough to
absorb this machine's `jobs = 2` cap without producing false timeout classifications. In practice
each individual mutant's own full rebuild ran far longer than the touch-rebuild estimate (~390-540s
build alone, likely shared-`target`-directory contention from other concurrent work on this
machine) — zero mutants were misclassified as `timeout` regardless, confirming the margin held.

## Result: 12 mutants — 8 caught, 4 missed, 0 unviable, 0 timeout

```
Found 12 mutants to test
ok       Unmutated baseline in 260s build + 21s test
12 mutants tested in 79m: 4 missed, 8 caught
```

**Caught (8)**:
```
handler.rs:100:9   replace extract_project_id -> Result<&str, Status> with Ok("")
handler.rs:100:9   replace extract_project_id -> Result<&str, Status> with Ok("xyzzy")
handler.rs:102:46  replace match guard !pid.is_empty() with false        [extract_project_id]
handler.rs:102:46  delete !                                              [extract_project_id]
handler.rs:3913:5  replace extract_project_id_from_listen_request -> Result<String, Status> with Ok(String::new())
handler.rs:3913:5  replace extract_project_id_from_listen_request -> Result<String, Status> with Ok("xyzzy".into())
rate_limit.rs:383:5  replace rest_rate_limit_middleware -> Response with Default::default()
rate_limit.rs:408:26 replace == with != in rest_rate_limit_middleware
```

**Missed (4)**:
```
handler.rs:102:46  replace match guard !pid.is_empty() with true         [extract_project_id]
handler.rs:3916:42 replace match guard !pid.is_empty() with true         [extract_project_id_from_listen_request]
handler.rs:3916:42 replace match guard !pid.is_empty() with false        [extract_project_id_from_listen_request]
handler.rs:3916:42 delete !                                              [extract_project_id_from_listen_request]
```

## Interpretation — every mutant touching genuinely NEW logic is caught

The two whole-function-return mutants on each extraction function (`Ok("")`/`Ok("xyzzy")` and
`Ok(String::new())`/`Ok("xyzzy".into())`) prove the new `ProjectId::new(pid).map_err(...)?` guard's
actual effect: if either function stopped calling it and just returned a project id unconditionally,
`rate_limiter.check()` would be invoked with a never-before-seen id and the
`embyr_rate_limit_requests_total{project_id="unconfirmed",outcome="allowed"}` round-trip metric
would move — caught by the round-trip-delta assertions in both
`empty_project_id_is_rejected_before_reaching_rate_limiter_exactly_as_today`-style tests and the
garbage-project-id tests in b17/b18. The `==`/`!=` mutant on `rate_limit.rs:408` is the new
action-aware dispatcher (`signInWithCustomToken` -> 400 vs everything else -> 401) — caught,
confirming AC-PDA-03's dual-shape contract is actually exercised. `Default::default()` on the whole
middleware function is caught by the broader existing regression net. **All 6 of these are the
diff's own new logic, and all 6 are caught.**

All 4 misses sit on `!pid.is_empty()` — a **pre-existing** match guard this diff did not change
(swept into `--in-diff` scope only because it shares a hunk with the newly-inserted
`ProjectId::new()` call inside the same match arm). Each was investigated against real source and
test text, not assumed:

**Miss 1 — `handler.rs:102:46`, guard -> `true` (extract_project_id): accepted, confirmed
equivalent for this input class.** The only test exercising an empty `project_id` segment through
this function is `empty_project_id_is_rejected_before_reaching_rate_limiter_exactly_as_today`
(b17:313-344), which asserts `status.code() == tonic::Code::InvalidArgument` and a flat DB
round-trip metric — it does not assert the exact message text. `ProjectId::new`'s validation regex
is `^[a-z][a-z0-9-]{0,62}$` (`crates/embyr-core/src/domain/project.rs:3,8-17`), which requires a
first `[a-z]` character and therefore **already rejects the empty string** with the same
`CoreError::InvalidArgument` -> same `Status::invalid_argument` -> same `Code::InvalidArgument`,
before `rate_limiter.check()` either way. Verified directly against `ProjectId::new`'s source, not
inferred. Loosening the guard to `true` is unobservable to this test only because the new charset
guard coincidentally provides the same rejection for the empty-string edge case — it is not a gap
in the new logic under test (which is proven elsewhere, above), and the real diff does not touch
this guard at all. **Not fixed** — fixing would mean adding a message-text-exact assertion to a
pre-existing behavior this feature makes no claim about (the report's own AC-PDA-05/Example-4 scope
note: "empty project_id -> already zero round trips... pre-existing").

**Misses 2-4 — `handler.rs:3916:42`, all three guard mutants (Listen path): accepted, a
deliberate DISTILL-wave test-scope decision, not a DELIVER-introduced gap.** b17's only Listen
test, `listen_rpc_with_garbage_project_id_never_invokes_rate_limiter_check` (b17:360-392), sends a
non-empty garbage project id (`"DROP_TABLE_123"`) and explicitly discards the RPC result: `let _ =
result;`, with the comment "Any gRPC status is acceptable here — what matters is zero DB activity,
not the exact status (Listen's own error surface is out of this feature's scope; AC-PDA-01 only
requires the round-trip elimination)". Every one of the three guard mutations
(`true`/`false`/delete `!`) still causes the function to return an `Err` **before**
`rate_limiter.check()` is ever reached for this non-empty input — none of them make the guard
observable through a round-trip-only assertion. This is not an oversight: the test file's own
header (`AC-PDA-01`...`AC-PDA-03`) and this specific test's docstring both name the Listen error
surface as explicitly out of scope, and the two mutants that *do* change round-trip behavior on
this same function (the `Ok(String::new())`/`Ok("xyzzy".into())` whole-function mutants above) are
both caught — proving the new guard's actual protective effect on the Listen path is tested, just
not this particular unchanged match-guard's polarity. **Not fixed** — tightening the Listen test to
assert on `result`'s status would exceed this feature's stated scope (AC-PDA-01 only) and risks
pinning down "Listen's own error surface", a decision left open by DISTILL.

## Final state

No test or production changes made — the mutation run found no gap in this diff's own new logic.
8/12 caught outright; the remaining 4 are all pre-existing-guard mutants, swept in by `--in-diff`
hunk proximity rather than genuinely new code, and each is confirmed (via source inspection of
`ProjectId::new` and the exact test assertions, not assumption) to be either behaviorally
equivalent for the one input class it affects, or explicitly out of this feature's documented
scope. Docker: 0 containers before and after the run (`docker ps -a` clean both times).

**Disposition summary**: 8 caught (100% of mutants on genuinely new logic), 4 missed (100%
pre-existing-guard sweep-in, 100% accepted with source-verified reasoning), 0 unviable, 0 timeout.
Finding #14 closed with no residual mutation-testing gap.
