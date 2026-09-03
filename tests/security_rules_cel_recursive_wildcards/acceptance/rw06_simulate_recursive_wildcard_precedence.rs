//! RW06 (Slice 06, US-06, Release 2) — Alex Simulates a Candidate
//! Recursive-Wildcard Pattern's Precedence Outcome Before Importing It.
//!
//! Acceptance criteria verified here (feature-delta.md US-06, ADR-064,
//! slice-06-simulate-recursive-wildcard-precedence.md):
//!   AC-17-259: simulating a candidate recursive-wildcard pattern against a
//!              synthetic identity/path pair with no other reaching pattern
//!              returns the same outcome real routing+evaluation would
//!              produce, attributed to the candidate.
//!   AC-17-260: simulating a candidate against a synthetic path that also
//!              matches an already-stored, more specific pattern returns
//!              the ALREADY-STORED pattern's own outcome, correctly
//!              demonstrating the candidate would be deferred, not
//!              silently applied.
//!   AC-17-261: a synthetic path that does not structurally match the
//!              candidate's own fixed prefix produces a distinguishable
//!              "no matching pattern" outcome, never a false "deny".
//!   AC-17-262: simulating a pattern has zero effect on live/imported
//!              traffic.
//!
//! `simulate_routed_access_rule` (`POST .../access_rules/simulate_route`,
//! introduced security-rules-cel-path-matching Slice 06) is EXTENDED here —
//! not a new route — to also accept a candidate ending in a terminal
//! recursive wildcard, mirroring 4b's own Slice 06 (thin wrapper over the
//! real routing+precedence mechanism, DDD-PM-9: never a second,
//! independently-maintained implementation).
//!
//! Driving ports: Admin HTTP :9090 only (`SecurityRulesAdminContext`,
//! AC-17-259/261 — pure candidate-only simulation, no stored pattern
//! involved) + Admin HTTP :9090 (import) for AC-17-260's already-stored
//! pattern precondition + Admin HTTP :9090 AND gRPC :8080 together
//! (`SecurityRulesFullContext`, AC-17-262 — needs a REAL published pattern
//! and a REAL getDoc call on the SAME composition root to prove zero
//! cross-effect, mirroring pm06's own zero-live-effect precedent exactly).
//!
//! Error ratio: 2 error/edge (deferred-to-stored, no_matching_pattern) out
//! of 4 = 50%.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    mint_client_identity_token, now_unix, SecurityRulesAdminContext, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

