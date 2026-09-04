//! CF01 (Slice 01, Walking Skeleton, US-01, Release 1) — Alex's Named
//! Helper Function Parses and Enforces on Reads.
//!
//! Acceptance criteria verified here (feature-delta.md US-01, ADR-067):
//!   AC-CF-01: a `function <name>() { return <expr>; }` block, declared as
//!             a sibling of the top-level `match /databases/{database}/
//!             documents { ... }` block, parses into a name -> body-text
//!             mapping.
//!   AC-CF-02: a call site `<name>()` inside an `allow` clause's condition
//!             expands, before `parse_condition` ever runs, into the
//!             function's own body text wrapped in parens.
//!   AC-CF-03: a real `GetDocument` call gated by the expanded condition
//!             allows the document's own editor and denies a different
//!             signed-in caller.
//!   AC-CF-04: a call to a name that matches no defined function anywhere
//!             in the file is a NAMED rejection (`UNDEFINED_FUNCTION`).
//!
//! Setup uses the REAL admin import endpoint (`POST .../access_rules/
//! import`), mirroring `security-rules-cel-parity`'s own cp02 setup exactly.
//!
//! Driving port: admin HTTP :9090 (import) + gRPC :8080 `GetDocument`, via
//! `SecurityRulesFullContext`.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, SecurityRulesFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

const EDITOR_RULES_FILE: &str = r#"
service cloud.firestore {
  function isEditor() {
    return request.auth.uid == resource.data.editor_id;
  }
  match /databases/{database}/documents {
    match /trail_guides/{guideId} {
      allow read: if isEditor();
    }
  }
}
"#;

fn trail_guide_resource_name(project_id: &str, document_id: &str) -> String {
    format!("projects/{project_id}/databases/(default)/documents/trail_guides/{document_id}")
}

/// AC-CF-01, AC-CF-02, AC-CF-03
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-CF-01 @AC-CF-02 @AC-CF-03
#[tokio::test]
async fn the_documents_own_editor_reads_successfully_a_different_caller_is_denied() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cf01-walking-skeleton").await;
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
        .json(&serde_json::json!({ "rules_file": EDITOR_RULES_FILE }))
        .send()
        .await
        .expect("import request failed");
    assert_eq!(
        import_resp.status().as_u16(),
        200,
        "AC-CF-01/AC-CF-02: a function definition + call site must import successfully"
    );

    ctx.seed_document(
        "trail_guides",
        "trek-2026",
        serde_json::json!({ "editor_id": {"t": "S", "v": "maria-santos"} }),
    )
    .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );
    let editor_resp = ctx
        .get_document(
            &trail_guide_resource_name(&ctx.project_id, "trek-2026"),
            Some(&marias_token),
        )
        .await;
    assert!(
        editor_resp.is_ok(),
        "AC-CF-03: the document's own editor must be allowed to read it: {:?}",
        editor_resp.err()
    );

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);
    let non_editor_resp = ctx
        .get_document(
            &trail_guide_resource_name(&ctx.project_id, "trek-2026"),
            Some(&danas_token),
        )
        .await;
    let err = non_editor_resp
        .expect_err("AC-CF-03: a different signed-in caller must be denied");
    assert_eq!(err.code(), tonic::Code::PermissionDenied, "got {:?}", err.code());
}

/// AC-CF-04
///
/// @driving_port @real-io @US-01 @AC-CF-04
#[tokio::test]
async fn a_call_to_an_undefined_function_is_rejected_at_import_time() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cf01-undefined").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let rules_file = r#"
    service cloud.firestore {
      match /databases/{database}/documents {
        match /trail_guides/{guideId} {
          allow read: if isEditor();
        }
      }
    }
    "#;
    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules/import",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": rules_file }))
        .send()
        .await
        .expect("import request failed");
    assert_ne!(
        resp.status().as_u16(),
        200,
        "AC-CF-04: a call to an undefined function must be rejected, not silently accepted"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    let offending = body["offending_blocks"]
        .as_array()
        .expect("response must carry offending_blocks");
    assert!(
        offending
            .iter()
            .any(|b| b["construct"] == "UNDEFINED_FUNCTION"),
        "AC-CF-04: expected an UNDEFINED_FUNCTION-tagged offending block, got: {body:?}"
    );
}
