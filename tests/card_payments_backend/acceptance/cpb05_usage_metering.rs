//! CPB05 (Slice 05, US-205, Release 2) — Nightly Usage Metering to Stripe.
//!
//! Acceptance criteria verified here (feature-delta.md US-205):
//!   AC-205-01: batch run reads yesterday's daily_project_metrics and pushes
//!              one Stripe usage record per project per non-zero dimension.
//!   AC-205-02: zero-usage project/dimension/day combinations are not pushed.
//!   AC-205-03: re-running for an already-processed day is idempotent.
//!   AC-205-04: a per-project Stripe API failure does not abort remaining projects.
//!   AC-205-05: POST /admin/v1/billing/run-metering is operator-authed only.
//!
//! Real-I/O strategy (D-13): real Stripe test-mode Usage Record API pushes.
//! AC-205-04's per-project-failure-isolation scenario uses a project whose
//! Stripe subscription-item id is deliberately invalid (a genuine Stripe API
//! rejection for that one project), not a simulated failure.
//!
//! All tests #[ignore] — RED, enable one at a time in DELIVER. Depends on
//! Slice 01 (stripe_customer_id must exist to attach usage records to).
//!
//! Error ratio: 2 error/edge (AC-205-05 auth rejection, AC-205-04 per-project
//! isolation) out of 5 total = 40% ✓

#[path = "../common/mod.rs"]
mod common;
use common::CpbTestContext;

const OPERATOR_KEY: &str = "test-admin-key-from-env";

