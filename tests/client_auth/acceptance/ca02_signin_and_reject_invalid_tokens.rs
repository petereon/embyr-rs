//! CA02 (Slice 02, Walking Skeleton half B, US-02, Release 1) — A Trailmark
//! User's Verified Identity Unlocks Their Firestore Session.
//!
//! This is the feature's acceptance-test center of gravity (feature-delta.md
//! DESIGN § Handoff Package flag #1): the "unsigned-in session unaffected"
//! guardrail (AC-16-08) is THE single most important scenario in this
//! entire feature, per DISTILL dispatch instructions.
//!
//! Acceptance criteria verified here:
//!   AC-16-06: valid token -> sign-in succeeds, subsequent Firestore call
//!             still succeeds (identity-carrying, observable bound per
//!             feature scope — Security Rules is out of scope, so there is
//!             no authorization-gating observable to assert beyond "still
//!             works", per feature-delta.md § Out of Scope).
//!   AC-16-07: missing/malformed/expired/wrong-project token -> rejected,
//!             each of the four reasons distinguishable from the other three.
//!   AC-16-08: (a) the full existing 72-scenario embyr-rs suite passes
//!             UNMODIFIED alongside these — run separately, see the module
//!             doc comment below for the exact command and its result.
//!             (b) valid api_key + EXPIRED x-embyr-client-identity still
//!             succeeds on an ordinary getDoc (failure branch never rejects).
//!             (c) header absent -> zero calls into embyr_core::client_identity
//!             (structural unreachability, not just unobserved effect) —
//!             proved two ways: the pure-function half in
//!             grpc/handler.rs::client_identity_extension_tests, and the
//!             black-box half here (a getDoc call with NO client-identity
//!             header succeeds — if the new branch were reachable without
//!             the header, it would call the RED scaffold, which panics,
//!             which would make this test fail; a pass is therefore
//!             affirmative proof the branch was never entered).
//!   AC-16-09: verified identity available to embyr's own request handling
//!             for the signed-in session's duration — bounded, per feature
//!             scope, to "the subsequent call still succeeds" (nothing
//!             downstream of this feature yet consumes the identity for
//!             authorization; see feature-delta.md § Out of Scope /
//!             Security Rules).
//!
//! ── AC-16-08(a): full regression suite confirmation ──────────────────────
//! Per ADR-026 § Enforcement, this is a DISTILL-wave instruction to actually
//! RUN, not defer. Run (from the workspace root):
//!   cargo test -p embyr-server --test us_01_configure_sdk --test us_02_write_document \
//!     --test us_03_read_document --test us_04_query_collection --test us_05_listen_realtime \
//!     --test us_06_transactions --test us_07_provision_project --test us_08_monitor_project \
//!     --test us_09_suspend_project --test us_10_aws_secrets --test us_11_gcp_secrets \
//!     --test us_12_agent_backend --test us_13_browser_transport --test us_14_rate_limiting \
//!     --test walking_skeleton
//! (embyr_agent's 6 files run via `cargo test -p embyr-server --test us_12_agent_backend`'s
//! sibling module, already included above — embyr_agent tests are declared as
//! submodules under `tests/acceptance/embyr_agent.rs`, not separate [[test]]
//! binaries.) DISTILL executed this command against the tree with this
//! feature's scaffolds applied — see feature-delta.md § DISTILL Pre-DELIVER
//! Gate for the captured pass/fail result.
//!
//! Driving ports: REST :8081 (sign-in) + gRPC :8080 (getDoc, via
//! `ClientAuthFullContext` — the exact `embyr_server::start_test_server`
//! composition root the 72-scenario suite itself uses, Pillar 3).
//!
//! Error ratio: 5 error/edge (missing/malformed/expired/wrong-project +
//! AC-16-08b) out of 8 = 62.5%.
//!
//! All scenarios `#[ignore]` except the walking skeleton.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, ClientAuthFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

use embyr_proto::firestore::firestore_client::FirestoreClient;

