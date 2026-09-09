// @error @boundary @US-01
//! stripe-webhook-body-limit (US-01) — an oversized, unauthenticated request
//! body to `POST /admin/v1/webhooks/stripe` can never force unbounded memory
//! allocation before signature verification runs; legitimate, normally-sized
//! Stripe traffic is completely unaffected.
//!
//! Acceptance criteria verified here (feature-delta.md, DISCUSS+DESIGN):
//!   AC-WBL-01: a body over the 5 MiB ceiling is rejected with 413 BEFORE
//!              being fully buffered — proven by RSS growth NOT scaling with
//!              the oversized body's actual size (bounded growth), not
//!              merely by the returned status code.
//!   AC-WBL-02 (non-negotiable regression guard): a legitimate, normally-
//!              sized, correctly-signed webhook succeeds end-to-end exactly
//!              as today.
//!   AC-WBL-03: rejection status is 413 for the size-limit case; a
//!              correctly-signed webhook comfortably within the 5 MiB
//!              ceiling still succeeds (boundary case).
//!   AC-WBL-04 (regression guard, composed with AC-WBL-01): an oversized,
//!              garbage-signed request is rejected before
//!              `verify_webhook_signature` is ever attempted (413, not 401)
//!              and with zero writes to `processed_webhook_events` — mirrors
//!              this middleware's own pre-existing AC-203-02 zero-DB-write
//!              guarantee.
//!   AC-WBL-05 (full regression suite) is NOT a new test here — orchestrator
//!              runs the full workspace suite once, at the pre-commit gate.
//!
//! Mechanism under test (DESIGN, locked): `stripe_signature.rs:44`'s
//! `axum::body::to_bytes(body, usize::MAX)` replaced with a concrete 5 MiB
//! constant; the resulting `LengthLimitError` is downcast to discriminate
//! 413 (size-limit) from the pre-existing 401 (every other `to_bytes` error
//! cause, unchanged).
//!
//! Real-I/O strategy (Strategy A, per DISCUSS's own Walking Skeleton
//! Strategy section — this feature IS its own walking skeleton): every
//! scenario is a real HTTP request against a real spawned `embyr-server`
//! subprocess + real Postgres testcontainer, mirroring pr06's own harness
//! and `webhook_route_is_reachable_when_stripe_correctly_configured`'s
//! server-spin-up pattern exactly.
//!
//! Why a garbage-but-PRESENT `Stripe-Signature` header (not an absent one)
//! on the oversized-body scenarios: the header-presence check
//! (`stripe_signature.rs:41`) runs BEFORE the body is ever read. An absent
//! header short-circuits to 401 with zero bytes read regardless of body
//! size — that path is not what this feature changes. Only a PRESENT (even
//! if cryptographically invalid) header reaches the `to_bytes` call this
//! feature's fix targets, matching DISCUSS's own Example 3 wording ("no
//! valid Stripe-Signature header VALUE", not "no header").
//!
//! Layer: WS/`@wiring_e2e` (real subprocess, real I/O, ~1-3s each) — per
//! `nw-test-design-mandates` Layered Test Discipline, this layer uses
//! traditional assertions (not `assert_state_delta`), example-only (no PBT),
//! matching pr01-pr06's own established style in this same file family.
//!
//! Scaffold state: NONE created by this DISTILL pass — no new production
//! module is imported (this feature is a 3-line change entirely inside the
//! pre-existing `stripe_signature.rs`, which already compiles and links).
//! All 4 tests are expected to FAIL against today's code for the right
//! reason (413 not yet returned; RSS growth not yet bounded), not a
//! test-setup bug. DELIVER implements DESIGN's fully-specified Handoff
//! Package. First test enabled (not `#[ignore]`) per this file family's own
//! one-at-a-time convention would normally be the walking skeleton — this
//! feature's single story means all 4 stay `#[ignore]` like pr06's own
//! error/boundary scenarios, enabled one at a time in DELIVER.

use std::time::Duration;

use crate::common::{
    read_rss_kb, sign_stripe_payload, start_postgres_container, ServerProcess,
    TEST_ENCRYPTION_KEY,
};

const STRIPE_SECRET_KEY: &str = "sk_live_stripe_webhook_body_limit_test";
const WEBHOOK_SECRET: &str = "whsec_stripe_webhook_body_limit_test";

