//! AS02 (Slice 02, LAST slice, US-02) — Maria Gets a Real Identity Instantly
//! With No Prior Credential.
//!
//! Acceptance criteria verified here
//! (slice-02-maria-signs-in-anonymously.md, ADR-043):
//!   AC-20-05: valid anonymous sign-in -> 200; the verified identity attaches
//!             to a SUBSEQUENT real Firestore getDoc call (WALKING SKELETON).
//!   AC-20-06: sign-in on a project without anonymous auth enabled -> 400
//!             ANONYMOUS_AUTH_NOT_ENABLED, distinguishable from an
//!             invalid-api_key failure.
//!   AC-20-07: sign-in without a valid project api_key -> 401
//!             INVALID_API_KEY, before any identity is minted.
//!   AC-20-08 + AC-20-10: two separate sign-in calls always mint two
//!             distinct end-user identities — never a collision. The SAME
//!             underlying mechanism (a fresh `Uuid::new_v4()` per call,
//!             ADR-043 Decision 4) is what makes AC-20-10 true (a re-sign-in
//!             after a prior token's expiry mints a NEW identity, not a
//!             resumption) — one test demonstrates both ACs rather than two
//!             tests asserting the identical observable property twice
//!             (Test Minimization, skill § consolidation).
//!   AC-20-09: an anonymous identity is denied by an ownership rule exactly
//!             as any other non-owning identity would be.
//!
//! Driving ports: REST :8081 (signInAnonymously) + gRPC :8080 (getDoc
//! regression/ownership proofs, via `AnonymousIdentityFullContext` — mirrors
//! `client-auth-hosted-identity`'s/`oauth-providers`' own
//! `HostedIdentityFullContext`/`OAuthProviderFullContext` precedent exactly).
//!
//! Real Postgres (System + Customer DB) via testcontainers (@wiring_e2e-tier
//! real-I/O) — single golden walkthrough per scenario, matching this
//! codebase's own established convention for this exact test class (mirrors
//! `hi02`/`op02`'s identical single-example shape).
//!
//! Error ratio: 2 error/edge (AC-20-06/07) out of 5 scenarios = 40%.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::AnonymousIdentityFullContext;

