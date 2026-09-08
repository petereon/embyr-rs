# Mutation Testing Report — firestore-tls-support

**Tool**: cargo-mutants 27.0.0
**Date**: 2026-09-08
**Scope**: `crates/embyr-server` only (no `embyr-core`/`embyr-pg-storage` changes — this feature is
transport-layer only) — `git diff 090b762 f927b4e -- crates/embyr-server`. 31 mutants.

`cargo mutants -p embyr-server --in-place --timeout 240 --in-diff <diff> -- --test
production_readiness --lib`

Single-package scope (no `--workspace`/`--test-workspace` needed, unlike every prior feature
this session touching `embyr-core`) — noticeably faster per-mutant than the OR-filter/IS_NULL
features' own cross-crate runs.

## Full run — 2 caught, 23 unviable, 6 missed

23 unviable mutants were overwhelmingly `-> TestServer with Default::default()` (the 8 existing
`start_test_server_with_*` constructors, none of which derive `Default`) and `JoinHandle::new()`-
family substitutions on `spawn_all_servers`/`spawn_admin_server`/`spawn_hybrid_server` (their own
`tokio::task::JoinHandle<()>` return types have no such constructor) — all structurally impossible
compile-time rejections, not real coverage gaps. 2 caught directly on the first pass.

6 missed, all in `crates/embyr-server/src/config.rs`:
- `TlsMaterial`'s `Debug` impl → `Ok(Default::default())` (line 140)
- Both `!` deletions on the `missing.is_empty()` checks (lines 255/256)
- All 3 match-arm deletions in the TLS var partial-config resolution: `(Some(_), None)`,
  `(None, Some(_))`, `(Some(cert_path), Some(key_path))` (lines 258/259/268)

## Discovery: cargo-mutants skips `#[ignore]`d tests by default

5 of the 6 misses are exactly the code this feature's own AC-TLS-05/06/07 cover —
`load_tls_material`'s error paths and the `from_env` TLS-var resolution. Those 3 acceptance tests
are `#[ignore]`d (real-subprocess tests spawning the actual `embyr-server` binary, mirroring
`pr04_graceful_shutdown.rs`'s own established precedent) — and `cargo-mutants`' default test
invocation does not pass `--ignored` through, so these tests never run under mutation testing at
all. Not a coverage gap in the tests themselves (all 3 were independently verified passing,
directly, by the orchestrator, before mutation testing even started) — a gap in what the mutation
tool exercises by default.

## First `--ignored` retest attempt — baseline failure, unrelated cause

Retried the 6 missed `config.rs` mutants with `-- --test production_readiness --lib --
--ignored` appended (second `--` separator passes `--ignored` through as a libtest arg). The
retest's own **baseline** (unmutated code) failed: `--ignored` bare pulls in *every* ignored test
in the `production_readiness` binary, including `pr02_dockerfile::cargo_chef_rebuild_fast` — a
pre-existing, already-known failure in this dev environment (no Docker image built), unrelated to
this feature. cargo-mutants refuses to test any mutant unless the baseline passes 100% clean, so
this attempt never got past its own baseline.

## Corrected retest — scoped `--ignored` to this feature's own tests

Fixed by adding a name filter: `-- --test production_readiness --lib -- --ignored pr05` (`pr05`
matches only this feature's own test module, `acceptance::pr05_tls_support::*`). Baseline passed
cleanly. Result: 4 of the 5 real mutants (lines 255, 256, 258, 268) now **caught**. One
(`(None, Some(_))` at line 259 — the "only `EMBYR_TLS_KEY_PATH` set" partial-config direction)
remained missed.

**Reusable lesson**: when mutation-testing a feature with `#[ignore]`d acceptance tests, don't
pass bare `--ignored` — scope it to a name filter matching only that feature's own tests
(`-- --ignored <feature-specific-substring>`), or a pre-existing unrelated `#[ignore]`d failure
elsewhere in the same binary will fail the baseline and block the entire mutation run.

## The one remaining real miss — a genuine test gap, not an artifact

`(None, Some(_))` staying missed even with the right tests running was NOT a tooling artifact —
direct investigation confirmed `tests/production_readiness/acceptance/pr05_tls_support.rs` (as
written by DISTILL) had a test for the cert-path-only partial-config direction
(`exits_nonzero_when_only_cert_path_is_set`, AC-TLS-05) but **no symmetric test for the
key-path-only direction**. `config.rs`'s own production code was already correct (verified by
direct reading — the match is symmetric, `(None, Some(_)) => missing.push("EMBYR_TLS_CERT_PATH")`)
— this was a real, if narrow, acceptance-test coverage gap, exactly what mutation testing is for.

**Fix**: added `exits_nonzero_when_only_key_path_is_set` to `pr05_tls_support.rs`, mirroring the
existing test's shape exactly (only `EMBYR_TLS_KEY_PATH` set, asserts non-zero exit + stderr names
`EMBYR_TLS_CERT_PATH` as the missing half). Verified passing directly first (`cargo test ... --
--ignored exits_nonzero_when_only_key_path_is_set` → 1 passed), then re-ran the targeted mutation
retest — the mutant is now **caught**.

## Final result — 7 caught, 23 unviable, 1 accepted cosmetic miss

| Mutant | Result |
|---|---|
| `TlsMaterial::fmt` (Debug) → `Ok(Default::default())` | Accepted miss — cosmetic, no test asserts on Debug output |
| `ConfigError::fmt` (Display) → `Ok(Default::default())` | **CAUGHT** |
| `!` deletion, `missing.is_empty()` check (×2) | **CAUGHT** |
| `(Some(_), None)` match arm deletion | **CAUGHT** |
| `(None, Some(_))` match arm deletion | **CAUGHT** (after adding `exits_nonzero_when_only_key_path_is_set`) |
| `(Some(cert_path), Some(key_path))` match arm deletion | **CAUGHT** |
| Plus the 2 originally-caught + 23 unviable from the full run | unchanged |

**31 total: 7 caught, 23 unviable, 1 accepted cosmetic miss, 0 unresolved real gaps.**

## Verdict

**PASS.** One genuinely real gap found and fixed (a missing symmetric acceptance test), one
accepted cosmetic miss (Debug formatting), everything else caught or structurally unviable. The
`--ignored`-scoping investigation is itself a durable, reusable finding for any future feature in
this codebase with subprocess-based `#[ignore]`d acceptance tests.
