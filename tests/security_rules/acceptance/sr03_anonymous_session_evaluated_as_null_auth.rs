//! SR03 (Slice 03, Walking Skeleton, US-03, Release 1) — A Session With No
//! Verified Identity Is Evaluated as Anonymous.
//!
//! Acceptance criteria verified here (feature-delta.md US-03):
//!   AC-17-11: a session with no verified identity is denied by a rule
//!             requiring `request.auth != null`, attributable to the rule.
//!   AC-17-12: a session with no verified identity succeeds against a rule
//!             that explicitly allows unauthenticated access (`allow read:
//!             if true`).
//!   AC-17-13: a session presenting an invalid (malformed/expired/wrong
//!             -project) client-identity header is evaluated IDENTICALLY,
//!             for rule purposes, to a session presenting no header at
//!             all — reusing `client-auth`'s existing ADR-026 DDD-CA-5
//!             "attach nothing" semantics unchanged, introducing no new
//!             rejection class. This scenario uses an EXPIRED token as the
//!             concrete example (one of the three equivalent invalid
//!             shapes DISCUSS names).
//!
//! Driving port: gRPC :8080 `GetDocument` (via `SecurityRulesFullContext`,
//! Pillar 3). Gives `client-auth`'s optional identity real behavioral
//! consequence for the anonymous case for the first time (feature-delta.md
//! § Prioritization, Slice 03 rationale).
//!
//! Error ratio: 2 error/edge (AC-17-11 denial, AC-17-13 equivalence) out of
//! 3 = 67%.
//!
//! One scenario enabled at a time (RED scaffold discipline, ADR-025 D2) —
//! the walking skeleton (scenario 1) is DISCUSS's own "Happy Path (expected
//! deny)" domain example for US-03.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, SecurityRulesFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-11: never-signed-in session denied by an auth-required rule
// (WALKING SKELETON — US-03 Domain Example 1, "Happy Path (expected deny)")
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — this file's own opening Given; scenario 3 below reuses
/// this file's auth-required-rule seeding shape, per Pillar 2):
///   Given: `journal_entries` has a rule requiring `request.auth != null`
///   When:  a session that never presented a client-identity token calls
///          `getDoc()` on a `journal_entries` document
///   Then:  the read is denied, attributable to the rule
///
/// AC-17-11
///
/// @walking_skeleton @driving_port @real-io @US-03 @AC-17-11
#[tokio::test]
async fn a_never_signed_in_session_is_denied_by_a_rule_requiring_identity() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sr03-denyanon").await;
    ctx.seed_access_rule("journal_entries", "request.auth != null").await;

    // No client-identity token presented at all.
    let resp = ctx
        .get_document(&ctx.document_resource_name("journal_entries", "maria-doc"), None)
        .await;

    let err = resp.expect_err(
        "AC-17-11: a never-signed-in session must be denied by a rule requiring request.auth != null",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-11: denial must be attributable to the rule (PermissionDenied), got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-12: never-signed-in session succeeds against a public-read rule
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-12
///
/// @driving_port @real-io @US-03 @AC-17-12
#[tokio::test]
async fn a_never_signed_in_session_succeeds_against_a_rule_allowing_public_read() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sr03-publicread").await;
    ctx.seed_access_rule("trail_guides", "true").await;

    let resp = ctx
        .get_document(&ctx.document_resource_name("trail_guides", "guide-doc"), None)
        .await;

    assert!(
        resp.is_ok(),
        "AC-17-12: anonymous access is not blanket-denied — a public-read rule must allow it: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-13: an invalid client-identity header is evaluated identically to
// no header at all (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// Reuses this file's own auth-required-rule Given from scenario 1
/// (`journal_entries` requires `request.auth != null`) — an EXPIRED token
/// (one of the three DISCUSS-named equivalent invalid shapes:
/// malformed/expired/wrong-project) must be evaluated identically to
/// presenting no token at all, per `client-auth`'s existing ADR-026
/// DDD-CA-5 semantics, unchanged.
///
/// AC-17-13
///
/// @error @driving_port @real-io @US-03 @AC-17-13
#[tokio::test]
async fn an_invalid_client_identity_header_is_evaluated_identically_to_no_header_at_all() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sr03-invalidheader").await;
    ctx.seed_access_rule("journal_entries", "request.auth != null").await;

    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    // Expired 1h ago — `attach_client_identity_if_present` attaches
    // nothing for this case (client-auth ADR-026 DDD-CA-5, unchanged).
    let expired_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() - 3600);

    let resp_no_header = ctx
        .get_document(&ctx.document_resource_name("journal_entries", "maria-doc"), None)
        .await;
    let resp_expired_header = ctx
        .get_document(
            &ctx.document_resource_name("journal_entries", "maria-doc"),
            Some(&expired_token),
        )
        .await;

    let err_no_header =
        resp_no_header.expect_err("AC-17-13: no-header session must be denied by the auth-required rule");
    let err_expired_header = resp_expired_header
        .expect_err("AC-17-13: expired-header session must be denied IDENTICALLY to no-header");

    // Anchor assertion (deliberately checked BEFORE the cross-comparison
    // below): without this, two RED-scaffold panics would ALSO compare
    // equal to each other, making the scenario pass vacuously for the
    // wrong reason (Critical Rule 7, No Fixture Theater) instead of for
    // AC-17-13's actual claim. Pinning both sides to the concrete expected
    // status keeps this scenario RED for the right reason today AND
    // meaningful once real evaluation exists.
    assert_eq!(
        err_no_header.code(),
        tonic::Code::PermissionDenied,
        "AC-17-13: the no-header session must be denied specifically as PermissionDenied \
         (attributable to the rule), got {:?}",
        err_no_header.code()
    );

    assert_eq!(
        err_no_header.code(),
        err_expired_header.code(),
        "AC-17-13: no new rejection class — an invalid client-identity header must be denied \
         with the EXACT same gRPC status as no header at all"
    );
    assert_eq!(
        err_no_header.message(),
        err_expired_header.message(),
        "AC-17-13: no new rejection class — identical message too, per ADR-026 DDD-CA-5"
    );
}