async fn import(ctx: &SecurityRulesAdminContext, cookie: &str, rules_file: &str) {
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", cookie)
        .json(&serde_json::json!({ "rules_file": rules_file }))
        .send()
        .await
        .expect("import request failed");
    assert_eq!(resp.status().as_u16(), 200, "setup: rules-file import must succeed");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-259: a candidate recursive-wildcard pattern with no other reaching
// pattern wins on its own — attributed to the candidate.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds the candidate `expeditions/{expeditionId}/{path=**}`
///          and a synthetic identity/path pair with no other stored rule
///          reaching it
///   When:  Alex calls the routed simulation action with that pair
///   Then:  the response shows "allow", attributed to the candidate
///          pattern itself
///
/// AC-17-259
///
/// @driving_port @real-io @US-06 @AC-17-259
#[tokio::test]
async fn simulating_an_unclaimed_recursive_wildcard_candidate_returns_the_candidates_own_outcome() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate_route"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "pattern": "expeditions/{expeditionId}/{path=**}",
            "condition": "request.auth.uid != \"\"",
            "auth": {"uid": "maria-santos"},
            "resource": {},
            "concrete_path": "expeditions/test-expedition/photos/test-img",
        }))
        .send()
        .await
        .expect("simulate_route request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-17-259: a valid recursive-wildcard candidate simulation must return 200"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["outcome"], "allow",
        "AC-17-259: a satisfying identity, unclaimed by any other pattern, must simulate as 'allow'"
    );
    assert_eq!(
        body["winning_pattern"], "candidate",
        "AC-17-259: with no other reaching pattern, the outcome must be attributed to the candidate"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-260: a synthetic path that ALSO matches an already-stored, more
// specific pattern returns the STORED pattern's own outcome, never the
// candidate's — proving the candidate would be deferred, not silently
// applied.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `expeditions/{expeditionId}/journal_entries/{entryId}` has an
///          ALREADY-STORED, owner-only 4b pattern
///   And:   Alex holds a candidate `expeditions/{expeditionId}/{path=**}`
///          that would allow ANY signed-in user
///   When:  Alex simulates the candidate against a synthetic path that
///          matches BOTH, with a signed-in identity that is NOT the owner
///   Then:  the response shows "deny" (the stored pattern's own owner-only
///          condition governs) and attributes the outcome to the stored
///          pattern, not the candidate — proving the candidate was
///          correctly deferred rather than silently overriding it (the
///          candidate's own condition, evaluated alone, would have allowed
///          this signed-in-but-not-owner caller)
///
/// AC-17-260
///
/// @error @driving_port @real-io @US-06 @AC-17-260
#[tokio::test]
async fn simulating_a_candidate_that_defers_to_an_already_stored_pattern_returns_the_storeds_own_outcome()
{
    const JOURNAL_ENTRIES_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /expeditions/{expeditionId}/journal_entries/{entryId} {
      allow read: if request.auth.uid == resource.data.owner_id;
    }
  }
}
"#;

    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;
    import(&ctx, &cookie, JOURNAL_ENTRIES_RULES_FILE).await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate_route"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "pattern": "expeditions/{expeditionId}/{path=**}",
            "condition": "request.auth.uid != \"\"",
            "auth": {"uid": "dana-kim"},
            "resource": {"owner_id": "maria-santos"},
            "concrete_path": "expeditions/test-expedition/journal_entries/test-entry",
        }))
        .send()
        .await
        .expect("simulate_route request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["outcome"], "deny",
        "AC-17-260: the stored owner-only pattern must govern — Dana is signed in (which the \
         candidate's own broader condition alone would allow) but is NOT the owner"
    );
    assert_eq!(
        body["winning_pattern"], "stored",
        "AC-17-260: the outcome must be attributed to the ALREADY-STORED pattern, proving the \
         candidate was correctly deferred, not silently applied"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-261: a synthetic path that does not structurally match the
// candidate's own fixed prefix returns a distinguishable
// "no_matching_pattern" outcome, never a false "deny".
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds a candidate `expeditions/{expeditionId}/{path=**}`
///   When:  Alex simulates it against a synthetic path in a completely
///          different, non-overlapping collection
///   Then:  the response shows a distinguishable "no_matching_pattern"
///          outcome, not a false "deny"
///
/// AC-17-261
///
/// @error @driving_port @real-io @US-06 @AC-17-261
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
            "pattern": "expeditions/{expeditionId}/{path=**}",
            "condition": "true",
            "auth": {"uid": "maria-santos"},
            "resource": {},
            // `trail_guides/guide-1` never structurally matches the
            // candidate's own `expeditions/{expeditionId}` fixed prefix.
            "concrete_path": "trail_guides/guide-1",
        }))
        .send()
        .await
        .expect("simulate_route request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["outcome"], "no_matching_pattern",
        "AC-17-261: a structural shape mismatch must be distinguishable from a routed 'deny'"
    );
    assert_eq!(
        body["bindings"].as_object().map(|o| o.is_empty()),
        Some(true),
        "AC-17-261: no bindings are resolved when routing itself never matched"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-262: simulation has zero effect on live/imported traffic.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-262
///
/// @error @driving_port @real-io @US-06 @AC-17-262
#[tokio::test]
async fn simulating_a_recursive_wildcard_candidate_has_zero_effect_on_live_traffic() {
    const CATCH_ALL_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /expeditions/{expeditionId}/{path=**} {
      allow read: if request.auth.uid == resource.data.owner_id;
    }
  }
}
"#;

    let ctx = SecurityRulesFullContext::new("trailmark-prod-rw06-liveunaffected").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    let import_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules/import",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": CATCH_ALL_RULES_FILE }))
        .send()
        .await
        .expect("import request failed");
    assert_eq!(
        import_resp.status().as_u16(),
        200,
        "precondition: catch-all pattern import must succeed"
    );

    ctx.seed_document(
        "expeditions/trek-2026/photos",
        "img-042",
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
        "projects/{}/databases/(default)/documents/expeditions/trek-2026/photos/img-042",
        ctx.project_id
    );
    let before = ctx.get_document(&resource_name, Some(&marias_token)).await;
    assert!(
        before.is_ok(),
        "precondition: Maria's real routed read must succeed before simulation: {:?}",
        before.err()
    );

    // Alex simulates a COMPLETELY DIFFERENT candidate pattern/data
    // (unconditional deny) — must never touch `access_rule_patterns` or
    // any live document.
    let simulate_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules/simulate_route",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "pattern": "{path=**}",
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
        "AC-17-262: Maria's real routed read must STILL succeed after the simulation — simulation \
         must have zero effect on live traffic: {:?}",
        after.err()
    );
}
