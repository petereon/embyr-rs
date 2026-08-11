//! CPB02 (Slice 02, US-202, Release 1) — Plan Change Persists to a Real Stripe Subscription.
//!
//! Acceptance criteria verified here (feature-delta.md US-202):
//!   AC-202-01: POST is session-authed, Owner/Admin role only (check_rbac).
//!   AC-202-02: Valid plan change calls Stripe's Subscription update API
//!              before any local write.
//!   AC-202-03: Local subscriptions.plan updates only after Stripe confirms.
//!   AC-202-04: Upgrading while status='free_cap_exceeded' clears status and
//!              reactivates the account's projects.
//!   AC-202-05: plan values other than "free"/"pro" return 422 (D-4).
//!
//! All tests #[ignore] — RED, enable one at a time in DELIVER. This slice
//! depends on Slice 01 (a real stripe_customer_id must exist first) — every
//! scenario's Given chains from cpb01's walking-skeleton provisioning step
//! (Pillar 2: Given of N reuses Given+When of N-1).
//!
//! Error ratio: 2 error/edge (AC-202-05 invalid plan, role-gating 403) out of
//! 5 total = 40% ✓

#[path = "../common/mod.rs"]
mod common;
use common::CpbTestContext;

/// Provisions a real Stripe customer + Free subscription first (chains from
/// cpb01's walking skeleton Given+When), returning the Owner session cookie.
async fn given_provisioned_account(ctx: &CpbTestContext, email: &str, role: &str) -> String {
    let cookie = ctx.seed_session(email, role).await;
    let _ = ctx
        .client
        .get(ctx.url("/admin/v1/billing/subscription"))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("provisioning GET failed");
    cookie
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-202-02/03: Upgrading Free to Pro updates the real Stripe subscription
// ─────────────────────────────────────────────────────────────────────────────

/// AC-202-02, AC-202-03
///
/// @driving_port @real-io @US-202 @AC-202-02 @AC-202-03
#[tokio::test]
async fn upgrading_free_to_pro_updates_the_real_stripe_subscription() {
    let ctx = CpbTestContext::new().await;
    let cookie = given_provisioned_account(&ctx, "chris@aperturelabs.example", "Owner").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/billing/subscription"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"plan": "pro"}))
        .send()
        .await
        .expect("POST failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-202-02: valid plan change must return 200"
    );
    let body: serde_json::Value = resp.json().await.expect("response JSON");
    assert_eq!(
        body["plan"].as_str(),
        Some("pro"),
        "AC-202-03: response reflects the new plan"
    );

    let (plan, _, stripe_subscription_id) = ctx
        .subscription_row()
        .await
        .expect("subscriptions row must exist");
    assert_eq!(
        plan, "pro",
        "AC-202-03: local subscriptions.plan updated after Stripe confirms"
    );
    assert!(
        stripe_subscription_id
            .as_deref()
            .is_some_and(|id| id.starts_with("sub_")),
        "AC-202-02: a real Stripe Subscription id must be persisted"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-202-03: A failed Stripe call does not update the local plan (error)
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — Given reuses cpb01's provisioning Given+When, but with
/// an invalid Stripe key so the update call genuinely fails):
///   Given: Aperture Labs is on the Free plan and Stripe's API call fails
///   When:  Chris calls POST /admin/v1/billing/subscription {"plan":"pro"}
///   Then:  the response is an error, local subscriptions.plan still shows free
///
/// AC-202-03
///
/// @error @driving_port @real-io @US-202 @AC-202-03
#[tokio::test]
async fn failed_stripe_call_does_not_update_local_plan() {
    let ctx = CpbTestContext::new_with_invalid_stripe_key().await;
    let cookie = given_provisioned_account(&ctx, "chris@aperturelabs.example", "Owner").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/billing/subscription"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"plan": "pro"}))
        .send()
        .await
        .expect("POST failed");

    assert!(
        resp.status().is_client_error() || resp.status().is_server_error(),
        "AC-202-03: a failed Stripe call must not report success; got {}",
        resp.status()
    );
    if let Some((plan, _, _)) = ctx.subscription_row().await {
        assert_eq!(
            plan, "free",
            "AC-202-03: no optimistic update on Stripe failure"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-202-04: Upgrading from a cap-exceeded state clears the suspension
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — Given extends cpb01's provisioning with a direct DB
/// seed representing "already cap-exceeded", mirroring US-207's end state):
///   Given: Solstice Analytics' subscriptions.status is free_cap_exceeded,
///          with 2 suspended projects
///   When:  Priya calls POST /admin/v1/billing/subscription {"plan":"pro"}
///   Then:  the response shows "status":"active" and both projects are
///          reactivated via the same activate path US-207 also uses
///
/// AC-202-04
///
/// @driving_port @real-io @US-202 @AC-202-04
#[tokio::test]
async fn upgrading_from_cap_exceeded_clears_suspension_and_reactivates_projects() {
    let ctx = CpbTestContext::new().await;
    let cookie = given_provisioned_account(&ctx, "priya@solsticeanalytics.example", "Owner").await;

    sqlx::query("UPDATE subscriptions SET status = 'free_cap_exceeded' WHERE account_id = $1")
        .bind(ctx.account_id)
        .execute(&ctx.pool)
        .await
        .expect("seed free_cap_exceeded status");
    ctx.insert_project("solstice-proj-1").await;
    ctx.insert_project("solstice-proj-2").await;
    sqlx::query("UPDATE projects SET status = 'suspended' WHERE account_id = $1")
        .bind(ctx.account_id)
        .execute(&ctx.pool)
        .await
        .expect("suspend seeded projects");

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/billing/subscription"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"plan": "pro"}))
        .send()
        .await
        .expect("POST failed");
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response JSON");
    assert_eq!(
        body["status"].as_str(),
        Some("active"),
        "AC-202-04: status must clear to active on upgrade"
    );

    for project_id in ["solstice-proj-1", "solstice-proj-2"] {
        assert_eq!(
            ctx.project_status(project_id).await.as_deref(),
            Some("active"),
            "AC-202-04: project '{project_id}' must be reactivated on upgrade"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-202-01: role gating (error)
// ─────────────────────────────────────────────────────────────────────────────

/// Only Owner/Admin roles may change the plan.
///
/// AC-202-01
///
/// @error @driving_port @real-io @US-202 @AC-202-01
#[tokio::test]
async fn viewer_role_cannot_change_plan() {
    let ctx = CpbTestContext::new().await;
    let cookie = given_provisioned_account(&ctx, "viewer@aperturelabs.example", "Viewer").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/billing/subscription"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"plan": "pro"}))
        .send()
        .await
        .expect("POST failed");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "AC-202-01: Viewer role must be rejected with 403 (check_rbac)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-202-05: unrecognized plan value rejected (error)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-202-05 (D-4: only "free"/"pro" are valid)
///
/// @error @driving_port @real-io @US-202 @AC-202-05
#[tokio::test]
async fn unrecognized_plan_value_is_rejected() {
    let ctx = CpbTestContext::new().await;
    let cookie = given_provisioned_account(&ctx, "chris@aperturelabs.example", "Owner").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/billing/subscription"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"plan": "enterprise"}))
        .send()
        .await
        .expect("POST failed");

    assert_eq!(
        resp.status().as_u16(),
        422,
        "AC-202-05: only 'free'/'pro' are valid plan values (D-4)"
    );
}
