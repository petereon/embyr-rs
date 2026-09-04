//! CEG05 (Slice 05, US-05, Release 2) — The Same Whitelist Grammar Gates
//! Reads and Is Simulatable.
//!
//! Acceptance criteria verified here (feature-delta.md US-05, ADR-065):
//!   AC-CEG-13: `resource.data.<field> in [...]` gates a real `GetDocument`
//!              call correctly.
//!   AC-CEG-14: `simulate_access_rule` evaluates an `in` candidate
//!              condition via the identical `evaluate()` routine.
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
// AC-CEG-13: a whitelist clause gates a real GetDocument call correctly.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex imports `resource.data.status in ["draft", "published",
///          "archived"]` on `journal_entries`
///   When:  Maria reads an entry whose `status` is "published" (in the
///          list), and separately one whose `status` is "deleted" (not in
///          the list)
///   Then:  the first read succeeds, the second is denied
///
/// AC-CEG-13
///
/// @driving_port @real-io @US-05 @AC-CEG-13
#[tokio::test]
async fn a_whitelist_clause_gates_a_real_read_in_both_directions() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ceg05-whitelistread").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let define_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": "resource.data.status in [\"draft\", \"published\", \"archived\"]",
        }))
        .send()
        .await
        .expect("define_access_rule request failed");
    assert_eq!(
        define_resp.status().as_u16(),
        200,
        "AC-CEG-13: a whitelist 'in' condition must be accepted"
    );

    ctx.seed_document(
        "journal_entries",
        "published-entry",
        serde_json::json!({ "status": {"t": "S", "v": "published"} }),
    )
    .await;
    ctx.seed_document(
        "journal_entries",
        "deleted-entry",
        serde_json::json!({ "status": {"t": "S", "v": "deleted"} }),
    )
    .await;

    let member = ctx
        .get_document(
            &resource_name(&ctx.project_id, "journal_entries/published-entry"),
            None,
        )
        .await;
    assert!(
        member.is_ok(),
        "AC-CEG-13: status='published' is a list member and must be allowed: {:?}",
        member.err()
    );

    let non_member = ctx
        .get_document(
            &resource_name(&ctx.project_id, "journal_entries/deleted-entry"),
            None,
        )
        .await;
    let err = non_member.expect_err("AC-CEG-13: status='deleted' is not a list member and must be denied");
    assert_eq!(err.code(), tonic::Code::PermissionDenied, "got {:?}", err.code());
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-CEG-14: simulate_access_rule shares evaluate() for an `in` candidate.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds a candidate `resource.data.status in ["draft",
///          "published", "archived"]` condition
///   When:  he simulates it against a synthetic resource whose `status` is
///          "published", and separately one whose `status` is "deleted"
///   Then:  the first simulates as "allow", the second as "deny"
///
/// AC-CEG-14
///
/// @driving_port @real-io @US-05 @AC-CEG-14
#[tokio::test]
async fn simulating_a_whitelist_candidate_returns_the_correct_outcome_in_both_directions() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let member_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "resource.data.status in [\"draft\", \"published\", \"archived\"]",
            "resource": {"status": "published"},
        }))
        .send()
        .await
        .expect("simulate_access_rule request failed");
    assert_eq!(member_resp.status().as_u16(), 200);
    let member_body: serde_json::Value = member_resp.json().await.expect("response body must be JSON");
    assert_eq!(
        member_body["outcome"], "allow",
        "AC-CEG-14: status='published' is a list member and must simulate as 'allow'"
    );

    let non_member_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "resource.data.status in [\"draft\", \"published\", \"archived\"]",
            "resource": {"status": "deleted"},
        }))
        .send()
        .await
        .expect("simulate_access_rule request failed");
    assert_eq!(non_member_resp.status().as_u16(), 200);
    let non_member_body: serde_json::Value =
        non_member_resp.json().await.expect("response body must be JSON");
    assert_eq!(
        non_member_body["outcome"], "deny",
        "AC-CEG-14: status='deleted' is not a list member and must simulate as 'deny'"
    );
}
