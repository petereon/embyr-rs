//! HI03 (Slice 03, US-03) — Maria Signs In With Her Hosted Email/Password
//! Credential.
//!
//! Acceptance criteria verified here
//! (slice-03-maria-signs-in-with-email-password.md):
//!   AC-18-10: valid email+password sign-in succeeds; identity attaches to a
//!             SUBSEQUENT real Firestore getDocument call carrying the
//!             returned token in x-embyr-client-identity.
//!   AC-18-11: wrong password AND unknown email are rejected with the
//!             byte-identical response shape (ONE test asserting both
//!             produce the exact same status+body — the point is proving
//!             indistinguishability, not just "both fail").
//!   AC-18-12: sign-in on a project without hosted identity enabled is
//!             rejected distinguishably (HOSTED_IDENTITY_NOT_ENABLED, not
//!             the oracle-protected 401).
//!   AC-18-13: regression guardrail — an ordinary Firestore call from a
//!             request that never signed in via ANY path (hosted or
//!             custom-token) continues to succeed unaffected. Proof
//!             obligation only — Decision 4's own structural argument says
//!             this requires zero new code.
//!
//! Driving ports: REST :8081 (signup precondition + signin) + gRPC :8080
//! (getDoc regression proof) — mirrors hi02 exactly.
//!
//! Error ratio: 2 error/edge (AC-18-11 + AC-18-12) out of 4 = 50%.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::HostedIdentityFullContext;

use embyr_proto::firestore::firestore_client::FirestoreClient;

