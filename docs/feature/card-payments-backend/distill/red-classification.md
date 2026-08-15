# RED Classification — card-payments-backend

Per `nw-distill` § Pre-DELIVER fail-for-the-right-reason gate. Verified by running the walking
skeleton (the only non-`#[ignore]` scenario) against the real Postgres testcontainers + real
Stripe test-mode API composition root, and by `cargo check --workspace --tests` for the remaining
35 `#[ignore]` scenarios (compile-verified, not executed — per DISTILL dispatch's resource/time
discipline instruction not to run the slow real-I/O suite dozens of times during authoring).

## Walking skeleton — executed, classified

**Scenario**: `first_billing_call_auto_provisions_real_stripe_customer`
(`tests/card_payments_backend/acceptance/cpb01_real_subscription_record.rs`)

**Command**: `cargo test -p embyr-server --test card_payments_backend_cpb01_real_subscription_record -- --nocapture`

**Result**: FAILED (1 failed, 2 passed [state_delta port self-tests], 4 ignored)

**Observed failure chain**:
1. Testcontainers Postgres 15-alpine started successfully.
2. `SystemDb::migrate()` applied all 20 migrations (0001-0020) without error —
   **confirms migrations 0019/0020 are syntactically valid against real Postgres**.
3. `StripeGateway::new(stripe_secret_key())` resolved the real key via `.env.local`
   (no "missing credential" failure — the harness's `.env.local` loader worked).
4. The HTTP request reached `billing_subscription::get_subscription`, which
   panicked immediately at function entry:
   ```
   thread 'first_billing_call_auto_provisions_real_stripe_customer' panicked at
   crates/embyr-server/src/admin/handlers/billing_subscription.rs:70:5:
   SCAFFOLD: true -- billing_subscription::get_subscription not yet implemented
   -- RED scaffold (DISTILL, card-payments-backend US-201/US-206)
   ```
5. Because the panic occurs inside an Axum handler with no `catch_panic` layer
   installed anywhere in this router (workspace has no `tower-http` dependency
   today — confirmed absent before this DISTILL run), the client-visible
   symptom is a connection reset (`hyper::Error(IncompleteMessage)`), not a
   clean `500`. The test's `.expect("GET ... failed")` then panics on that
   `reqwest::Error`, which is the line pytest/cargo reports as the failure
   site.

**Classification: `MISSING_FUNCTIONALITY` — correct RED.**

The root cause is unambiguous from the full `--nocapture` output: the
production handler's own `SCAFFOLD: true` panic message appears in the log
immediately before the connection-reset symptom. This is not an
`IMPORT_ERROR` (the binary compiled and started), not a `FIXTURE_BROKEN`
(Postgres, migrations, and the Stripe key all resolved correctly — the
precise failure mode DISTILL's dispatch instructions explicitly flagged as
unacceptable if it had occurred), and not a `SETUP_FAILURE` (the request
reached production handler code). The Stripe key worked; no wasted Stripe API
call occurred because the scaffold panics before any Stripe SDK call.

**Note for DELIVER**: this project's admin router has no `tower_http::catch_panic`
layer. Every RED-scaffolded handler in this feature (`get_subscription`,
`post_subscription`, `stripe_webhook_handler`, `run_metering`,
`stripe_signature_middleware`) will exhibit the identical
connection-reset-with-server-side-panic-message symptom until each is
unskipped and implemented. This is flagged as an observation, not a blocker —
the server-side panic message is sufficient to classify RED correctly, and
adding a `CatchPanicLayer` (new `tower-http` dependency + router-wide
middleware change affecting all five sub-routers, not just this feature's)
was judged out of DISTILL's scope for this feature and is not a locked
decision from any prior wave.

## Remaining 35 `#[ignore]` scenarios — compile-verified

`cargo check --workspace --tests` passes with zero new errors (only
pre-existing warnings in unrelated files). Every `#[ignore]`d scenario:
- Imports only `des.application`/production-composition-root paths (no
  internal-component imports) — verified by code review during authoring.
- Calls a handler/adapter/domain-function whose body is `panic!("SCAFFOLD: true
  -- ... -- RED scaffold ...")` (or, for `run_cycle`/background-task
  scenarios, a panic inside the spawned task that manifests as an
  assertion-timeout in the polling helper — see
  `tests/card_payments_backend/acceptance/cpb06_cumulative_cap_check.rs::poll_for_cap_status`
  and `cpb07_cap_exceeded_suspension.rs::poll_for_project_status`).

Per the same reasoning as the executed walking skeleton, every `#[ignore]`d
scenario's eventual first run (when DELIVER removes its `#[ignore]`) will
fail with the identical `MISSING_FUNCTIONALITY` classification: the
production code path it drives through is a `SCAFFOLD: true` panic, not an
infrastructure gap. `CapUsageRefresher::run_cycle`'s panic happens inside a
detached `tokio::spawn`ed task — this does NOT crash the test process (Tokio
isolates panics to the spawned task); the calling test instead times out on
its own polling loop and fails via its own `assert_eq!`, which is itself a
clean `MISSING_FUNCTIONALITY` RED, not a `SETUP_FAILURE` (documented inline in
`crates/embyr-server/src/sweepers/cap_usage_refresher.rs::run_cycle`'s doc
comment).

## Zero "does not compile" / "harness bug" scenarios

Confirmed via `cargo check --workspace --tests` (zero new errors) and
`cargo test -p embyr-core --lib` (45 passed) / `cargo test -p embyr-server
--lib -- config:: sweepers::` (21 passed) — no regression in any existing
passing test, and the two new pure-function unit tests
(`advisory_lock_key_is_deterministic`, `advisory_lock_key_differs_for_different_input`)
pass immediately (not RED — mechanical helper, not business logic per the
scaffolding boundary documented in `cap_usage_refresher.rs`).
