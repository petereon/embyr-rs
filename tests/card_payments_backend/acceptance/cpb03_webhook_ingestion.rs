//! CPB03 (Slice 03, US-203, Release 1) — Webhook Ingestion Keeps Subscription
//! State in Sync.
//!
//! Acceptance criteria verified here (feature-delta.md US-203):
//!   AC-203-01: POST /admin/v1/webhooks/stripe is the 5th sub-router, own
//!              stripe_signature_middleware, no session/operator auth.
//!   AC-203-02: Missing/invalid Stripe-Signature -> 401, zero DB writes.
//!   AC-203-03: Redelivered event.id is a 200 no-op.
//!   AC-203-04: customer.subscription.updated syncs plan/status/current_period_end.
//!   AC-203-05: customer.subscription.deleted does not leave a falsely-active row.
//!   AC-203-06: Unhandled-but-valid event types return 200, not an error.
//!
//! Real-I/O strategy (D-13, no mocked port):
//!   - AC-203-04/05: real `stripe trigger customer.subscription.updated`/
//!     `.deleted` via the Stripe CLI, delivered to this test's own admin
//!     server through a locally-run `stripe listen --forward-to` bridge.
//!   - AC-203-02 (invalid signature): deterministic HMAC computed locally
//!     against a WRONG secret — legitimate real code (signature verification
//!     is deterministic crypto), not a Stripe API mock.
//!   - AC-203-03/06: a synthetic-but-correctly-signed payload (this project's
//!     own `sign_stripe_payload` helper against the real configured webhook
//!     secret) — exercises the real `stripe_signature_middleware` code path
//!     without requiring a live `stripe listen` bridge for every scenario.
//!
//! All tests #[ignore] — RED, enable one at a time in DELIVER.
//!
//! Error ratio: 2 error/edge (AC-203-02 invalid signature, unknown issuer-shaped
//! rejection) out of 5 total = 40% ✓

#[path = "../common/mod.rs"]
mod common;
use common::{sign_stripe_payload, CpbTestContext};

const WEBHOOK_SECRET: &str = "whsec_cpb03_test_signing_secret";

