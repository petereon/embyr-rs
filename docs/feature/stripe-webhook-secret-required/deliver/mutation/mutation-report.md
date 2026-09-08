# Mutation Testing Report — stripe-webhook-secret-required

**Tool**: cargo-mutants 27.0.0
**Date**: 2026-09-08/09
**Scope**: `crates/embyr-server` only (config.rs, main.rs, admin/router.rs) — single crate,
`--test-workspace true` not needed. 10 mutants across the full DELIVER diff
(`git diff 0d9708e 9852f85 -- crates/`).

This QUALITY_GATE was unusually eventful — it surfaced 2 genuine, security-relevant
acceptance-test gaps, and separately taught a new operational lesson about mutation-testing
discipline that's worth documenting in full rather than glossing over.

## Attempt 1 — full 10-mutant run, default invocation

`cargo mutants --workspace --in-place --timeout 240 --in-diff <diff> -- --test
production_readiness --lib`. Baseline confirmed clean, real coverage (64 lib tests, 6
non-ignored `production_readiness` tests). Result: **2 caught, 1 unviable, 7 missed** — every
miss landed on exactly the security-critical logic this feature added (`main.rs:212/217`'s
`stripe_billing_enabled`/`webhook_signing_secret` computation, `config.rs:274/278/280`'s
fail-fast validation, `router.rs:125/484`'s conditional-mount construction).

**Cause, confirmed not assumed**: this feature's own 2 acceptance tests (`pr06_stripe_webhook_
secret_required.rs`) are `#[ignore]`d subprocess tests (this project's own established
convention for real-binary-spawning tests), and `cargo-mutants`' default invocation does not
run `#[ignore]`d tests at all — a lesson already established twice this session
(`firestore-tls-support`'s own QUALITY_GATE hit the identical issue).

## Attempt 2 — targeted retest, `--ignored pr06` included

`git diff 0d9708e 9852f85 -- crates/embyr-server/src/{config.rs,main.rs,admin/router.rs} >
/tmp/stripe_targeted_diff.diff`, then `cargo mutants -p embyr-server --in-place --timeout 240
--in-diff <that diff> -- --test production_readiness --lib -- --ignored pr06` (name-filtered,
not a bare `--ignored` — a bare one pulls in `pr02_dockerfile`'s own known-flaky test and fails
the retest's own baseline, per `firestore-tls-support`'s own established fix for the identical
trap). Baseline passed. Result: **4 of 7 previously-missed mutants now caught** — but 3 remained
missed: `main.rs:212`, `main.rs:217`, `config.rs:278`.

**Investigated, not hand-waved**: tracing the actual test scenarios against the mutated
branches found 2 genuine, previously-uncovered scenarios:

1. **`config.rs:278`** (`.filter(|v| !v.is_empty())` on `STRIPE_WEBHOOK_SIGNING_SECRET`) — the
   existing AC-WHS-02 test only covers the *fully absent* var; it never tested the var being
   present but set to an **empty string** — the exact GitHub Actions repo-secret-defaults-to-
   empty scenario this whole feature exists to close. A `!` inverted to nothing here would go
   completely undetected by the existing test suite while still satisfying the letter of
   AC-WHS-02.
2. **`main.rs:212`/`main.rs:217`** (`stripe_billing_enabled`/`webhook_signing_secret`'s own
   computation) — the existing AC-WHS-01 test has `STRIPE_SECRET_KEY` absent, so
   `is_some_and`'s closure never runs regardless of the mutation; the existing AC-WHS-02 test
   exits (via `config.rs`'s own earlier validation) *before* main.rs's own mount logic ever
   executes. Neither existing test — nor the `cpb03`/`cpb04` regression guards, which construct
   `TestServer` directly via `build_admin_router` rather than spawning the real `embyr-server`
   binary — ever exercised `main.rs`'s own "correctly configured, should mount" branch through
   the actual production entry point.

**Fixed directly by the orchestrator** (not a subagent — a narrow, well-understood gap found
during QUALITY_GATE, matching this session's own established practice): 2 new tests added to
`pr06_stripe_webhook_secret_required.rs`:
- `exits_nonzero_when_stripe_webhook_signing_secret_is_empty_string` — proves the empty-string
  case fails exactly like the absent case.
- `webhook_route_is_reachable_when_stripe_correctly_configured` — proves the real `main.rs`
  binary genuinely mounts the route (asserts `!= 404`) when both vars are correctly, non-empty
  configured.

Both verified passing directly against the correct code before re-running mutation testing.

## Detour — a manual file-restore raced a live cargo-mutants process

Before the 3rd retest, `webhook_route_is_reachable_when_stripe_correctly_configured` appeared
to fail with a genuine 404 even against what should have been correct code. Root-cause
investigation (manually running the real binary + curl against the endpoint) found
`crates/embyr-server/src/admin/router.rs` still had a **leftover, uncommitted mutation on
disk** — `build_admin_router`'s own body had literally been replaced with `Default::default()
/* ~ changed by cargo-mutants ~ */`, never restored. `git checkout -- <file>` fixed it (confirmed
0 diff afterward), and a repo-wide grep confirmed no other file carried the same marker.

**However**: this fix was applied *while a subsequent mutation-testing retest (the same targeted
invocation, relaunched) was still actively running against that same file* — a distinct,
previously-undocumented hazard from the already-known cargo-sweep external corruption: manually
touching a file that a live `cargo-mutants --in-place` process still owns races its own
mutate/test/restore cycle unpredictably. That retest's own results (4 missed, 1 build-failed/
"unclassified failure", 1 timeout) were discarded entirely as untrustworthy — not analyzed,
not partially trusted, fully thrown out.

## Attempt 3 — genuinely clean retest

Confirmed via `pgrep -fl "cargo-sweep sweep"` (cleared) and `git status --short crates/`
(clean apart from known pre-existing unrelated dirty files) before launching. Warm-up build
run first. The retest itself ran for 28 minutes completely uninterrupted — no file was touched,
read-for-modification, or `git checkout`'d while the process (PID 95982) was alive; only after
`ps -p 95982` confirmed genuine exit was `git diff` run on the 3 scoped files (empty — clean).

**Result: 8 caught, 1 unviable, 1 missed.**

| Mutant | Result |
|---|---|
| `main -> ()` | **CAUGHT** |
| `main.rs:212` `delete !` | **CAUGHT** (by the new empty-string/correctly-configured tests) |
| `main.rs:217` `delete !` | **CAUGHT** |
| `config.rs:274` `delete !` | **CAUGHT** |
| `config.rs:278` `delete !` | **CAUGHT** (by the new empty-string test) |
| `config.rs:280` `&& -> \|\|` | **CAUGHT** |
| `config.rs:280` `delete !` | **CAUGHT** |
| `router.rs:125` `build_admin_router -> Default::default()` | **CAUGHT** |
| `config.rs:246` `from_env -> Ok(Default::default())` | unviable (`ConfigError` has no `Default`) |
| `router.rs:484` `build_with_secret_fetchers -> Default::default()` | missed — **scoping artifact** |

## The 1 remaining miss — confirmed scoping artifact

`build_with_secret_fetchers` is not called by `production_readiness` or `embyr-server`'s own
`--lib` unit tests — it's a backward-compatible test-helper wrapper (per its own doc comment:
"called by lib.rs test server constructors"), reached only via `build_with_aws`/`build_with_gcp`,
which `crates/embyr-server/src/lib.rs` calls from 8 separate `TestServer` constructors used by
`us_10_aws_secrets.rs`, `us_11_gcp_secrets.rs`, and 6 other acceptance test files — none of which
were included in this mutation run's own 2-target scope (chosen for per-mutant rebuild cost, not
coverage completeness, matching this session's own established precedent from
`security-rules-cel-chaining-detection`'s own `simulate_routed_access_rule` finding). Verified
via `grep -rn "build_with_secret_fetchers\|build_with_aws\|build_with_gcp"` across the whole
repo, not assumed.

## Verdict

**PASS.** Every mutant in the actual security fix this feature added is caught. The 1 remaining
miss is a confirmed scoping artifact with real coverage elsewhere, and the 1 unviable mutant is
a genuine `Default`-impl absence. Two real, meaningful acceptance-test gaps were found and fixed
along the way — mutation testing earned its cost concretely on this feature, not just
theoretically.
