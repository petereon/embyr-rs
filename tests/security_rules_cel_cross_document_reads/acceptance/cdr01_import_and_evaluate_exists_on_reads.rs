//! CDR01 (Slice 01, US-01, Release 1, Walking Skeleton) — Alex's Role-Lookup
//! `exists()` Clause Parses and Enforces on Reads.
//!
//! Acceptance criteria verified here (feature-delta.md US-01, ADR-066):
//!   AC-CDR-01: `exists(/databases/$(database)/documents/<segments>/
//!              $(request.auth.uid))` parses into a new `Operand::
//!              CrossDocumentExists`.
//!   AC-CDR-02: a real `GetDocument` request from a caller whose own
//!              referenced organization document exists succeeds; one
//!              whose own referenced document does not exist is denied.
//!   AC-CDR-03: `$(request.path.<var>)` substitution is ALSO accepted.
//!   AC-CDR-04: a candidate condition using any other substitution shape
//!              is a NAMED, distinguishable rejection.
//!
//! Driving ports: Admin HTTP :9090 (`define_access_rule`) + gRPC :8080
//! `GetDocument` (`SecurityRulesFullContext`) — real enforcement, real
//! fetch of a real second document.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, SecurityRulesFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

fn resource_name(project_id: &str, path: &str) -> String {
    format!("projects/{project_id}/databases/(default)/documents/{path}")
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-CDR-01/02: a role-lookup exists() clause imports, and gates a real
// GetDocument call correctly in both directions.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex imports `allow read: if exists(/databases/$(database)/
///          documents/organizations/$(request.auth.uid));` on
///          `journal_entries`
///   When:  Maria (whose own `organizations/maria-santos` document exists)
///          reads a journal entry, and separately Dana (whose own
///          `organizations/dana-kim` document does NOT exist) reads one
///   Then:  Maria's read succeeds, Dana's is denied
///
/// AC-CDR-01, AC-CDR-02
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-CDR-01 @AC-CDR-02
#[tokio::test]
async fn a_role_lookup_exists_clause_imports_and_gates_a_real_read_in_both_directions() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cdr01-existsread").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    let define_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": "exists(/databases/$(database)/documents/organizations/$(request.auth.uid))",
        }))
        .send()
        .await
        .expect("define_access_rule request failed");
    assert_eq!(
        define_resp.status().as_u16(),
        200,
        "AC-CDR-01: a role-lookup exists() condition must be accepted"
    );

    // Maria's own referenced organization document IS seeded.
    ctx.seed_document(
        "organizations",
        "maria-santos",
        serde_json::json!({}),
    )
    .await;
    // Dana's own referenced organization document is deliberately NEVER
    // seeded.

    ctx.seed_document(
        "journal_entries",
        "trek-2026",
        serde_json::json!({}),
    )
    .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );
    let danas_token = mint_client_identity_token(
        &signing_key,
        "dana-kim",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let resource = resource_name(&ctx.project_id, "journal_entries/trek-2026");

    let marias_read = ctx.get_document(&resource, Some(&marias_token)).await;
    assert!(
        marias_read.is_ok(),
        "AC-CDR-02: Maria's own referenced organization document exists, must be allowed: {:?}",
        marias_read.err()
    );

    let danas_read = ctx.get_document(&resource, Some(&danas_token)).await;
    let err = danas_read
        .expect_err("AC-CDR-02: Dana's own referenced organization document does not exist, must be denied");
    assert_eq!(err.code(), tonic::Code::PermissionDenied, "got {:?}", err.code());
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-CDR-03: $(request.path.<var>) substitution is also accepted.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-CDR-03
///
/// @driving_port @real-io @US-01 @AC-CDR-03
#[tokio::test]
async fn a_path_variable_substitution_in_a_cross_document_path_parses_and_enforces() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cdr01-pathvar").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let define_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": "exists(/databases/$(database)/documents/trail_guides/$(request.path.entryId))",
        }))
        .send()
        .await
        .expect("define_access_rule request failed");
    assert_eq!(
        define_resp.status().as_u16(),
        200,
        "AC-CDR-03: a $(request.path.<var>) substitution must be accepted"
    );

    ctx.seed_document("trail_guides", "trek-2026", serde_json::json!({}))
        .await;
    ctx.seed_document("journal_entries", "trek-2026", serde_json::json!({}))
        .await;

    let resp = ctx
        .get_document(
            &resource_name(&ctx.project_id, "journal_entries/trek-2026"),
            None,
        )
        .await;
    assert!(
        resp.is_ok(),
        "AC-CDR-03: the path-variable-substituted document exists, must be allowed: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-CDR-04: any other substitution shape is a named rejection.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-CDR-04
///
/// @error @driving_port @real-io @US-01 @AC-CDR-04
#[tokio::test]
async fn an_unsupported_substitution_shape_is_rejected_as_a_named_construct() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cdr01-badsubst").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": "exists(/databases/$(database)/documents/organizations/$(resource.data.orgId))",
        }))
        .send()
        .await
        .expect("define_access_rule request failed");

    assert_eq!(
        resp.status().as_u16(),
        400,
        "AC-CDR-04: an unsupported substitution shape must be rejected"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["reason"], "UNSUPPORTED_CONSTRUCT");
}