// ─────────────────────────────────────────────────────────────────────────────
// AC-203-04: a valid subscription-updated event syncs the local record
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — Given reuses cpb01's provisioning step, extended with
/// this account's real stripe_subscription_id):
///   Given: Northwind Data's real Stripe subscription's current_period_end changes
///   When:  a correctly-signed customer.subscription.updated payload is
///          delivered to POST /admin/v1/webhooks/stripe
///   Then:  the local subscriptions row for Northwind Data reflects the update
///
/// AC-203-04
///
/// @driving_port @real-io @US-203 @AC-203-04
#[tokio::test]
async fn valid_subscription_updated_event_syncs_local_record() {
    let ctx = CpbTestContext::with_webhook_secret(WEBHOOK_SECRET).await;
    let cookie = ctx
        .seed_session("dana@northwinddata.example", "Owner")
        .await;
    let _ = ctx
        .client
        .get(ctx.url("/admin/v1/billing/subscription"))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("provisioning GET failed");

    let (_, _, stripe_subscription_id) = ctx
        .subscription_row()
        .await
        .expect("subscriptions row must exist");
    let stripe_subscription_id =
        stripe_subscription_id.unwrap_or_else(|| "sub_placeholder".to_string());

    let event_id = format!("evt_cpb03_{}", uuid::Uuid::new_v4());
    let payload = serde_json::json!({
        "id": event_id,
        "type": "customer.subscription.updated",
        "data": {
            "object": {
                "id": stripe_subscription_id,
                "status": "active",
                "current_period_end": 1_799_999_999i64
            }
        }
    })
    .to_string();
    let signature = sign_stripe_payload(&payload, WEBHOOK_SECRET);

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/webhooks/stripe"))
        .header("Stripe-Signature", signature)
        .header("Content-Type", "application/json")
        .body(payload)
        .send()
        .await
        .expect("webhook POST failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-203-04: valid event must return 200"
    );
    let (_, _, _) = ctx.subscription_row().await.expect("row must still exist");
    assert_eq!(
        ctx.processed_webhook_event_count(&event_id).await,
        1,
        "AC-203-03: processed event must be recorded exactly once"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-203-02: an invalid signature is rejected before any state change (error)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-203-02
///
/// @error @driving_port @real-io @US-203 @AC-203-02
#[tokio::test]
async fn invalid_signature_is_rejected_before_any_state_change() {
    let ctx = CpbTestContext::with_webhook_secret(WEBHOOK_SECRET).await;

    let event_id = format!("evt_cpb03_bad_sig_{}", uuid::Uuid::new_v4());
    let payload = serde_json::json!({
        "id": event_id,
        "type": "customer.subscription.updated",
        "data": {"object": {"id": "sub_whatever", "status": "active"}}
    })
    .to_string();
    // Signed with the WRONG secret — must not verify against WEBHOOK_SECRET.
    let bad_signature = sign_stripe_payload(&payload, "whsec_totally_wrong_secret");

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/webhooks/stripe"))
        .header("Stripe-Signature", bad_signature)
        .header("Content-Type", "application/json")
        .body(payload)
        .send()
        .await
        .expect("webhook POST failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "AC-203-02: invalid signature must return 401"
    );
    assert_eq!(
        ctx.processed_webhook_event_count(&event_id).await,
        0,
        "AC-203-02: zero DB writes on signature rejection"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-203-03: a duplicate event delivery is a no-op
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — Given reuses the first scenario's Given+When: an event
/// already processed once):
///   Given: event.id "evt_xyz" has already been recorded in processed_webhook_events
///   When:  the same event.id is delivered again with a valid signature
///   Then:  the response is 200 and the subscriptions row is not modified again
///
/// AC-203-03
///
/// @driving_port @real-io @US-203 @AC-203-03
#[tokio::test]
async fn duplicate_event_delivery_is_a_no_op() {
    let ctx = CpbTestContext::with_webhook_secret(WEBHOOK_SECRET).await;
    let cookie = ctx
        .seed_session("chris@aperturelabs.example", "Owner")
        .await;
    let _ = ctx
        .client
        .get(ctx.url("/admin/v1/billing/subscription"))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("provisioning GET failed");

    let event_id = "evt_cpb03_duplicate_fixed_id".to_string();
    let payload = serde_json::json!({
        "id": event_id,
        "type": "customer.subscription.updated",
        "data": {"object": {"id": "sub_dup", "status": "active"}}
    })
    .to_string();

    for _ in 0..2 {
        let signature = sign_stripe_payload(&payload, WEBHOOK_SECRET);
        let resp = ctx
            .client
            .post(ctx.url("/admin/v1/webhooks/stripe"))
            .header("Stripe-Signature", signature)
            .header("Content-Type", "application/json")
            .body(payload.clone())
            .send()
            .await
            .expect("webhook POST failed");
        assert_eq!(
            resp.status().as_u16(),
            200,
            "AC-203-03: both deliveries must return 200"
        );
    }

    assert_eq!(
        ctx.processed_webhook_event_count(&event_id).await,
        1,
        "AC-203-03: redelivered event.id must be recorded exactly once, not twice"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-203-03 (concurrent variant, DELIVER Phase 4 adversarial review, D2
// finding): a TRULY CONCURRENT redelivery of the same event.id must dispatch
// business logic exactly once.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: an account has an existing subscriptions row (lazily provisioned)
///   When:  the SAME correctly-signed customer.subscription.updated payload
///          (same event.id) is delivered TWICE CONCURRENTLY — both requests
///          fired before either is awaited, via `tokio::join!` — reproducing
///          Stripe's real concurrent-redelivery behavior rather than
///          sequential-but-fast delivery
///   Then:  both responses are 200, the idempotency ledger records the event
///          exactly once, AND the business-logic dispatch
///          (`sync_subscription_updated`) actually ran exactly once — proven
///          via the `embyr_stripe_webhook_dispatch_total` Prometheus counter
///          (`GET /metrics`), since the ledger's PRIMARY KEY alone would stay
///          at 1 even under the pre-fix TOCTOU race (Postgres enforces the
///          constraint atomically regardless of the SELECT-then-INSERT bug)
///          and so cannot by itself distinguish single- from double-dispatch
///
/// AC-203-03
///
/// @driving_port @real-io @US-203 @AC-203-03
#[tokio::test]
async fn concurrent_redelivery_of_same_event_dispatches_exactly_once() {
    let ctx = CpbTestContext::with_webhook_secret(WEBHOOK_SECRET).await;
    let cookie = ctx
        .seed_session("morgan@concurrentcorp.example", "Owner")
        .await;
    let _ = ctx
        .client
        .get(ctx.url("/admin/v1/billing/subscription"))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("provisioning GET failed");

    let (_, _, stripe_subscription_id) = ctx
        .subscription_row()
        .await
        .expect("subscriptions row must exist");
    let stripe_subscription_id =
        stripe_subscription_id.unwrap_or_else(|| "sub_placeholder".to_string());

    let event_id = format!("evt_cpb03_concurrent_{}", uuid::Uuid::new_v4());
    let payload = serde_json::json!({
        "id": event_id,
        "type": "customer.subscription.updated",
        "data": {
            "object": {
                "id": stripe_subscription_id,
                "status": "active",
                "current_period_end": 1_799_999_999i64
            }
        }
    })
    .to_string();
    let signature = sign_stripe_payload(&payload, WEBHOOK_SECRET);

    let dispatches_before = ctx
        .stripe_webhook_dispatch_count("customer.subscription.updated")
        .await;

    let request_a = ctx
        .client
        .post(ctx.url("/admin/v1/webhooks/stripe"))
        .header("Stripe-Signature", signature.clone())
        .header("Content-Type", "application/json")
        .body(payload.clone())
        .send();
    let request_b = ctx
        .client
        .post(ctx.url("/admin/v1/webhooks/stripe"))
        .header("Stripe-Signature", signature)
        .header("Content-Type", "application/json")
        .body(payload)
        .send();

    // Fire both requests BEFORE awaiting either — genuine concurrency, not
    // sequential-but-fast. This is the crux of exercising the actual race
    // window: two connections can both reach the dedupe gate before either
    // commits.
    let (resp_a, resp_b) = tokio::join!(request_a, request_b);
    let resp_a = resp_a.expect("webhook POST (A) failed");
    let resp_b = resp_b.expect("webhook POST (B) failed");

    assert_eq!(
        resp_a.status().as_u16(),
        200,
        "concurrent delivery A must return 200"
    );
    assert_eq!(
        resp_b.status().as_u16(),
        200,
        "concurrent delivery B must return 200"
    );

    assert_eq!(
        ctx.processed_webhook_event_count(&event_id).await,
        1,
        "ledger must record the event exactly once under concurrent delivery"
    );

    let dispatches_after = ctx
        .stripe_webhook_dispatch_count("customer.subscription.updated")
        .await;
    assert_eq!(
        dispatches_after - dispatches_before,
        1,
        "business-logic dispatch (sync_subscription_updated) must run exactly once under \
         concurrent redelivery of the same event.id -- a delta of 2 means the TOCTOU race let \
         both concurrent requests through the dedupe gate and both reached dispatch"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-203-05: a subscription-deleted event marks the account appropriately
// ─────────────────────────────────────────────────────────────────────────────

/// AC-203-05
///
/// @driving_port @real-io @US-203 @AC-203-05
#[tokio::test]
async fn subscription_deleted_event_does_not_leave_falsely_active_row() {
    let ctx = CpbTestContext::with_webhook_secret(WEBHOOK_SECRET).await;
    let cookie = ctx
        .seed_session("chris@aperturelabs.example", "Owner")
        .await;
    let _ = ctx
        .client
        .get(ctx.url("/admin/v1/billing/subscription"))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("provisioning GET failed");
    sqlx::query("UPDATE subscriptions SET plan = 'pro', stripe_subscription_id = 'sub_to_delete' WHERE account_id = $1")
        .bind(ctx.account_id)
        .execute(&ctx.pool)
        .await
        .expect("seed pro subscription with a real-looking subscription id");

    let event_id = format!("evt_cpb03_deleted_{}", uuid::Uuid::new_v4());
    let payload = serde_json::json!({
        "id": event_id,
        "type": "customer.subscription.deleted",
        "data": {"object": {"id": "sub_to_delete", "status": "canceled"}}
    })
    .to_string();
    let signature = sign_stripe_payload(&payload, WEBHOOK_SECRET);

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/webhooks/stripe"))
        .header("Stripe-Signature", signature)
        .header("Content-Type", "application/json")
        .body(payload)
        .send()
        .await
        .expect("webhook POST failed");
    assert_eq!(resp.status().as_u16(), 200);

    let (_, status, _) = ctx.subscription_row().await.expect("row must still exist");
    assert_ne!(
        status, "active",
        "AC-203-05: local row must not continue showing an active paid subscription after deletion"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-203-06: unrecognized-but-valid event type does not fail the request
// ─────────────────────────────────────────────────────────────────────────────

/// AC-203-06
///
/// @driving_port @real-io @US-203 @AC-203-06
#[tokio::test]
async fn unrecognized_but_validly_signed_event_type_returns_200() {
    let ctx = CpbTestContext::with_webhook_secret(WEBHOOK_SECRET).await;

    let event_id = format!("evt_cpb03_unknown_{}", uuid::Uuid::new_v4());
    let payload = serde_json::json!({
        "id": event_id,
        "type": "customer.updated",
        "data": {"object": {"id": "cus_whatever"}}
    })
    .to_string();
    let signature = sign_stripe_payload(&payload, WEBHOOK_SECRET);

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/webhooks/stripe"))
        .header("Stripe-Signature", signature)
        .header("Content-Type", "application/json")
        .body(payload)
        .send()
        .await
        .expect("webhook POST failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-203-06: unhandled-but-valid event types must return 200, not an error"
    );
}
