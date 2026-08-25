//! Slice 04 (US-04, ADR-031) — AND-Composed Query Compliance.
//!
//! Acceptance criteria verified here (feature-delta.md US-04):
//!   AC-17-61: a query satisfying every AND'd conjunct of a composed rule
//!             (`request.auth != null && request.auth.uid ==
//!             resource.data.curator_id`) is admitted.
//!   AC-17-62: a query satisfying only a SUBSET of the AND'd conjuncts is
//!             rejected — the ownership conjunct unsatisfied (no matching
//!             filter) even though the caller is signed in.
//!   AC-17-63: each conjunct is checked INDEPENDENTLY — an anonymous session
//!             has BOTH the auth-presence and ownership conjuncts reported
//!             as unsatisfied (the ownership conjunct's own satisfaction is
//!             itself gated on a signed-in caller's uid), never masked by
//!             one another.
//!
//! Domain example (this slice's own Trailmark grounding): `trip_photos`,
//! curator-owned shared galleries, rule `request.auth != null &&
//! request.auth.uid == resource.data.curator_id`.
//!
//! `decompose_decidable`'s `Condition::And` arm recursively decomposes both
//! sides and flattens the atom lists — reusing Slice 01's `OwnershipEquality`
//! atom and Slice 03's `AuthRequired` atom verbatim (no new per-conjunct
//! semantics, AC-17-64). That architectural claim — same atoms, no parallel
//! matching path — is proven directly at the domain-function level in
//! `crates/embyr-core/src/access_control/mod.rs`'s own unit tests; this file
//! proves the SAME composition end-to-end through the real `handle_run_query`
//! composition (an already-wired call site this slice does not modify).
//!
//! Driving port: gRPC :8080 `RunQuery` (via `SecurityRulesFullContext` +
//! this feature's own `run_query` helper — Pillar 3, real Postgres, real
//! gRPC, no mocks).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{create_document, mint_client_identity_token, now_unix, run_query, string_field, SecurityRulesFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

const TRIP_PHOTOS_RULE: &str = "request.auth != null && request.auth.uid == resource.data.curator_id";

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-61: a query satisfying every AND'd conjunct of a composed rule is
// admitted.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trip_photos` has a READ rule AND-composing an auth-presence
///          check with an ownership-equality check on `curator_id`
///   And:   Maria Santos holds a verified identity and curates a seeded
///          gallery document
///   When:  Maria calls `RunQuery` with a filter binding `curator_id` to her
///          OWN uid
///   Then:  the query is admitted and returns the seeded document — BOTH
///          conjuncts are independently satisfied
///
/// AC-17-61
///
/// @driving_port @real-io @US-04 @AC-17-61
#[tokio::test]
async fn a_query_satisfying_every_and_composed_conjunct_is_admitted() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp04-admit").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("trip_photos", TRIP_PHOTOS_RULE).await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let mut fields = std::collections::HashMap::new();
    fields.insert("curator_id".to_string(), string_field("maria-santos"));
    fields.insert("gallery_title".to_string(), string_field("patagonia-2026"));
    create_document(&ctx, "trip_photos", "sqp04-gallery-doc", fields, Some(&marias_token))
        .await
        .expect("seed trip_photos document");

    let admitted = run_query(
        &ctx,
        "trip_photos",
        &[("curator_id", "maria-santos")],
        Some(&marias_token),
    )
    .await
    .expect("AC-17-61: a query satisfying both AND'd conjuncts must be admitted");

    assert_eq!(admitted.len(), 1, "expected the seeded gallery document");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-62: a query satisfying only a SUBSET of the AND'd conjuncts is
// rejected — the ownership conjunct unsatisfied even though the caller is
// signed in.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trip_photos` has the same AND-composed rule
///   And:   Maria Santos holds a verified identity (satisfies the
///          auth-presence conjunct)
///   When:  Maria calls `RunQuery` with NO filter at all (does NOT satisfy
///          the ownership-equality conjunct)
///   Then:  the query is rejected, naming the unmet `OWNERSHIP_FILTER_MISSING`
///          conjunct specifically — being signed in does not substitute for
///          the missing ownership filter
///
/// AC-17-62
///
/// @error @driving_port @real-io @US-04 @AC-17-62
#[tokio::test]
async fn a_signed_in_query_missing_the_ownership_filter_conjunct_is_rejected() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp04-missingownership").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("trip_photos", TRIP_PHOTOS_RULE).await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let result = run_query(&ctx, "trip_photos", &[], Some(&marias_token)).await;

    let err = result.expect_err(
        "AC-17-62: a signed-in caller with no ownership-binding filter must still be rejected",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-62: rejection must be attributable to the rule (PermissionDenied), got {:?}: {}",
        err.code(),
        err.message()
    );
    assert!(
        err.message().contains("[OWNERSHIP_FILTER_MISSING]") && err.message().contains("curator_id"),
        "AC-17-62: rejection must name the unmet ownership conjunct specifically, got: {}",
        err.message()
    );
    assert!(
        !err.message().contains("[AUTH_REQUIRED]"),
        "AC-17-62: the satisfied auth-presence conjunct must NOT be listed as unmet, got: {}",
        err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-63: each conjunct is checked INDEPENDENTLY — an anonymous session is
// rejected by the auth-presence conjunct even if the filter would otherwise
// satisfy the ownership conjunct.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trip_photos` has the same AND-composed rule
///   When:  an ANONYMOUS caller (no client-identity token at all) calls
///          `RunQuery` with a filter binding `curator_id` to a value that
///          WOULD satisfy the ownership conjunct if a matching caller were
///          signed in
///   Then:  the query is rejected, naming BOTH the unmet `AUTH_REQUIRED` and
///          `OWNERSHIP_FILTER_MISSING` conjuncts — the ownership conjunct's
///          own satisfaction is itself gated on a signed-in caller's uid
///          (there is none to bind the filter against), so it is
///          independently evaluated and independently reported, exactly like
///          the auth conjunct; neither conjunct's failure masks or is masked
///          by the other's
///
/// AC-17-63
///
/// @error @driving_port @real-io @US-04 @AC-17-63
#[tokio::test]
async fn an_anonymous_session_has_both_and_composed_conjuncts_reported_independently() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp04-anonindependent").await;
    ctx.seed_access_rule("trip_photos", TRIP_PHOTOS_RULE).await;

    let result = run_query(
        &ctx,
        "trip_photos",
        &[("curator_id", "maria-santos")],
        None,
    )
    .await;

    let err = result.expect_err(
        "AC-17-63: an anonymous caller must be rejected even with a filter that would satisfy \
         the ownership conjunct for a matching signed-in caller",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-63: rejection must be attributable to the rule (PermissionDenied), got {:?}: {}",
        err.code(),
        err.message()
    );
    assert!(
        err.message().contains("[AUTH_REQUIRED]"),
        "AC-17-63: rejection must name the unmet auth-presence conjunct specifically, got: {}",
        err.message()
    );
    assert!(
        err.message().contains("[OWNERSHIP_FILTER_MISSING]"),
        "AC-17-63: the ownership conjunct is ALSO independently unsatisfied (there is no signed-in \
         caller's uid to bind the filter against) and must be named too — its evaluation is not \
         skipped just because the auth conjunct already failed, got: {}",
        err.message()
    );
}
