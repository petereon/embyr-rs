//! PM06 (Slice 06, US-06, Release 2) — Alex Simulates a Multi-Segment
//! Pattern Before Importing It.
//!
//! Acceptance criteria verified here (feature-delta.md US-06, ADR-063 §
//! Decision — Admin Surface Extensions / DDD-PM-9,
//! slice-06-simulate-multi-segment-pattern.md):
//!   AC-17-228: simulating a multi-segment candidate pattern against a
//!              synthetic identity/concrete-path pair returns the same
//!              allow/deny outcome and bound variable values real
//!              routing+evaluation would produce.
//!   AC-17-229: the simulation request accepts a synthetic CONCRETE PATH as
//!              an explicit input (not merely a synthetic document ID).
//!   AC-17-230: a synthetic path that does not structurally match the
//!              candidate pattern produces a distinguishable
//!              "no_matching_pattern" outcome, never a false "deny".
//!   AC-17-231: simulating a pattern has zero effect on live/imported
//!              traffic.
//!
//! A NEW, sibling handler/route to `simulate_access_rule`
//! (`POST .../access_rules/simulate_route`) — the response contract
//! genuinely differs (3-state outcome + resolved bindings), mirroring
//! `simulate_group_query_compliance`'s own precedent for a new sibling
//! simulation route (DDD-PM-9).
//!
//! Driving ports: Admin HTTP :9090 only (`SecurityRulesAdminContext`,
//! AC-17-228/229/230 — pure computation, no stored pattern involved) +
//! Admin HTTP :9090 AND gRPC :8080 together (`SecurityRulesFullContext`,
//! AC-17-231 — needs a REAL published pattern and a REAL getDoc call on the
//! SAME composition root to prove zero cross-effect, mirroring cp06's own
//! `simulating_a_path_variable_rule_has_zero_effect_on_live_traffic`).
//!
//! Error ratio: 3 error/edge (deny, no_matching_pattern, zero-live-effect)
//! out of 4 = 75%.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    mint_client_identity_token, now_unix, SecurityRulesAdminContext, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

