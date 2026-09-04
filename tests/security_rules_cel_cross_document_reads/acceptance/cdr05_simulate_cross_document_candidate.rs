//! CDR05 (Slice 05, US-05, Release 2, LAST slice) — Alex Simulates a
//! Cross-Document Candidate Rule Before Publishing It.
//!
//! Acceptance criteria verified here (feature-delta.md US-05, ADR-066):
//!   AC-CDR-13: `simulate_access_rule` accepts a new synthetic
//!              `cross_document_reads` map (concrete path -> synthetic field
//!              values) and resolves `get()`/`exists()` operands against
//!              it — zero real backend fetch.
//!   AC-CDR-14: a path referenced by the candidate condition but ABSENT from
//!              the synthetic map simulates identically to a real
//!              nonexistent document (`exists()` -> false; `get()`'s own
//!              `.data` access -> fails closed).
//!
//! Driving port: admin HTTP :9090 `POST .../access_rules/simulate`
//! (`SecurityRulesAdminContext` — admin-only, mirrors CEG07's own
//! `simulate_access_rule` precedent, zero gRPC needed).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesAdminContext;

const CONDITION: &str = "exists(/databases/$(database)/documents/organizations/$(request.auth.uid)) && \
     get(/databases/$(database)/documents/organizations/$(request.auth.uid)).data.role == \"admin\"";

/// AC-CDR-13
///
/// @driving_port @real-io @US-05 @AC-CDR-13
#[tokio::test]
async fn a_synthetic_cross_document_reads_map_resolves_exists_and_get_correctly() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let allow_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": CONDITION,
            "auth": {"uid": "maria-santos"},
            "cross_document_reads": {
                "organizations/maria-santos": {"role": "admin"},
            },
        }))
        .send()
        .await
        .expect("simulate_access_rule request failed");
    assert_eq!(allow_resp.status().as_u16(), 200);
    let allow_body: serde_json::Value = allow_resp.json().await.expect("response body must be JSON");
    assert_eq!(
        allow_body["outcome"], "allow",
        "AC-CDR-13: a synthetic organizations/maria-santos with role=='admin' must simulate as 'allow'"
    );

    let deny_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": CONDITION,
            "auth": {"uid": "maria-santos"},
            "cross_document_reads": {
                "organizations/maria-santos": {"role": "member"},
            },
        }))
        .send()
        .await
        .expect("simulate_access_rule request failed");
    assert_eq!(deny_resp.status().as_u16(), 200);
    let deny_body: serde_json::Value = deny_resp.json().await.expect("response body must be JSON");
    assert_eq!(
        deny_body["outcome"], "deny",
        "AC-CDR-13: a synthetic organizations/maria-santos with role=='member' must simulate as 'deny'"
    );
}

/// AC-CDR-14
///
/// @driving_port @real-io @US-05 @AC-CDR-14
#[tokio::test]
async fn a_path_absent_from_the_synthetic_map_simulates_as_a_nonexistent_document() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    // `cross_document_reads` deliberately omits `organizations/maria-santos`
    // entirely — never seeded, never referenced.
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": CONDITION,
            "auth": {"uid": "maria-santos"},
        }))
        .send()
        .await
        .expect("simulate_access_rule request failed");
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["outcome"], "deny",
        "AC-CDR-14: a path referenced by the candidate condition but absent from the synthetic \
         map must simulate identically to a real nonexistent document — 'deny'"
    );
}
