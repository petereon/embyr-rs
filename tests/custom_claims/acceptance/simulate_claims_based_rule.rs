//! Slice 07 (US-07, ADR-034, LAST slice of custom-claims) — Alex Simulates
//! a Claims-Based Rule Before Publishing.
//!
//! Acceptance criteria verified here (feature-delta.md/slice-07 US-07):
//!   AC-17-154: simulating a candidate claims-referencing rule (exercising
//!              BOTH the claim operand, Slice 02, and string-literal
//!              support, Slice 06 — a simulated role check) against a
//!              synthetic identity carrying a synthetic claims map returns
//!              the same allow/deny outcome real evaluation would produce.
//!   AC-17-155: simulation supports the missing-claim case identically to
//!              real fail-closed evaluation (US-04).
//!
//! Driving port: Admin HTTP :9090 `POST .../access_rules/simulate`
//! (`simulate_access_rule`, extended in place per ADR-034 § Decision —
//! Simulation Extension — no new route). Calls the IDENTICAL `evaluate()`
//! routine real enforcement uses (ADR-029/034), never a second,
//! independently-maintained copy.
//!
//! Zero-effect-on-live-traffic (mirrors every prior sibling's own
//! simulation-extension precedent): NOT re-proven here.
//! `simulate_access_rule` is structurally read-only — it never calls
//! `upsert_access_rule`/`upsert_write_access_rule` anywhere in its body
//! (verified by direct code read), and `tests/security_rules/acceptance/
//! sr05_simulate_rule_before_publish.rs::simulation_has_zero_effect_on_live_traffic`
//! already proves this property for the handler this slice extends
//! in-place, with zero change to that structural guarantee from adding one
//! optional request field.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, mint_client_identity_token_with_claims, now_unix, string_field,
    SecurityRulesFullContext,
};

fn resource_name(ctx: &SecurityRulesFullContext, collection: &str, doc_id: &str) -> String {
    format!(
        "projects/{}/databases/(default)/documents/{}/{}",
        ctx.project_id, collection, doc_id
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-154: a synthetic claims map satisfying a claim-referencing role
// check simulates as "allow" — and agrees with real enforcement given an
// equivalent real minted token + published rule.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds a candidate rule `request.auth.token.role == "admin"`
///          (the claim operand, Slice 02, compared to a string literal,
///          Slice 06 — the feature's own most valuable domain example)
///   And:   a synthetic identity carrying claims `{"role": "admin"}`
///   When:  Alex simulates the candidate rule against that synthetic identity
///   Then:  the response shows "allow" — and a REAL caller with an
///          equivalent minted token against the equivalent PUBLISHED rule
///          is really admitted too, proving the two mechanisms agree
///
/// AC-17-154
///
/// @driving_port @real-io @US-07 @AC-17-154
#[tokio::test]
async fn simulating_a_matching_role_claim_returns_allow_and_agrees_with_real_enforcement() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc07-allow").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let client = reqwest::Client::new();

    let resp = client
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules/simulate",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "request.auth.token.role == \"admin\"",
            "auth": {"uid": "priya-nair", "claims": {"role": "admin"}},
        }))
        .send()
        .await
        .expect("simulate request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["outcome"], "allow",
        "AC-17-154: a synthetic claims map satisfying the rule must simulate as 'allow'"
    );

    // Cross-check: a real caller with an equivalent minted token against an
    // equivalent PUBLISHED rule is really admitted too — proving simulation
    // and real enforcement agree, not merely that simulation looks right in
    // isolation.
    let signing_key = ed25519_dalek::SigningKey::generate(&mut rand_core::OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("flagged_content", "request.auth.token.role == \"admin\"")
        .await;

    let mut fields = std::collections::HashMap::new();
    fields.insert("text".to_string(), string_field("reported content"));
    create_document(&ctx, "flagged_content", "cc07-allow-doc", fields, None)
        .await
        .expect("seed flagged_content document");

    let priyas_token = mint_client_identity_token_with_claims(
        &signing_key,
        "priya-nair",
        &ctx.project_id,
        now_unix() + 3600,
        serde_json::json!({"role": "admin"}),
    );
    let real_resp = ctx
        .get_document(&resource_name(&ctx, "flagged_content", "cc07-allow-doc"), Some(&priyas_token))
        .await;

    assert!(
        real_resp.is_ok(),
        "AC-17-154: real enforcement must agree with the simulated 'allow': {:?}",
        real_resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-154: a synthetic claims map NOT satisfying the role check simulates
// as "deny".
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds the same candidate rule
///          `request.auth.token.role == "admin"`
///   And:   a synthetic identity carrying claims `{"role": "member"}`
///   When:  Alex simulates the candidate rule against that synthetic identity
///   Then:  the response shows "deny"
///
/// AC-17-154 (reverse case)
///
/// @error @driving_port @real-io @US-07 @AC-17-154
#[tokio::test]
async fn simulating_a_mismatched_role_claim_returns_deny() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc07-mismatch").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let client = reqwest::Client::new();

    let resp = client
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules/simulate",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "request.auth.token.role == \"admin\"",
            "auth": {"uid": "sam-osei", "claims": {"role": "member"}},
        }))
        .send()
        .await
        .expect("simulate request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["outcome"], "deny",
        "AC-17-154: a synthetic claims map NOT satisfying the rule must simulate as 'deny'"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-155: simulating with no synthetic claims fails closed, identically
// to a real missing claim (US-04's own real behavior).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds the same candidate rule
///          `request.auth.token.role == "admin"`
///   And:   a synthetic identity carrying NO claims at all (either the
///          "claims" key omitted entirely, or present as an empty map)
///   When:  Alex simulates the candidate rule against that synthetic identity
///   Then:  the response shows "deny" — identically to how a real caller
///          whose token was minted without the referenced claim fails
///          closed (US-04)
///
/// AC-17-155
///
/// @error @driving_port @real-io @US-07 @AC-17-155
#[tokio::test]
async fn simulating_with_no_synthetic_claims_denies_identically_to_real_fail_closed() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc07-missing").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let client = reqwest::Client::new();

    // Both shapes of "no claims" — key omitted vs. present-but-empty — must
    // fail closed identically (input variations of the SAME behavior,
    // parametrized via a loop, mirroring string_literal_claim_comparison.rs's
    // own precedent for suspicious-value variations).
    for auth_body in [
        serde_json::json!({"uid": "dana-kim"}),
        serde_json::json!({"uid": "dana-kim", "claims": {}}),
    ] {
        let resp = client
            .post(ctx.admin_url(&format!(
                "/admin/v1/projects/{}/access_rules/simulate",
                ctx.project_id
            )))
            .header("Cookie", &cookie)
            .json(&serde_json::json!({
                "condition": "request.auth.token.role == \"admin\"",
                "auth": auth_body,
            }))
            .send()
            .await
            .expect("simulate request failed");

        assert_eq!(resp.status().as_u16(), 200);
        let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
        assert_eq!(
            body["outcome"], "deny",
            "AC-17-155: a synthetic identity with no claims must fail closed \
             (auth body: {auth_body:?})"
        );
    }
}
