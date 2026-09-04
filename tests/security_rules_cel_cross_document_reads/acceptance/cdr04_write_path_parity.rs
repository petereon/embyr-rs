//! CDR04 (Slice 04, US-04, Release 2) — Cross-Document Reads Gate Writes,
//! Not Just Reads.
//!
//! Acceptance criteria verified here (feature-delta.md US-04, ADR-066):
//!   AC-CDR-11: a real `CreateDocument` is gated by an `exists()` condition
//!              over a document distinct from the one being written —
//!              two-phase evaluation (discover → fetch → evaluate) runs on
//!              the write path exactly as it does on `GetDocument`.
//!   AC-CDR-12: a real `UpdateDocument` is gated by a combined
//!              `exists() && get().data.<field>` condition over a distinct
//!              document — same mechanism, proven on the update verb.
//!
//! Domain example reused unchanged from Slices 01-03: `organizations/{uid}`
//! is the cross-referenced document; `journal_entries` is the collection
//! being written.
//!
//! Driving port: gRPC :8080 `CreateDocument`/`UpdateDocument`
//! (`SecurityRulesFullContext`, imported directly from
//! `security_rules_write_path`'s own fixture module — mirrors CP03's own
//! precedent for why not re-exported via this feature's own `common/mod.rs`).

#![allow(unused_imports)]

#[path = "../../security_rules_write_path/common/mod.rs"]
mod common;
use common::{
    create_document, mint_client_identity_token, now_unix, string_field, update_document,
    SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

fn journal_entry_resource_name(project_id: &str, document_id: &str) -> String {
    format!("projects/{project_id}/databases/(default)/documents/journal_entries/{document_id}")
}

async fn setup(project_id: &str, condition: &str) -> (SecurityRulesFullContext, SigningKey) {
    let ctx = SecurityRulesFullContext::new(project_id).await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let define_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{project_id}/write_access_rules")))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": condition,
        }))
        .send()
        .await
        .expect("define_write_access_rule request failed");
    assert_eq!(define_resp.status().as_u16(), 200, "setup: write rule define must succeed");

    (ctx, signing_key)
}

/// AC-CDR-11
///
/// @driving_port @real-io @US-04 @AC-CDR-11
#[tokio::test]
async fn a_create_is_gated_by_exists_on_a_document_distinct_from_the_one_being_written() {
    let (ctx, signing_key) = setup(
        "trailmark-prod-cdr04-create",
        "exists(/databases/$(database)/documents/organizations/$(request.auth.uid))",
    )
    .await;
    // organizations/maria-santos deliberately NOT seeded yet — her first
    // create attempt must be denied.

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let denied = create_document(
        &ctx,
        "journal_entries",
        "trek-2026",
        std::collections::HashMap::new(),
        Some(&marias_token),
    )
    .await;
    let err = denied.expect_err(
        "AC-CDR-11: create must be denied while the cross-referenced organizations/maria-santos \
         document does not exist",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-CDR-11: denial must be attributable to the rule, got {:?}",
        err.code()
    );

    ctx.seed_document("organizations", "maria-santos", serde_json::json!({}))
        .await;

    let allowed = create_document(
        &ctx,
        "journal_entries",
        "trek-2026",
        std::collections::HashMap::new(),
        Some(&marias_token),
    )
    .await;
    assert!(
        allowed.is_ok(),
        "AC-CDR-11: once organizations/maria-santos exists, the create must be allowed: {:?}",
        allowed.err()
    );
}

/// AC-CDR-12
///
/// @driving_port @real-io @US-04 @AC-CDR-12
#[tokio::test]
async fn an_update_is_gated_by_exists_and_get_data_field_on_a_distinct_document() {
    let (ctx, signing_key) = setup(
        "trailmark-prod-cdr04-update",
        "exists(/databases/$(database)/documents/organizations/$(request.auth.uid)) && \
         get(/databases/$(database)/documents/organizations/$(request.auth.uid)).data.role == \"admin\"",
    )
    .await;
    ctx.seed_document("journal_entries", "trek-2026", serde_json::json!({}))
        .await;
    ctx.seed_document(
        "organizations",
        "maria-santos",
        serde_json::json!({ "role": {"t": "S", "v": "member"} }),
    )
    .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let mut fields = std::collections::HashMap::new();
    fields.insert("title".to_string(), string_field("Trek Log"));

    let denied = update_document(
        &ctx,
        &journal_entry_resource_name(&ctx.project_id, "trek-2026"),
        fields.clone(),
        Some(&marias_token),
    )
    .await;
    let err = denied.expect_err(
        "AC-CDR-12: update must be denied while organizations/maria-santos.role != 'admin'",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-CDR-12: denial must be attributable to the rule, got {:?}",
        err.code()
    );

    // `organizations` carries no write rule of its own — this real
    // `UpdateDocument` call is unrestricted, the same way a test setup would
    // seed via the driving port rather than a second `seed_document` insert
    // (which would violate the primary key).
    let mut org_fields = std::collections::HashMap::new();
    org_fields.insert("role".to_string(), string_field("admin"));
    update_document(
        &ctx,
        &format!(
            "projects/{}/databases/(default)/documents/organizations/maria-santos",
            ctx.project_id
        ),
        org_fields,
        Some(&marias_token),
    )
    .await
    .expect("test setup: promoting organizations/maria-santos to admin must succeed");

    let allowed = update_document(
        &ctx,
        &journal_entry_resource_name(&ctx.project_id, "trek-2026"),
        fields,
        Some(&marias_token),
    )
    .await;
    assert!(
        allowed.is_ok(),
        "AC-CDR-12: once organizations/maria-santos.role == 'admin', the update must be allowed: {:?}",
        allowed.err()
    );
}
