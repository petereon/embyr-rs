//! CF04 (Slice 04, US-04, Release 2, LAST slice) — Multiple Functions,
//! Multiple Call Sites, Every Scoped-Out Construct Named.
//!
//! Acceptance criteria verified here (feature-delta.md US-04, ADR-067):
//!   AC-CF-07: a file with 2 distinct function definitions, each called
//!             from a different `match` block, imports and enforces both
//!             correctly.
//!   AC-CF-08: the SAME function called from 2 different `match` blocks
//!             expands correctly at both call sites independently.
//!   AC-CF-09: a function definition or call site with a non-empty
//!             parameter/argument list is a NAMED rejection
//!             (`FUNCTION_PARAMETERS_UNSUPPORTED`).
//!   AC-CF-10: two function definitions sharing the same name is a NAMED
//!             rejection (`DUPLICATE_FUNCTION`).
//!   AC-CF-11: a function whose own body calls another function (nesting)
//!             is a NAMED rejection — reusing the pre-existing
//!             `CUSTOM_FUNCTION` construct tag unchanged.
//!
//! Driving port: admin HTTP :9090 (import) + gRPC :8080 `GetDocument`, via
//! `SecurityRulesFullContext`.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, SecurityRulesFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

fn resource_name(project_id: &str, path: &str) -> String {
    format!("projects/{project_id}/databases/(default)/documents/{path}")
}

/// AC-CF-07, AC-CF-08: two distinct functions, one of which is called from
/// TWO different match blocks (`trail_guides` and `photos`, both gated by
/// `isEditor()`); the other (`isOwner()`) gates a third, independent
/// collection (`journal_entries`). One import, three real `GetDocument`
/// enforcement proofs.
///
/// @driving_port @real-io @US-04 @AC-CF-07 @AC-CF-08
#[tokio::test]
async fn multiple_functions_and_multiple_call_sites_all_expand_and_enforce_independently() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cf04-multi").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let rules_file = r#"
    service cloud.firestore {
      function isEditor() {
        return request.auth.uid == resource.data.editor_id;
      }
      function isOwner() {
        return request.auth.uid == resource.data.owner_id;
      }
      match /databases/{database}/documents {
        match /trail_guides/{guideId} {
          allow read: if isEditor();
        }
        match /photos/{photoId} {
          allow read: if isEditor();
        }
        match /journal_entries/{entryId} {
          allow read: if isOwner();
        }
      }
    }
    "#;
    let import_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules/import",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": rules_file }))
        .send()
        .await
        .expect("import request failed");
    assert_eq!(import_resp.status().as_u16(), 200, "AC-CF-07/08: import must succeed");

    ctx.seed_document(
        "trail_guides",
        "trek-2026",
        serde_json::json!({ "editor_id": {"t": "S", "v": "maria-santos"} }),
    )
    .await;
    ctx.seed_document(
        "photos",
        "sunset",
        serde_json::json!({ "editor_id": {"t": "S", "v": "dana-kim"} }),
    )
    .await;
    ctx.seed_document(
        "journal_entries",
        "day-one",
        serde_json::json!({ "owner_id": {"t": "S", "v": "maria-santos"} }),
    )
    .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );
    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    // AC-CF-08: `isEditor()` at the `trail_guides` call site resolves
    // against `trail_guides/trek-2026`'s own editor_id (maria).
    let trail_guide_ok = ctx
        .get_document(&resource_name(&ctx.project_id, "trail_guides/trek-2026"), Some(&marias_token))
        .await;
    assert!(trail_guide_ok.is_ok(), "AC-CF-08: maria is trail_guides/trek-2026's own editor: {:?}", trail_guide_ok.err());

    // AC-CF-08: the SAME `isEditor()` at the `photos` call site resolves
    // INDEPENDENTLY against `photos/sunset`'s own editor_id (dana) — maria
    // is denied there, proving the two call sites don't share state.
    let photo_denied = ctx
        .get_document(&resource_name(&ctx.project_id, "photos/sunset"), Some(&marias_token))
        .await;
    let err = photo_denied.expect_err("AC-CF-08: maria is not photos/sunset's own editor, must be denied");
    assert_eq!(err.code(), tonic::Code::PermissionDenied, "got {:?}", err.code());

    let photo_ok = ctx
        .get_document(&resource_name(&ctx.project_id, "photos/sunset"), Some(&danas_token))
        .await;
    assert!(photo_ok.is_ok(), "AC-CF-08: dana IS photos/sunset's own editor: {:?}", photo_ok.err());

    // AC-CF-07: the SECOND, independently-defined function (`isOwner()`)
    // gates a third collection correctly.
    let journal_ok = ctx
        .get_document(&resource_name(&ctx.project_id, "journal_entries/day-one"), Some(&marias_token))
        .await;
    assert!(journal_ok.is_ok(), "AC-CF-07: maria is journal_entries/day-one's own owner: {:?}", journal_ok.err());
}

