//! CP03 (Slice 03, Walking Skeleton — Activity B, US-03, Release 1) — The
//! Same Path-Captured Variable Gates Writes, Not Just Reads.
//!
//! Acceptance criteria verified here (feature-delta.md US-03,
//! slice-03-path-variable-write-evaluation.md):
//!   AC-17-184: the document's own owner (by path-captured variable) can
//!              create/update/delete their own path-keyed document.
//!   AC-17-185: a different signed-in end user is denied create/update
//!              /delete against a path-keyed document that is not theirs,
//!              before the write reaches the storage adapter.
//!   AC-17-186: the path-captured variable resolves identically for create
//!              (no pre-existing document) as for update/delete (document
//!              exists) — derived from the request's own target path, never
//!              from fetched document content.
//!   AC-17-187: a denied write has zero observable side effect (no partial
//!              write, no state change) — proven the same way
//!              `security-rules-write-path`'s own tests prove it (a denied
//!              write is `PermissionDenied`, returned by `evaluate_write_*`
//!              structurally BEFORE the handler's own adapter write call is
//!              ever reached — see `delete_gated_by_resource.rs`/
//!              `update_gated_by_both_resource_states.rs` for the precedent).
//!
//! Setup uses the REAL admin import endpoint from Slice 01
//! (`POST .../access_rules/import`), mirroring CP02's own setup exactly —
//! the SAME imported `profiles/{userId}` rule (`allow read, write`) proves
//! both verbs are gated by ONE import.
//!
//! Driving port: admin HTTP :9090 (import) + gRPC :8080
//! `CreateDocument`/`UpdateDocument`/`DeleteDocument`, via
//! `SecurityRulesFullContext` (imported directly from
//! `security_rules_write_path`'s own fixture module — see that module's
//! doc comment for why not re-exported via this feature's own `common/mod.rs`).
//!
//! Error ratio: 2 error/edge (AC-17-185/187 folded into 2 of the 4 tests) out
//! of 4 = 50%; every test also carries an allow-case, mirroring the slice
//! brief's own 4 UAT scenarios exactly (1:1, no scenario invented).

#![allow(unused_imports)]

#[path = "../../security_rules_write_path/common/mod.rs"]
mod common;
use common::{
    create_document, delete_document, mint_client_identity_token, now_unix, string_field,
    update_document, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

const PROFILES_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /profiles/{userId} {
      allow read, write: if request.auth.uid == userId;
    }
  }
}
"#;

/// Shared setup: a fresh `SecurityRulesFullContext`, the `profiles` rule
/// imported via Slice 01's real admin endpoint (not hand-seeded), and a
/// signing key registered for client-identity token verification — mirrors
/// CP02's own `setup()` exactly (the SAME imported rule gates both verbs).
async fn setup(project_id: &str) -> (SecurityRulesFullContext, SigningKey) {
    let ctx = SecurityRulesFullContext::new(project_id).await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{project_id}/access_rules/import")))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": PROFILES_RULES_FILE }))
        .send()
        .await
        .expect("import request failed");
    assert_eq!(resp.status().as_u16(), 200, "setup: profiles rule import must succeed");

    (ctx, signing_key)
}

fn profile_resource_name(project_id: &str, document_id: &str) -> String {
    format!("projects/{project_id}/databases/(default)/documents/profiles/{document_id}")
}

