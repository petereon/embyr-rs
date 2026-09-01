//! CP02 (Slice 02, Walking Skeleton — Activity B, US-02, Release 1) — A
//! Path-Captured Variable Gates a Signed-In End User's Read of Their Own
//! Document.
//!
//! Acceptance criteria verified here (feature-delta.md US-02,
//! slice-02-path-variable-read-evaluation.md):
//!   AC-17-179: a signed-in end user reading a document whose path-captured
//!              variable satisfies the rule's condition succeeds.
//!   AC-17-180: a signed-in end user reading a document whose path-captured
//!              variable does NOT satisfy the condition is denied
//!              PermissionDenied, attributable to the rule.
//!   AC-17-181: an anonymous session is evaluated against a path-variable
//!              -bound condition using the identical `request.auth == null`
//!              semantics `security-rules`'s own US-03 already established.
//!   AC-17-182: the path-captured variable is resolved per-document, never
//!              shared or cached across sibling documents in the same
//!              collection.
//!   AC-17-183: a denied read's response is identical whether or not the
//!              target document actually exists, reusing the existing
//!              existence-non-leakage mechanism (AC-17-10) unchanged.
//!
//! Setup uses the REAL admin import endpoint from Slice 01
//! (`POST .../access_rules/import`) — not a hand-seeded `access_rules` row
//! — per the DELIVER dispatch instructions, proving Slice 01's import and
//! Slice 02's evaluation compose end-to-end.
//!
//! Driving port: admin HTTP :9090 (import) + gRPC :8080 `GetDocument`, via
//! `SecurityRulesFullContext` — the same `embyr_server::start_test_server`
//! composition root `security_rules`'s own sr02 uses (Pillar 3).
//!
//! Error ratio: 4 error/edge (AC-17-180/181/182/183) out of 5 = 80%.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, SecurityRulesFullContext};
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
/// signing key registered for client-identity token verification.
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
// AC-17-179 (WALKING SKELETON — Activity B, feature-delta.md § Story Map):
// the document's own owner (by path-captured variable) reads successfully.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `profiles` has an imported rule requiring `request.auth.uid ==
///          userId` where `userId` is captured from the document's own path
///   And:   Maria Santos holds a verified identity and `profiles/maria
///          -santos` exists
///   When:  Maria calls `getDoc()` on `profiles/maria-santos`
///   Then:  the read succeeds and returns the document
///
/// AC-17-179
///
/// @walking_skeleton @driving_port @real-io @US-02 @AC-17-179
#[tokio::test]
async fn a_signed_in_end_user_reading_their_own_path_keyed_document_succeeds() {
    let (ctx, signing_key) = setup("trailmark-prod-cp02-owner").await;
    ctx.seed_document("profiles", "maria-santos", serde_json::json!({})).await;

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);

    let resp = ctx
        .get_document(&profile_resource_name(&ctx.project_id, "maria-santos"), Some(&marias_token))
        .await;

    assert!(
        resp.is_ok(),
        "AC-17-179: Maria's read of her own path-keyed document must succeed: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-180: a different signed-in end user's read of the same document is
// denied (error/edge).
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-180
///
/// @error @driving_port @real-io @US-02 @AC-17-180
#[tokio::test]
async fn a_different_signed_in_end_users_read_of_the_same_path_keyed_document_is_denied() {
    let (ctx, signing_key) = setup("trailmark-prod-cp02-nonowner").await;
    ctx.seed_document("profiles", "maria-santos", serde_json::json!({})).await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let resp = ctx
        .get_document(&profile_resource_name(&ctx.project_id, "maria-santos"), Some(&danas_token))
        .await;

    let err = resp.expect_err("AC-17-180: Dana's read of Maria's path-keyed document must be denied");
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-180: denial must be attributable to the rule (PermissionDenied), got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-181: an anonymous session is denied, reusing the existing
// `request.auth == null` fail-closed semantics.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-181
///
/// @error @driving_port @real-io @US-02 @AC-17-181
#[tokio::test]
async fn an_anonymous_session_is_denied_reusing_existing_null_auth_semantics() {
    let (ctx, _signing_key) = setup("trailmark-prod-cp02-anon").await;
    ctx.seed_document("profiles", "maria-santos", serde_json::json!({})).await;

    let resp = ctx
        .get_document(&profile_resource_name(&ctx.project_id, "maria-santos"), None)
        .await;

    let err = resp.expect_err("AC-17-181: an anonymous read of a path-keyed document must be denied");
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-181: anonymous denial must be PermissionDenied, got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-182: the captured variable is scoped to its own document, never a
// sibling.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `profiles` has the same rule, and `profiles/dana-kim` also exists
///   When:  Maria calls `getDoc()` on `profiles/dana-kim`
///   Then:  the read is denied — the `userId` captured for THIS document is
///          `"dana-kim"`, not `"maria-santos"`, so Maria's own uid does not
///          match
///
/// AC-17-182
///
/// @error @driving_port @real-io @US-02 @AC-17-182
#[tokio::test]
async fn the_captured_variable_is_scoped_to_its_own_document_never_a_sibling() {
    let (ctx, signing_key) = setup("trailmark-prod-cp02-sibling").await;
    ctx.seed_document("profiles", "maria-santos", serde_json::json!({})).await;
    ctx.seed_document("profiles", "dana-kim", serde_json::json!({})).await;

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);

    // Control: Maria reading her OWN document still succeeds (proves the
    // sibling seed above didn't corrupt per-document resolution).
    let own_resp = ctx
        .get_document(&profile_resource_name(&ctx.project_id, "maria-santos"), Some(&marias_token))
        .await;
    assert!(own_resp.is_ok(), "AC-17-182 control: Maria's own document must still succeed");

    let sibling_resp = ctx
        .get_document(&profile_resource_name(&ctx.project_id, "dana-kim"), Some(&marias_token))
        .await;
    let err = sibling_resp
        .expect_err("AC-17-182: Maria's read of Dana's sibling document must be denied");
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-182: sibling-document read must be denied, got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-183: existence non-leakage — a denied read never reveals whether the
// target document exists, reusing AC-17-10's mechanism unchanged.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-183
///
/// @error @driving_port @real-io @US-02 @AC-17-183 @security-regression
#[tokio::test]
async fn a_denied_read_never_reveals_whether_the_path_keyed_document_exists() {
    let (ctx, signing_key) = setup("trailmark-prod-cp02-nonleak").await;
    ctx.seed_document("profiles", "maria-santos", serde_json::json!({})).await;
    // profiles/does-not-exist is deliberately NEVER seeded.

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let resp_existing_wrong_owner = ctx
        .get_document(&profile_resource_name(&ctx.project_id, "maria-santos"), Some(&danas_token))
        .await;
    let resp_nonexistent = ctx
        .get_document(&profile_resource_name(&ctx.project_id, "does-not-exist"), Some(&danas_token))
        .await;

    let err_existing = resp_existing_wrong_owner
        .expect_err("AC-17-183: wrong-owner read of an EXISTING path-keyed document must be denied");
    let err_nonexistent = resp_nonexistent
        .expect_err("AC-17-183: read of a NON-EXISTENT path-keyed document must ALSO be denied");

    assert_eq!(
        err_existing.code(),
        err_nonexistent.code(),
        "AC-17-183: existence non-leakage — both denials must carry the IDENTICAL gRPC status code"
    );
    assert_eq!(
        err_existing.code(),
        tonic::Code::PermissionDenied,
        "AC-17-183: must be PermissionDenied, never NotFound (that would leak non-existence)"
    );
    assert_eq!(
        err_existing.message(),
        err_nonexistent.message(),
        "AC-17-183: existence non-leakage — both denials must carry the IDENTICAL message"
    );
}