// ─────────────────────────────────────────────────────────────────────────────
// AC-16-06: valid token signs in and the subsequent Firestore call still succeeds
// (WALKING SKELETON — Activity B, feature-delta.md § Story Map)
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — Given reuses ca01's registration precondition
/// conceptually; this file hand-seeds the credential row directly per
/// Slice 02's own "can be tested against a hand-seeded credential row"
/// allowance, feature-delta.md § Elephant Carpaccio Slices):
///   Given: `trailmark-prod` has an active registered verification credential
///   And:   Maria Santos holds a token minted against it, unexpired
///   When:  Maria's session calls sign-in with that token
///   Then:  sign-in succeeds, and her subsequent getDoc call still succeeds
///
/// AC-16-06
///
/// @walking_skeleton @driving_port @real-io @US-02 @AC-16-06
#[tokio::test]
async fn marias_valid_token_signs_in_and_her_subsequent_getdoc_call_succeeds() {
    let ctx = ClientAuthFullContext::new("trailmark-prod-ca02").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_credential(&signing_key.verifying_key().to_bytes()).await;

    let token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let sign_in_resp = reqwest::Client::new()
        .post(ctx.rest_url(&format!(
            "/v1/projects/{}/accounts:signInWithCustomToken",
            ctx.project_id
        )))
        .json(&serde_json::json!({"token": token}))
        .send()
        .await
        .expect("sign-in request failed");

    assert_eq!(sign_in_resp.status().as_u16(), 200, "AC-16-06: valid token sign-in must succeed");
    let sign_in_body: serde_json::Value = sign_in_resp.json().await.expect("sign-in body must be JSON");
    assert_eq!(sign_in_body["localId"], "maria-santos");
    assert!(sign_in_body.get("expiresIn").is_some());

    // Maria's subsequent getDoc call — still succeeds, exactly as any other
    // authenticated Firestore call (AC-16-06's "identity carried forward",
    // observable bound per feature scope: Security Rules is out of scope).
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
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {}", ctx.api_key).parse().unwrap());

    let get_doc_resp = client.get_document(request).await;
    assert!(
        get_doc_resp.is_ok(),
        "AC-16-06: Maria's subsequent getDoc call must succeed after a valid sign-in: {:?}",
        get_doc_resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-16-07: the four distinguishable rejection reasons (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

async fn sign_in(ctx: &ClientAuthFullContext, token: Option<&str>) -> (u16, serde_json::Value) {
    let body = match token {
        Some(t) => serde_json::json!({"token": t}),
        None => serde_json::json!({}),
    };
    let resp = reqwest::Client::new()
        .post(ctx.rest_url(&format!(
            "/v1/projects/{}/accounts:signInWithCustomToken",
            ctx.project_id
        )))
        .json(&body)
        .send()
        .await
        .expect("sign-in request failed");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
    (status, body)
}

/// AC-16-07 (missing)
///
/// @error @driving_port @real-io @US-02 @AC-16-07
#[tokio::test]
#[ignore = "RED — enable after the walking skeleton (AC-16-06) is GREEN"]
async fn signing_in_with_no_token_is_rejected_with_missing_token_reason() {
    let ctx = ClientAuthFullContext::new("trailmark-prod-ca02-missing").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_credential(&signing_key.verifying_key().to_bytes()).await;

    let (status, body) = sign_in(&ctx, None).await;

    assert_eq!(status, 400, "AC-16-07: missing token must be rejected");
    assert_eq!(body["reason"], "MISSING_TOKEN");
}

/// AC-16-07 (malformed), distinguishable from missing
///
/// @error @driving_port @real-io @US-02 @AC-16-07
#[tokio::test]
#[ignore = "RED — enable after the walking skeleton (AC-16-06) is GREEN"]
async fn signing_in_with_a_corrupted_token_is_rejected_with_malformed_reason() {
    let ctx = ClientAuthFullContext::new("trailmark-prod-ca02-malformed").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_credential(&signing_key.verifying_key().to_bytes()).await;

    let (status, body) = sign_in(&ctx, Some("not-a-valid-jwt")).await;

    assert_eq!(status, 400, "AC-16-07: malformed token must be rejected");
    assert_eq!(body["reason"], "MALFORMED_TOKEN");
}

/// AC-16-07 (expired), distinguishable from malformed — Dana Kim's token
/// minted 2h ago against a 1h validity window (US-02 Domain Example 2).
///
/// @error @driving_port @real-io @US-02 @AC-16-07
#[tokio::test]
#[ignore = "RED — enable after the walking skeleton (AC-16-06) is GREEN"]
async fn danas_expired_token_is_rejected_with_expiry_reason() {
    let ctx = ClientAuthFullContext::new("trailmark-prod-ca02-expired").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_credential(&signing_key.verifying_key().to_bytes()).await;

    let token = mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() - 3600);
    let (status, body) = sign_in(&ctx, Some(&token)).await;

    assert_eq!(status, 400, "AC-16-07: expired token must be rejected");
    assert_eq!(body["reason"], "TOKEN_EXPIRED");
}

/// AC-16-07 (wrong project), distinguishable from expiry — a token minted
/// against `trailmark-staging`'s credential, presented to `trailmark-prod`.
///
/// @error @driving_port @real-io @US-02 @AC-16-07
#[tokio::test]
#[ignore = "RED — enable after the walking skeleton (AC-16-06) is GREEN"]
async fn token_minted_for_a_different_project_is_rejected_with_project_mismatch_reason() {
    let ctx = ClientAuthFullContext::new("trailmark-prod-ca02-mismatch").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_credential(&signing_key.verifying_key().to_bytes()).await;

    let token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        "trailmark-staging-different",
        now_unix() + 3600,
    );
    let (status, body) = sign_in(&ctx, Some(&token)).await;

    assert_eq!(status, 400, "AC-16-07: wrong-project token must be rejected");
    assert_eq!(body["reason"], "PROJECT_MISMATCH");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-16-08(b): an EXPIRED identity header never breaks the underlying data call
// ─────────────────────────────────────────────────────────────────────────────

/// The single most important negative-space guarantee in this feature per
/// DISTILL dispatch instructions: presenting a valid api_key alongside an
/// EXPIRED x-embyr-client-identity token must not turn a previously-working
/// getDoc call into a failure (ADR-026 — the failure branch of step 4
/// attaches nothing, never rejects the underlying request).
///
/// @driving_port @real-io @US-02 @AC-16-08
#[tokio::test]
#[ignore = "RED — enable after the walking skeleton (AC-16-06) is GREEN"]
async fn an_expired_client_identity_header_does_not_break_an_ordinary_getdoc_call() {
    let ctx = ClientAuthFullContext::new("trailmark-prod-ca02-guardrail-b").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_credential(&signing_key.verifying_key().to_bytes()).await;
    let expired_token = mint_client_identity_token(
        &signing_key,
        "dana-kim",
        &ctx.project_id,
        now_unix() - 3600,
    );

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
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {}", ctx.api_key).parse().unwrap());
    request
        .metadata_mut()
        .insert("x-embyr-client-identity", format!("Bearer {expired_token}").parse().unwrap());

    let resp = client.get_document(request).await;
    assert!(
        resp.is_ok(),
        "AC-16-08(b): an EXPIRED client-identity header must never reject an ordinary getDoc call: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-16-08(c): header absent -> the new branch is structurally unreachable
// ─────────────────────────────────────────────────────────────────────────────

/// Black-box half of the structural-unreachability proof (pure-function half
/// lives in grpc/handler.rs::client_identity_extension_tests). A session
/// that never presents the header exercises the SAME code path the 72
/// existing scenarios exercise. If step 4 were reachable without the
/// header, it would call the RED scaffold (which panics unconditionally
/// today) — so a PASS here is affirmative proof the branch was never
/// entered, not merely an absence of an observed side effect.
///
/// @driving_port @real-io @US-02 @AC-16-08
#[tokio::test]
#[ignore = "RED — enable after the walking skeleton (AC-16-06) is GREEN"]
async fn a_session_that_never_presents_the_client_identity_header_never_reaches_the_new_verification_branch() {
    let ctx = ClientAuthFullContext::new("trailmark-prod-ca02-guardrail-c").await;
    // Deliberately: no client_identity_credentials row seeded at all — if
    // the new branch were reachable, it would panic looking up a
    // nonexistent credential too, reinforcing the unreachability proof.

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
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {}", ctx.api_key).parse().unwrap());
    // No x-embyr-client-identity header at all.

    let resp = client.get_document(request).await;
    assert!(
        resp.is_ok(),
        "AC-16-08(c): a session with no client-identity header must never reach the new \
         verification branch (structural unreachability): {:?}",
        resp.err()
    );
}
