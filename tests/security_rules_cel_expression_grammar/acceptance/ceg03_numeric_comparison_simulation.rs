//! CEG03 (Slice 03, US-03, Release 1) — Alex Simulates a Numeric-Bound
//! Candidate Rule Before Publishing It.
//!
//! Acceptance criteria verified here (feature-delta.md US-03, ADR-065):
//!   AC-CEG-08: `simulate_access_rule` evaluates a numeric-comparison
//!              candidate condition against a synthetic resource via the
//!              IDENTICAL `evaluate()` routine real enforcement uses.
//!
//! Driving port: Admin HTTP :9090 only (`simulate_access_rule`) — pure
//! computation, no stored pattern involved.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesAdminContext;

/// Journey:
///   Given: Alex holds a candidate `resource.data.photo_count <= 20`
///          condition
///   When:  he simulates it against a synthetic resource whose
///          `photo_count` is 15, and separately one whose `photo_count` is
///          25
///   Then:  the first simulates as "allow", the second as "deny"
///
/// AC-CEG-08
///
/// @driving_port @real-io @US-03 @AC-CEG-08
#[tokio::test]
async fn simulating_a_numeric_bound_candidate_returns_the_correct_outcome_in_both_directions() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let within_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "resource.data.photo_count <= 20",
            "resource": {"photo_count": 15},
        }))
        .send()
        .await
        .expect("simulate_access_rule request failed");
    assert_eq!(within_resp.status().as_u16(), 200);
    let within_body: serde_json::Value = within_resp.json().await.expect("response body must be JSON");
    assert_eq!(
        within_body["outcome"], "allow",
        "AC-CEG-08: photo_count=15 satisfies '<= 20' and must simulate as 'allow'"
    );

    let over_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "resource.data.photo_count <= 20",
            "resource": {"photo_count": 25},
        }))
        .send()
        .await
        .expect("simulate_access_rule request failed");
    assert_eq!(over_resp.status().as_u16(), 200);
    let over_body: serde_json::Value = over_resp.json().await.expect("response body must be JSON");
    assert_eq!(
        over_body["outcome"], "deny",
        "AC-CEG-08: photo_count=25 violates '<= 20' and must simulate as 'deny'"
    );
}
