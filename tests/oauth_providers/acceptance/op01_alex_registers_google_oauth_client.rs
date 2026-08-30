//! OP01 (Slice 01, US-01) — Alex Registers His Google OAuth Client ID For
//! Trailmark.
//!
//! Acceptance criteria verified here
//! (slice-01-alex-registers-google-oauth-client.md, ADR-037):
//!   AC-19-01: valid first-time registration -> 201; both
//!             `oauth_provider_credentials` and `oauth_signing_keys` rows
//!             exist.
//!   AC-19-02: re-registration with a DIFFERENT client_id -> 200; the
//!             STORED client_id is the new one; the signing key's
//!             public_key/private_key_enc are byte-identical before/after
//!             (no regeneration).
//!   AC-19-03: missing/invalid admin session -> 401.
//!   AC-19-04: registration for a non-existent/deleted project -> 404.
//!
//! Plus a Viewer-role boundary scenario (403), mirroring
//! `client_identity.rs`/`hosted_identity.rs`'s own established precedent.
//!
//! Driving port: Admin HTTP :9090 (`OAuthProviderAdminContext`, real
//! `build_admin_router` composition root).
//!
//! Error ratio: 3 error/edge (AC-19-03/04 + Viewer) out of 5 scenarios = 60%.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{assert_state_delta, set_to, universe, OAuthProviderAdminContext};
use std::collections::HashMap;

const CLIENT_ID: &str = "123456789-abc.apps.googleusercontent.com";
const NEW_CLIENT_ID: &str = "987654321-xyz.apps.googleusercontent.com";

