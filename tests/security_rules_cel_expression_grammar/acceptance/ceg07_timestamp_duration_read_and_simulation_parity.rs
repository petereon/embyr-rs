//! CEG07 (Slice 07, US-07, Release 3, LAST slice) — The Same Time-Window
//! Grammar Gates Every Surface and Is Simulatable.
//!
//! Acceptance criteria verified here (feature-delta.md US-07, ADR-065):
//!   AC-CEG-20: a real `CreateDocument`/`UpdateDocument` call is gated
//!              correctly by a timestamp/duration condition (Slice 06
//!              already covers Create; this closes read-path parity).
//!   AC-CEG-21: `simulate_access_rule` evaluates a timestamp/duration
//!              candidate condition via the identical `evaluate()`
//!              routine, accepting a caller-supplied synthetic "now" value.
//!
//! Driving ports: Admin HTTP :9090 (`define_access_rule`, `simulate_access_
//! rule`) + gRPC :8080 `GetDocument` (`SecurityRulesFullContext` /
//! `SecurityRulesAdminContext`).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{SecurityRulesAdminContext, SecurityRulesFullContext};

fn resource_name(project_id: &str, path: &str) -> String {
    format!("projects/{project_id}/databases/(default)/documents/{path}")
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-CEG-20: a time-window clause gates a real GetDocument call correctly.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex imports `resource.data.submitted_at + duration.value(24,
///          'h') > request.time` on `journal_entries`
///   When:  Maria reads an entry whose `submitted_at` is 1 hour ago (still
///          fresh), and separately one whose `submitted_at` is 48 hours ago
///          (stale)
///   Then:  the first read succeeds, the second is denied
///
/// AC-CEG-20
///
/// @driving_port @real-io @US-07 @AC-CEG-20
#[tokio::test]
async fn a_time_window_clause_gates_a_real_read_in_both_directions() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ceg07-timewindowread").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let define_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": "resource.data.submitted_at + duration.value(24, 'h') > request.time",
        }))
        .send()
        .await
        .expect("define_access_rule request failed");
    assert_eq!(
        define_resp.status().as_u16(),
        200,
        "AC-CEG-20: a time-window read condition must be accepted"
    );

    let now = chrono::Utc::now().timestamp();
    ctx.seed_document(
        "journal_entries",
        "fresh-entry",
        serde_json::json!({ "submitted_at": {"t": "TS", "s": now - 3600, "n": 0} }),
    )
    .await;
    ctx.seed_document(
        "journal_entries",
        "stale-entry",
        serde_json::json!({ "submitted_at": {"t": "TS", "s": now - (48 * 3600), "n": 0} }),
    )
    .await;

    let fresh = ctx
        .get_document(
            &resource_name(&ctx.project_id, "journal_entries/fresh-entry"),
            None,
        )
        .await;
    assert!(
        fresh.is_ok(),
        "AC-CEG-20: submitted 1 hour ago is still within the 24h window and must be allowed: {:?}",
        fresh.err()
    );

    let stale = ctx
        .get_document(
            &resource_name(&ctx.project_id, "journal_entries/stale-entry"),
            None,
        )
        .await;
    let err = stale.expect_err("AC-CEG-20: submitted 48 hours ago is outside the 24h window and must be denied");
    assert_eq!(err.code(), tonic::Code::PermissionDenied, "got {:?}", err.code());
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-CEG-21: simulate_access_rule shares evaluate() for a timestamp/
// duration candidate, accepting a synthetic "now".
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds a candidate `resource.data.submitted_at +
///          duration.value(24, 'h') > request.time` condition and a
///          synthetic "now"
///   When:  he simulates it against a synthetic resource whose
///          `submitted_at` is 1 hour before that synthetic "now", and
///          separately one 48 hours before it
///   Then:  the first simulates as "allow", the second as "deny"
///
/// AC-CEG-21
///
/// @driving_port @real-io @US-07 @AC-CEG-21
#[tokio::test]
async fn simulating_a_time_window_candidate_returns_the_correct_outcome_in_both_directions() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let synthetic_now = 1_800_000_000_i64;

    let fresh_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "resource.data.submitted_at + duration.value(24, 'h') > request.time",
            "resource": {"submitted_at": {"t": "TS", "s": synthetic_now - 3600, "n": 0}},
            "request_time": synthetic_now,
        }))
        .send()
        .await
        .expect("simulate_access_rule request failed");
    assert_eq!(fresh_resp.status().as_u16(), 200);
    let fresh_body: serde_json::Value = fresh_resp.json().await.expect("response body must be JSON");
    assert_eq!(
        fresh_body["outcome"], "allow",
        "AC-CEG-21: submitted_at 1 hour before the synthetic now is still within the 24h window \
         and must simulate as 'allow'"
    );

    let stale_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "resource.data.submitted_at + duration.value(24, 'h') > request.time",
            "resource": {"submitted_at": {"t": "TS", "s": synthetic_now - (48 * 3600), "n": 0}},
            "request_time": synthetic_now,
        }))
        .send()
        .await
        .expect("simulate_access_rule request failed");
    assert_eq!(stale_resp.status().as_u16(), 200);
    let stale_body: serde_json::Value = stale_resp.json().await.expect("response body must be JSON");
    assert_eq!(
        stale_body["outcome"], "deny",
        "AC-CEG-21: submitted_at 48 hours before the synthetic now is outside the 24h window and \
         must simulate as 'deny'"
    );
}
