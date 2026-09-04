//! CDR02 (Slice 02, US-02, Release 1) — `get()`'s Own `.data.<field>`
//! Access Fails Closed on a Nonexistent Document.
//!
//! Acceptance criteria verified here (feature-delta.md US-02, ADR-066):
//!   AC-CDR-05: `get(<path>).data.<field>` parses into
//!              `Operand::CrossDocumentGet`.
//!   AC-CDR-06: a real read gated by `get(...).data.<field> == <value>`
//!              succeeds when the referenced document exists with a
//!              matching field value.
//!   AC-CDR-07: the same condition denies (never panics) when the
//!              referenced document does not exist at all.
//!   AC-CDR-08: the same condition denies when the referenced document
//!              exists but its own field value does not match.
//!
//! Driving ports: Admin HTTP :9090 (`define_access_rule`) + gRPC :8080
//! `GetDocument` (`SecurityRulesFullContext`).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, SecurityRulesFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

fn resource_name(project_id: &str, path: &str) -> String {
    format!("projects/{project_id}/databases/(default)/documents/{path}")
}

/// Journey:
///   Given: Alex imports `allow read: if get(/databases/$(database)/
///          documents/organizations/$(request.auth.uid)).data.role ==
///          "admin";` on `journal_entries`
///   When:  Maria (whose own `organizations/maria-santos` document has
///          `role: "admin"`) reads an entry; Dana (whose own document
///          does not exist at all) reads one; a third caller Priya
///          (whose own document exists but with `role: "member"`) reads
///          one
///   Then:  Maria's read succeeds, Dana's is denied, Priya's is denied
///
/// AC-CDR-05, AC-CDR-06, AC-CDR-07, AC-CDR-08
///
/// @driving_port @real-io @US-02 @AC-CDR-05 @AC-CDR-06 @AC-CDR-07 @AC-CDR-08
#[tokio::test]
async fn a_role_lookup_get_clause_gates_a_real_read_across_all_three_outcomes() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cdr02-getrole").await;
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
            "condition": "get(/databases/$(database)/documents/organizations/$(request.auth.uid)).data.role == \"admin\"",
        }))
        .send()
        .await
        .expect("define_access_rule request failed");
    assert_eq!(
        define_resp.status().as_u16(),
        200,
        "AC-CDR-05: a get().data.<field> role-lookup condition must be accepted"
    );

    // Maria: organizations/maria-santos exists, role == "admin".
    ctx.seed_document(
        "organizations",
        "maria-santos",
        serde_json::json!({ "role": {"t": "S", "v": "admin"} }),
    )
    .await;
    // Dana: organizations/dana-kim is deliberately NEVER seeded.
    // Priya: organizations/priya-nair exists, but role == "member".
    ctx.seed_document(
        "organizations",
        "priya-nair",
        serde_json::json!({ "role": {"t": "S", "v": "member"} }),
    )
    .await;

    ctx.seed_document("journal_entries", "trek-2026", serde_json::json!({}))
        .await;
    let resource = resource_name(&ctx.project_id, "journal_entries/trek-2026");

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
    let priyas_token = mint_client_identity_token(
        &signing_key,
        "priya-nair",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let marias_read = ctx.get_document(&resource, Some(&marias_token)).await;
    assert!(
        marias_read.is_ok(),
        "AC-CDR-06: Maria's own referenced document has role=='admin', must be allowed: {:?}",
        marias_read.err()
    );

    let danas_read = ctx.get_document(&resource, Some(&danas_token)).await;
    let dana_err = danas_read
        .expect_err("AC-CDR-07: Dana's own referenced document does not exist, must be denied (never panic)");
    assert_eq!(dana_err.code(), tonic::Code::PermissionDenied, "got {:?}", dana_err.code());

    let priyas_read = ctx.get_document(&resource, Some(&priyas_token)).await;
    let priya_err = priyas_read
        .expect_err("AC-CDR-08: Priya's own referenced document has role=='member', must be denied");
    assert_eq!(priya_err.code(), tonic::Code::PermissionDenied, "got {:?}", priya_err.code());
}
