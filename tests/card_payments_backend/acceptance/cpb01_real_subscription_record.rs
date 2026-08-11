//! CPB01 (Slice 01, US-201, Release 1) — A Real Subscription Record Exists.
//!
//! Walking skeleton for card-payments-backend (DISCUSS/DESIGN Decision 2):
//! narrowly scoped to the riskiest new assumption — a real Stripe SDK network
//! round-trip inside an existing async Axum session-authed handler, no mocked
//! port to fall back on (D-13).
//!
//! Acceptance criteria verified here (feature-delta.md US-201):
//!   AC-201-01: GET /admin/v1/billing/subscription is session-authed, any role.
//!   AC-201-02: First call creates a real Stripe test-mode Customer, persists
//!              accounts.stripe_customer_id.
//!   AC-201-03: Subsequent calls reuse the existing stripe_customer_id — no
//!              duplicate Customer.
//!   AC-201-04: A new subscriptions row is seeded plan='free', status='active'.
//!   AC-201-05: Stripe API failure during provisioning returns 502, no partial state.
//!   AC-201-06: Response shape includes plan, status, stripe_customer_id,
//!              current_period_end, card.
//!
//! Driving port: session-authed HTTP (reqwest::Client against the real Axum
//! admin router, production composition root — Pillar 3).
//! Assertion level: real Postgres (testcontainers) + real Stripe test-mode API
//! (D-13, no mock).
//!
//! Walking skeleton: `first_billing_call_auto_provisions_real_stripe_customer`
//!   — NOT #[ignore]. All other tests: #[ignore], RED — enable one at a time
//!   in DELIVER.
//!
//! Error ratio: 2 error/edge (AC-201-05 unreachable-Stripe, AC-201-01
//! unauthenticated) out of 5 total = 40% ✓

#[path = "../common/mod.rs"]
mod common;
use common::CpbTestContext;

// ─────────────────────────────────────────────────────────────────────────────
// AC-201-02/04/06: Walking skeleton
// ─────────────────────────────────────────────────────────────────────────────

/// First billing call for an account with no `stripe_customer_id` creates a
/// real Stripe test-mode Customer, seeds a `subscriptions` row (plan=free,
/// status=active), and returns that real data.
///
/// AC-201-01, AC-201-02, AC-201-04, AC-201-06
///
/// @walking_skeleton @driving_port @real-io @US-201 @AC-201-02 @AC-201-04 @AC-201-06
#[tokio::test]
async fn first_billing_call_auto_provisions_real_stripe_customer() {
    let ctx = CpbTestContext::new().await;
    let cookie = ctx.seed_session("chris@aperturelabs.example", "Owner").await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/billing/subscription"))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("GET /admin/v1/billing/subscription failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-201-01/02: first billing call must return 200"
    );

    let body: serde_json::Value = resp.json().await.expect("response must be JSON");

    assert_eq!(body["plan"].as_str(), Some("free"), "AC-201-04: new account seeds plan=free");
    assert_eq!(
        body["status"].as_str(),
        Some("active"),
        "AC-201-04: new account seeds status=active"
    );
    let stripe_customer_id = body["stripe_customer_id"]
        .as_str()
        .expect("AC-201-02: response must include a real stripe_customer_id");
    assert!(
        stripe_customer_id.starts_with("cus_"),
        "AC-201-02: stripe_customer_id must be a real Stripe Customer id; got: {stripe_customer_id}"
    );
    for field in ["plan", "status", "stripe_customer_id", "current_period_end", "card"] {
        assert!(
            body.get(field).is_some(),
            "AC-201-06: response must include '{field}'; got: {body}"
        );
    }

    // DB verification: real state was actually persisted, not just returned.
    let persisted_customer_id = ctx
        .account_stripe_customer_id()
        .await
        .expect("AC-201-02: accounts.stripe_customer_id must be persisted");
    assert_eq!(persisted_customer_id, stripe_customer_id);

    let (plan, status, _) = ctx
        .subscription_row()
        .await
        .expect("AC-201-04: subscriptions row must exist after first call");
    assert_eq!(plan, "free");
    assert_eq!(status, "active");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-201-03: idempotent provisioning
// ─────────────────────────────────────────────────────────────────────────────

