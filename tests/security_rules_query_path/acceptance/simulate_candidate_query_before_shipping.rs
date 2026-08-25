//! Slice 07 (US-07, LAST slice, ADR-031) — Simulate a Candidate Query Before
//! Shipping Client Code.
//!
//! `simulate_query_compliance` is a NEW, DISTINCT admin handler/route (ADR-031
//! § Decision — Release 2 Simulation Extension) — a SIBLING to
//! `simulate_access_rule`, not a further extension of its body/response
//! shape. It calls the SAME `check_query_compliance` function Slices 01-05
//! built and real `handle_run_query` enforcement uses — no second,
//! independently-maintained implementation.
//!
//! Acceptance criteria verified here (feature-delta.md US-07):
//!   AC-17-73: simulating a candidate query filter against a candidate rule
//!             returns the same admit/reject outcome real enforcement would
//!             produce.
//!   AC-17-74: simulation correctly reports "missing required filter" for
//!             candidate filters lacking a required conjunct.
//!   AC-17-75: simulation correctly reports "rule shape not supported" for
//!             candidate rules outside the decidable set.
//!   AC-17-76: simulating a query shape has zero effect on live/published
//!             `RunQuery` traffic.
//!
//! Driving ports: Admin HTTP :9090 (`SecurityRulesAdminContext`, AC-17-73/74/
//! 75) + Admin HTTP :9090 AND gRPC :8080 together (`SecurityRulesFullContext`,
//! AC-17-76 — needs BOTH the simulate action and a REAL RunQuery on the SAME
//! composition root to prove zero cross-effect, mirroring
//! `simulate_write_rule.rs`'s own AC-17-47 scenario shape).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    mint_client_identity_token, now_unix, run_query, SecurityRulesAdminContext,
    SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-73: simulating a candidate query filter against a candidate rule
