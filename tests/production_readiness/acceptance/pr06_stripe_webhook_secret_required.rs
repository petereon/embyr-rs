// @error @boundary @US-01
//! stripe-webhook-secret-required (US-01) — the "billing enabled, webhook
//! secret missing" configuration state can never be running in production.
//!
//! Acceptance criteria verified here:
//!   AC-WHS-01: with STRIPE_SECRET_KEY unset (Stripe billing fully
//!              unconfigured), the server starts normally and the webhook
//!              route is not mounted — POST to it returns 404, never a
//!              signature-verification response.
//!   AC-WHS-02: with STRIPE_SECRET_KEY set and STRIPE_WEBHOOK_SIGNING_SECRET
//!              unset, startup exits non-zero before any port binds, with
//!              stderr naming STRIPE_WEBHOOK_SIGNING_SECRET specifically.
//!
//! AC-WHS-03/04 (regression guards — real correctly-signed and forged-empty-
//! key-signed webhook requests) are NOT re-tested here: they are already
//! covered by `tests/card_payments_backend/acceptance/cpb03_webhook_ingestion.rs`
//! and `cpb04_dunning_suspension_recovery.rs`, both of which always configure
//! a real, non-empty webhook secret via `CpbTestContext::with_webhook_secret`
//! / `new_with_stripe_key`. AC-WHS-05 is the full regression suite, run by
//! the orchestrator, not a scenario here.
//!
//! Driving port: `embyr-server` binary subprocess via ENV/STDOUT/STDERR, and
//!   a real HTTP POST to `/admin/v1/webhooks/stripe` on the admin port.
//!
//! Both tests are #[ignore] (subprocess-based error/boundary scenarios),
//! mirroring `pr05_tls_support.rs`'s own AC-TLS-05/06/07 convention.
//!
//! Scaffold state: NONE created by this DISTILL pass. `ServerConfig::from_env()`
//!   does not yet validate STRIPE_WEBHOOK_SIGNING_SECRET (config.rs Decision 1),
//!   and `build_admin_router` still mounts the webhook route unconditionally
//!   (router.rs Decision 2) — both tests are expected to FAIL against today's
//!   code for the right reason (missing production behavior), not a test-setup
//!   bug. DELIVER implements DESIGN's fully-specified Handoff Package.

use std::time::Duration;

use crate::common::{start_postgres_container, ServerProcess, TEST_ENCRYPTION_KEY};

// ─── AC-WHS-01: Stripe fully unconfigured → webhook route not mounted ───────

/// A deployment with Stripe billing fully unconfigured starts normally, and
/// the webhook route is not reachable — 404, not a signature-verification
/// response.
///
/// Journey:
///   Given: STRIPE_SECRET_KEY and STRIPE_WEBHOOK_SIGNING_SECRET are both
///          unset (this harness's `ServerProcess::start` now explicitly
///          `.env_remove()`s both — see Decision 3 fix in this file's own
///          `common/mod.rs`, so CI's job-level STRIPE_SECRET_KEY cannot leak
///          in here)
///   When:  Sam Chen starts embyr-server and it becomes healthy
///   And:   a request is POSTed to /admin/v1/webhooks/stripe
///   Then:  the response is 404 — the route does not exist, exactly like any
///          other nonexistent path, not a 401/400 from signature middleware
///
/// Today's actual behavior (before Decision 2 lands): the webhook route is
/// mounted unconditionally, so this POST is rejected by
/// `stripe_signature_middleware` with 401 (missing `Stripe-Signature`
/// header) — this test fails today for exactly that reason, not a fixture
/// bug.
///
/// @error @boundary @US-01 @AC-WHS-01
#[tokio::test]
#[ignore]
async fn webhook_route_not_reachable_when_stripe_fully_unconfigured() {
    let (_pg, db_url) = start_postgres_container().await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
            // STRIPE_SECRET_KEY / STRIPE_WEBHOOK_SIGNING_SECRET intentionally
            // absent — Decision 3's env_remove() guarantees they are not
            // inherited from the test-runner's own environment either.
        ],
    );

    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(
        healthy,
        "server with no Stripe config at all must start normally and become healthy"
    );

    let client = reqwest::Client::new();
    let url = format!(
        "http://127.0.0.1:{}/admin/v1/webhooks/stripe",
        server.admin_port
    );
    let resp = client
        .post(&url)
        .body("{}")
        .send()
        .await
        .expect("POST to webhook route must complete (connection refused would be a different bug)");

    assert_eq!(
        resp.status().as_u16(),
        404,
        "webhook route must not be mounted when Stripe billing is fully unconfigured — \
         expected 404, got {}",
        resp.status()
    );

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(10)).await;
}

// ─── AC-WHS-02: billing enabled, webhook secret missing → exit non-zero ─────

/// Startup fails fast when STRIPE_SECRET_KEY is set but
/// STRIPE_WEBHOOK_SIGNING_SECRET is not — never a silent empty-key fallback.
///
/// Journey (error path):
///   Given: DATABASE_URL/EMBYR_ADMIN_KEY/EMBYR_ENCRYPTION_KEY are all set
///   And:   STRIPE_SECRET_KEY is set to a live-shaped Stripe secret key
///   And:   STRIPE_WEBHOOK_SIGNING_SECRET is unset
///   When:  Sam Chen starts embyr-server
///   Then:  the process exits with code 1 before any port is bound
///   And:   stderr names STRIPE_WEBHOOK_SIGNING_SECRET specifically as
///          required because STRIPE_SECRET_KEY is set
///
/// Today's actual behavior (before Decision 1 lands): `ServerConfig::from_env()`
/// performs no Stripe-pair validation at all, so the server starts
/// successfully with an empty-string webhook secret
/// (`main.rs:216`'s `.unwrap_or_default()`) — this test fails today because
/// the process never exits within the timeout, not because of a fixture bug.
///
/// @error @US-01 @AC-WHS-02
#[tokio::test]
#[ignore]
async fn exits_nonzero_when_stripe_secret_key_set_without_webhook_signing_secret() {
    let mut server = ServerProcess::start_env_only(&[
        (
            "DATABASE_URL",
            "postgres://postgres:postgres@127.0.0.1:65535/embyr",
        ),
        ("EMBYR_ADMIN_KEY", "testkey"),
        ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ("STRIPE_SECRET_KEY", "sk_live_stripe_webhook_secret_required_test"),
        // STRIPE_WEBHOOK_SIGNING_SECRET intentionally absent
    ]);

    let exit_code = server.wait_for_exit(Duration::from_secs(3)).await;
    let stderr = server.drain_stderr();

    assert_eq!(
        exit_code,
        Some(1),
        "server must exit 1 when STRIPE_SECRET_KEY is set without \
         STRIPE_WEBHOOK_SIGNING_SECRET; got {exit_code:?}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("STRIPE_WEBHOOK_SIGNING_SECRET"),
        "stderr must name STRIPE_WEBHOOK_SIGNING_SECRET as the missing required \
         variable; got: {stderr}"
    );
    assert!(
        !ServerProcess::port_is_bound(server.grpc_port),
        "no port may bind when Stripe billing is half-configured"
    );
}
