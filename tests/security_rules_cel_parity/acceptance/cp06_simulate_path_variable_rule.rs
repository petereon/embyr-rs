//! CP06 (Slice 06, US-06, Release 2) — Alex Simulates a Path-Variable-Bound
//! Rule Before Importing It.
//!
//! Acceptance criteria verified here (feature-delta.md US-06,
//! slice-06-simulate-path-variable-rule.md):
//!   AC-17-198: simulating a path-variable-bound candidate rule against a
//!              synthetic identity/document-ID pair returns the same
//!              allow/deny outcome real evaluation would produce.
//!   AC-17-199: the simulation request accepts a synthetic document ID as
//!              an explicit input (exercised implicitly by every scenario
//!              below — there is no document to derive it from).
//!   AC-17-200: simulating a rule has zero effect on live/imported traffic.
//!   AC-17-201: simulation supports the anonymous (no synthetic identity)
//!              case, matching real anonymous-evaluation semantics.
//!
//! Extends the EXISTING `simulate_access_rule` handler in place (additive
//! `path_variable` field on the request body) — mirrors
//! `security-rules-write-path`'s own `request_resource` precedent, per this
//! slice's own brief. Not a new endpoint.
//!
//! Driving ports: Admin HTTP :9090 (`SecurityRulesAdminContext`, AC-17-198/
//! 201) + Admin HTTP :9090 AND gRPC :8080 together (`SecurityRulesFullContext`,
//! AC-17-200 — needs a REAL published rule and a REAL getDoc call on the
//! SAME composition root to prove zero cross-effect, mirroring sr05's own
//! `simulation_has_zero_effect_on_live_traffic`).
//!
//! Error ratio: 3 edge (bug-surfacing, zero-live-effect, anonymous) out of
//! 4 = 75%.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, SecurityRulesAdminContext, SecurityRulesFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

/// Mirrors cp02's own helper exactly — `profiles`' document ID IS the
/// owner's plain `uid` (`maria-santos`), never `document_resource_name`'s
/// generic `{project_id}-{doc_suffix}` shape (which only fits
/// `journal_entries`-style ownership-by-field collections).
fn profile_resource_name(project_id: &str, document_id: &str) -> String {
    format!("projects/{project_id}/databases/(default)/documents/profiles/{document_id}")
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-198/199: a matching synthetic identity/document-ID pair simulates
// as "allow", the same outcome real evaluation of an imported
// `request.auth.uid == userId` rule would produce.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds a candidate path-variable-bound condition
///          (`request.auth.uid == userId`) and a synthetic identity/
///          document-ID pair that should satisfy it
///   When:  Alex calls the simulation action with the candidate condition,
///          the synthetic identity, and the synthetic document ID
///   Then:  the response shows "allow"
///
/// AC-17-198, AC-17-199
///
/// @driving_port @real-io @US-06 @AC-17-198 @AC-17-199
#[tokio::test]
async fn simulating_a_path_variable_rule_against_a_matching_pair_returns_allow() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "request.auth.uid == request.path.userId",
            "auth": {"uid": "test-user-001"},
            "path_variable": "test-user-001",
        }))
        .send()
        .await
        .expect("simulate request failed");

    assert_eq!(resp.status().as_u16(), 200, "AC-17-198: a valid simulation request must return 200");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["outcome"], "allow",
        "AC-17-198: a matching identity/synthetic-document-ID pair must simulate as 'allow'"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-198: simulation surfaces a reversed-comparison bug in a
// path-variable-bound rule before importing (error/edge, bug-surfacing).
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-198 (bug-surfacing case)
///
/// @error @driving_port @real-io @US-06 @AC-17-198
#[tokio::test]
async fn simulation_surfaces_a_reversed_comparison_bug_in_a_path_variable_rule() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    // Alex mistakenly wrote `!=` instead of `==` — a MISMATCHED
    // identity/document-ID pair evaluates to "allow" instead of the "deny"
    // Alex expected, catching the bug before it reaches Maria or Dana.
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "request.auth.uid != request.path.userId",
            "auth": {"uid": "test-user-002"},
            "path_variable": "test-user-001",
        }))
        .send()
        .await
        .expect("simulate request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["outcome"], "allow",
        "AC-17-198: the reversed-comparison bug must surface as 'allow' — the discrepancy from \
         Alex's expectation ('deny') IS the bug simulation is meant to catch pre-import"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-200: simulation has zero effect on live/imported traffic
// (error/edge, guardrail).
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-200
///
/// @error @driving_port @real-io @US-06 @AC-17-200
#[tokio::test]
async fn simulating_a_path_variable_rule_has_zero_effect_on_live_traffic() {
    const PROFILES_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /profiles/{userId} {
      allow read, write: if request.auth.uid == userId;
    }
  }
}
"#;

    let ctx = SecurityRulesFullContext::new("trailmark-prod-cp06-liveunaffected").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    // `profiles` has an ACTIVE, IMPORTED rule (real import, Slice 01).
    let import_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules/import",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": PROFILES_RULES_FILE }))
        .send()
        .await
        .expect("import request failed");
    assert_eq!(import_resp.status().as_u16(), 200, "precondition: profiles rule import must succeed");

    ctx.seed_document("profiles", "maria-santos", serde_json::json!({})).await;
    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);

    let before = ctx
        .get_document(&profile_resource_name(&ctx.project_id, "maria-santos"), Some(&marias_token))
        .await;
    assert!(
        before.is_ok(),
        "precondition: Maria's real read must succeed before simulation: {:?}",
        before.err()
    );

    // Alex simulates a COMPLETELY DIFFERENT candidate path-variable rule
    // (unconditional deny) with synthetic data — must never touch
    // `access_rules` or any live document.
    let simulate_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules/simulate",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "false",
            "auth": {"uid": "test-user-999"},
            "path_variable": "someone-else",
        }))
        .send()
        .await
        .expect("simulate request failed");
    assert_eq!(simulate_resp.status().as_u16(), 200);

    let after = ctx
        .get_document(&profile_resource_name(&ctx.project_id, "maria-santos"), Some(&marias_token))
        .await;
    assert!(
        after.is_ok(),
        "AC-17-200: Maria's real read must STILL succeed after the simulation — simulation must \
         have zero effect on live traffic: {:?}",
        after.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-201: simulation supports the anonymous case identically to real
// evaluation, with the new `path_variable` field present (error/edge).
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-201
///
/// @error @driving_port @real-io @US-06 @AC-17-201
#[tokio::test]
async fn simulation_supports_the_anonymous_case_with_path_variable_present() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    // No "auth" field at all — anonymous caller — with `path_variable`
    // present, confirming the new field doesn't disturb the existing
    // `request.auth == null` fail-closed mechanism (US-03).
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "request.auth.uid == request.path.userId",
            "path_variable": "test-user-001",
        }))
        .send()
        .await
        .expect("simulate request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["outcome"], "deny",
        "AC-17-201: simulating with no synthetic identity must match what a real anonymous \
         caller receives (denied), regardless of the synthetic path_variable being present"
    );
}
