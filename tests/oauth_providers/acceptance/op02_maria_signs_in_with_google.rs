//! OP02 (Slice 02, LAST slice, US-02) — Maria Signs In With Her Google
//! Account.
//!
//! Acceptance criteria verified here
//! (slice-02-maria-signs-in-with-google.md, ADR-037):
//!   AC-19-05: valid unexpired Google ID token (aud matches registered
//!             client_id) -> 200; the verified identity attaches to a
//!             SUBSEQUENT real Firestore getDoc call (WALKING SKELETON).
//!   AC-19-06: the SAME Google account (same `sub`) signing in twice
//!             produces the IDENTICAL `localId` both times.
//!   AC-19-07: `aud` mismatch -> 400 AUDIENCE_MISMATCH, distinguishable
//!             from GOOGLE_SIGN_IN_NOT_ENABLED.
//!   AC-19-08: sign-in on a project with no Google Client ID registered ->
//!             400 GOOGLE_SIGN_IN_NOT_ENABLED, distinguishable from a
//!             token-validation failure.
//!   AC-19-09: expired ID token -> 400 ID_TOKEN_EXPIRED, distinguishable
//!             from a signature/malformed/audience failure.
//!   AC-19-10: regression guardrail — a Firestore call from a session that
//!             never signed in via any path continues to succeed exactly
//!             as before.
//!   AC-19-11: Google's JWKS temporarily unreachable -> 503
//!             GOOGLE_JWKS_UNREACHABLE, never a crash, never a
//!             silently-accepted unverified token.
//!
//! Driving ports: REST :8081 (signInWithIdp) + gRPC :8080 (getDoc
//! regression proofs, via `OAuthProviderFullContext` — mirrors
//! `client-auth-hosted-identity`'s own `HostedIdentityFullContext`/hi02
//! precedent exactly).
//!
//! Test infrastructure: `MockJwksServer` stands up a REAL local JWKS HTTP
//! endpoint backed by a REAL RSA keypair, and mints REAL RS256-signed
//! Google-ID-token-shaped JWTs — never a live call to Google, never a
//! hand-mocked verification result (per this codebase's own
//! "driven external / non-deterministic -> fake with output capture"
//! convention).
//!
//! Error ratio: 4 error/edge (AC-19-07/08/09/11) out of 6 scenarios = 67%.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{MockJwksServer, OAuthProviderFullContext};

use embyr_proto::firestore::firestore_client::FirestoreClient;

const CLIENT_ID: &str = "123456789-abc.apps.googleusercontent.com";

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

async fn sign_in_with_idp(
    ctx: &OAuthProviderFullContext,
    id_token: &str,
) -> (u16, serde_json::Value) {
    let url = ctx.rest_url(&format!(
        "/v1/projects/{}/accounts:signInWithIdp",
        ctx.project_id
    ));
    let resp = reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({"idToken": id_token}))
        .send()
        .await
        .expect("sign_in_with_idp request failed");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("sign_in_with_idp body must be JSON");
    (status, body)
}

