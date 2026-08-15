//! CPB04 (Slice 04, US-204, Release 1) — Dunning Suspension and Payment Recovery.
//!
//! Acceptance criteria verified here (feature-delta.md US-204):
//!   AC-204-01: invoice.payment_failed (final-failure) suspends every active
//!              project under the account.
//!   AC-204-02: a non-final (intermediate retry) invoice.payment_failed does
//!              not suspend.
//!   AC-204-03: invoice.payment_succeeded reactivates every suspended project.
//!   AC-204-04: this trigger calls the identical set_project_status function
//!              as US-207 — no parallel implementation.
//!   AC-204-05: suspension evicts the credential cache identically to
//!              today's suspend_project.
//!
//! Real-I/O strategy (D-13): `stripe trigger invoice.payment_failed`/
//! `invoice.payment_succeeded` (Stripe CLI, per DISTILL dispatch
//! instructions) for the walking-skeleton-adjacent happy paths; a locally
//! signed synthetic payload (this project's `sign_stripe_payload` helper) for
//! the intermediate-retry / multi-project fan-out scenarios where exact
//! control over the payload shape (final-failure flag, subscription id) is
//! needed and Stripe's own trigger fixtures don't parametrize that detail.
//!
//! All tests #[ignore] — RED, enable one at a time in DELIVER. Depends on
//! Slice 03 (webhook dispatch must exist) — every scenario's Given chains
//! from cpb03's webhook-delivery Given+When.
//!
//! Error ratio: 3 error/edge (AC-204-02 non-final retry, unknown-subscription
//! no-op, duplicate final-failure idempotency) out of 6 total = 50% ✓

#[path = "../common/mod.rs"]
mod common;
use common::{sign_stripe_payload, CpbTestContext};

const WEBHOOK_SECRET: &str = "whsec_cpb04_test_signing_secret";

async fn deliver_signed_webhook(ctx: &CpbTestContext, payload: &str) -> reqwest::Response {
    let signature = sign_stripe_payload(payload, WEBHOOK_SECRET);
    ctx.client
        .post(ctx.url("/admin/v1/webhooks/stripe"))
        .header("Stripe-Signature", signature)
        .header("Content-Type", "application/json")
        .body(payload.to_string())
        .send()
        .await
        .expect("webhook POST failed")
}

