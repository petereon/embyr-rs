//! security-rules-collection-group-rules Slice 07 (US-07, LAST slice,
//! ADR-032) — Alex Can Pre-Check Whether a Candidate Collection-Group Query
//! Would Be Accepted.
//!
//! `simulate_group_query_compliance` is a NEW, DISTINCT admin
//! handler/route (ADR-032 § `simulate_group_query_compliance` — a genuine,
//! evaluated departure from both DISCUSS's own Technical Note and
//! ADR-031's own precedent) — a SIBLING to `simulate_query_compliance`, not
//! an extension of it in place. It calls the SAME `check_query_compliance`
//! function Slices 01-04 built and real `handle_run_query`'s
//! `all_descendants = true` arm uses — no second, independently-maintained
//! implementation.
//!
//! Acceptance criteria verified here (feature-delta.md US-07):
//!   AC-17-101: simulating a candidate group condition that WOULD be
//!             compliant against a candidate filter returns the same admit
//!             outcome real enforcement would produce.
//!   AC-17-102: simulation correctly reports "missing required filter" for
//!             candidate filters lacking a required conjunct.
//!   AC-17-103: simulation correctly reports "no collection-group rule
//!             defined" when no candidate group rule is supplied — direct
//!             from the request body, without ever touching
//!             `group_access_rules`.
//!   AC-17-104: simulating a group-query shape has zero effect on
//!             live/published `RunQuery` traffic.
//!
//! Driving ports: Admin HTTP :9090 (`SecurityRulesAdminContext`, AC-17-101/
//! 102/103) + Admin HTTP :9090 AND gRPC :8080 together
//! (`SecurityRulesFullContext`, AC-17-104 — needs BOTH the simulate action
//! and a REAL group-query `RunQuery` on the SAME composition root to prove
//! zero cross-effect, mirroring `simulate_candidate_query_before_shipping
//! .rs`'s own AC-17-76 scenario shape for the non-group case).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    mint_client_identity_token, now_unix, run_query, seed_group_access_rule,
    seed_group_access_rule_full, SecurityRulesAdminContext, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-101: simulating a candidate group condition that WOULD be compliant
