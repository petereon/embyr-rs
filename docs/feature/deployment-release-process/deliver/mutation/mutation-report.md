# Mutation Testing Report — deployment-release-process

**Tool**: cargo-mutants 27.0.0
**Scope**: `crates/embyr-server/src/main.rs`, `--in-diff` scoped to the DELIVER commit
(`git diff e01ab7d 39bd8a9 -- crates/embyr-server/src/main.rs`).

Closes High finding #19 (DevOps) from `docs/product/production-readiness-audit-2026-09-08.md`.

## Command

```
cargo mutants -p embyr-server --in-place --in-diff deployment-release-process.diff \
  --timeout 60 -- --test deployment_release_process -- --test-threads=1
```

Baseline (unmutated) measured first: `cargo test -p embyr-server --test deployment_release_process
-- --test-threads=1` → 11 passed, 1 ignored, **8.79s wall**. `--timeout 60` set with ~7x margin.

**Deviation from the originally-planned `--include-ignored`**: the 12th test in this target,
`drp02_compose_lifecycle`, is a real-Docker-Compose test whose own file header says "real Docker
Compose I/O, 2 containers... #[ignore] ... Run explicitly, NEVER as part of automated suite." It is
untouched by this diff (docker-compose lifecycle logic, not `main.rs`). A trial run with
`--include-ignored` confirmed it: pulled in 12 tests, added ~300s of Docker overhead, and failed on
its own (environment-dependent, unrelated to the 2-line diff under test). Running it under every
mutant would multiply that cost across the run and violates this machine's Docker-contention
constraint for no coverage benefit. Excluded; scope stayed at the 11 non-ignored tests, matching the
task's own stated count.

## Result: 1 candidate mutant — 1 caught, 0 missed, 0 unviable, 0 timeout

```
Found 1 mutant to test
ok       Unmutated baseline in 346s build + 10s test
1 mutant tested in 7m: 1 caught
```

| Mutant | Location | Outcome |
|---|---|---|
| `replace main with ()` | `crates/embyr-server/src/main.rs:74:5` | **Caught** |

## Interpretation

**This is the expected, correct result for this diff's own shape.** The two production edits are:

1. A structured `version = env!("CARGO_PKG_VERSION")` field + human-readable message on the
   startup-ready log line.
2. `.with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))` — gating ANSI color codes to
   interactive terminals only, fixing a real bug where escape codes spliced into structured log
   fields when output was piped.

Neither edit introduces a new conditional, arithmetic expression, or comparison for cargo-mutants
to mutate individually — `env!(...)` is a compile-time macro (not a mutable call site), and
`IsTerminal::is_terminal` is a single trait-method call feeding a bool argument, not a
mutants-eligible function body in this crate. The only mutant cargo-mutants could construct that
overlapped the diff's changed lines was the whole-function stub (`replace main with ()`), which is
`cargo-mutants`' generic catch-all for any `fn main()` edit — consistent with the "whole-function-stub
mutation-noise" pattern seen in prior small features this session (`sanitize-backend-error-messages`).

The mutant was **caught**: with `main` replaced by a no-op, the process never starts embyr-server,
so `drp01_startup_version_log`'s acceptance test (which spawns the real binary and asserts the
version field appears in the startup log line) fails to observe any log output — genuine kill, not
a coincidental one, since that test is precisely the walking-skeleton proof for this feature's
version-in-log claim.

No genuine gap found. No test-only fix required. Given the diff's shape (2 small edits, no new
branching logic, one touching a macro and one a single trait-method call), a result of "1 mutant,
caught" is the expected ceiling here — matching this session's own "0 or 1 viable mutants is a
valid QUALITY_GATE result" precedent from `agent-field-path-validation` and
`healthz-dependency-checks`.

## Final state

11/11 `deployment_release_process` non-ignored tests pass (8.79s). 1/1 viable mutant caught (100%
kill rate). `drp02_compose_lifecycle` (Docker-only, `#[ignore]`) intentionally excluded from this
run per its own file-header instruction; unaffected by this diff. Docker/testcontainers: none
spawned by this run — nothing to clean up.