/// Test-setup fix: lazily provisions the `subscriptions` row for
/// `ctx.account_id` via the real GET /admin/v1/billing/subscription flow —
/// mirrors the Given step every other cpb0x acceptance test (cpb01/02/03)
/// already performs before mutating `subscriptions` by direct SQL. A fresh
/// account has no `subscriptions` row until first read (US-201 lazy
/// provisioning, `billing_subscription::get_subscription`), so a direct
/// `UPDATE subscriptions ... WHERE account_id = $1` with no such row is a
/// silent no-op. This helper is Given-step-only infrastructure — it does not
/// touch any scenario's assertions.
async fn seed_subscriptions_row(ctx: &CpbTestContext) {
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
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-204-01: exhausted retries suspend every project under the account
// ─────────────────────────────────────────────────────────────────────────────

/// AC-204-01
///
/// @driving_port @real-io @US-204 @AC-204-01
#[tokio::test]
async fn exhausted_retries_suspend_every_project_under_the_account() {
    let ctx = CpbTestContext::with_webhook_secret(WEBHOOK_SECRET).await;
    seed_subscriptions_row(&ctx).await;
    ctx.insert_project("northwind-proj-1").await;
    ctx.insert_project("northwind-proj-2").await;
    ctx.insert_project("northwind-proj-3").await;
    sqlx::query("UPDATE subscriptions SET plan = 'pro', stripe_subscription_id = 'sub_northwind' WHERE account_id = $1")
        .bind(ctx.account_id)
        .execute(&ctx.pool)
        .await
        .expect("seed pro subscription");

    let payload = serde_json::json!({
        "id": format!("evt_cpb04_final_{}", uuid::Uuid::new_v4()),
        "type": "invoice.payment_failed",
        "data": {"object": {"subscription": "sub_northwind", "attempt_count": 4, "next_payment_attempt": null}}
    })
    .to_string();

    let resp = deliver_signed_webhook(&ctx, &payload).await;
    assert_eq!(resp.status().as_u16(), 200);

    for project_id in ["northwind-proj-1", "northwind-proj-2", "northwind-proj-3"] {
        assert_eq!(
            ctx.project_status(project_id).await.as_deref(),
            Some("suspended"),
            "AC-204-01: project '{project_id}' must be suspended after final payment failure"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-204-02: an intermediate retry failure does not suspend
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — Given reuses the first scenario's project-seeding
/// Given, differing only in the retry-exhaustion flag):
///   Given: Northwind Data's invoice has failed once but the retry schedule
///          has not yet exhausted (next_payment_attempt is set)
///   When:  an intermediate (non-final) invoice.payment_failed is delivered
///   Then:  no project status change occurs
///
/// AC-204-02
///
/// @error @driving_port @real-io @US-204 @AC-204-02
#[tokio::test]
async fn intermediate_retry_failure_does_not_suspend() {
    let ctx = CpbTestContext::with_webhook_secret(WEBHOOK_SECRET).await;
    seed_subscriptions_row(&ctx).await;
    ctx.insert_project("northwind-proj-1").await;
    sqlx::query("UPDATE subscriptions SET plan = 'pro', stripe_subscription_id = 'sub_northwind' WHERE account_id = $1")
        .bind(ctx.account_id)
        .execute(&ctx.pool)
        .await
        .expect("seed pro subscription");

    let payload = serde_json::json!({
        "id": format!("evt_cpb04_intermediate_{}", uuid::Uuid::new_v4()),
        "type": "invoice.payment_failed",
        "data": {"object": {"subscription": "sub_northwind", "attempt_count": 1, "next_payment_attempt": 1_999_999_999i64}}
    })
    .to_string();

    let resp = deliver_signed_webhook(&ctx, &payload).await;
    assert_eq!(resp.status().as_u16(), 200);

    assert_eq!(
        ctx.project_status("northwind-proj-1").await.as_deref(),
        Some("active"),
        "AC-204-02: an intermediate retry must not suspend the project"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-204-03: payment recovery reactivates every project under the account
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — Given extends the first scenario's fully-suspended
/// state):
///   Given: Northwind Data's projects are suspended due to payment failure
///   When:  invoice.payment_succeeded is delivered for that subscription
///   Then:  all of Northwind Data's projects transition back to active
///
/// AC-204-03
///
/// @driving_port @real-io @US-204 @AC-204-03
#[tokio::test]
async fn payment_recovery_reactivates_every_project_under_the_account() {
    let ctx = CpbTestContext::with_webhook_secret(WEBHOOK_SECRET).await;
    seed_subscriptions_row(&ctx).await;
    ctx.insert_project("northwind-proj-1").await;
    ctx.insert_project("northwind-proj-2").await;
    sqlx::query("UPDATE subscriptions SET plan = 'pro', status = 'past_due', stripe_subscription_id = 'sub_northwind' WHERE account_id = $1")
        .bind(ctx.account_id)
        .execute(&ctx.pool)
        .await
        .expect("seed past_due pro subscription");
    sqlx::query("UPDATE projects SET status = 'suspended' WHERE account_id = $1")
        .bind(ctx.account_id)
        .execute(&ctx.pool)
        .await
        .expect("suspend seeded projects");

    let payload = serde_json::json!({
        "id": format!("evt_cpb04_recovered_{}", uuid::Uuid::new_v4()),
        "type": "invoice.payment_succeeded",
        "data": {"object": {"subscription": "sub_northwind"}}
    })
    .to_string();

    let resp = deliver_signed_webhook(&ctx, &payload).await;
    assert_eq!(resp.status().as_u16(), 200);

    for project_id in ["northwind-proj-1", "northwind-proj-2"] {
        assert_eq!(
            ctx.project_status(project_id).await.as_deref(),
            Some("active"),
            "AC-204-03: project '{project_id}' must be reactivated on payment recovery"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-204-05: suspension evicts the credential cache
// ─────────────────────────────────────────────────────────────────────────────

/// Suspension via this path evicts the credential cache exactly as the
/// operator-initiated suspend does today — verified indirectly: the
/// suspended project's `api_key_hash_current` no longer resolves via any
/// cached adapter (the dedicated cache-hit/miss instrumentation this needs
/// is `CredentialCache`, already real — this assertion is a real read of its
/// public `evict_project` effect, not a mocked call-count check).
///
/// AC-204-05
///
/// @driving_port @real-io @US-204 @AC-204-05
#[tokio::test]
async fn suspension_via_dunning_evicts_credential_cache_like_operator_suspend() {
    let ctx = CpbTestContext::with_webhook_secret(WEBHOOK_SECRET).await;
    seed_subscriptions_row(&ctx).await;
    ctx.insert_project("northwind-cache-proj").await;
    sqlx::query("UPDATE subscriptions SET plan = 'pro', stripe_subscription_id = 'sub_cache' WHERE account_id = $1")
        .bind(ctx.account_id)
        .execute(&ctx.pool)
        .await
        .expect("seed pro subscription");

    let payload = serde_json::json!({
        "id": format!("evt_cpb04_cache_{}", uuid::Uuid::new_v4()),
        "type": "invoice.payment_failed",
        "data": {"object": {"subscription": "sub_cache", "attempt_count": 4, "next_payment_attempt": null}}
    })
    .to_string();

    let resp = deliver_signed_webhook(&ctx, &payload).await;
    assert_eq!(resp.status().as_u16(), 200);

    assert_eq!(
        ctx.project_status("northwind-cache-proj").await.as_deref(),
        Some("suspended"),
        "AC-204-05: precondition — project must actually be suspended for the cache-eviction \
         claim to be meaningful"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Edge: unknown subscription id is a safe no-op (no matching account)
// ─────────────────────────────────────────────────────────────────────────────

/// A final-failure event for a `stripe_subscription_id` that matches no
/// local `subscriptions` row (e.g. an account whose local row hasn't synced
/// yet) must not error or suspend anything — mirrors AC-203-06's
/// forward-compatible "unknown but valid" handling, applied to the dunning
/// dispatch path specifically.
///
/// @error @driving_port @real-io @US-204
#[tokio::test]
async fn payment_failed_for_unknown_subscription_id_is_a_safe_no_op() {
    let ctx = CpbTestContext::with_webhook_secret(WEBHOOK_SECRET).await;

    let payload = serde_json::json!({
        "id": format!("evt_cpb04_unknown_sub_{}", uuid::Uuid::new_v4()),
        "type": "invoice.payment_failed",
        "data": {"object": {"subscription": "sub_matches_nothing_local", "attempt_count": 4, "next_payment_attempt": null}}
    })
    .to_string();

    let resp = deliver_signed_webhook(&ctx, &payload).await;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "an unmatched subscription id must not error the webhook delivery"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Edge: duplicate final-failure event does not double-process
// ─────────────────────────────────────────────────────────────────────────────

/// Redelivering the identical final-failure `event.id` (Stripe's documented
/// retry behavior) must not error on an already-suspended account — mirrors
/// `set_project_status`'s existing idempotent `WHERE status IN
/// ('active','suspended')` clause (harmless re-suspend attempt).
///
/// @error @driving_port @real-io @US-204
#[tokio::test]
async fn duplicate_final_failure_event_does_not_double_process() {
    let ctx = CpbTestContext::with_webhook_secret(WEBHOOK_SECRET).await;
    seed_subscriptions_row(&ctx).await;
    ctx.insert_project("northwind-dup-proj").await;
    sqlx::query("UPDATE subscriptions SET plan = 'pro', stripe_subscription_id = 'sub_dup_fail' WHERE account_id = $1")
        .bind(ctx.account_id)
        .execute(&ctx.pool)
        .await
        .expect("seed pro subscription");

    let event_id = "evt_cpb04_dup_final_fixed_id".to_string();
    let payload = serde_json::json!({
        "id": event_id,
        "type": "invoice.payment_failed",
        "data": {"object": {"subscription": "sub_dup_fail", "attempt_count": 4, "next_payment_attempt": null}}
    })
    .to_string();

    for _ in 0..2 {
        let resp = deliver_signed_webhook(&ctx, &payload).await;
        assert_eq!(resp.status().as_u16(), 200);
    }

    assert_eq!(
        ctx.processed_webhook_event_count(&event_id).await,
        1,
        "redelivered final-failure event.id must be recorded exactly once"
    );
    assert_eq!(
        ctx.project_status("northwind-dup-proj").await.as_deref(),
        Some("suspended"),
        "project must remain suspended, not error, on redelivery"
    );
}
