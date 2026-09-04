//! CF02 (Slice 02, US-02, Release 1) — The Same Expanded Condition Gates
//! Writes, Zero New Production Code.
//!
//! Acceptance criteria verified here (feature-delta.md US-02, ADR-067):
//!   AC-CF-05: a real `CreateDocument`/`UpdateDocument` call is gated
//!             correctly by a function-call-bearing condition, using the
//!             identical import mechanism as Slice 01 — no new handler
//!             wiring.
//!
//! This slice is CONFIRMATORY (ADR-067 § Resolution 5): the fully-expanded
//! condition text Slice 01's own import produces is indistinguishable, to
//! `handle_create_document`/`handle_update_document`, from a hand-authored
//! condition that never used a function at all. If this slice needed ANY
//! production code, ADR-067's own central claim would be wrong.
//!
//! Driving port: admin HTTP :9090 (import) + gRPC :8080 `CreateDocument`/
//! `UpdateDocument`, via `SecurityRulesFullContext` (imported directly from
//! `security_rules_write_path`'s own fixture module — mirrors CDR04's own
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

const EDITOR_WRITE_RULES_FILE: &str = r#"
service cloud.firestore {
  function isEditor() {
    return request.auth.uid == resource.data.editor_id;
  }
  match /databases/{database}/documents {
    match /trail_guides/{guideId} {
      allow write: if isEditor();
    }
  }
}
"#;

fn trail_guide_resource_name(project_id: &str, document_id: &str) -> String {
    format!("projects/{project_id}/databases/(default)/documents/trail_guides/{document_id}")
}

/// AC-CF-05
///
/// @driving_port @real-io @US-02 @AC-CF-05
#[tokio::test]
async fn a_real_update_is_gated_correctly_by_a_function_call_bearing_condition() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cf02-update").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let import_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules/import",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": EDITOR_WRITE_RULES_FILE }))
        .send()
        .await
        .expect("import request failed");
    assert_eq!(import_resp.status().as_u16(), 200, "AC-CF-05: import must succeed");

    ctx.seed_document(
        "trail_guides",
        "trek-2026",
        serde_json::json!({ "editor_id": {"t": "S", "v": "maria-santos"} }),
    )
    .await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);
    let mut fields = std::collections::HashMap::new();
    fields.insert("title".to_string(), string_field("Hijacked"));
    let denied = update_document(
        &ctx,
        &trail_guide_resource_name(&ctx.project_id, "trek-2026"),
        fields.clone(),
        Some(&danas_token),
    )
    .await;
    let err = denied.expect_err("AC-CF-05: a non-editor's update must be denied");
    assert_eq!(err.code(), tonic::Code::PermissionDenied, "got {:?}", err.code());

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );
    let allowed = update_document(
        &ctx,
        &trail_guide_resource_name(&ctx.project_id, "trek-2026"),
        fields,
        Some(&marias_token),
    )
    .await;
    assert!(
        allowed.is_ok(),
        "AC-CF-05: the document's own editor must be allowed to update it: {:?}",
        allowed.err()
    );
}
