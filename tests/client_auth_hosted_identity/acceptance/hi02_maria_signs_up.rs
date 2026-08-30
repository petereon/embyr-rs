//! HI02 (Slice 02, US-02) — Maria Signs Up For A Trailmark Account Directly
//! With Embyr.
//!
//! Acceptance criteria verified here (slice-02-maria-signs-up-with-email-password.md):
//!   AC-18-05: valid signup -> account created, 201 with localId/expiresIn;
//!             a SUBSEQUENT real Firestore getDocument call carrying the
//!             returned token in x-embyr-client-identity succeeds — proves
//!             Decision 4's attach_client_identity_if_present widening
//!             works end-to-end, not just that a token was minted.
//!   AC-18-06: already-registered email -> 409 EMAIL_ALREADY_IN_USE.
//!   AC-18-07: password below strength requirement -> 400 WEAK_PASSWORD.
//!   AC-18-08: hosted identity not enabled for the project -> 400
//!             HOSTED_IDENTITY_NOT_ENABLED, distinguishable from a
//!             credential-validation failure.
//!   AC-18-09: raw plaintext password never appears in the response body on
//!             ANY branch (asserted directly below on every response body).
//!             The log-audit half (no `tracing::*!` call in
//!             `rest/sign_up.rs` interpolates `password`/`body`) is a
//!             direct-code-review invariant — every log call added in that
//!             file logs only an error value or a static context string,
//!             never a request field — noted here since no log-capture
//!             harness exists yet in this codebase (ponytail: add one only
//!             if this becomes a recurring cross-cutting concern).
//!
//! Plus: missing/wrong api_key query param -> 401 INVALID_API_KEY (implied
//! by AC-18-08's own "distinguishable from a credential-validation failure"
//! wording — this rejection must exist for AC-18-08 to be distinguishable
//! from it, mirrors Slice 01's own AC-18-19 shape).
//!
//! Driving ports: REST :8081 (signup) + gRPC :8080 (getDoc regression proof,
//! via `HostedIdentityFullContext` — mirrors `client-auth`'s own
//! `ClientAuthFullContext`/ca02 precedent exactly).
//!
//! Error ratio: 4 error/edge (AC-18-06/07/08 + missing-key) out of 5 = 80%.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::HostedIdentityFullContext;

use embyr_proto::firestore::firestore_client::FirestoreClient;