// ─────────────────────────────────────────────────────────────────────────────
// AC-19-01: valid first-time registration activates Google sign-in
// (WALKING SKELETON)
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — opening Given of this feature's story line; AC-19-02
/// reuses this registration as its own precondition, Pillar 2):
///   Given: project `trailmark-prod` exists, no Google OAuth client
///          registered yet
///   When:  Alex registers his Google Client ID using a valid admin session
///   Then:  a provider-credential row AND a signing-key row are both
///          created (Google sign-in is "active")
///
/// AC-19-01
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-19-01
#[tokio::test]
async fn alex_registers_google_client_id_and_both_rows_are_created() {
    let ctx = OAuthProviderAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let before: HashMap<&str, bool> = HashMap::from([
        (
            universe::CREDENTIAL_ROW_EXISTS,
            ctx.credential_row_exists("trailmark-prod").await,
        ),
        (
            universe::SIGNING_KEY_ROW_EXISTS,
            ctx.signing_key_row_exists("trailmark-prod").await,
        ),
    ]);

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/oauth_providers/google"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"client_id": CLIENT_ID}))
        .send()
        .await
        .expect("register request failed");

    assert_eq!(
        resp.status().as_u16(),
        201,
        "AC-19-01: valid first-time registration must return 201"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["project_id"], "trailmark-prod");
    assert_eq!(body["provider"], "google");
    assert_eq!(body["client_id"], CLIENT_ID);
    assert!(body.get("created_at").is_some());
    assert!(
        body.get("public_key").is_none() && body.get("private_key_enc").is_none(),
        "AC-19-01: no signing material of any kind must appear in the response"
    );

    let after: HashMap<&str, bool> = HashMap::from([
        (
            universe::CREDENTIAL_ROW_EXISTS,
            ctx.credential_row_exists("trailmark-prod").await,
        ),
        (
            universe::SIGNING_KEY_ROW_EXISTS,
            ctx.signing_key_row_exists("trailmark-prod").await,
        ),
    ]);
    let mut expected = HashMap::new();
    expected.insert(universe::CREDENTIAL_ROW_EXISTS, set_to(true));
    expected.insert(universe::SIGNING_KEY_ROW_EXISTS, set_to(true));
    assert_state_delta(
        &before,
        &after,
        &[
            universe::CREDENTIAL_ROW_EXISTS,
            universe::SIGNING_KEY_ROW_EXISTS,
        ],
        &expected,
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-19-02: redefine with a different client_id — stored value changes,
// signing key never regenerates
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — Given reuses this file's own walking-skeleton
/// registration, then re-registers with a DIFFERENT client_id):
///   Given: `trailmark-prod` already has Google OAuth registered with
///          `CLIENT_ID`
///   When:  Alex re-registers with `NEW_CLIENT_ID`
///   Then:  the call succeeds (200), the STORED client_id is now
///          `NEW_CLIENT_ID` (read back, not just asserted from the 200), and
///          the signing key's public_key/private_key_enc are byte-identical
///          to before the redefine
///
/// AC-19-02
///
/// @driving_port @real-io @US-01 @AC-19-02
#[tokio::test]
async fn redefining_the_client_id_updates_storage_without_regenerating_the_signing_key() {
    let ctx = OAuthProviderAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let first = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/oauth_providers/google"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"client_id": CLIENT_ID}))
        .send()
        .await
        .expect("first register request failed");
    assert_eq!(
        first.status().as_u16(),
        201,
        "first registration must succeed"
    );
    let first_signing_material = ctx
        .signing_key_material("trailmark-prod")
        .await
        .expect("signing key row must exist after first registration");

    let second = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/oauth_providers/google"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"client_id": NEW_CLIENT_ID}))
        .send()
        .await
        .expect("second register request failed");
    assert_eq!(
        second.status().as_u16(),
        200,
        "AC-19-02: redefine with a different client_id must return 200"
    );
    let body: serde_json::Value = second.json().await.expect("response body must be JSON");
    assert_eq!(body["client_id"], NEW_CLIENT_ID);

    let stored_client_id = ctx
        .stored_client_id("trailmark-prod")
        .await
        .expect("credential row must still exist after redefine");
    assert_eq!(
        stored_client_id, NEW_CLIENT_ID,
        "AC-19-02: the STORED client_id must be the new one, not the stale first one"
    );

    let second_signing_material = ctx
        .signing_key_material("trailmark-prod")
        .await
        .expect("signing key row must still exist after redefine");
    assert_eq!(
        first_signing_material, second_signing_material,
        "AC-19-02: redefining the client_id must never regenerate the signing key — \
         public_key/private_key_enc must be byte-identical"
    );

    let row_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM oauth_provider_credentials WHERE project_id = $1")
            .bind("trailmark-prod")
            .fetch_one(&ctx.pool)
            .await
            .expect("count rows");
    assert_eq!(row_count, 1, "AC-19-02: no duplicate row must be created");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-19-03: missing/invalid admin session (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-19-03
///
/// @error @driving_port @real-io @US-01 @AC-19-03
#[tokio::test]
async fn registration_without_a_valid_session_is_rejected_same_as_any_other_admin_endpoint() {
    let ctx = OAuthProviderAdminContext::new().await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/oauth_providers/google"))
        // No Cookie header at all.
        .json(&serde_json::json!({"client_id": CLIENT_ID}))
        .send()
        .await
        .expect("register request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "AC-19-03: missing session must be rejected exactly as other admin endpoints reject it"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-19-04: registration against a non-existent/deleted project (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-19-04
///
/// @error @driving_port @real-io @US-01 @AC-19-04
#[tokio::test]
async fn registration_against_a_non_existent_project_is_rejected_as_not_found() {
    let ctx = OAuthProviderAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    // trailmark-staging-old is never inserted.

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-staging-old/oauth_providers/google"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"client_id": CLIENT_ID}))
        .send()
        .await
        .expect("register request failed");

    assert_eq!(
        resp.status().as_u16(),
        404,
        "AC-19-04: non-existent project must return 404"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Boundary: a Viewer (not Owner/Admin) cannot register a Google OAuth client
// ─────────────────────────────────────────────────────────────────────────────

/// Boundary scenario — role gate mirrors
/// `client_identity.rs::register_client_identity_credential`'s and
/// `hosted_identity.rs::enable_hosted_identity`'s identical Owner/Admin-only
/// precedent (this codebase's established convention for every admin write
/// action in this Identity track).
///
/// @error @driving_port @real-io @US-01
#[tokio::test]
async fn a_viewer_role_cannot_register_a_google_oauth_client() {
    let ctx = OAuthProviderAdminContext::new().await;
    let cookie = ctx.seed_session("viewer@trailmark.example", "Viewer").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/oauth_providers/google"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"client_id": CLIENT_ID}))
        .send()
        .await
        .expect("register request failed");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "Viewer role must not be able to register a Google OAuth client"
    );
}