/// DESIGN's locked ceiling (`stripe_signature.rs`'s own
/// `MAX_STRIPE_WEBHOOK_BODY_BYTES` constant) — duplicated here as a plain
/// `usize` so the test can express "just under" / "an order of magnitude
/// over" relative to the SAME number DESIGN locked, not a hand-picked guess.
const CEILING_BYTES: usize = 5 * 1024 * 1024; // 5 MiB

/// Start a correctly-configured server (Stripe billing enabled, webhook
/// route mounted) against a fresh Postgres testcontainer. Shared setup
/// across all four scenarios in this file (Pillar 2: every scenario's
/// `Given` reuses this same startup step).
async fn start_correctly_configured_server() -> (
    testcontainers_modules::testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
    ServerProcess,
) {
    let (pg, db_url) = start_postgres_container().await;
    let server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
            ("STRIPE_SECRET_KEY", STRIPE_SECRET_KEY),
            ("STRIPE_WEBHOOK_SIGNING_SECRET", WEBHOOK_SECRET),
        ],
    );
    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(
        healthy,
        "server with correctly-configured Stripe billing must start normally"
    );
    (pg, server)
}

fn webhook_url(server: &ServerProcess) -> String {
    format!(
        "http://127.0.0.1:{}/admin/v1/webhooks/stripe",
        server.admin_port
    )
}

// ─── AC-WBL-01/03: oversized body rejected with 413, RSS growth bounded ────

/// A body an order of magnitude over the 5 MiB ceiling is rejected with 413,
/// and the process's own resident memory does not grow proportionally to
/// the oversized body's actual size — proving the body was NOT fully
/// buffered before rejection (the actual security property AC-WBL-01
/// requires, not merely the status code).
///
/// Journey:
///   Given: Stripe billing is correctly configured (webhook route mounted)
///   When:  a request just under the 5 MiB ceiling is sent (garbage
///          signature, so it reaches the buffering step)
///   And:   a request an order of magnitude over the ceiling is sent
///   Then:  the oversized request is rejected with 413
///   And:   the oversized request's RSS growth is not ~10x the under-limit
///          request's growth (which unbounded buffering would produce) —
///          bounded regardless of how large the attacker's payload claims
///          to be
///
/// Today's actual behavior (before DESIGN's fix lands): `to_bytes` is
/// called with `usize::MAX`, so BOTH requests buffer fully; the oversized
/// request returns 401 (garbage signature fails HMAC, same as the
/// under-limit request) instead of 413, and its RSS growth scales with its
/// real ~50 MiB size — this test fails today on the status-code assertion
/// first (right reason: 413 not implemented), not a fixture bug.
///
/// RSS threshold (20 MB) is a deliberately generous, heuristic ceiling — not
/// exact science, since allocator/OS memory-reclaim behavior is noisy —
/// chosen to sit far below the ~45 MB gap unbounded buffering of a 50 MiB
/// body would produce, and far above ordinary measurement jitter.
/// ponytail: fixed heuristic threshold, revisit if CI shows false failures.
///
/// @error @boundary @US-01 @AC-WBL-01 @AC-WBL-03 @real-io @wiring_e2e
#[tokio::test]
async fn oversized_body_rejected_with_413_and_bounded_memory_growth() {
    let (_pg, mut server) = start_correctly_configured_server().await;
    let client = reqwest::Client::new();
    let url = webhook_url(&server);
    let garbage_signature = "t=0,v1=deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdead";

    let rss_before = read_rss_kb(server.child.id());

    // Just under the ceiling — reaches buffering, fails HMAC verify (401),
    // establishes the "ordinary buffering" RSS growth baseline.
    let under_limit_body = vec![b'a'; CEILING_BYTES - (1024 * 1024)]; // 4 MiB
    let under_resp = client
        .post(&url)
        .header("Stripe-Signature", garbage_signature)
        .body(under_limit_body)
        .send()
        .await
        .expect("under-limit POST must complete");
    let rss_after_under = read_rss_kb(server.child.id());

    // An order of magnitude over the ceiling.
    let over_limit_body = vec![b'a'; CEILING_BYTES * 10]; // 50 MiB
    let over_resp = client
        .post(&url)
        .header("Stripe-Signature", garbage_signature)
        .body(over_limit_body)
        .send()
        .await
        .expect("over-limit POST must complete");
    let rss_after_over = read_rss_kb(server.child.id());

    assert_eq!(
        under_resp.status().as_u16(),
        401,
        "a body under the ceiling with a garbage signature must still be \
         rejected the pre-existing way (401, unchanged) — regression guard"
    );
    assert_eq!(
        over_resp.status().as_u16(),
        413,
        "a body an order of magnitude over the 5 MiB ceiling must be \
         rejected with 413 Payload Too Large, got {}",
        over_resp.status()
    );

    if let (Some(before), Some(after_under), Some(after_over)) =
        (rss_before, rss_after_under, rss_after_over)
    {
        let growth_under_kb = after_under.saturating_sub(before);
        let growth_over_kb = after_over.saturating_sub(after_under);
        assert!(
            growth_over_kb < growth_under_kb + 20_000,
            "oversized (50 MiB) request's RSS growth ({growth_over_kb} KB) must be \
             bounded, not scaling with the actual payload size — under-limit \
             (4 MiB) request's growth was {growth_under_kb} KB; unbounded \
             buffering of the full 50 MiB body would blow well past a 20 MB \
             headroom over that baseline"
        );
    }
    // rss_kb() returning None (e.g. non-Linux dev box without /proc) means
    // the memory assertion is skipped, not silently passed as "fine" — the
    // status-code assertions above already gate RED/GREEN correctness.

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(10)).await;
}