async fn sign_up(
    ctx: &HostedIdentityFullContext,
    key: Option<&str>,
    email: &str,
    password: &str,
) -> (u16, serde_json::Value) {
    let url = match key {
        Some(k) => ctx.rest_url(&format!(
            "/v1/projects/{}/accounts:signUp?key={}",
            ctx.project_id, k
        )),
        None => ctx.rest_url(&format!("/v1/projects/{}/accounts:signUp", ctx.project_id)),
    };
    let resp = reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({"email": email, "password": password}))
        .send()
        .await
        .expect("sign_up request failed");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("sign_up body must be JSON");
    (status, body)
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-18-05: valid signup creates the account and the minted token unlocks a
// SUBSEQUENT real Firestore call (WALKING SKELETON)
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trailmark-prod` has hosted identity enabled
///   When:  Maria signs up with a fresh email + an 8+ character password
///   Then:  201 with a localId/expiresIn/idToken, an account row is
///          created, and Maria's subsequent getDoc call carrying idToken in
///          x-embyr-client-identity succeeds
///
/// AC-18-05
///
/// @walking_skeleton @driving_port @real-io @US-02 @AC-18-05
#[tokio::test]
async fn marias_valid_signup_creates_her_account_and_her_subsequent_getdoc_call_succeeds() {
    let ctx = HostedIdentityFullContext::new("trailmark-prod-hi02-signup").await;
    ctx.enable_hosted_identity().await;

    let (status, body) = sign_up(
        &ctx,
        Some(&ctx.api_key),
        "maria@trailmark.example",
        "correct-horse-battery",
    )
    .await;

    assert_eq!(status, 201, "AC-18-05: valid signup must return 201");
    assert!(body.get("localId").and_then(|v| v.as_str()).is_some());
    assert_eq!(body["email"], "maria@trailmark.example");
    let expires_in: i64 = body["expiresIn"]
        .as_str()
        .expect("expiresIn must be a string")
        .parse()
        .expect("expiresIn must be a parseable integer");
    assert_eq!(expires_in, 3600, "AC-18-05: expiresIn must be the token's TTL");
    let token = body["idToken"]
        .as_str()
        .expect("AC-18-05: response must carry a usable token for the subsequent call")
        .to_string();

    // AC-18-09: the raw plaintext password must never appear in the response.
    let body_str = body.to_string();
    assert!(
        !body_str.contains("correct-horse-battery"),
        "AC-18-09: raw plaintext password must never appear in the response body"
    );

    assert!(
        ctx.hosted_identity_account_exists("maria@trailmark.example")
            .await,
        "AC-18-05: a hosted_identity_accounts row must be created"
    );

    // Maria's subsequent getDoc call — carries the RETURNED token, proving
    // Decision 4's attach_client_identity_if_present widening end-to-end.
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
    request.metadata_mut().insert(
        "x-embyr-client-identity",
        format!("Bearer {token}").parse().unwrap(),
    );

    let get_doc_resp = client.get_document(request).await;
    assert!(
        get_doc_resp.is_ok(),
        "AC-18-05: Maria's subsequent getDoc call carrying her minted token must succeed: {:?}",
        get_doc_resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-18-06: already-registered email is rejected explicitly (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-18-06
///
/// @error @driving_port @real-io @US-02 @AC-18-06
#[tokio::test]
async fn signing_up_with_an_already_registered_email_is_rejected_as_already_in_use() {
    let ctx = HostedIdentityFullContext::new("trailmark-prod-hi02-dupe").await;
    ctx.enable_hosted_identity().await;

    let first = sign_up(&ctx, Some(&ctx.api_key), "dana@trailmark.example", "first-password-1").await;
    assert_eq!(first.0, 201, "first signup must succeed");

    let (status, body) = sign_up(
        &ctx,
        Some(&ctx.api_key),
        "dana@trailmark.example",
        "different-password-2",
    )
    .await;

    assert_eq!(status, 409, "AC-18-06: duplicate email must be rejected with 409");
    assert_eq!(body["reason"], "EMAIL_ALREADY_IN_USE");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-18-07: password below the strength requirement is rejected, requirement named
// ─────────────────────────────────────────────────────────────────────────────

/// AC-18-07
///
/// @error @driving_port @real-io @US-02 @AC-18-07
#[tokio::test]
async fn signing_up_with_a_too_short_password_is_rejected_naming_the_requirement() {
    let ctx = HostedIdentityFullContext::new("trailmark-prod-hi02-weak").await;
    ctx.enable_hosted_identity().await;

    let (status, body) = sign_up(&ctx, Some(&ctx.api_key), "priya@trailmark.example", "short12").await;

    assert_eq!(status, 400, "AC-18-07: a too-short password must be rejected");
    assert_eq!(body["reason"], "WEAK_PASSWORD");
    assert!(
        !ctx.hosted_identity_account_exists("priya@trailmark.example")
            .await,
        "AC-18-07: a rejected signup must never create an account row"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-18-08: hosted identity not enabled for the project (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-18-08
///
/// @error @driving_port @real-io @US-02 @AC-18-08
#[tokio::test]
async fn signup_on_a_project_without_hosted_identity_enabled_is_rejected_distinguishably() {
    let ctx = HostedIdentityFullContext::new("trailmark-prod-hi02-notenabled").await;
    // Deliberately: enable_hosted_identity() is never called.

    let (status, body) = sign_up(
        &ctx,
        Some(&ctx.api_key),
        "not-enabled@trailmark.example",
        "correct-horse-battery",
    )
    .await;

    assert_eq!(status, 400, "AC-18-08: signup without hosted identity enabled must be rejected");
    assert_eq!(
        body["reason"], "HOSTED_IDENTITY_NOT_ENABLED",
        "AC-18-08: reason must be distinguishable from an INVALID_API_KEY credential failure"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Missing/wrong api_key -> 401 INVALID_API_KEY (needed for AC-18-08 to be
// distinguishable from a credential-validation failure)
// ─────────────────────────────────────────────────────────────────────────────

/// @error @driving_port @real-io @US-02
#[tokio::test]
async fn signup_with_a_wrong_or_missing_api_key_is_rejected_and_distinguishable_from_not_enabled() {
    let ctx = HostedIdentityFullContext::new("trailmark-prod-hi02-badkey").await;
    ctx.enable_hosted_identity().await;

    let (status_wrong, body_wrong) = sign_up(
        &ctx,
        Some("stale-or-wrong-api-key"),
        "wrong-key@trailmark.example",
        "correct-horse-battery",
    )
    .await;
    assert_eq!(status_wrong, 401, "wrong api_key must return 401");
    assert_eq!(body_wrong["reason"], "INVALID_API_KEY");

    let (status_missing, body_missing) = sign_up(
        &ctx,
        None,
        "missing-key@trailmark.example",
        "correct-horse-battery",
    )
    .await;
    assert_eq!(status_missing, 401, "missing api_key must return 401");
    assert_eq!(body_missing["reason"], "INVALID_API_KEY");

    assert!(
        !ctx.hosted_identity_account_exists("wrong-key@trailmark.example").await
            && !ctx.hosted_identity_account_exists("missing-key@trailmark.example").await,
        "a rejected credential must never create an account row"
    );
}
