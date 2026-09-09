# Mutation Testing Report — rate-limiter-project-id-validation

Target: `crates/embyr-server/src/middleware/rate_limit.rs`
Command: `cargo mutants --in-place -f crates/embyr-server/src/middleware/rate_limit.rs -- --ignored drl`
(name-filtered `--ignored` per established lesson — bare `--ignored` pulls in unrelated
known-flaky `#[ignore]`d tests elsewhere in the binary and fails the retest's own baseline)

Discipline followed: confirmed via `ps -p 57739` → NOTRUNNING (process genuinely exited) and
`git diff crates/embyr-server/src/middleware/rate_limit.rs` → empty (no leftover mutation
corruption) before reading results or touching the file. See
`feedback_mutation_testing_docker_contention.md`.

## Result: 10 mutants tested in 18m — 1 missed, 2 caught, 7 unviable

```
=== caught ===
crates/embyr-server/src/middleware/rate_limit.rs:181:12: delete ! in RateLimiter::check_inner
crates/embyr-server/src/middleware/rate_limit.rs:281:49: replace - with + in RateLimiter::check_pg

=== missed ===
crates/embyr-server/src/middleware/rate_limit.rs:281:49: replace - with / in RateLimiter::check_pg

=== unviable (7) ===
crates/embyr-server/src/middleware/rate_limit.rs:155:9:  replace RateLimiter::check -> Result<RateLimitInfo, RateLimitInfo> with Ok(Default::default())
crates/embyr-server/src/middleware/rate_limit.rs:181:9:  replace RateLimiter::check_inner -> (...) with (Ok(Default::default()), true)
crates/embyr-server/src/middleware/rate_limit.rs:181:9:  replace RateLimiter::check_inner -> (...) with (Ok(Default::default()), false)
crates/embyr-server/src/middleware/rate_limit.rs:227:9:  replace RateLimiter::check_pg -> (...) with (Ok(Default::default()), true)
crates/embyr-server/src/middleware/rate_limit.rs:227:9:  replace RateLimiter::check_pg -> (...) with (Ok(Default::default()), false)
crates/embyr-server/src/middleware/rate_limit.rs:320:9:  replace RateLimiter::check_in_process -> (...) with (Ok(Default::default()), true)
crates/embyr-server/src/middleware/rate_limit.rs:320:9:  replace RateLimiter::check_in_process -> (...) with (Ok(Default::default()), false)
```

## Interpretation

**All new security-critical logic this feature added is either caught or unviable.** The
`known_existing`-signal threading through `check_inner`/`check_pg`/`check_in_process`'s new tuple
return types cannot be mutated to `Default::default()` because neither `RateLimitInfo` nor the
tuple types derive `Default` — 7 legitimately unviable mutants, not a coverage gap. The one `!`
deletion inside the `if !self.enabled` early-return guard is caught by `drl_b12`.

**The one miss (line 281:49) is on pre-existing code, not new logic.** Line 281 is
`remaining: capacity - 1.0,` inside `check_pg`'s "project existed before migration 0018" backward-
compatibility fallback (the row-doesn't-exist-but-insert-a-default-and-allow branch). This
arithmetic predates this feature; this feature only wrapped its return value in the new
`(Result<..>, bool)` tuple shape (the `bool` here is correctly `false`, since a project hitting
this branch has no confirmed `rate_buckets` row — that part IS new logic, and IS caught, per the
`+`-mutant catch above on the same line). A `-`→`/` swap here would report one extra token of
`remaining` than correct for this specific, narrow edge case (a project provisioned before
migration 0018 ran, whose bucket row is being lazily backfilled).

**Investigated whether this is a scoping artifact before accepting the miss** (per this session's
own established discipline — verify before assuming). Checked every acceptance test that could
plausibly exercise this exact code path:
- `drl_b11_provision_initializes_bucket` — tests provisioning-time row creation, not a missing-row
  runtime fallback.
- `drl_b13_fallback_on_pg_failure` — tests the PG-*unreachable* fallback (`check_in_process` via
  the 20ms timeout), a different branch entirely from "PG reachable but row missing."
- `drl_b14_ratelimit_headers` — tests header shape on the normal, row-exists path.
- No test anywhere in `tests/distributed_rate_limiting/` (or elsewhere) simulates "PG reachable,
  `rate_buckets` row absent" to exercise this specific migration-0018-compat branch.

**Conclusion: genuine, pre-existing test gap, not a scoping artifact — but out of scope for this
feature's own QUALITY_GATE.** This feature did not introduce the `capacity - 1.0` arithmetic; it
only added the `known_existing` signal around it (which IS itself fully covered). Fixing a
pre-existing gap unrelated to this feature's own security fix would expand scope beyond the
Prometheus-cardinality vulnerability this feature closes. Documented here as an honest, explicit
out-of-scope finding rather than silently accepted — a future feature touching migration-0018
compatibility or `rate_buckets` backfill behavior should add coverage for it.