// ─── AC-WBL-02: legitimate, normally-sized, correctly-signed webhook ───────

/// A legitimate, normally-sized, correctly-signed webhook event must always
/// succeed end-to-end — the single most important regression guard in this
/// feature (repeated 3x across DISCUSS): a false-positive rejection here
/// would break real production billing traffic.
///
/// Journey:
///   Given: Stripe billing is correctly configured (webhook route mounted)
///   When:  a real-shaped, correctly-signed customer.subscription.updated
///          event (a few hundred bytes, typical of real Stripe payloads) is
///          delivered
///   Then:  the request succeeds (200) — signature verification and event
///          dispatch are completely unaffected by the body-limit change
///
/// `sync_subscription_updated` returns 200 even for an unmatched
/// `stripe_subscription_id` (`UPDATE ... WHERE` affecting 0 rows is still
/// `Ok`) — the same synthetic-but-correctly-signed-payload shape
/// `cpb03_webhook_ingestion.rs`'s own AC-203-03/06 scenarios already use, so
/// this test needs no seeded billing account to prove genuine signature
/// verification + handler dispatch succeeded (full DB-state proof for a
/// *matched* account is already covered by `cpb03`/`cpb04` and is not
/// duplicated here — this feature only changes body-buffering behavior,
/// not the sync logic itself).
///
/// Today's actual behavior: passes already (this scenario is unaffected by
/// the vulnerable line) — included as the explicit non-negotiable
/// regression guard DISCUSS names 3 times, proven against DESIGN's actual
/// changed code once DELIVER lands the fix, not merely assumed unchanged.
///
/// @driving_port @US-01 @AC-WBL-02 @real-io @wiring_e2e
#[tokio::test]
async fn legitimate_normally_sized_webhook_succeeds_end_to_end() {
    let (_pg, mut server) = start_correctly_configured_server().await;
    let client = reqwest::Client::new();
    let url = webhook_url(&server);

    let event_id = format!("evt_wbl_{}", uuid::Uuid::new_v4());
    let payload = serde_json::json!({
        "id": event_id,
        "type": "customer.subscription.updated",
        "data": {
            "object": {
                "id": "sub_wbl_test_normally_sized",
                "status": "active",
                "current_period_end": 1_799_999_999i64
            }
        }
    })
    .to_string();
    let signature = sign_stripe_payload(&payload, WEBHOOK_SECRET);

    let resp = client
        .post(&url)
        .header("Stripe-Signature", signature)
        .header("Content-Type", "application/json")
        .body(payload)
        .send()
        .await
        .expect("webhook POST failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-WBL-02: a legitimate, normally-sized, correctly-signed webhook \
         must always succeed end-to-end — got {}",
        resp.status()
    );

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(10)).await;
}

// ─── AC-WBL-03 (boundary): large-but-legitimate webhook still succeeds ─────