// ─────────────────────────────────────────────────────────────────────────────
// AC-205-01: a day's usage is pushed as one record per project per dimension
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — Given reuses cpb01's provisioning step so a real
/// stripe_customer_id exists to attach usage records to):
///   Given: Aperture Labs' daily_project_metrics for yesterday shows non-zero
///          reads, writes, and deletes for project "prod-orders"
///   When:  an operator triggers POST /admin/v1/billing/run-metering
///   Then:  Stripe usage records are pushed for "prod-orders" — one per
///          non-zero dimension
///
/// AC-205-01
///
/// @driving_port @real-io @US-205 @AC-205-01
#[tokio::test]
async fn a_days_usage_is_pushed_as_one_record_per_project_per_dimension() {
    let ctx = CpbTestContext::new().await;
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
    ctx.insert_project("prod-orders").await;
    ctx.insert_yesterday_metrics("prod-orders", 620_000, 41_000, 900)
        .await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/billing/run-metering"))
        .header("Authorization", format!("Bearer {OPERATOR_KEY}"))
        .send()
        .await
        .expect("POST run-metering failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-205-01: metering run must return 200"
    );
    let body: serde_json::Value = resp.json().await.expect("response JSON");
    assert!(
        body["usage_records_pushed"].as_u64().unwrap_or(0) >= 3,
        "AC-205-01: at least 3 non-zero-dimension records must be pushed for prod-orders; got: {body}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-205-02: zero-usage dimensions are not pushed
// ─────────────────────────────────────────────────────────────────────────────

/// AC-205-02
///
/// @driving_port @real-io @US-205 @AC-205-02
#[tokio::test]
async fn zero_usage_dimensions_are_not_pushed() {
    let ctx = CpbTestContext::new().await;
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
    ctx.insert_project("staging-orders").await;
    // Zero deletes — only reads/writes are non-zero.
    ctx.insert_yesterday_metrics("staging-orders", 100, 50, 0)
        .await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/billing/run-metering"))
        .header("Authorization", format!("Bearer {OPERATOR_KEY}"))
        .send()
        .await
        .expect("POST run-metering failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response JSON");
    assert_eq!(
        body["usage_records_pushed"].as_u64(),
        Some(2),
        "AC-205-02: exactly 2 records (reads, writes) must be pushed — deletes is zero; got: {body}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-205-03: re-running the same day's metering does not double-count
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — Given reuses the first scenario's Given+When: a
/// metering run has already pushed records for the day):
///   Given: yesterday's metering run already pushed records for prod-orders
///   When:  an operator re-triggers POST /admin/v1/billing/run-metering
///   Then:  no dimension's usage record is pushed a second time
///
/// AC-205-03
///
/// @driving_port @real-io @US-205 @AC-205-03
#[tokio::test]
async fn rerunning_the_same_days_metering_does_not_double_count() {
    let ctx = CpbTestContext::new().await;
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
    ctx.insert_project("prod-orders").await;
    ctx.insert_yesterday_metrics("prod-orders", 1_000, 500, 10)
        .await;

    let first = ctx
        .client
        .post(ctx.url("/admin/v1/billing/run-metering"))
        .header("Authorization", format!("Bearer {OPERATOR_KEY}"))
        .send()
        .await
        .expect("first run failed");
    assert_eq!(first.status().as_u16(), 200);

    let second = ctx
        .client
        .post(ctx.url("/admin/v1/billing/run-metering"))
        .header("Authorization", format!("Bearer {OPERATOR_KEY}"))
        .send()
        .await
        .expect("second run failed");
    assert_eq!(second.status().as_u16(), 200);
    let second_body: serde_json::Value = second.json().await.expect("second response JSON");
    assert_eq!(
        second_body["usage_records_pushed"].as_u64(),
        Some(0),
        "AC-205-03: re-running for an already-processed day must push zero new records; got: {second_body}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-205-04: a single project's Stripe failure does not abort the whole run
// ─────────────────────────────────────────────────────────────────────────────

/// AC-205-04
///
/// ESCALATION (step 02-01, DELIVER): re-`#[ignore]`d after real-infra
/// investigation. All 3 projects here share ONE `self.account_id`, hence one
/// Stripe customer (confirmed: `insert_project` binds `self.account_id`
/// unconditionally). US-205's only viable Stripe usage-recording primitive
/// given the Free-plan-only fixtures across ALL cpb05 scenarios (no test
/// ever calls `POST /billing/subscription` to create a real Stripe
/// Subscription) is Billing Meter Events, which key ONLY on
/// `stripe_customer_id` — not on any project-level identifier (`projects`
/// has no Stripe-side column). With identical customer + identical payload
/// shape across "prod-good-1"/"prod-bad"/"prod-good-2", there is no
/// fixture-derivable, non-hardcoded signal that would make Stripe accept 2
/// of 3 structurally-identical real API calls and reject the 3rd — empirically
/// confirmed via a real run: `{"projects_failed":0,"projects_processed":3,
/// "usage_records_pushed":6}`. Making "prod-bad" fail would require either
/// (a) hardcoding that literal project id in production code (Testing
/// Theater / Iron Rule violation) or (b) a fixture change (DISTILL owns AT
/// authorship, crafter does not re-author). Per-project isolation itself
/// (`run_metering`'s per-project `Result` handling — a `StripeError` for one
/// project increments `projects_failed` and the loop continues rather than
/// aborting) IS implemented and code-reviewable in
/// `billing_metering::run_metering`; only this scenario's fixture cannot
/// exercise it with real Stripe I/O as currently written.
///
/// @error @driving_port @real-io @US-205 @AC-205-04
#[tokio::test]
#[ignore] // ESCALATED — see doc comment above (step 02-01)
async fn a_single_projects_stripe_failure_does_not_abort_the_whole_run() {
    let ctx = CpbTestContext::new().await;
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
    ctx.insert_project("prod-good-1").await;
    ctx.insert_project("prod-bad").await;
    ctx.insert_project("prod-good-2").await;
    ctx.insert_yesterday_metrics("prod-good-1", 100, 100, 0)
        .await;
    ctx.insert_yesterday_metrics("prod-bad", 100, 100, 0).await;
    ctx.insert_yesterday_metrics("prod-good-2", 100, 100, 0)
        .await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/billing/run-metering"))
        .header("Authorization", format!("Bearer {OPERATOR_KEY}"))
        .send()
        .await
        .expect("POST run-metering failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-205-04: a per-project Stripe failure must not fail the overall run response"
    );
    let body: serde_json::Value = resp.json().await.expect("response JSON");
    assert!(
        body["projects_processed"].as_u64().unwrap_or(0) >= 2,
        "AC-205-04: the two healthy projects must still be processed; got: {body}"
    );
    assert!(
        body["projects_failed"].as_u64().unwrap_or(0) >= 1,
        "AC-205-04: the failing project must be reported, not silently dropped; got: {body}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-205-05: manual trigger is operator-only (error)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-205-05
///
/// @error @driving_port @real-io @US-205 @AC-205-05
#[tokio::test]
async fn manual_trigger_is_operator_only() {
    let ctx = CpbTestContext::new().await;
    let cookie = ctx
        .seed_session("chris@aperturelabs.example", "Owner")
        .await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/billing/run-metering"))
        .header("Cookie", &cookie) // session cookie, not operator Bearer
        .send()
        .await
        .expect("POST run-metering failed");

    let status = resp.status().as_u16();
    assert!(
        status == 401 || status == 403,
        "AC-205-05: a non-operator caller must be rejected (401/403); got {status}"
    );
}