/// Real gRPC getDoc call, optionally carrying `x-embyr-client-identity`.
async fn get_doc_succeeds(ctx: &OAuthProviderFullContext, client_identity_token: Option<&str>) -> bool {
    let channel = tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
        .expect("valid endpoint")
        .connect()
        .await
        .expect("connect to gRPC server");
    let mut client = FirestoreClient::new(channel);
    let mut request = tonic::Request::new(embyr_proto::firestore::GetDocumentRequest {
        name: ctx.document_resource_name(),
        ..Default::default()
    });
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {}", ctx.api_key).parse().unwrap(),
    );
    if let Some(token) = client_identity_token {
        request.metadata_mut().insert(
            "x-embyr-client-identity",
            format!("Bearer {token}").parse().unwrap(),
        );
    }

    client.get_document(request).await.is_ok()
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-19-05: valid sign-in mints a token that unlocks a SUBSEQUENT real
// Firestore call (WALKING SKELETON)
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trailmark-prod` has Google sign-in registered for `CLIENT_ID`
///   When:  Maria presents a valid, unexpired Google ID token whose `aud`
///          matches `CLIENT_ID`
///   Then:  200 with a localId/idToken/expiresIn, and Maria's subsequent
///          getDoc call carrying idToken in x-embyr-client-identity succeeds
///
/// AC-19-05
///
/// @walking_skeleton @driving_port @real-io @US-02 @AC-19-05
#[tokio::test]
async fn marias_valid_google_sign_in_unlocks_her_subsequent_getdoc_call() {
    let jwks_server = MockJwksServer::start().await;
    let ctx = OAuthProviderFullContext::new("trailmark-prod-op02-signin", jwks_server.jwks_url())
        .await;
    ctx.register_google_oauth_provider(CLIENT_ID).await;

    let id_token = jwks_server.mint_id_token("maria-google-sub", CLIENT_ID, now_unix() + 3600);
    let (status, body) = sign_in_with_idp(&ctx, &id_token).await;

    assert_eq!(status, 200, "AC-19-05: valid Google sign-in must return 200");
    assert_eq!(body["localId"], "google:maria-google-sub");
    let expires_in: i64 = body["expiresIn"]
        .as_str()
        .expect("expiresIn must be a string")
        .parse()
        .expect("expiresIn must be a parseable integer");
    assert!(expires_in > 0, "AC-19-05: expiresIn must be the token's positive TTL");
    let token = body["idToken"]
        .as_str()
        .expect("AC-19-05: response must carry a usable token for the subsequent call")
        .to_string();

    assert!(
        get_doc_succeeds(&ctx, Some(&token)).await,
        "AC-19-05: Maria's subsequent getDoc call carrying her minted token must succeed"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-19-06: the same Google account signing in twice resolves to the
// identical localId
// ─────────────────────────────────────────────────────────────────────────────

/// AC-19-06
///
/// @driving_port @real-io @US-02 @AC-19-06
#[tokio::test]
async fn the_same_google_account_signing_in_twice_resolves_to_the_identical_local_id() {
    let jwks_server = MockJwksServer::start().await;
    let ctx = OAuthProviderFullContext::new("trailmark-prod-op02-repeat", jwks_server.jwks_url())
        .await;
    ctx.register_google_oauth_provider(CLIENT_ID).await;

    let first_token = jwks_server.mint_id_token("dana-google-sub", CLIENT_ID, now_unix() + 3600);
    let (status_first, body_first) = sign_in_with_idp(&ctx, &first_token).await;
    assert_eq!(status_first, 200, "first sign-in must succeed");

    let second_token = jwks_server.mint_id_token("dana-google-sub", CLIENT_ID, now_unix() + 7200);
    let (status_second, body_second) = sign_in_with_idp(&ctx, &second_token).await;
    assert_eq!(status_second, 200, "second sign-in must succeed");

    assert_eq!(
        body_first["localId"], body_second["localId"],
        "AC-19-06: the same Google account must resolve to the identical localId both times"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-19-07: audience mismatch is rejected, distinguishable from not-enabled
// (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-19-07
///
/// @error @driving_port @real-io @US-02 @AC-19-07
#[tokio::test]
async fn a_token_whose_audience_does_not_match_the_registered_client_id_is_rejected() {
    let jwks_server = MockJwksServer::start().await;
    let ctx = OAuthProviderFullContext::new("trailmark-prod-op02-audmismatch", jwks_server.jwks_url())
        .await;
    ctx.register_google_oauth_provider(CLIENT_ID).await;

    let wrong_aud_token =
        jwks_server.mint_id_token("maria-google-sub", "some-other-client-id.apps.googleusercontent.com", now_unix() + 3600);
    let (status, body) = sign_in_with_idp(&ctx, &wrong_aud_token).await;

    assert_eq!(status, 400, "AC-19-07: audience mismatch must be rejected with 400");
    assert_eq!(body["reason"], "AUDIENCE_MISMATCH");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-19-08: sign-in on a project with no Google Client ID registered
// (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-19-08
///
/// @error @driving_port @real-io @US-02 @AC-19-08
#[tokio::test]
async fn sign_in_on_a_project_without_google_sign_in_registered_is_rejected_distinguishably() {
    let jwks_server = MockJwksServer::start().await;
    let ctx = OAuthProviderFullContext::new("trailmark-prod-op02-notenabled", jwks_server.jwks_url())
        .await;
    // Deliberately: register_google_oauth_provider() is never called.

    let id_token = jwks_server.mint_id_token("maria-google-sub", CLIENT_ID, now_unix() + 3600);
    let (status, body) = sign_in_with_idp(&ctx, &id_token).await;

    assert_eq!(status, 400, "AC-19-08: sign-in without Google sign-in registered must be rejected");
    assert_eq!(
        body["reason"], "GOOGLE_SIGN_IN_NOT_ENABLED",
        "AC-19-08: reason must be distinguishable from a token-validation failure"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-19-09: expired ID token is rejected, distinguishable from every other
// reason (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-19-09
///
/// @error @driving_port @real-io @US-02 @AC-19-09
#[tokio::test]
async fn an_expired_google_id_token_is_rejected_distinguishably() {
    let jwks_server = MockJwksServer::start().await;
    let ctx = OAuthProviderFullContext::new("trailmark-prod-op02-expired", jwks_server.jwks_url())
        .await;
    ctx.register_google_oauth_provider(CLIENT_ID).await;

    let expired_token = jwks_server.mint_id_token("maria-google-sub", CLIENT_ID, now_unix() - 3600);
    let (status, body) = sign_in_with_idp(&ctx, &expired_token).await;

    assert_eq!(status, 400, "AC-19-09: an expired token must be rejected with 400");
    assert_eq!(body["reason"], "ID_TOKEN_EXPIRED");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-19-10: regression guardrail — a session that never signed in via any
// path continues to succeed exactly as before (pure regression proof)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-19-10
///
/// @error @driving_port @real-io @US-02 @AC-19-10
#[tokio::test]
async fn a_firestore_call_with_no_client_identity_header_continues_to_succeed_unaffected() {
    let jwks_server = MockJwksServer::start().await;
    let ctx = OAuthProviderFullContext::new("trailmark-prod-op02-regression", jwks_server.jwks_url())
        .await;
    ctx.register_google_oauth_provider(CLIENT_ID).await;
    // Deliberately: no sign-in of any kind happens for this getDoc call —
    // no x-embyr-client-identity header at all.

    assert!(
        get_doc_succeeds(&ctx, None).await,
        "AC-19-10: a getDoc call from a session that never signed in via any \
         path must continue to succeed exactly as before this feature shipped"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-19-11: Google's JWKS temporarily unreachable (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-19-11
///
/// @error @driving_port @real-io @US-02 @AC-19-11
#[tokio::test]
async fn jwks_unreachable_fails_gracefully_never_crashes_never_accepts_unverified() {
    // Deliberately: point GoogleJwksCache at an address nothing listens on
    // (simulates unreachability) rather than starting a MockJwksServer.
    let ctx = OAuthProviderFullContext::new(
        "trailmark-prod-op02-jwksdown",
        "http://127.0.0.1:9/certs".to_string(),
    )
    .await;
    ctx.register_google_oauth_provider(CLIENT_ID).await;

    // The token's own content is irrelevant here — the JWKS fetch is
    // attempted (and fails) before any signature verification happens.
    let placeholder_token = "not-even-a-real-jwt";
    let (status, body) = sign_in_with_idp(&ctx, placeholder_token).await;

    assert_eq!(status, 503, "AC-19-11: JWKS unreachable must be rejected with 503");
    assert_eq!(body["reason"], "GOOGLE_JWKS_UNREACHABLE");
}