// returns the same admit/reject outcome real enforcement would produce.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds a candidate ownership-equality rule
///          (`request.auth.uid == resource.data.owner_id`)
///   When:  Alex simulates it against a candidate filter binding `owner_id`
///          to the synthetic caller's own uid
///   Then:  the response shows `compliant: true` — the same admit outcome
///          real enforcement would produce for an identical filter
///
/// AC-17-73
///
/// @driving_port @real-io @US-07 @AC-17-73
#[tokio::test]
async fn simulating_a_query_filter_bound_to_the_callers_own_uid_reports_compliant() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url(
            "/admin/v1/projects/trailmark-prod/access_rules/simulate_query",
        ))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "request.auth.uid == resource.data.owner_id",
            "auth": {"uid": "maria-santos"},
            "query_filters": [
                {"field_path": "owner_id", "op": "==", "value": "maria-santos"},
            ],
        }))
        .send()
        .await
        .expect("simulate_query request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-17-73: a valid simulate_query request must return 200"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["compliant"], true,
        "AC-17-73: a candidate filter binding owner_id to the caller's own uid must be reported \
         compliant — the same admit outcome real check_query_compliance enforcement would \
         produce, got: {body}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-74: simulation correctly reports "missing required filter" for
// candidate filters lacking a required conjunct.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds a candidate ownership-equality rule
///          (`request.auth.uid == resource.data.owner_id`)
///   When:  Alex simulates it with NO query filters at all
///   Then:  the response shows `compliant: false` with reason
///          `OWNERSHIP_FILTER_MISSING` — the SAME reason code/message shape
///          real enforcement's rejection uses (Slice 02, ADR-031), since
///          it's literally the same function's output
///
/// AC-17-74
///
/// @error @driving_port @real-io @US-07 @AC-17-74
#[tokio::test]
async fn simulating_a_query_with_no_filters_reports_ownership_filter_missing() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url(
            "/admin/v1/projects/trailmark-prod/access_rules/simulate_query",
        ))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "request.auth.uid == resource.data.owner_id",
            "auth": {"uid": "maria-santos"},
            "query_filters": [],
        }))
        .send()
        .await
        .expect("simulate_query request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["compliant"], false,
        "AC-17-74: a candidate query with no filters must be reported non-compliant, got: {body}"
    );
    assert_eq!(
        body["reasons"],
        serde_json::json!(["OWNERSHIP_FILTER_MISSING"]),
        "AC-17-74: the missing conjunct must be reported via the SAME reason-code vocabulary \
         real enforcement's rejection uses, got: {body}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-75: simulation correctly reports "rule shape not supported" for
// candidate rules outside the decidable set.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds a candidate rule OUTSIDE the locked decidable set —
///          an `Or`-shaped condition (`request.auth.uid ==
///          resource.data.owner_id || true`)
///   When:  Alex simulates it against any candidate filter/identity
///   Then:  the response shows `compliant: false` with reason
///          `UNSUPPORTED_RULE_SHAPE` — mirrors Slice 05's real-enforcement
///          rejection of the identical undecidable shape
///
/// AC-17-75
///
/// @error @driving_port @real-io @US-07 @AC-17-75
#[tokio::test]
async fn simulating_an_or_shaped_rule_reports_unsupported_rule_shape() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url(
            "/admin/v1/projects/trailmark-prod/access_rules/simulate_query",
        ))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "request.auth.uid == resource.data.owner_id || true",
            "auth": {"uid": "maria-santos"},
            "query_filters": [
                {"field_path": "owner_id", "op": "==", "value": "maria-santos"},
            ],
        }))
        .send()
        .await
        .expect("simulate_query request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["compliant"], false,
        "AC-17-75: an Or-shaped candidate rule must be reported non-compliant regardless of \
         filter/identity, got: {body}"
    );
    assert_eq!(
        body["reasons"],
        serde_json::json!(["UNSUPPORTED_RULE_SHAPE"]),
        "AC-17-75: an Or-shaped candidate rule must be reported via the whole-rule \
         UNSUPPORTED_RULE_SHAPE reason — mirrors Slice 05's real-enforcement rejection of the \
         identical undecidable shape, got: {body}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-76: simulating a query shape has zero effect on live/published
// RunQuery traffic.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has a REAL, PUBLISHED read rule (owner-only)
///          and Maria Santos holds a verified identity, and her real
///          `RunQuery` against it succeeds
///   When:  Alex simulates a COMPLETELY DIFFERENT candidate rule/filter
///          combination (unconditional deny, unrelated identity) via
///          `simulate_query_compliance`
///   Then:  Maria's real `RunQuery` against the published rule STILL
///          behaves exactly as it did before the simulation call —
///          `simulate_query_compliance` never issues a real `RunQuery` and
///          never touches `access_rules`
///
/// AC-17-76
///
/// @error @driving_port @real-io @US-07 @AC-17-76
#[tokio::test]
async fn simulating_a_candidate_query_has_zero_effect_on_live_run_query_traffic() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp07-liveunaffected").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    // Real caller succeeds against the PUBLISHED rule before simulation.
    let before = run_query(
        &ctx,
        "journal_entries",
        &[("owner_id", "maria-santos")],
        Some(&marias_token),
    )
    .await;
    assert!(
        before.is_ok(),
        "precondition: Maria's real RunQuery must succeed before simulation: {:?}",
        before.err()
    );

    // Alex simulates a COMPLETELY DIFFERENT candidate rule/filter/identity —
    // this must never touch `access_rules` or issue a real RunQuery.
    let simulate_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules/simulate_query",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "false",
            "auth": {"uid": "test-user-999"},
            "query_filters": [
                {"field_path": "owner_id", "op": "==", "value": "someone-else"},
            ],
        }))
        .send()
        .await
        .expect("simulate_query request failed");
    assert_eq!(simulate_resp.status().as_u16(), 200);

    // Maria's real RunQuery against the PUBLISHED rule STILL succeeds
    // afterward, completely unaffected by the simulation just run.
    let after = run_query(
        &ctx,
        "journal_entries",
        &[("owner_id", "maria-santos")],
        Some(&marias_token),
    )
    .await;
    assert!(
        after.is_ok(),
        "AC-17-76: Maria's real RunQuery must STILL succeed after the simulation — \
         simulate_query_compliance must have zero effect on live traffic: {:?}",
        after.err()
    );
    assert_eq!(
        after.unwrap().len(),
        1,
        "AC-17-76: the real RunQuery result must be unaffected by the simulation call"
    );
}