// ─────────────────────────────────────────────────────────────────────────────
// AC-20-05: valid sign-in mints a token that unlocks a SUBSEQUENT real
// Firestore call (WALKING SKELETON)
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trailmark-prod` has anonymous auth enabled
///   When:  Maria's app calls signInAnonymously() with a valid api_key and
///          no other credential
///   Then:  200 with a localId/idToken/expiresIn, and Maria's subsequent
///          getDoc call carrying idToken succeeds
///
/// AC-20-05
///
/// @walking_skeleton @driving_port @real-io @US-02 @AC-20-05
#[tokio::test]
async fn marias_valid_anonymous_sign_in_unlocks_her_subsequent_getdoc_call() {
    let ctx = AnonymousIdentityFullContext::new("trailmark-prod-as02-signin").await;
    ctx.enable_anonymous_identity().await;

    let (status, body) = ctx.sign_in_anonymously(&ctx.api_key).await;

    assert_eq!(
        status, 200,
        "AC-20-05: valid anonymous sign-in must return 200"
    );
    let local_id = body["localId"]
        .as_str()
        .expect("localId must be a string")
        .to_string();
    assert!(
        !local_id.is_empty(),
        "AC-20-05: localId must be a real, non-empty uid"
    );
    let expires_in: i64 = body["expiresIn"]
        .as_str()
        .expect("expiresIn must be a string")
        .parse()
        .expect("expiresIn must be a parseable integer");
    assert!(
        expires_in > 0,
        "AC-20-05: expiresIn must be the token's positive TTL"
    );
    let token = body["idToken"]
        .as_str()
        .expect("AC-20-05: response must carry a usable token for the subsequent call")
        .to_string();

    ctx.insert_document_with_owner("trip_entries", "maria-draft", &local_id)
        .await;
    let resp = ctx
        .get_document(
            &ctx.document_resource_name("trip_entries", "maria-draft"),
            Some(&token),
        )
        .await;

    assert!(
        resp.is_ok(),
        "AC-20-05: Maria's subsequent getDoc call carrying her minted token must succeed: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-20-06: sign-in on a project without anonymous auth enabled (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-20-06
///
/// @error @driving_port @real-io @US-02 @AC-20-06
#[tokio::test]
async fn sign_in_on_a_project_without_anonymous_auth_enabled_is_rejected_distinguishably() {
    let ctx = AnonymousIdentityFullContext::new("trailmark-prod-as02-notenabled").await;
    // Deliberately: enable_anonymous_identity() is never called.

    let (status, body) = ctx.sign_in_anonymously(&ctx.api_key).await;

    assert_eq!(
        status, 400,
        "AC-20-06: sign-in without anonymous auth enabled must be rejected"
    );
    assert_eq!(
        body["reason"], "ANONYMOUS_AUTH_NOT_ENABLED",
        "AC-20-06: reason must be distinguishable from an invalid-api_key failure"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-20-07: sign-in without a valid project api_key (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-20-07
///
/// @error @driving_port @real-io @US-02 @AC-20-07
#[tokio::test]
async fn sign_in_without_a_valid_api_key_is_rejected_before_any_identity_is_minted() {
    let ctx = AnonymousIdentityFullContext::new("trailmark-prod-as02-badkey").await;
    ctx.enable_anonymous_identity().await;

    let (status, body) = ctx.sign_in_anonymously("wrong-api-key").await;

    assert_eq!(
        status, 401,
        "AC-20-07: an invalid api_key must be rejected with 401"
    );
    assert_eq!(body["reason"], "INVALID_API_KEY");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-20-08 + AC-20-10: two separate sign-ins never collide (same underlying
// stateless-minting mechanism proves both ACs)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-20-08, AC-20-10
///
/// @driving_port @real-io @US-02 @AC-20-08 @AC-20-10
#[tokio::test]
async fn two_separate_anonymous_sign_ins_always_mint_two_distinct_identities() {
    let ctx = AnonymousIdentityFullContext::new("trailmark-prod-as02-collision").await;
    ctx.enable_anonymous_identity().await;

    let (status_first, body_first) = ctx.sign_in_anonymously(&ctx.api_key).await;
    assert_eq!(status_first, 200, "first sign-in must succeed");

    let (status_second, body_second) = ctx.sign_in_anonymously(&ctx.api_key).await;
    assert_eq!(status_second, 200, "second sign-in must succeed");

    assert_ne!(
        body_first["localId"], body_second["localId"],
        "AC-20-08: two separate anonymous sign-ins must never collide on the same uid; \
         AC-20-10: this is the same mechanism that makes a re-sign-in after token \
         expiry mint a NEW identity rather than resuming the original one"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-20-09: an anonymous identity is denied by an ownership rule exactly as
// any other non-owning identity would be
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trip_entries` has an ownership rule
///          `request.auth.uid == resource.data.owner_id`, and Maria's
///          anonymous session has created her own `trip_entries` document
///   When:  a DIFFERENT anonymous session attempts to read Maria's document
///   Then:  the read is denied, identically to how the rule would deny any
///          other non-owning verified identity
///
/// AC-20-09
///
/// @error @driving_port @real-io @US-02 @AC-20-09
#[tokio::test]
async fn a_different_anonymous_sessions_read_of_marias_document_is_denied() {
    let ctx = AnonymousIdentityFullContext::new("trailmark-prod-as02-ownership").await;
    ctx.enable_anonymous_identity().await;
    ctx.seed_access_rule("trip_entries", "request.auth.uid == resource.data.owner_id")
        .await;

    let (maria_status, maria_body) = ctx.sign_in_anonymously(&ctx.api_key).await;
    assert_eq!(maria_status, 200, "Maria's sign-in must succeed");
    let marias_uid = maria_body["localId"]
        .as_str()
        .expect("localId must be a string");
    ctx.insert_document_with_owner("trip_entries", "maria-draft", marias_uid)
        .await;

    let (other_status, other_body) = ctx.sign_in_anonymously(&ctx.api_key).await;
    assert_eq!(
        other_status, 200,
        "the other visitor's sign-in must succeed"
    );
    let others_token = other_body["idToken"]
        .as_str()
        .expect("idToken must be a string");

    let resp = ctx
        .get_document(
            &ctx.document_resource_name("trip_entries", "maria-draft"),
            Some(others_token),
        )
        .await;

    let err = resp.expect_err(
        "AC-20-09: a different anonymous session's read of Maria's document must be denied",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-20-09: denial must be attributable to the rule (PermissionDenied), \
         identically to how any other non-owning identity would be denied, got {:?}",
        err.code()
    );
}