/// Repeated calls do not create a duplicate Stripe Customer.
///
/// Journey (chained — Given reuses the walking skeleton's Given + When:
/// account has already called the endpoint once):
///   Given: Aperture Labs already has a real stripe_customer_id (from the WS call)
///   When:  Chris calls GET /admin/v1/billing/subscription again
///   Then:  no new Stripe Customer is created; the response's stripe_customer_id is unchanged
///
/// AC-201-03
///
/// @driving_port @real-io @US-201 @AC-201-03
#[tokio::test]
async fn repeated_calls_do_not_create_duplicate_stripe_customer() {
    let ctx = CpbTestContext::new().await;
    let cookie = ctx.seed_session("chris@aperturelabs.example", "Owner").await;

    let first = ctx
        .client
        .get(ctx.url("/admin/v1/billing/subscription"))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("first GET failed");
    let first_body: serde_json::Value = first.json().await.expect("first response JSON");
    let first_customer_id = first_body["stripe_customer_id"]
        .as_str()
        .expect("first call must return stripe_customer_id")
        .to_string();

    let second = ctx
        .client
        .get(ctx.url("/admin/v1/billing/subscription"))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("second GET failed");
    assert_eq!(second.status().as_u16(), 200, "AC-201-03: second call must also return 200");
    let second_body: serde_json::Value = second.json().await.expect("second response JSON");

    assert_eq!(
        second_body["stripe_customer_id"].as_str(),
        Some(first_customer_id.as_str()),
        "AC-201-03: stripe_customer_id must be unchanged across repeated calls"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-201-06: Pro-plan account returns its real period end
// ─────────────────────────────────────────────────────────────────────────────

/// A Pro-plan account (subscriptions row already seeded directly, simulating
/// a prior US-202 upgrade) returns its real `current_period_end`.
///
/// Journey (chained — Given extends the walking skeleton's provisioned
/// account with a direct DB seed representing "already upgraded"):
///   Given: Northwind Data's subscriptions row has plan='pro', current_period_end='2026-09-12'
///   When:  Dana calls GET /admin/v1/billing/subscription
///   Then:  the response includes "plan":"pro" and the matching current_period_end
///
/// AC-201-06
///
/// @driving_port @real-io @US-201 @AC-201-06
#[tokio::test]
async fn pro_plan_account_returns_its_real_period_end() {
    let ctx = CpbTestContext::new().await;
    let cookie = ctx.seed_session("dana@northwinddata.example", "Owner").await;

    // Seed a Pro subscription directly (simulates a prior US-202 upgrade —
    // this scenario is scoped to US-201's read path, not the upgrade flow).
    sqlx::query(
        "INSERT INTO subscriptions (account_id, plan, status, current_period_end) \
         VALUES ($1, 'pro', 'active', '2026-09-12T00:00:00Z')",
    )
    .bind(ctx.account_id)
    .execute(&ctx.pool)
    .await
    .expect("seed pro subscription");

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/billing/subscription"))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("GET failed");
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response JSON");

    assert_eq!(body["plan"].as_str(), Some("pro"), "AC-201-06: plan must be pro");
    assert_eq!(
        body["current_period_end"].as_str(),
        Some("2026-09-12T00:00:00Z"),
        "AC-201-06: current_period_end must reflect the real Pro-plan period end"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-201-05: Stripe unavailability leaves no partial state (error)
// ─────────────────────────────────────────────────────────────────────────────

/// Stripe API unreachable during first-time provisioning: 502, no partial
/// `subscriptions`/`stripe_customer_id` state.
///
/// Journey (error path — Given uses a deliberately invalid Stripe key,
/// simulating "unreachable"):
///   Given: Solstice Analytics has never called the endpoint, and the
///          configured Stripe key is invalid (simulates unreachable API)
///   When:  Priya calls GET /admin/v1/billing/subscription
///   Then:  the response is 502 and no subscriptions row exists afterward
///
/// AC-201-05
///
/// @error @driving_port @real-io @US-201 @AC-201-05
#[tokio::test]
async fn stripe_unavailability_during_first_time_provisioning_leaves_no_partial_state() {
    // Deliberately invalid key — real Stripe API rejects it (401), exercising
    // the same failure family as "unreachable" without needing a network
    // partition. Real I/O: the failure is a genuine Stripe API response, not
    // a simulated/mocked one (D-13).
    let ctx = CpbTestContext::new_with_invalid_stripe_key().await;
    let cookie = ctx.seed_session("priya@solsticeanalytics.example", "Owner").await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/billing/subscription"))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("GET failed");

    assert_eq!(
        resp.status().as_u16(),
        502,
        "AC-201-05: Stripe failure during provisioning must return 502"
    );
    assert!(
        ctx.subscription_row().await.is_none(),
        "AC-201-05: no subscriptions row must exist after a failed provisioning attempt"
    );
    assert!(
        ctx.account_stripe_customer_id().await.is_none(),
        "AC-201-05: no partial stripe_customer_id must be persisted"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-201-01: unauthenticated request rejected (error)
// ─────────────────────────────────────────────────────────────────────────────

/// Only session-authed callers may read subscription data — mirrors the
/// existing `GET /admin/v1/billing` auth shape.
///
/// AC-201-01
///
/// @error @driving_port @real-io @US-201 @AC-201-01
#[tokio::test]
async fn unauthenticated_request_is_rejected() {
    let ctx = CpbTestContext::new().await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/billing/subscription"))
        .send()
        .await
        .expect("GET failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "AC-201-01: no session cookie or admin Bearer key must return 401"
    );
}