async fn setup_admin_only(project_id: &str) -> (SecurityRulesFullContext, String) {
    let ctx = SecurityRulesFullContext::new(project_id).await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    (ctx, cookie)
}

async fn import_and_expect_construct(ctx: &SecurityRulesFullContext, cookie: &str, rules_file: &str, expected_construct: &str) {
    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules/import",
            ctx.project_id
        )))
        .header("Cookie", cookie)
        .json(&serde_json::json!({ "rules_file": rules_file }))
        .send()
        .await
        .expect("import request failed");
    assert_ne!(resp.status().as_u16(), 200, "expected import to be rejected");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    let offending = body["offending_blocks"].as_array().expect("response must carry offending_blocks");
    assert!(
        offending.iter().any(|b| b["construct"] == expected_construct),
        "expected an offending block tagged '{expected_construct}', got: {body:?}"
    );
}

/// AC-CF-09 (definition site: a non-empty parameter list).
///
/// @error @driving_port @real-io @US-04 @AC-CF-09
#[tokio::test]
async fn a_function_definition_with_a_parameter_is_rejected_via_real_import() {
    let (ctx, cookie) = setup_admin_only("trailmark-prod-cf04-param-def").await;
    let rules_file = r#"
    service cloud.firestore {
      function isEditor(userId) { return request.auth.uid == userId; }
      match /databases/{database}/documents {
        match /trail_guides/{guideId} {
          allow read: if isEditor();
        }
      }
    }
    "#;
    import_and_expect_construct(&ctx, &cookie, rules_file, "FUNCTION_PARAMETERS_UNSUPPORTED").await;
}

/// AC-CF-09 (call site: a non-empty argument list).
///
/// @error @driving_port @real-io @US-04 @AC-CF-09
#[tokio::test]
async fn a_call_site_passing_an_argument_is_rejected_via_real_import() {
    let (ctx, cookie) = setup_admin_only("trailmark-prod-cf04-param-call").await;
    let rules_file = r#"
    service cloud.firestore {
      function isEditor() { return request.auth.uid == resource.data.editor_id; }
      match /databases/{database}/documents {
        match /trail_guides/{guideId} {
          allow read: if isEditor(guideId);
        }
      }
    }
    "#;
    import_and_expect_construct(&ctx, &cookie, rules_file, "FUNCTION_PARAMETERS_UNSUPPORTED").await;
}

/// AC-CF-10
///
/// @error @driving_port @real-io @US-04 @AC-CF-10
#[tokio::test]
async fn two_function_definitions_sharing_a_name_are_rejected_via_real_import() {
    let (ctx, cookie) = setup_admin_only("trailmark-prod-cf04-dup").await;
    let rules_file = r#"
    service cloud.firestore {
      function isEditor() { return true; }
      function isEditor() { return false; }
      match /databases/{database}/documents {
        match /trail_guides/{guideId} {
          allow read: if isEditor();
        }
      }
    }
    "#;
    import_and_expect_construct(&ctx, &cookie, rules_file, "DUPLICATE_FUNCTION").await;
}

/// AC-CF-11
///
/// @error @driving_port @real-io @US-04 @AC-CF-11
#[tokio::test]
async fn a_function_body_calling_another_function_is_rejected_via_real_import() {
    let (ctx, cookie) = setup_admin_only("trailmark-prod-cf04-nest").await;
    let rules_file = r#"
    service cloud.firestore {
      function isOwner() { return true; }
      function isEditor() { return isOwner(); }
      match /databases/{database}/documents {
        match /trail_guides/{guideId} {
          allow read: if isEditor();
        }
      }
    }
    "#;
    import_and_expect_construct(&ctx, &cookie, rules_file, "CUSTOM_FUNCTION").await;
}