// ─────────────────────────────────────────────────────────────────────────────
// UAT Scenario 1 (AC-17-184): the document's own owner can update their
// path-keyed document.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `profiles` has the imported rule requiring `request.auth.uid ==
///          userId`
///   And:   Maria Santos holds a verified identity and `profiles/maria
///          -santos` exists
///   When:  Maria calls `updateDoc()` on `profiles/maria-santos`
///   Then:  the write succeeds
///
/// AC-17-184
///
/// @walking_skeleton @driving_port @real-io @US-03 @AC-17-184
#[tokio::test]
async fn the_documents_own_owner_can_update_their_path_keyed_document() {
    let (ctx, signing_key) = setup("trailmark-prod-cp03-update-owner").await;
    ctx.seed_document("profiles", "maria-santos", serde_json::json!({})).await;

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);

    let mut fields = std::collections::HashMap::new();
    fields.insert("display_name".to_string(), string_field("Maria S."));

    let resp = update_document(
        &ctx,
        &profile_resource_name(&ctx.project_id, "maria-santos"),
        fields,
        Some(&marias_token),
    )
    .await;

    assert!(
        resp.is_ok(),
        "AC-17-184: the owner's update of their own path-keyed document must succeed: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// UAT Scenario 2 (AC-17-185, AC-17-187): a different signed-in end user
// cannot update someone else's path-keyed document, denied before any
// change reaches storage.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: the same `profiles` rule
///   And:   Dana Kim holds a verified identity distinct from `maria-santos`
///   When:  Dana calls `updateDoc()` on `profiles/maria-santos`
///   Then:  the write is denied with PermissionDenied, before any change
///          reaches storage (AC-17-187: structurally proven — the handler's
///          `evaluate()` `Deny` arm returns `Err` before its own
///          `adapter.update_document(...)` call is ever reached, the exact
///          mechanism `security-rules-write-path`'s own equivalent tests
///          rely on)
///
/// AC-17-185, AC-17-187
///
/// @error @driving_port @real-io @US-03 @AC-17-185 @AC-17-187
#[tokio::test]
async fn a_different_signed_in_end_user_cannot_update_someone_elses_path_keyed_document() {
    let (ctx, signing_key) = setup("trailmark-prod-cp03-update-nonowner").await;
    ctx.seed_document("profiles", "maria-santos", serde_json::json!({})).await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let mut fields = std::collections::HashMap::new();
    fields.insert("display_name".to_string(), string_field("Hijacked"));

    let resp = update_document(
        &ctx,
        &profile_resource_name(&ctx.project_id, "maria-santos"),
        fields,
        Some(&danas_token),
    )
    .await;

    let err = resp.expect_err(
        "AC-17-185: a non-owner's update of someone else's path-keyed document must be denied",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-185/187: denial must be attributable to the rule (PermissionDenied) — the same \
         status the handler returns BEFORE any write reaches the storage adapter, got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// UAT Scenario 3 (AC-17-184, AC-17-185, AC-17-186): creating a new
// path-keyed document is gated identically to updating one.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: the same `profiles` rule and NO document yet exists at
///          `profiles/maria-santos`
///   When:  Maria calls `createDoc()` (Firestore's `setDoc()`-equivalent) to
///          create `profiles/maria-santos`
///   Then:  the create succeeds (AC-17-186: `userId` resolves from the
///          request's own TARGET path, not fetched content — there is
///          nothing to fetch yet); the identical call from Dana at that same
///          path is denied (AC-17-185)
///
/// AC-17-184, AC-17-185, AC-17-186
///
/// @driving_port @real-io @US-03 @AC-17-184 @AC-17-185 @AC-17-186
#[tokio::test]
async fn creating_a_new_path_keyed_document_is_gated_identically_to_updating_one() {
    let (ctx, signing_key) = setup("trailmark-prod-cp03-create").await;
    // profiles/maria-santos deliberately NEVER seeded — Create is the FIRST
    // write to this document.

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);
    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let dana_resp = create_document(
        &ctx,
        "profiles",
        "maria-santos",
        std::collections::HashMap::new(),
        Some(&danas_token),
    )
    .await;
    let err = dana_resp.expect_err(
        "AC-17-185: Dana creating a path-keyed document that isn't hers must be denied",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-185: denial must be attributable to the rule, got {:?}",
        err.code()
    );

    let maria_resp = create_document(
        &ctx,
        "profiles",
        "maria-santos",
        std::collections::HashMap::new(),
        Some(&marias_token),
    )
    .await;
    assert!(
        maria_resp.is_ok(),
        "AC-17-184/186: Maria creating her own path-keyed document (no pre-existing document) \
         must succeed — `userId` resolves from the target path, not fetched content: {:?}",
        maria_resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// UAT Scenario 4 (AC-17-184, AC-17-185): deleting a path-keyed document is
// gated identically.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: the same `profiles` rule and `profiles/maria-santos` exists
///   When:  Dana calls `deleteDoc()` on `profiles/maria-santos`
///   Then:  the delete is denied; Maria's identical call succeeds
///
/// AC-17-184, AC-17-185
///
/// @driving_port @real-io @US-03 @AC-17-184 @AC-17-185
#[tokio::test]
async fn deleting_a_path_keyed_document_is_gated_identically() {
    let (ctx, signing_key) = setup("trailmark-prod-cp03-delete").await;
    ctx.seed_document("profiles", "maria-santos", serde_json::json!({})).await;

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);
    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let dana_resp = delete_document(
        &ctx,
        &profile_resource_name(&ctx.project_id, "maria-santos"),
        Some(&danas_token),
    )
    .await;
    let err =
        dana_resp.expect_err("AC-17-185: Dana deleting Maria's path-keyed document must be denied");
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-185: denial must be attributable to the rule, got {:?}",
        err.code()
    );

    let maria_resp = delete_document(
        &ctx,
        &profile_resource_name(&ctx.project_id, "maria-santos"),
        Some(&marias_token),
    )
    .await;
    assert!(
        maria_resp.is_ok(),
        "AC-17-184: Maria deleting her own path-keyed document must succeed: {:?}",
        maria_resp.err()
    );
}
