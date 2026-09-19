# Mutation Testing Report — structured-json-logging

**Tool**: cargo-mutants 27.0.0
**Scope**: `crates/embyr-server/src/main.rs`, `--in-diff` scoped to the DELIVER commit
(`git diff 5114146 e9bbad9 -- crates/embyr-server/src/main.rs`).

Closes Medium finding #25 (Reliability) from `docs/product/production-readiness-audit-2026-09-08.md`.

## Command

```
cargo mutants -p embyr-server --in-place --in-diff diff.patch \
  -C --test -C deployment_release_process --timeout 60 -- -- --test-threads=1
```

`-C --test -C deployment_release_process` scoped the *build* to the one relevant test binary —
confirmed via `ps aux` right after start: only `rustc --crate-name embyr_server` compiling, no
unrelated test targets pulled in.

Baseline (unmutated) measured first: `cargo test -p embyr-server --test deployment_release_process
-- --test-threads=1` → 11 passed, 1 ignored, 7.3s wall. `--timeout 60` set with ~2x margin over the
real per-mutant cost measured below.

## Real per-mutant cost (measured after confirming scoping)

| Phase | Baseline | Mutant |
|---|---|---|
| Build | 1.5s | 3.7s |
| Test | 10.4s | 34.7s |

Mutant test phase runs longer than baseline because the mutant guts `main()` entirely — the server
process never starts, so `drp01_startup_version_log`'s `wait_for_healthy(30s)` runs its full timeout
before failing. Still well inside the 60s cap.

## Result: 1 candidate mutant — 1 caught, 0 missed, 0 unviable, 0 timeout

```
Found 1 mutant to test
ok       Unmutated baseline in 1s build + 10s test
1 mutant tested in 50s: 1 caught
```

| Mutant | Location | Outcome |
|---|---|---|
| `replace main with ()` | `crates/embyr-server/src/main.rs:74:5` | **Caught** |

Only whole-function mutant available: the diff itself (`.json()` added, `.with_ansi(...)` line
removed) is two builder-chain method calls with no literals, booleans, or comparisons for
cargo-mutants to swap — nothing else in the diff's span is independently mutable. Consistent with
the "tiny diff, 0-2 mutants" expectation.

## Classification

- **Caught (1/1)**: `replace main with ()` — caught by `drp01_startup_version_log`'s
  `wait_for_healthy` assertion failing (server never starts). Genuine coverage, no gap.
- **Missed**: none.
- No test-only fix needed. Kill rate 100% (1/1).

## Verdict

Clean. Proceeding to FINALIZE.