/// A correctly-signed webhook comfortably within the 5 MiB ceiling — sized
/// to represent the largest realistic Stripe payload (an `invoice.finalized`-
/// shaped event with many line items, per DISCUSS's own Example 2) — still
/// succeeds, proving the generous headroom is real, not theoretical.
///
/// Journey (chained — reuses the correctly-signed-payload construction from
/// `legitimate_normally_sized_webhook_succeeds_end_to_end`, extended with a
/// 1 MiB filler field):
///   Given: Stripe billing is correctly configured
///   When:  a correctly-signed webhook event with a ~1 MiB body (well under
///          the 5 MiB ceiling, comfortably larger than any real Stripe
///          payload) is delivered
///   Then:  the request still succeeds (200)
///
/// Today's actual behavior: passes already (5 MiB vs. today's `usize::MAX`
/// both admit a 1 MiB body) — this is the explicit boundary regression
/// guard DESIGN's Handoff Package calls out, proven against the real
/// changed code once DELIVER lands the fix.
///
/// @US-01 @AC-WBL-03 @real-io @wiring_e2e
#[tokio::test]
async fn correctly_signed_webhook_comfortably_within_ceiling_still_succeeds() {
    let (_pg, mut server) = start_correctly_configured_server().await;
    let client = reqwest::Client::new();
    let url = webhook_url(&server);

    let event_id = format!("evt_wbl_{}", uuid::Uuid::new_v4());
    let filler = "x".repeat(1024 * 1024); // 1 MiB — comfortably under the 5 MiB ceiling
    let payload = serde_json::json!({
        "id": event_id,
        "type": "customer.subscription.updated",
        "data": {
            "object": {
                "id": "sub_wbl_test_large_but_legitimate",
                "status": "active",
                "current_period_end": 1_799_999_999i64,
                "line_items_padding": filler
            }
        }
    })
    .to_string();
    let signature = sign_stripe_payload(&payload, WEBHOOK_SECRET);

    let resp = client
        .post(&url)
        .header("Stripe-Signature", signature)
        .header("Content-Type", "application/json")
        .body(payload)
        .send()
        .await
        .expect("webhook POST failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "a correctly-signed webhook comfortably within the 5 MiB ceiling \
         must succeed — got {}",
        resp.status()
    );

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(10)).await;
}

// ─── AC-WBL-01/04 composition: oversized+unsigned never reaches DB ─────────

/// An oversized request with an invalid `Stripe-Signature` value is rejected
/// before signature verification is even attempted (413, not 401 — 401
/// would mean `verify_webhook_signature` ran and failed) and with zero
/// writes to `processed_webhook_events` — mirrors this middleware's own
/// pre-existing AC-203-02 zero-DB-write-on-rejection guarantee.
///
/// Journey (error path):
///   Given: no valid Stripe-Signature header VALUE is presented (garbage,
///          but present, so the request reaches the buffering step)
///   When:  an oversized body is sent to POST /admin/v1/webhooks/stripe
///   Then:  the request is rejected with 413 — never reaching
///          `verify_webhook_signature` (which would produce 401 instead)
///   And:   no tenant's subscription state is modified — zero rows in
///          `processed_webhook_events`
///
/// Today's actual behavior: the full body is buffered, `verify_webhook_signature`
/// runs and fails on the garbage signature → 401, not 413 — this test fails
/// today on the status-code assertion (right reason: size-limit
/// discrimination not yet implemented), not a fixture bug.
///
/// @error @US-01 @AC-WBL-01 @AC-WBL-04 @real-io @wiring_e2e
#[tokio::test]
async fn oversized_unsigned_request_never_reaches_signature_verification_or_db() {
    let (pg, mut server) = start_correctly_configured_server().await;
    let db_url = format!(
        "postgres://postgres:postgres@127.0.0.1:{}/postgres",
        pg.get_host_port_ipv4(5432).await.expect("pg host port")
    );
    let pool = sqlx::PgPool::connect(&db_url)
        .await
        .expect("connect to same Postgres container the server migrated");

    let client = reqwest::Client::new();
    let url = webhook_url(&server);
    let garbage_signature = "t=0,v1=deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdead";
    let oversized_body = vec![b'a'; CEILING_BYTES * 10]; // 50 MiB

    let resp = client
        .post(&url)
        .header("Stripe-Signature", garbage_signature)
        .body(oversized_body)
        .send()
        .await
        .expect("oversized POST must complete");

    assert_eq!(
        resp.status().as_u16(),
        413,
        "an oversized request with an invalid signature value must be \
         rejected with 413 BEFORE signature verification is attempted \
         (401 would mean verify_webhook_signature ran and failed instead) \
         — got {}",
        resp.status()
    );

    let processed_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM processed_webhook_events")
            .fetch_one(&pool)
            .await
            .expect("query processed_webhook_events");
    assert_eq!(
        processed_count, 0,
        "a rejected oversized request must never reach stripe_webhook_handler \
         — zero writes to processed_webhook_events, mirroring AC-203-02's \
         own zero-DB-write-on-rejection guarantee"
    );

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(10)).await;
}