// against a candidate filter returns the same admit outcome real
// enforcement would produce.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds a candidate group ownership-equality condition
///          (`request.auth.uid == resource.data.owner_id`)
///   When:  Alex simulates it against a candidate filter binding `owner_id`
///          to the synthetic caller's own uid
///   Then:  the response shows `compliant: true` — the same admit outcome
///          real group-query enforcement would produce for an identical
///          group rule/filter
///
/// AC-17-101
///
/// @driving_port @real-io @US-07 @AC-17-101
#[tokio::test]
async fn simulating_a_compliant_candidate_group_condition_reports_compliant() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url(
            "/admin/v1/projects/trailmark-prod/access_rules/simulate_group_query",
        ))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "group_condition": "request.auth.uid == resource.data.owner_id",
            "auth": {"uid": "maria-santos"},
            "query_filters": [
                {"field_path": "owner_id", "op": "==", "value": "maria-santos"},
            ],
        }))
        .send()
        .await
        .expect("simulate_group_query request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-17-101: a valid simulate_group_query request must return 200"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["compliant"], true,
        "AC-17-101: a candidate filter binding owner_id to the caller's own uid must be reported \
         compliant against a compliant candidate group condition — the same admit outcome real \
         check_query_compliance group-query enforcement would produce, got: {body}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-102: simulation correctly reports "missing required filter" for
// candidate filters lacking a required conjunct.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds a candidate group ownership-equality condition
///          (`request.auth.uid == resource.data.owner_id`)
///   When:  Alex simulates it against a candidate filter missing the
///          required conjunct
///   Then:  the response shows `compliant: false` with reason
///          `OWNERSHIP_FILTER_MISSING` — the SAME reason-code vocabulary
///          real group-query rejection uses (Slice 03, ADR-032), since it's
///          literally the same function's output
///
/// AC-17-102
///
/// @error @driving_port @real-io @US-07 @AC-17-102
#[tokio::test]
async fn simulating_a_candidate_group_query_missing_the_required_conjunct_reports_ownership_filter_missing(
) {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url(
            "/admin/v1/projects/trailmark-prod/access_rules/simulate_group_query",
        ))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "group_condition": "request.auth.uid == resource.data.owner_id",
            "auth": {"uid": "maria-santos"},
            "query_filters": [
                {"field_path": "archived", "op": "==", "value": false},
            ],
        }))
        .send()
        .await
        .expect("simulate_group_query request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["compliant"], false,
        "AC-17-102: a candidate query missing the required conjunct must be reported \
         non-compliant, got: {body}"
    );
    assert_eq!(
        body["reasons"],
        serde_json::json!(["OWNERSHIP_FILTER_MISSING"]),
        "AC-17-102: the missing conjunct must be reported via the SAME reason-code vocabulary \
         real group-query enforcement's rejection uses, got: {body}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-103: simulation correctly reports "no collection-group rule
// defined" when no candidate group rule is supplied — direct from the
// request body, without ever touching group_access_rules.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex has NOT supplied a candidate group condition
///          (`group_condition: null`)
///   When:  Alex simulates a group query anyway
///   Then:  the response shows `compliant: false` with reason
///          `GROUP_RULE_NOT_DEFINED` — matching US-04's real default,
///          reached directly from the request body alone
///
/// AC-17-103
///
/// @error @driving_port @real-io @US-07 @AC-17-103
#[tokio::test]
async fn simulating_with_group_condition_explicitly_null_reports_group_rule_not_defined() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url(
            "/admin/v1/projects/trailmark-prod/access_rules/simulate_group_query",
        ))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "group_condition": null,
            "auth": {"uid": "maria-santos"},
            "query_filters": [
                {"field_path": "owner_id", "op": "==", "value": "maria-santos"},
            ],
        }))
        .send()
        .await
        .expect("simulate_group_query request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["compliant"], false,
        "AC-17-103: with no candidate group condition supplied, the simulation must report \
         non-compliant, matching US-04's real default, got: {body}"
    );
    assert_eq!(
        body["reasons"],
        serde_json::json!(["GROUP_RULE_NOT_DEFINED"]),
        "AC-17-103: absence of a candidate group condition must be reported via the SAME \
         GROUP_RULE_NOT_DEFINED reason real group-query enforcement's US-04 rejection uses, \
         got: {body}"
    );
}

/// Journey (field omitted entirely, not just `null`):
///   Given: Alex's JSON body omits `group_condition` entirely (relying on
///          `#[serde(default)]`-free `Option<String>`'s own missing-field
///          deserialization, which resolves to `None` identically to an
///          explicit `null`)
///   When:  Alex simulates a group query
///   Then:  the response is identical to the explicit-`null` case above —
///          `compliant: false, reasons: ["GROUP_RULE_NOT_DEFINED"]`
///
/// AC-17-103
///
/// @error @driving_port @real-io @US-07 @AC-17-103
#[tokio::test]
async fn simulating_with_group_condition_field_omitted_reports_group_rule_not_defined() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url(
            "/admin/v1/projects/trailmark-prod/access_rules/simulate_group_query",
        ))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "auth": {"uid": "maria-santos"},
            "query_filters": [
                {"field_path": "owner_id", "op": "==", "value": "maria-santos"},
            ],
        }))
        .send()
        .await
        .expect("simulate_group_query request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["compliant"], false,
        "AC-17-103: an omitted group_condition field must be treated identically to an explicit \
         null, got: {body}"
    );
    assert_eq!(
        body["reasons"],
        serde_json::json!(["GROUP_RULE_NOT_DEFINED"]),
        "AC-17-103: an omitted group_condition field must report GROUP_RULE_NOT_DEFINED \
         identically to the explicit-null case, got: {body}"
    );
}