const CANDIDATE_PATTERN: &str = "expeditions/{expeditionId}/journal_entries/{entryId}";
const CANDIDATE_CONDITION: &str = "request.auth.uid == resource.data.owner_id";

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-228/229: a matching synthetic identity/concrete-path pair simulates
// as "allow", reporting the routing's own resolved bindings.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds a candidate multi-segment pattern and a synthetic
///          identity/concrete-path pair that should satisfy it
///   When:  Alex calls the routed simulation action with the candidate
///          pattern, the synthetic identity, and the synthetic concrete path
///   Then:  the response shows "allow" and the resolved
///          expeditionId/entryId bindings
///
/// AC-17-228, AC-17-229
///
/// @driving_port @real-io @US-06 @AC-17-228 @AC-17-229
#[tokio::test]
async fn simulating_a_matching_synthetic_path_returns_allow_and_resolved_bindings() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate_route"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "pattern": CANDIDATE_PATTERN,
            "condition": CANDIDATE_CONDITION,
            "auth": {"uid": "test-user-001"},
            "resource": {"owner_id": "test-user-001"},
            "concrete_path": "expeditions/test-expedition/journal_entries/test-entry",
        }))
        .send()
        .await
        .expect("simulate_route request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-17-228: a valid routed simulation request must return 200"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["outcome"], "allow",
        "AC-17-228: a matching identity/concrete-path pair must simulate as 'allow'"
    );
    assert_eq!(
        body["bindings"]["expeditionId"], "test-expedition",
        "AC-17-228/229: the routing's own bound expeditionId must be reported"
    );
    assert_eq!(
        body["bindings"]["entryId"], "test-entry",
        "AC-17-228/229: the routing's own bound entryId must be reported"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-228 (edge): a routed-but-unsatisfied identity simulates as "deny",
// distinguishable from a structural non-match (AC-17-230, below).
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-228 (deny case — Domain Example 2)
///
/// @error @driving_port @real-io @US-06 @AC-17-228
#[tokio::test]
async fn simulating_a_non_owner_identity_returns_deny_not_no_matching_pattern() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate_route"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "pattern": CANDIDATE_PATTERN,
            "condition": CANDIDATE_CONDITION,
            "auth": {"uid": "someone-else"},
            "resource": {"owner_id": "test-user-001"},
            "concrete_path": "expeditions/test-expedition/journal_entries/test-entry",
        }))
        .send()
        .await
        .expect("simulate_route request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["outcome"], "deny",
        "AC-17-228: the path structurally routes but the identity fails the condition — 'deny', \
         never 'no_matching_pattern'"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-230: a synthetic path that does not structurally match the
// candidate pattern's own shape returns a distinguishable
// "no_matching_pattern" outcome, never a false "deny".
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds a candidate pattern and a synthetic path of the
///          WRONG collection depth (missing the journal_entries/{entryId}
///          leaf entirely)
///   When:  Alex calls the routed simulation action with that pair
///   Then:  the response shows a distinguishable "no_matching_pattern"
///          outcome, not a false "deny"
///
/// AC-17-230
///
/// @error @driving_port @real-io @US-06 @AC-17-230
#[tokio::test]
async fn simulating_a_structurally_mismatched_path_returns_no_matching_pattern() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate_route"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "pattern": CANDIDATE_PATTERN,
            "condition": CANDIDATE_CONDITION,
            "auth": {"uid": "test-user-001"},
            "resource": {"owner_id": "test-user-001"},
            // Wrong collection depth — a bare `expeditions/{id}` shape, not
            // `expeditions/{id}/journal_entries/{id}` — never structurally
            // matches CANDIDATE_PATTERN's own 4-segment shape.
            "concrete_path": "expeditions/test-expedition",
        }))
        .send()
        .await
        .expect("simulate_route request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["outcome"], "no_matching_pattern",
        "AC-17-230: a structural shape mismatch must be distinguishable from a routed 'deny'"
    );
    assert_eq!(
        body["bindings"].as_object().map(|o| o.is_empty()),
        Some(true),
        "AC-17-230: no bindings are resolved when routing itself never matched"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-231: simulation has zero effect on live/imported traffic.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-231
///
/// @error @driving_port @real-io @US-06 @AC-17-231
#[tokio::test]
async fn simulating_a_routed_pattern_has_zero_effect_on_live_traffic() {
    const EXPEDITIONS_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /expeditions/{expeditionId}/journal_entries/{entryId} {
      allow read, write: if request.auth.uid == resource.data.owner_id;
    }
  }
}
"#;

    let ctx = SecurityRulesFullContext::new("trailmark-prod-pm06-liveunaffected").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    // `expeditions/{expeditionId}/journal_entries/{entryId}` has an ACTIVE,
    // IMPORTED pattern (real import, Slice 01).
    let import_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules/import",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": EXPEDITIONS_RULES_FILE }))
        .send()
        .await
        .expect("import request failed");
    assert_eq!(
        import_resp.status().as_u16(),
        200,
        "precondition: expeditions pattern import must succeed"
    );

    ctx.seed_document(
        "expeditions/trek-2026/journal_entries",
        "entry-042",
        serde_json::json!({ "owner_id": {"t": "S", "v": "maria-santos"} }),
    )
    .await;
    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let resource_name = format!(
        "projects/{}/databases/(default)/documents/expeditions/trek-2026/journal_entries/entry-042",
        ctx.project_id
    );
    let before = ctx.get_document(&resource_name, Some(&marias_token)).await;
    assert!(
        before.is_ok(),
        "precondition: Maria's real routed read must succeed before simulation: {:?}",
        before.err()
    );

    // Alex simulates a COMPLETELY DIFFERENT candidate pattern/data
    // (unconditional deny) — must never touch `access_rule_patterns` or any
    // live document.
    let simulate_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules/simulate_route",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "pattern": "trails/{trailId}/waypoints/{waypointId}",
            "condition": "false",
            "auth": {"uid": "test-user-999"},
            "resource": {},
            "concrete_path": "trails/test-trail/waypoints/test-waypoint",
        }))
        .send()
        .await
        .expect("simulate_route request failed");
    assert_eq!(simulate_resp.status().as_u16(), 200);

    let after = ctx.get_document(&resource_name, Some(&marias_token)).await;
    assert!(
        after.is_ok(),
        "AC-17-231: Maria's real routed read must STILL succeed after the simulation — simulation \
         must have zero effect on live traffic: {:?}",
        after.err()
    );
}
