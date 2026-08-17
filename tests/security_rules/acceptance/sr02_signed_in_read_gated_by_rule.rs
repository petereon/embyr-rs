//! SR02 (Slice 02, Walking Skeleton — Activity B, US-02, Release 1) — A
//! Signed-In End User's Read Is Gated by Their Own Rule.
//!
//! Acceptance criteria verified here (feature-delta.md US-02):
//!   AC-17-06: a signed-in end user reading a document that satisfies the
//!             rule's condition succeeds, unchanged from pre-feature
//!             behavior.
//!   AC-17-07: a signed-in end user reading a document that fails the
//!             rule's condition is denied PermissionDenied, attributable
//!             to the rule.
//!   AC-17-08: a rule not based on document ownership (e.g. "any signed-in
//!             caller") correctly allows callers who are not the owner —
//!             proving the grammar is not owner-equality-only.
//!   AC-17-09: a condition referencing an absent document field evaluates
//!             to denied (fails closed), never an internal error.
//!   AC-17-10: a denied read's response is IDENTICAL whether or not the
//!             target document actually exists — this is a locked
//!             security-observable behavior (ADR-029 § Existence
//!             non-leakage), given EXTRA care per the DISTILL dispatch
//!             instructions. Written ONLY against the content-referencing
//!             ownership rule, per OQ-SR-06's scoping (see
//!             feature-delta.md § DISTILL / Open Question Resolutions) —
//!             NOT against a content-blind rule like `allow read: if true`.
//!
//! Driving port: gRPC :8080 `GetDocument` (via `SecurityRulesFullContext` —
//! the exact `embyr_server::start_test_server` composition root the
//! 113-scenario regression suite itself uses, Pillar 3).
//!
//! Error ratio: 3 error/edge (AC-17-07/09/10) out of 5 = 60%.
//!
//! One scenario enabled at a time (RED scaffold discipline, ADR-025 D2).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, SecurityRulesFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-06: own-document read succeeds unchanged
// (WALKING SKELETON — Activity B, feature-delta.md § Story Map)
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — this is the opening Given of the read-evaluation
/// story line within this file; scenarios 2-5 below reuse this file's own
/// rule/identity seeding shape, per Pillar 2):
///   Given: `journal_entries` has a rule requiring
///          `request.auth.uid == resource.data.owner_id`
///   And:   Maria Santos holds a verified identity and
///          `journal_entries/{project}-maria-doc` has `owner_id:
///          "maria-santos"`
///   When:  Maria calls `getDoc()` on that document
///   Then:  the read succeeds and returns the document exactly as it would
///          have before this feature shipped
///
/// AC-17-06
///
/// @walking_skeleton @driving_port @real-io @US-02 @AC-17-06
#[tokio::test]
async fn a_signed_in_end_user_reading_their_own_document_succeeds_unchanged() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sr02-owner").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);

    let resp = ctx
        .get_document(
            &ctx.document_resource_name("journal_entries", "maria-doc"),
            Some(&marias_token),
        )
        .await;

    assert!(
        resp.is_ok(),
        "AC-17-06: Maria's read of her own document must succeed: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-07: a different signed-in end user's read of the same document is denied
// (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-07
///
/// @error @driving_port @real-io @US-02 @AC-17-07
#[tokio::test]
#[ignore = "one-scenario-at-a-time RED discipline — DELIVER unskips per step"]
async fn a_different_signed_in_end_users_read_of_the_same_document_is_denied() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sr02-nonowner").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let resp = ctx
        .get_document(
            &ctx.document_resource_name("journal_entries", "maria-doc"),
            Some(&danas_token),
        )
        .await;

    let err = resp.expect_err("AC-17-07: Dana's read of Maria's document must be denied");
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-07: denial must be attributable to the rule (PermissionDenied), got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-08: a non-ownership rule allows any signed-in caller
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-08
///
/// @driving_port @real-io @US-02 @AC-17-08
#[tokio::test]
#[ignore = "one-scenario-at-a-time RED discipline — DELIVER unskips per step"]
async fn a_rule_not_based_on_ownership_allows_any_signed_in_caller() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sr02-authreq").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("trail_guides", "request.auth != null").await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let resp = ctx
        .get_document(
            &ctx.document_resource_name("trail_guides", "guide-doc"),
            Some(&danas_token),
        )
        .await;

    assert!(
        resp.is_ok(),
        "AC-17-08: any signed-in caller must be allowed by an auth-required (non-ownership) rule: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-09: a condition referencing a missing field fails closed, never crashes
// (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-09
///
/// @error @driving_port @real-io @US-02 @AC-17-09
#[tokio::test]
#[ignore = "one-scenario-at-a-time RED discipline — DELIVER unskips per step"]
async fn a_condition_referencing_a_missing_field_fails_closed_not_with_an_error() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sr02-missingfield").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);

    // journal_entries/{project}-no-owner-doc has NO owner_id field at all
    // (seeded by SecurityRulesFullContext::new — see common/mod.rs doc comment).
    let resp = ctx
        .get_document(
            &ctx.document_resource_name("journal_entries", "no-owner-doc"),
            Some(&marias_token),
        )
        .await;

    let err = resp.expect_err("AC-17-09: a missing-field condition must deny, not succeed");
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-09: must fail CLOSED (PermissionDenied) — never Internal (a crash), got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-10: existence non-leakage — a denied read never reveals whether the
// target document exists (error/edge, EXTRA CARE per DISTILL dispatch)
// ─────────────────────────────────────────────────────────────────────────────

/// Locked security-observable behavior (ADR-029 § Existence non-leakage).
/// Written ONLY against the ownership rule (content-referencing —
/// `request.auth.uid == resource.data.owner_id`), per OQ-SR-06's explicit
/// scoping (feature-delta.md § DISTILL) — this feature does NOT claim
/// non-leakage for a content-blind rule (e.g. `allow read: if true`), and
/// no scenario in this file tests that unscoped claim.
///
/// Compares Dana's denied read of Maria's REAL (existing, wrong-owner)
/// document against Dana's denied read of a document ID that was NEVER
/// seeded at all — both must produce a byte-identical `PermissionDenied`,
/// with no distinguishing detail (status code AND message).
///
/// AC-17-10
///
/// @error @driving_port @real-io @US-02 @AC-17-10 @security-regression
#[tokio::test]
#[ignore = "one-scenario-at-a-time RED discipline — DELIVER unskips per step"]
async fn a_denied_read_never_reveals_whether_the_target_document_exists() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sr02-nonleak").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let resp_existing_wrong_owner = ctx
        .get_document(
            &ctx.document_resource_name("journal_entries", "maria-doc"),
            Some(&danas_token),
        )
        .await;
    let resp_nonexistent = ctx
        .get_document(
            &ctx.nonexistent_document_resource_name("journal_entries"),
            Some(&danas_token),
        )
        .await;

    let err_existing = resp_existing_wrong_owner
        .expect_err("AC-17-10: wrong-owner read of an EXISTING document must be denied");
    let err_nonexistent = resp_nonexistent
        .expect_err("AC-17-10: read of a NON-EXISTENT document must ALSO be denied (not NotFound)");

    assert_eq!(
        err_existing.code(),
        err_nonexistent.code(),
        "AC-17-10: existence non-leakage — both denials must carry the IDENTICAL gRPC status code"
    );
    assert_eq!(
        err_existing.code(),
        tonic::Code::PermissionDenied,
        "AC-17-10: must be PermissionDenied, never NotFound (that would leak non-existence)"
    );
    assert_eq!(
        err_existing.message(),
        err_nonexistent.message(),
        "AC-17-10: existence non-leakage — both denials must carry the IDENTICAL message, \
         with no detail distinguishing 'wrong owner' from 'no such document'"
    );
}