/// Journey (structural proof — never touches storage):
///   Given: `journal_entries` ALREADY has a REAL group rule defined in
///          `group_access_rules`
///   When:  Alex simulates a group query with `group_condition: null`
///          against that SAME collection id
///   Then:  the response is STILL `compliant: false, reasons:
///          ["GROUP_RULE_NOT_DEFINED"]` — byte-identical to the no-real-rule
///          case above — proving the `None` arm decides purely from the
///          request body and never reads `group_access_rules` at all,
///          mirroring how AC-17-91 (Slice 04) proved its own "single
///          indexed lookup only" claim structurally
///
/// AC-17-103
///
/// @error @driving_port @real-io @US-07 @AC-17-103
#[tokio::test]
async fn simulating_with_no_candidate_group_condition_ignores_a_real_group_rule_for_the_same_collection_id(
) {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    // A REAL group rule for journal_entries already exists in storage.
    seed_group_access_rule(
        &ctx,
        "trailmark-prod",
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let resp = ctx
        .client
        .post(ctx.url(
            "/admin/v1/projects/trailmark-prod/access_rules/simulate_group_query",
        ))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "group_condition": null,
            "auth": {"uid": "maria-santos"},
            "query_filters": [
                {"field_path": "owner_id", "op": "==", "value": "maria-santos"},
            ],
        }))
        .send()
        .await
        .expect("simulate_group_query request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["compliant"], false,
        "AC-17-103: a real, stored group rule for the same collection id must have zero bearing \
         on the None-condition simulation outcome — the simulation only ever looks at the \
         request body, got: {body}"
    );
    assert_eq!(
        body["reasons"],
        serde_json::json!(["GROUP_RULE_NOT_DEFINED"]),
        "AC-17-103: the outcome must be byte-identical whether or not a real group rule exists \
         for this collection id elsewhere in group_access_rules — proving the simulation never \
         reads storage for the None-condition arm, got: {body}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-104: simulating a group-query shape has zero effect on
// live/published RunQuery traffic.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has a REAL, PUBLISHED collection-group rule
///          (owner-only) and Maria Santos holds a verified identity, and
///          her real `collectionGroup('journal_entries')` RunQuery
///          succeeds
///   When:  Alex simulates a COMPLETELY DIFFERENT candidate group
///          condition/filter combination (unconditional deny, unrelated
///          identity) via `simulate_group_query_compliance`
///   Then:  Maria's real group `RunQuery` against the published rule STILL
///          behaves exactly as it did before the simulation call —
///          `simulate_group_query_compliance` never issues a real
///          `RunQuery` and never mutates `group_access_rules`
///
/// AC-17-104
///
/// @error @driving_port @real-io @US-07 @AC-17-104
#[tokio::test]
async fn simulating_a_candidate_group_query_has_zero_effect_on_live_run_query_traffic() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-scgr07-liveunaffected").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    seed_group_access_rule_full(
        &ctx,
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

    // Real caller succeeds against the PUBLISHED group rule before
    // simulation.
    let before = run_query(
        &ctx,
        "journal_entries",
        true,
        &[("owner_id", "maria-santos")],
        Some(&marias_token),
    )
    .await;
    assert!(
        before.is_ok(),
        "precondition: Maria's real collection-group RunQuery must succeed before simulation: \
         {:?}",
        before.err()
    );

    // Alex simulates a COMPLETELY DIFFERENT candidate group
    // condition/filter/identity — this must never touch
    // `group_access_rules` or issue a real RunQuery.
    let simulate_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules/simulate_group_query",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "group_condition": "false",
            "auth": {"uid": "test-user-999"},
            "query_filters": [
                {"field_path": "owner_id", "op": "==", "value": "someone-else"},
            ],
        }))
        .send()
        .await
        .expect("simulate_group_query request failed");
    assert_eq!(simulate_resp.status().as_u16(), 200);

    // Maria's real collection-group RunQuery against the PUBLISHED rule
    // STILL succeeds afterward, completely unaffected by the simulation
    // just run.
    let after = run_query(
        &ctx,
        "journal_entries",
        true,
        &[("owner_id", "maria-santos")],
        Some(&marias_token),
    )
    .await;
    assert!(
        after.is_ok(),
        "AC-17-104: Maria's real collection-group RunQuery must STILL succeed after the \
         simulation — simulate_group_query_compliance must have zero effect on live traffic: \
         {:?}",
        after.err()
    );
    assert_eq!(
        after.unwrap().len(),
        1,
        "AC-17-104: the real collection-group RunQuery result must be unaffected by the \
         simulation call"
    );
}
