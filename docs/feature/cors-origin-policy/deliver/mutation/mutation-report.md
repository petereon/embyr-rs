# Mutation Testing Report — cors-origin-policy

**Tool**: cargo-mutants 27.0.0
**Scope**: `crates/embyr-server/src/config.rs`, `crates/embyr-server/src/lib.rs`,
`--in-diff` scoped to the DELIVER commit (`git diff 97f1638..f54335d00c400525e31a11cced511b4bceaab3de
-- crates/embyr-server/src/config.rs crates/embyr-server/src/lib.rs`).

Closes Medium finding #21 (Security) from `docs/product/production-readiness-audit-2026-09-08.md`.

## Command

```
cargo mutants -p embyr-server --in-place --in-diff cors-origin-policy.diff \
  --timeout 60 -- --test cors_origin_policy -- --test-threads=1
```

Baseline (unmutated) measured first: `cargo test -p embyr-server --test cors_origin_policy
-- --test-threads=1` → 4 passed, 0 failed, **5.59s wall**. `--timeout 60` set with ~10x margin.

## Result: 19 candidate mutants — 0 caught, 5 missed, 14 unviable, 0 timeout

```
Found 19 mutants to test
ok       Unmutated baseline in 347s build + 8s test
19 mutants tested in 32m: 5 missed, 14 unviable
```

| Mutant | Location | Outcome |
|---|---|---|
| `spawn_all_servers` → `JoinHandle::new()`/`from_iter`/`new(())`/`from(())` (4) | `lib.rs:346:5` | **Unviable** (build fail — `JoinHandle` has no such ctor) |
| `start_test_server_with_*` (7 fns) → `Default::default()` | `lib.rs:581/640/718/781/839/897/961/1023/1087` | **Unviable** (build fail — `TestServer: !Default`) |
| `ServerConfig::from_env` → `Ok(Default::default())` | `config.rs:306:9` | **Unviable** (build fail — `ServerConfig: !Default`) |
| `<impl Display for ConfigError>::fmt` → `Ok(Default::default())` | `config.rs:234:9` | **Missed** — see below |
| `parse_cors_allowed_origins` → `vec![]` / `vec![String::new()]` / `vec!["xyzzy".into()]` (3) | `config.rs:474:5` | **Missed** — see below |
| `parse_cors_allowed_origins`: delete `!` in `.filter(\|s\| !s.is_empty())` | `config.rs:479:29` | **Missed** — see below |

## Interpretation

**14 unviable**: cargo-mutants' generic whole-function-stub genre (`FnValue`), same
"whole-function-stub mutation-noise" pattern as prior small diffs this session
(`rate-limiter-fail-open`, `deployment-release-process`). All require `Default` on types
(`JoinHandle`, `TestServer`, `ServerConfig`) that don't implement it — confirmed genuine
build failures, not scratch-dir artifacts.

**4 of 5 missed — scope artifact, not a real gap.** `parse_cors_allowed_origins` and
`<Display for ConfigError>::fmt` are both exercised by `config.rs`'s own unit tests
(`parse_cors_allowed_origins_absent_returns_empty`,
`parse_cors_allowed_origins_trims_and_drops_empty_entries`, added in this same commit),
which run under the `--lib` test binary — a different binary from the `--test
cors_origin_policy` real-IO integration target this run was scoped to per the QUALITY_GATE
instructions. Manually verified by hand-mutating `parse_cors_allowed_origins` to `vec![]`
and running `cargo test -p embyr-server --lib config::tests::parse_cors_allowed_origins`:

```
test config::tests::parse_cors_allowed_origins_absent_returns_empty ... ok
test config::tests::parse_cors_allowed_origins_trims_and_drops_empty_entries ... FAILED
  left: []
 right: ["https://a.example.com", "https://b.example.com"]
```

Confirms `vec![]` / `vec![String::new()]` / `vec!["xyzzy".into()]` are all caught (any fixed
constant mismatches the assertion); the `!`-deletion mutant inverts the filter to keep only
empty strings, which the same assertion also catches. Mutation reverted after verification.

**1 of 5 missed — accepted, not fixed.** `<Display for ConfigError>::fmt` → `Ok(Default::default())`
is a whole-function stub covering all 14 `ConfigError` variants, not just the new
`InvalidCorsOrigin` arm. `config.rs` has never unit-tested `Display` output for any
`ConfigError` variant (existing tests assert `matches!(result, Err(ConfigError::InvalidX { .. }))`,
never the formatted string) — this diff doesn't break that pre-existing convention. The
mutation is cosmetic only: `ServerConfig::from_env`'s validation loop still constructs and
returns `Err(ConfigError::InvalidCorsOrigin { .. })` regardless of how `Display` renders it,
so startup still correctly refuses a malformed origin — only the diagnostic text would degrade.
No security/functional impact. No test-only fix added, to avoid introducing a one-off
Display-string test with no precedent elsewhere in the file.

No genuine gap in the security-relevant logic (origin parsing, allowlist matching, deny-by-default)
was found. No production-code fix required.

## Final state

4/4 `cors_origin_policy` tests pass (5.59s). 0/19 viable-and-uncaught mutants after manual
classification (14 unviable, 4 scope-artifact-but-covered, 1 accepted cosmetic gap).
Docker/testcontainers: none left running after the run — `docker ps -a` clean, nothing to
remove. `mutants.out/` removed after the run (gitignored scratch output).
