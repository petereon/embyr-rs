//! CPB06 (Slice 06, US-206, Release 3) — Real-Time Cumulative Usage Is
//! Computed Against the Plan Cap.
//!
//! Acceptance criteria verified here (feature-delta.md US-206):
//!   AC-206-01: cap_status sums usage across ALL of an account's projects,
//!              keyed by account_id.
//!   AC-206-02: usage exactly at the cap reports 100% (boundary-inclusive).
//!   AC-206-03: cap_status is only computed/returned for Free-plan accounts.
//!   AC-206-04: computation unavailability fails open (never fabricates an
//!              over-cap reading).
//!   AC-206-05: usage resets at the billing-cycle boundary, not
//!              account-creation date.
//!
//! Per DISTILL handoff flag #1 (feature-delta.md § DESIGN Handoff Package):
//! AC-207's "next request triggers the cap check" is enforced by the
//! background `CapUsageRefresher` (ADR-020), not the request itself. This
//! slice's scenarios wait up to `2 * EMBYR_CAP_CHECK_INTERVAL_SECS` for the
//! cache to observably reflect a computed `cap_status`, not immediate-next-
//! request. Layer: subprocess/FS acceptance (real Postgres, real background
//! task, real HTTP) — example-only per Mandate 11, no PBT.
//!
//! All tests #[ignore] — RED, enable one at a time in DELIVER. Depends on
//! Slice 01 (subscriptions.plan needed to know which accounts are Free).
//!
//! Error ratio: 2 error/edge (AC-206-03 Pro-plan omission, AC-206-04
//! fail-open on staleness) out of 5 total = 40% ✓

#[path = "../common/mod.rs"]
mod common;
use common::CpbTestContext;

use std::time::Duration;

