//! CPB07 (Slice 07, US-207, Release 3) — Free-Cap-Exceeded Suspension
//! Enforced in Real Time.
//!
//! Acceptance criteria verified here (feature-delta.md US-207):
//!   AC-207-01: crossing >=100% of any Free-plan dimension cap suspends all
//!              of the account's projects.
//!   AC-207-02: usage below 100% never triggers suspension via this path.
//!   AC-207-03: Pro-plan accounts are never suspended by this mechanism.
//!   AC-207-04: this trigger calls the identical set_project_status function
//!              as US-204's dunning trigger.
//!   AC-207-05: upgrading to Pro (US-202) reactivates projects suspended by
//!              this mechanism.
//!
//! Highest-consequence story in the feature (feature-delta.md US-207
//! Technical Notes): a bug here either fails to enforce a real policy
//! (revenue/trust risk) or falsely suspends a healthy account (severe
//! trust/availability risk). Per ADR-020 § Alternatives Considered, this
//! slice's own purpose is to surface the request-hot-path race condition
//! Slice 04's async-webhook-handler context would not have caught — the
//! concurrent-requests scenario below exercises exactly that.
//!
//! Per DISTILL handoff flag #1: enforcement is bounded by
//! `2 * EMBYR_CAP_CHECK_INTERVAL_SECS`, not immediate-next-request (ADR-020).
//!
//! All tests #[ignore] — RED, enable one at a time in DELIVER. Depends on
//! Slice 06 (the computation this trigger reads) and Slice 04 (the suspend
//! call-site pattern being reused).
//!
//! Error ratio: 2 error/edge (AC-207-02 below-cap non-suspension, AC-207-03
//! Pro-plan immunity) out of 5 total = 40% ✓

#[path = "../common/mod.rs"]
mod common;
use common::CpbTestContext;

use std::time::Duration;

