# RCA — `sm03_encryption_key_rotation` 4-test failure ("server must start")

Author: Rex (nw-troubleshooter) — root cause analysis only. No fixes were
applied to the repository by this analysis (see Constraints). Exact fix
diffs are included below for the implementing agent/engineer to apply.

## Problem Statement (Scope)

**Symptom**: 4 of 6 tests in `tests/secrets_management/acceptance/sm03_encryption_key_rotation.rs`
(`totp_signin_decrypts_with_current_key`, `totp_signin_decrypts_with_previous_key_during_rotation_window`,
`totp_signin_fails_when_neither_current_nor_previous_key_decrypts`,
`totp_signin_malformed_ciphertext_below_nonce_minimum_fails_cleanly`) deterministically fail the
assertion `server.wait_for_healthy(Duration::from_secs(30)).await` ("server must start"), each
run taking ~33.3s, 100% reproducible.

**Boundary**: confirmed regression, introduced between `63a53d2` (passes, 5.87s) and `HEAD` (`b3679f6`)
via `card-payments-backend` steps `876ddde`/`ac9f9b5`/`b3679f6`. Confirmed NOT a "hang" — see WHY 2.
Sibling suites (`admin_api_v2`, `production_readiness`, sm03's other 2 tests) are unaffected — see
WHY 3/4 for why the blast radius is exactly these 4 tests and no others.

## Evidence Log

1. Reproduced the failure directly: `cargo test -p embyr-server --test secrets_management
   sm03_encryption_key_rotation::totp_signin_decrypts_with_current_key -- --test-threads=1` →
   `FAILED ... finished in 33.34s`, panic at `sm03_encryption_key_rotation.rs:90:5: server must start`.
2. Found a stray `embyr-server` process left running from a prior session (PID 42303, started
   Monday, holding ports 18080/18081/19090) — killed it (`kill -9 42303`) before further testing;
   unrelated to the regression itself but was contaminating manual repro attempts (false 200 on
   `/healthz`).
3. `ls -la target/release/embyr-server target/debug/embyr-server`:
   - `target/release/embyr-server` — **Aug 10 08:50**, 27,345,584 bytes
   - `target/debug/embyr-server` — Aug 11 (today), 110,722,112 bytes, rebuilt every time source changes
4. Empirically proved `cargo test` keeps `target/debug/embyr-server` fresh but never touches
   `target/release/embyr-server`: appended a comment to `main.rs`, ran
   `cargo test -p embyr-server --test secrets_management --no-run`, confirmed `target/debug/embyr-server`
   relinked (new mtime) while `target/release/embyr-server` mtime stayed at Aug 10 08:50. Reverted the
   `main.rs` edit via `git checkout -- crates/embyr-server/src/main.rs` (verified clean diff after).
5. Ran the **stale** `target/release/embyr-server` binary directly (`env -i` matching the test
   harness's env shape) against a Postgres DB that had already been migrated through migration 20 by
   the current (fresh) library code:
   ```
   ERROR embyr_server: startup failed: migration
     error=backend unavailable: migration 19 was previously applied but is missing in the resolved migrations
   ```
   Process exited immediately (`ps` confirmed no longer running); `/healthz` returned no response
   (curl: connection refused, `000`).
6. Ran the same stale release binary against a **freshly created, untouched** Postgres container
   (no pre-migration): reached `embyr-server ready` and `/healthz` returned `200` in ~200ms — proving
   the stale binary is not broken per se, only incompatible with a DB that a newer migrator has
   already advanced.
7. Ran the current **debug** binary (freshly built) with a real Stripe network probe reachable:
   reached `embyr-server ready` in <1s (Stripe probe returned a real 401 in ~500ms, logged as a
   soft-failure `WARN`, does not block startup) — this **directly refutes** the `CryptoProvider`
   conflict hypothesis raised in the initial investigation notes; `StripeGateway::new()`/`probe()`
   execute correctly and quickly, and are never even reached in the actual failure path (see WHY 2).
8. `grep` confirmed `system_db.migrate()` is called directly in the TEST PROCESS, before
   `ServerProcess::start(...)`, in exactly the 4 failing scenarios (`sm03_encryption_key_rotation.rs`
   lines 75→82, 147→154, 213→223, 281→289) and in no other scenario in that file (the 5th
   `ServerProcess` call, line 342, uses `start_env_only` with no pre-migration — this is
   `startup_rejects_identical_current_and_previous_encryption_key`, confirmed passing).
9. `grep` confirmed `admin_api_v2`'s test harness never spawns a subprocess at all — it drives
   `build_admin_router(...)` in-process via `tokio::spawn` inside the test binary — so it is
   structurally immune to any prebuilt-binary staleness issue.
10. `grep` confirmed `production_readiness`'s only scenario that pre-migrates via `SystemDb` before
    spawning `ServerProcess` (`exits_1_when_db_probe_fails_projects_table_missing`, pr01 line
    264-266) is `#[ignore]`d — explaining why that suite is otherwise 100% green.
11. `cargo tree -p embyr-server -i ring` / `-i aws-lc-rs`: confirmed both crypto backends are linked
    transitively (real, pre-existing fact) but this is inert — `StripeGateway::new()` installs
    `ring` explicitly and successfully every time (evidence #7); not a contributing factor.
12. `docs/architecture/atdd-infrastructure-policy.md` line 19 documents the intended binary
    resolution as `target/debug/` primary, `target/release/` as an alternate — the shipped code in
    both `embyr_server_binary()` copies does the reverse (release preferred whenever present, no
    freshness check), contradicting its own team's documented intent.

## Toyota 5 Whys

### Branch A — why the subprocess never answers `/healthz`

```
WHY 1A: 4 sm03 tests time out waiting for /healthz==200 within 30s.
  [Evidence #1 — reproduced, 33.34s, deterministic]

WHY 2A: The spawned embyr-server subprocess is not slow — it EXITS within ~300ms with a real,
        logged ERROR ("startup failed: migration ... migration 19 was previously applied but is
        missing in the resolved migrations"), and ServerProcess::wait_for_healthy() has no
        process-liveness check: it blindly polls GET /healthz every 200ms until the fixed 30s
        deadline regardless of whether the child process is even still alive.
  [Evidence #5 — direct repro of the stale binary against a pre-migrated DB reproduces the exact
   stderr message and immediate exit; tests/secrets_management/common/mod.rs::wait_for_healthy
   (lines 486-501) contains no child.try_wait() check]

WHY 3A: The subprocess's own sqlx migrator does not recognize migration versions 19/20 as valid,
        because the running binary is tests/*/common/mod.rs::embyr_server_binary()'s resolved
        target/release/embyr-server — built 2026-08-10 08:50, which predates
        migrations/0019_subscriptions.sql and migrations/0020_processed_webhook_events.sql
        (added by card-payments-backend steps 876ddde/ac9f9b5/b3679f6, still untracked/uncommitted
        in this working tree as of this session). Meanwhile the SAME test, in its own process,
        already ran system_db.migrate() using the freshly-compiled library (which DOES embed
        0019/0020 via sqlx::migrate!()) against the same DB before ever spawning the subprocess.
  [Evidence #3, #4, #5, #6, #8 — mtime diff, direct repro under both DB states, source-level
   confirmation of the pre-migrate-then-spawn pattern in exactly the 4 failing tests]

WHY 4A: embyr_server_binary() prefers target/release/embyr-server whenever it merely EXISTS on
        disk, with zero freshness/mtime comparison against current source — while
        target/debug/embyr-server is proven to be reliably rebuilt by `cargo test` itself (Cargo's
        own target graph rebuilds all of a package's targets — lib, [[bin]], and the integration
        test binary — as part of `cargo test` for that package). The helper trusts the WRONG
        artifact by default.
  [Evidence #4 — direct experiment: touching main.rs source and rerunning cargo test relinked
   target/debug/embyr-server but never touched target/release/embyr-server; #12 — the team's own
   documented intent lists debug as primary]

WHY 5A (ROOT CAUSE A): embyr_server_binary() was written under an implicit, undocumented
        assumption — "if target/release/embyr-server exists, it must have just been built as part
        of the current test run's setup, so it is fresh and should be preferred" — reasonable only
        for a CI job that always does `cargo build --release && cargo test` back-to-back in a
        disposable workspace. It breaks down in a long-lived, shared local `target/` directory
        (this repo/session), where a release binary built once for an EARLIER, unrelated purpose
        (most plausibly `production-readiness`'s ADR-017 D-PR-2 Dockerfile/CI validation, which
        requires a `cargo build --release`, or a prior agent session's manual sanity build) persists
        untouched across many subsequent `cargo test` runs and multiple feature branches' worth of
        migration/source changes, with NOTHING in the test suite to invalidate, warn about, or
        rebuild it. This exact flawed heuristic was hand-copied (not shared/reused) into at least 2
        independent test-harness files, per the harness's own explicit "independent copy... test
        suites in this workspace do not cross-import harnesses" design precedent — so the defect is
        systemic to the harness-authoring pattern used across this workspace's acceptance suites,
        not a one-off typo in a single file.
  [Evidence #12 — policy doc contradiction; #9, #10 — the pattern's blast radius maps exactly onto
   "which suites/tests combine subprocess-spawning with pre-migration", confirming this is a
   structural property of the harness pattern, not incidental]
```

### Branch B — why the failure was hard to diagnose (30s "hang" instead of an instant, clear error)

```
WHY 1B: The failure investigation initially treated this as a genuine startup hang/deadlock
        (network stack, crypto provider conflict, background task deadlock) rather than a fast,
        already-logged, self-explanatory error.
  [Evidence — original investigation notes ruled out CapUsageRefresher/StripeGateway individually
   and jointly, none of which affected the outcome, consistent with the failure occurring at
   main.rs Step 5 (migrate), long before either CapUsageRefresher (Step 10) or StripeGateway
   (Step 10, built between listener-bind and admin-router-build) are ever reached]

WHY 2B: ServerProcess::wait_for_healthy() cannot distinguish "still starting" from "already dead" —
        it has no child.try_wait() check in its polling loop.
  [Evidence #2 in WHY 2A — code inspection, tests/secrets_management/common/mod.rs:486-501]

WHY 3B: The harness's only failure-diagnosis primitive, drain_stderr(), is never invoked on this
        assertion's failure path — sm03's `assert!(server.wait_for_healthy(...).await, "server must
        start")` (line ~90) produces a fixed static message with no stderr excerpt, even though the
        subprocess's stderr (piped, per Stdio::piped()) contained the exact root cause the entire
        time.
  [Evidence — direct file read of sm03_encryption_key_rotation.rs around line 90; the
   #[ignore]'d production_readiness scenario, by contrast, DOES call server.drain_stderr() on its
   own exit-code assertion, showing the pattern exists elsewhere in the codebase but wasn't applied
   here]

WHY 4B: wait_for_healthy() was designed purely as a "port readiness" poll (mirroring a common
        integration-test idiom: retry until the HTTP endpoint responds), under the assumption that
        the only failure mode worth handling is "still starting up," not "has already exited" —
        an assumption reasonable when this harness pattern was first written (originally for
        production_readiness, per its own doc comment "independent copy of the ServerProcess shape
        already established in tests/production_readiness/common/mod.rs") but never revisited as
        the harness was reused for new suites with different, faster failure modes (a migration
        version mismatch fails in milliseconds, not seconds).
  [Evidence — doc comment lineage in tests/secrets_management/common/mod.rs line 20-24]

WHY 5B (ROOT CAUSE B, contributing/secondary): No test-harness convention in this workspace
        requires or encourages combining a bounded health-poll with a process-liveness check by
        default — each of the (currently 2, independently-copied) ServerProcess harnesses reimplements
        wait_for_healthy() from scratch with the same gap, rather than sharing one vetted
        implementation. This turns what the subprocess itself already diagnoses cleanly (a specific,
        logged ERROR line, exit code 1, sub-second) into a 30-second, symptom-only assertion failure
        that gives the investigator zero diagnostic signal and forced this session's multi-hour,
        multi-hypothesis investigation (CryptoProvider conflicts, CapUsageRefresher, StripeGateway
        probe network hangs) that all turned out to be irrelevant, because none of that code is ever
        reached before the actual failure point.
```

## Backward-Chain Validation

- **Root Cause A** ⇒ predicts: only tests that (a) pre-migrate via fresh library code AND (b) spawn
  `ServerProcess` will fail, in every suite using this harness pattern. Checked against all 3
  affected/adjacent suites (`secrets_management`, `production_readiness`, `admin_api_v2`) — the
  prediction matches the observed pass/fail set exactly (4-for-4 sm03 failures map 1:1 onto the 4
  pre-migrate+spawn call sites; `production_readiness`'s only matching scenario is `#[ignore]`d;
  `admin_api_v2` never spawns a subprocess). No unexplained failures, no unexplained passes.
- **Root Cause A** ⇒ predicts: rebuilding/removing the stale `target/release/embyr-server` (with
  no source change) would immediately turn all 4 tests green. Not executed as a live repo mutation
  by this RCA (implementation is out of scope for this role — see Constraints), but is fully
  supported by evidence #5/#6 (identical binary behaves correctly against a fresh DB, and fails with
  the EXACT observed class of error only when the DB has been advanced past what it knows).
- **Root Cause B** ⇒ predicts: adding a `try_wait()` check to `wait_for_healthy()` would not, by
  itself, turn the 4 tests green (Root Cause A must still be fixed) — but it WOULD change the
  failure's own diagnostic quality (exit code + stderr surfaced immediately instead of a 30s blind
  wait). This is consistent with Root Cause A/B being independent, non-contradictory, and additive:
  A explains the crash, B explains why the crash was 100x harder to diagnose than it needed to be.
- **Cross-check**: both roots collectively and exclusively explain every fact in the original
  investigation brief, including the negative results (disabling CapUsageRefresher/StripeGateway
  did nothing, because neither is ever reached — the process dies at Step 5, both of those are Step
  7/10) and the "clean diffs" finding (correct — none of the reviewed source diffs caused this; the
  cause is a stale *build artifact* sitting outside version control entirely, which is exactly why
  diff review could never have found it).

## Alternative Hypotheses Considered and Ruled Out

- **rustls `CryptoProvider` conflict** (the original investigation's leading hypothesis): directly
  refuted by evidence #7 — the debug binary constructs `StripeGateway` and runs `probe()` to
  completion (real Stripe 401 in ~500ms) without incident, and that code path (main.rs Step 10) is
  structurally unreachable in the failing scenario anyway, since the process already exits at Step 5.
- **Flaky `testcontainers` Postgres startup race**: inconsistent with the reported symptom
  (100% reproducible across 3 consecutive runs, not flaky) and with this RCA's own reproduction
  (evidence #1, deterministic 33.34s on a single targeted run). A startup race would also not
  produce the specific, consistent `migration 19 was previously applied but is missing` stderr
  line seen in every direct reproduction against a pre-migrated DB (evidence #5) — that message is
  sqlx's own version-mismatch error, not a connection/timing error, and the DB is fully up and
  serving queries by the time the subprocess's `migrate()` call runs (the test process itself
  already connected to and migrated the same DB moments earlier).
- **Port-allocation collision** (`find_free_port()` racing between the 3 ports it allocates per
  test, or across concurrently-running tests): does not explain why specifically the SAME 4
  scenarios fail every single time while sm03's other 2 scenarios and every scenario in sibling
  suites reliably avoid it — a genuine ephemeral-port race would be flaky/non-deterministic and
  would not correlate with the pre-migrate-then-spawn code shape. It also does not explain the
  captured stderr content (evidence #5), which a port collision would not produce (a bind conflict
  fails with `Address already in use`, a distinct and previously-seen error in this investigation's
  own tooling — see stray-process cleanup note above — not the migration-version error actually
  observed).
- **Migration deadlock/lock contention**: ruled out by evidence #5/#6 — the stale binary's
  `migrate()` call fails IMMEDIATELY with a resolved error value (not a hang/timeout), and succeeds
  immediately against a fresh DB with the identical binary and identical Postgres image/version.

## Solutions

| # | Root Cause | Immediate Mitigation | Permanent Fix | Early Detection |
|---|---|---|---|---|
| A | `embyr_server_binary()` blindly prefers a possibly-stale `target/release/embyr-server` over the cargo-guaranteed-fresh `target/debug/embyr-server` | `rm target/release/embyr-server` (or `cargo build --release` to resync it) — unblocks the 4 tests today with zero source change | Flip the preference order in both copies of `embyr_server_binary()` to prefer `target/debug/` (cargo-guaranteed fresh), falling back to `target/release/` only when debug is absent. See exact diff below. | Add a one-line CI/pre-test check (or a `build.rs`/xtask assertion) that fails fast if `target/release/embyr-server` exists and is older than the newest file under `crates/embyr-server/src/` or `migrations/` — turns "silently stale artifact" into an explicit, loud error instead of a 30s misdiagnosed test failure |
| B | `ServerProcess::wait_for_healthy()` has no process-liveness check, converting fast/clear failures into slow/opaque ones | N/A (diagnostic-quality issue, not a correctness bug — no user-facing mitigation needed) | Add `if let Ok(Some(status)) = self.child.try_wait() { return HealthCheckOutcome::ProcessExited(status, self.drain_stderr()) }` (or equivalent) inside the poll loop, and have callers assert on a richer outcome that includes stderr — mirrors the pattern already used correctly in `production_readiness`'s `#[ignore]`d exit-code scenario | Consolidate the currently-duplicated `ServerProcess`/`embyr_server_binary()` harness (2 independent copies today, explicitly documented as intentionally non-shared) into one shared, single-sourced test-support module, so a fix to the liveness-check gap (or the binary-freshness gap) only has to be made once and can't silently diverge between suites again |

### Exact fix diff (for the implementing agent — not applied by this RCA)

Both `tests/secrets_management/common/mod.rs` and `tests/production_readiness/common/mod.rs`,
function `embyr_server_binary()`:

```diff
-    let release = workspace_root.join("target/release/embyr-server");
-    if release.exists() {
-        release
-    } else {
-        workspace_root.join("target/debug/embyr-server")
-    }
+    // `cargo test` always rebuilds every target of the package under test
+    // (lib + [[bin]] + the integration-test binary) as part of its own build
+    // graph, so target/debug/embyr-server is guaranteed fresh on every run.
+    // target/release/embyr-server is NOT — it's only produced by a separate,
+    // explicit `cargo build --release` that `cargo test` never triggers, so
+    // it can silently go stale in a shared target/ directory across
+    // unrelated source/migration changes. Prefer debug; fall back to
+    // release only when debug doesn't exist.
+    let debug = workspace_root.join("target/debug/embyr-server");
+    if debug.exists() {
+        debug
+    } else {
+        workspace_root.join("target/release/embyr-server")
+    }
```

## Verification Plan (for the implementing agent)

After applying the diff above (both files):
1. `cargo test -p embyr-server --test secrets_management` — expect all 6 sm03 tests + the rest of
   the binary green (~34/35, 1 pre-existing `#[ignore]`), per the task's stated target.
2. `cargo test -p embyr-server --test card_payments_backend_cpb01_real_subscription_record --test card_payments_backend_cpb02_real_plan_change`
   — unaffected by this change (that suite never uses `embyr_server_binary()`/`ServerProcess`,
   confirmed via `grep` in evidence gathering); should remain 5/5 each.
3. `cargo test -p embyr-server --test admin_api_v2_walking_skeleton --test admin_api_v2_b02_project_list --test production_readiness`
   — `admin_api_v2` unaffected (in-process router, no subprocess); `production_readiness` unaffected
   in its default (non-`#[ignore]`) run, and should ALSO now pass if the `#[ignore]` is ever lifted
   on `exits_1_when_db_probe_fails_projects_table_missing`.
4. `cargo build --workspace --features embyr-admin-ui/csr` clean, per the task's stated
   pre-commit gate.

## Note on scope

This analysis stopped short of applying the fix, running the full build/test/commit sequence, or
touching git history, because implementing fixes and writing/editing code (including test-harness
code) is outside this role's defined constraints ("Investigates and analyzes only. Does not
implement fixes or write application code."). Two speculative edits made mid-investigation to
validate the fix diff's shape were reverted via named-path `git checkout --` before this document
was finalized; the working tree is unchanged from the state at task start (verified: no diff on
`tests/secrets_management/common/mod.rs` or `tests/production_readiness/common/mod.rs`). The exact
diff above is ready for an implementing agent (e.g. `nw-software-crafter`) to apply, build, test,
and commit per the task's original acceptance criteria.