async fn sign_up(
    ctx: &HostedIdentityFullContext,
    email: &str,
    password: &str,
) -> (u16, serde_json::Value) {
    let url = ctx.rest_url(&format!(
        "/v1/projects/{}/accounts:signUp?key={}",
        ctx.project_id, ctx.api_key
    ));
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

async fn sign_in(
    ctx: &HostedIdentityFullContext,
    key: Option<&str>,
    email: &str,
    password: &str,
) -> (u16, serde_json::Value) {
    let url = match key {
        Some(k) => ctx.rest_url(&format!(
            "/v1/projects/{}/accounts:signInWithPassword?key={}",
            ctx.project_id, k
        )),
        None => ctx.rest_url(&format!(
            "/v1/projects/{}/accounts:signInWithPassword",
            ctx.project_id
        )),
    };
    let resp = reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({"email": email, "password": password}))
        .send()
        .await
        .expect("sign_in request failed");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("sign_in body must be JSON");
    (status, body)
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-18-10: valid signin succeeds and the minted token unlocks a SUBSEQUENT
// real Firestore call
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trailmark-prod` has hosted identity enabled and Maria already
///          signed up
///   When:  Maria signs in with her correct email + password
///   Then:  200 with a localId/expiresIn/idToken, and her subsequent getDoc
///          call carrying idToken in x-embyr-client-identity succeeds
///
/// AC-18-10
///
/// @driving_port @real-io @US-03 @AC-18-10
#[tokio::test]
async fn marias_valid_signin_returns_a_token_and_her_subsequent_getdoc_call_succeeds() {
    let ctx = HostedIdentityFullContext::new("trailmark-prod-hi03-signin").await;
    ctx.enable_hosted_identity().await;
    let (signup_status, _) = sign_up(&ctx, "maria@trailmark.example", "correct-horse-battery").await;
    assert_eq!(signup_status, 201, "precondition: signup must succeed");

    let (status, body) = sign_in(
        &ctx,
        Some(&ctx.api_key),
        "maria@trailmark.example",
        "correct-horse-battery",
    )
    .await;

    assert_eq!(status, 200, "AC-18-10: valid signin must return 200");
    assert!(body.get("localId").and_then(|v| v.as_str()).is_some());
    assert_eq!(body["email"], "maria@trailmark.example");
    let expires_in: i64 = body["expiresIn"]
        .as_str()
        .expect("expiresIn must be a string")
        .parse()
        .expect("expiresIn must be a parseable integer");
    assert_eq!(expires_in, 3600, "AC-18-10: expiresIn must be the token's TTL");
    let token = body["idToken"]
        .as_str()
        .expect("AC-18-10: response must carry a usable token for the subsequent call")
        .to_string();

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
        "AC-18-10: Maria's subsequent getDoc call carrying her signed-in token must succeed: {:?}",
        get_doc_resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-18-11: wrong password AND unknown email are byte-identical rejections
// ─────────────────────────────────────────────────────────────────────────────

/// AC-18-11: proves indistinguishability, not merely "both fail" — asserts
/// the two failure responses are the exact same status + body.
///
/// @error @driving_port @real-io @US-03 @AC-18-11
#[tokio::test]
async fn wrong_password_and_unknown_email_produce_the_byte_identical_rejection() {
    let ctx = HostedIdentityFullContext::new("trailmark-prod-hi03-oracle").await;
    ctx.enable_hosted_identity().await;
    let (signup_status, _) = sign_up(&ctx, "maria@trailmark.example", "correct-horse-battery").await;
    assert_eq!(signup_status, 201, "precondition: signup must succeed");

    let wrong_password = sign_in(
        &ctx,
        Some(&ctx.api_key),
        "maria@trailmark.example",
        "totally-wrong-password",
    )
    .await;

    let unknown_email = sign_in(
        &ctx,
        Some(&ctx.api_key),
        "never-signed-up@trailmark.example",
        "any-password-at-all",
    )
    .await;

    assert_eq!(wrong_password.0, 401, "AC-18-11: wrong password must be 401");
    assert_eq!(
        wrong_password, unknown_email,
        "AC-18-11: wrong password and unknown email must be byte-identical (status + body), \
         proving no account-enumeration oracle exists"
    );
    assert_eq!(wrong_password.1["message"], "Invalid credentials");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-18-12: signin on a project without hosted identity enabled (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-18-12
///
/// @error @driving_port @real-io @US-03 @AC-18-12
#[tokio::test]
async fn signin_on_a_project_without_hosted_identity_enabled_is_rejected_distinguishably() {
    let ctx = HostedIdentityFullContext::new("trailmark-prod-hi03-notenabled").await;
    // Deliberately: enable_hosted_identity() is never called.

    let (status, body) = sign_in(
        &ctx,
        Some(&ctx.api_key),
        "anyone@trailmark.example",
        "any-password-at-all",
    )
    .await;

    assert_eq!(
        status, 400,
        "AC-18-12: signin without hosted identity enabled must be rejected"
    );
    assert_eq!(
        body["reason"], "HOSTED_IDENTITY_NOT_ENABLED",
        "AC-18-12: must be distinguishable from the oracle-protected credential rejection"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-18-13: regression guardrail — an unsigned-in request's ordinary
// Firestore call continues to succeed unaffected
// ─────────────────────────────────────────────────────────────────────────────

/// Proof-obligation test, not new feature code (Decision 4's own "physically
/// unreachable" structural argument): a request that never carries
/// x-embyr-client-identity at all must be entirely unaffected by this
/// slice's own new signin endpoint existing.
///
/// AC-18-13
///
/// @regression @driving_port @real-io @US-03 @AC-18-13
#[tokio::test]
async fn an_ordinary_firestore_call_with_no_client_identity_header_still_succeeds() {
    let ctx = HostedIdentityFullContext::new("trailmark-prod-hi03-regression").await;
    ctx.enable_hosted_identity().await;

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
    // Deliberately: no x-embyr-client-identity header at all.

    let get_doc_resp = client.get_document(request).await;
    assert!(
        get_doc_resp.is_ok(),
        "AC-18-13: an ordinary unsigned-in Firestore call must still succeed: {:?}",
        get_doc_resp.err()
    );
}