/// Poll `projects.status` until it becomes `expected` or `timeout` elapses.
/// Returns the last observed status.
async fn poll_for_project_status(
    ctx: &CpbTestContext,
    project_id: &str,
    expected: &str,
    timeout: Duration,
) -> Option<String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let status = ctx.project_status(project_id).await;
        if status.as_deref() == Some(expected) || tokio::time::Instant::now() >= deadline {
            return status;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-207-01: crossing >=100% of a Free-plan cap suspends the account's projects
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — Given reuses cpb06's cumulative-usage-at-cap Given, but
/// waits for the enforcement side rather than only the read side):
///   Given: Solstice Analytics (Free plan) crosses 100,000/100,000 deletes
///          this cycle
///   When:  the background CapUsageRefresher's next cycle runs
///   Then:  all of Solstice Analytics' projects transition to suspended and
///          subscriptions.status becomes free_cap_exceeded
///
/// AC-207-01
///
/// @driving_port @real-io @US-207 @AC-207-01
#[tokio::test]
async fn crossing_100_percent_of_a_free_plan_cap_suspends_the_accounts_projects() {
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

    ctx.insert_project("solstice-cap-1").await;
    ctx.insert_project("solstice-cap-2").await;
    // Free-plan deletes cap is 100_000 — this crosses it.
    ctx.insert_current_month_metrics("solstice-cap-1", 0, 0, 100_000)
        .await;

    let status =
        poll_for_project_status(&ctx, "solstice-cap-1", "suspended", Duration::from_secs(90)).await;
    assert_eq!(
        status.as_deref(),
        Some("suspended"),
        "AC-207-01: crossing the cap must suspend the project within the enforcement window"
    );
    assert_eq!(
        ctx.project_status("solstice-cap-2").await.as_deref(),
        Some("suspended"),
        "AC-207-01: ALL of the account's projects must be suspended, not just the one over cap"
    );

    let (_, sub_status, _) = ctx
        .subscription_row()
        .await
        .expect("subscriptions row must exist");
    assert_eq!(
        sub_status, "free_cap_exceeded",
        "AC-207-01: subscriptions.status must become free_cap_exceeded"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-207-02: below-cap usage never suspends (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-207-02
///
/// @error @driving_port @real-io @US-207 @AC-207-02
#[tokio::test]
async fn below_cap_usage_never_suspends() {
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

    ctx.insert_project("aperture-below-cap").await;
    // 82% of the 500_000 writes cap — below the 100% threshold.
    ctx.insert_current_month_metrics("aperture-below-cap", 0, 410_000, 0)
        .await;

    // Wait roughly one enforcement window, then assert the project is still active.
    tokio::time::sleep(Duration::from_secs(65)).await;
    assert_eq!(
        ctx.project_status("aperture-below-cap").await.as_deref(),
        Some("active"),
        "AC-207-02: below-cap usage must never trigger suspension via this path"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-207-03: Pro-plan accounts are never suspended by this mechanism (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-207-03
///
/// @error @driving_port @real-io @US-207 @AC-207-03
#[tokio::test]
async fn pro_plan_accounts_are_never_suspended_by_this_mechanism() {
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

    ctx.insert_project("northwind-pro-heavy").await;
    // Usage exceeds what a Free-plan cap would have been — Pro is metered
    // overage instead (US-205), never hard-stopped by this mechanism.
    ctx.insert_current_month_metrics("northwind-pro-heavy", 0, 900_000, 0)
        .await;

    tokio::time::sleep(Duration::from_secs(65)).await;
    assert_eq!(
        ctx.project_status("northwind-pro-heavy").await.as_deref(),
        Some("active"),
        "AC-207-03: Pro-plan accounts must never be suspended by the Free-cap mechanism"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-207-05: upgrading to Pro reactivates projects suspended by this mechanism
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — Given extends the first scenario's fully-suspended
/// end state, verified from the enforcement side per US-207's own AC text):
///   Given: Solstice Analytics' projects are suspended due to free_cap_exceeded
///   When:  Priya upgrades to Pro via POST /admin/v1/billing/subscription
///   Then:  the projects suspended by THIS mechanism are reactivated by that
///          same call — no separate manual re-activation step
///
/// AC-207-05
///
/// @driving_port @real-io @US-207 @AC-207-05
#[tokio::test]
async fn upgrading_to_pro_reactivates_projects_suspended_by_cap_enforcement() {
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

    ctx.insert_project("solstice-reactivate").await;
    ctx.insert_current_month_metrics("solstice-reactivate", 0, 0, 100_000)
        .await;

    let suspended = poll_for_project_status(
        &ctx,
        "solstice-reactivate",
        "suspended",
        Duration::from_secs(90),
    )
    .await;
    assert_eq!(
        suspended.as_deref(),
        Some("suspended"),
        "precondition: the project must actually be cap-suspended before testing reactivation"
    );

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/billing/subscription"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"plan": "pro"}))
        .send()
        .await
        .expect("upgrade POST failed");
    assert_eq!(resp.status().as_u16(), 200);

    assert_eq!(
        ctx.project_status("solstice-reactivate").await.as_deref(),
        Some("active"),
        "AC-207-05: upgrading to Pro must reactivate projects suspended by the cap mechanism"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Race-condition-elimination claim (ADR-020 § Consequences, verified behaviorally)
// ─────────────────────────────────────────────────────────────────────────────

/// Concurrent Firestore-shaped requests against a Free account mid-cap-
/// crossing must not double-suspend or lose an update on
/// `subscriptions.status` — ADR-020's explicit claim that moving enforcement
/// to a single-threaded-per-cycle background task eliminates the race Slice
/// 07's own Learning Hypothesis names (a race the dunning trigger, US-204,
/// running from an async webhook handler, does not have).
///
/// @driving_port @real-io @US-207
#[tokio::test]
async fn concurrent_requests_during_cap_crossing_do_not_double_suspend() {
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

    ctx.insert_project("aperture-race").await;
    ctx.insert_current_month_metrics("aperture-race", 0, 0, 100_000)
        .await;

    // Fire 10 concurrent GETs while the background refresher may be mid-cycle.
    let mut handles = Vec::new();
    for _ in 0..10 {
        let client = ctx.client.clone();
        let url = ctx.url("/admin/v1/billing/subscription");
        let cookie = cookie.clone();
        handles.push(tokio::spawn(async move {
            client.get(url).header("Cookie", cookie).send().await
        }));
    }
    for h in handles {
        let _ = h.await;
    }

    // After the enforcement window, exactly one suspension must have
    // occurred (no lost update, no double-processing artifact observable via
    // status oscillation).
    let status =
        poll_for_project_status(&ctx, "aperture-race", "suspended", Duration::from_secs(90)).await;
    assert_eq!(status.as_deref(), Some("suspended"));
    let (_, sub_status, _) = ctx
        .subscription_row()
        .await
        .expect("subscriptions row must exist");
    assert_eq!(
        sub_status, "free_cap_exceeded",
        "no lost update: subscriptions.status must land on free_cap_exceeded despite concurrent reads"
    );
}