/// Poll `GET /admin/v1/billing/subscription` until `cap_status` is present
/// (i.e. the background `CapUsageRefresher` has completed at least one
/// cycle), or `timeout` elapses. Returns the last observed response body.
async fn poll_for_cap_status(
    ctx: &CpbTestContext,
    cookie: &str,
    timeout: Duration,
) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let resp = ctx
            .client
            .get(ctx.url("/admin/v1/billing/subscription"))
            .header("Cookie", cookie)
            .send()
            .await
            .expect("GET failed");
        let body: serde_json::Value = resp.json().await.expect("response JSON");
        if body.get("cap_status").is_some() || tokio::time::Instant::now() >= deadline {
            return body;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-206-01: cumulative usage sums across all of an account's projects
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — Given reuses cpb01's provisioning step; extends with
/// multiple projects under the same Free account):
///   Given: Aperture Labs (Free) has 3 projects with combined writes of
///          412,000 this billing cycle
///   When:  Chris calls GET /admin/v1/billing/subscription (after the
///          background refresher has run at least once)
///   Then:  cap_status reports writes at 82%, summed across all 3 projects
///
/// AC-206-01
///
/// @driving_port @real-io @US-206 @AC-206-01
#[tokio::test]
async fn cumulative_usage_sums_across_all_of_an_accounts_projects() {
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

    ctx.insert_project("aperture-1").await;
    ctx.insert_project("aperture-2").await;
    ctx.insert_project("aperture-3").await;
    ctx.insert_current_month_metrics("aperture-1", 0, 200_000, 0)
        .await;
    ctx.insert_current_month_metrics("aperture-2", 0, 150_000, 0)
        .await;
    ctx.insert_current_month_metrics("aperture-3", 0, 62_000, 0)
        .await;

    let body = poll_for_cap_status(&ctx, &cookie, Duration::from_secs(90)).await;
    let entries = body["cap_status"]
        .as_array()
        .expect("AC-206-01: cap_status must be present for a Free-plan account");
    let writes_entry = entries
        .iter()
        .find(|e| e["dimension"].as_str() == Some("writes"))
        .expect("writes entry must be present");
    assert_eq!(
        writes_entry["pct"].as_u64(),
        Some(82),
        "AC-206-01: writes must report 82% summed across all 3 projects; got: {writes_entry}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-206-02: usage at exactly the cap reports 100%, not over/under
// ─────────────────────────────────────────────────────────────────────────────

/// AC-206-02
///
/// @driving_port @real-io @US-206 @AC-206-02
#[tokio::test]
async fn usage_at_exactly_the_cap_reports_100_percent() {
    let ctx = CpbTestContext::new().await;
    let cookie = ctx
        .seed_session("priya@solsticeanalytics.example", "Owner")
        .await;
    let _ = ctx
        .client
        .get(ctx.url("/admin/v1/billing/subscription"))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("provisioning GET failed");

    ctx.insert_project("solstice-1").await;
    // Free-plan deletes cap is 100_000 (embyr_core::admin::billing::free_plan_caps).
    ctx.insert_current_month_metrics("solstice-1", 0, 0, 100_000)
        .await;

    let body = poll_for_cap_status(&ctx, &cookie, Duration::from_secs(90)).await;
    let entries = body["cap_status"]
        .as_array()
        .expect("cap_status must be present");
    let deletes_entry = entries
        .iter()
        .find(|e| e["dimension"].as_str() == Some("deletes"))
        .expect("deletes entry must be present");
    assert_eq!(
        deletes_entry["pct"].as_u64(),
        Some(100),
        "AC-206-02: usage exactly at cap must report exactly 100%, boundary-inclusive; got: {deletes_entry}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-206-03: Pro-plan accounts do not receive cap_status (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-206-03
///
/// @error @driving_port @real-io @US-206 @AC-206-03
#[tokio::test]
async fn pro_plan_accounts_do_not_receive_cap_status() {
    let ctx = CpbTestContext::new().await;
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
    sqlx::query("UPDATE subscriptions SET plan = 'pro' WHERE account_id = $1")
        .bind(ctx.account_id)
        .execute(&ctx.pool)
        .await
        .expect("upgrade to pro directly");

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/billing/subscription"))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("GET failed");
    let body: serde_json::Value = resp.json().await.expect("response JSON");

    assert!(
        body.get("cap_status").is_none()
            || body["cap_status"].as_array().is_some_and(|a| a.is_empty()),
        "AC-206-03: Pro-plan accounts must not receive a populated cap_status; got: {body}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-206-04: a stale or unavailable computation fails open, not closed (error)
// ─────────────────────────────────────────────────────────────────────────────

/// A never-yet-refreshed Free-plan account (before the background
/// `CapUsageRefresher`'s first cycle completes) must never fabricate an
/// over-cap reading — `cap_status` is simply absent.
///
/// AC-206-04
///
/// @error @driving_port @real-io @US-206 @AC-206-04
#[tokio::test]
async fn stale_or_unavailable_computation_fails_open() {
    let ctx = CpbTestContext::new().await;
    let cookie = ctx
        .seed_session("chris@aperturelabs.example", "Owner")
        .await;

    // Immediately after provisioning — before any refresh cycle can plausibly
    // have completed.
    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/billing/subscription"))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("GET failed");
    let body: serde_json::Value = resp.json().await.expect("response JSON");

    if let Some(entries) = body["cap_status"].as_array() {
        for entry in entries {
            let pct = entry["pct"].as_u64().unwrap_or(0);
            assert!(
                pct < 100,
                "AC-206-04: must never fabricate an over-cap reading before real usage exists; got: {entry}"
            );
        }
    }
    // Absence of cap_status entirely is equally acceptable (fail-open).
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-206-05: the cycle boundary resets cumulative usage
// ─────────────────────────────────────────────────────────────────────────────

/// Usage recorded BEFORE the current UTC calendar month (ADR-020 § Billing
/// Cycle Boundary) must not count toward this cycle's cumulative total —
/// only `daily_project_metrics` rows within the current calendar month are
/// summed.
///
/// AC-206-05
///
/// @driving_port @real-io @US-206 @AC-206-05
#[tokio::test]
async fn usage_from_a_prior_billing_cycle_does_not_count_toward_this_cycle() {
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
    ctx.insert_project("aperture-prior-cycle").await;

    // A large write count recorded 2 months ago — must not count toward the
    // current UTC-calendar-month cycle.
    sqlx::query(
        "INSERT INTO daily_project_metrics (project_id, date, write_ops) \
         VALUES ($1, DATE_TRUNC('month', CURRENT_DATE) - INTERVAL '2 months', 490000)",
    )
    .bind("aperture-prior-cycle")
    .execute(&ctx.pool)
    .await
    .expect("insert prior-cycle metrics row");

    let body = poll_for_cap_status(&ctx, &cookie, Duration::from_secs(90)).await;
    if let Some(entries) = body["cap_status"].as_array() {
        let writes_entry = entries
            .iter()
            .find(|e| e["dimension"].as_str() == Some("writes"));
        if let Some(entry) = writes_entry {
            assert_eq!(
                entry["used"].as_u64(),
                Some(0),
                "AC-206-05: usage from a prior billing cycle must not count toward this cycle; got: {entry}"
            );
        }
    }
}
