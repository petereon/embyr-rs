//! aggregation-queries Slice 01 (US-01, ADR-038/039/040) — COUNT Aggregation
//! Rejection Paths (Walking Skeleton).
//!
//! Acceptance criteria verified here (feature-delta.md US-01):
//!   AC-01-02: a COUNT aggregation attempting to count another end user's
//!             documents is rejected as a permission denial, using the
//!             identical `check_query_compliance()` mechanism RunQuery
//!             already uses for the same scenario (ADR-039 § Decision 1).
//!   AC-01-05: an access rule using an undecidable `Condition` shape rejects
//!             every aggregation against that collection, regardless of
//!             filter shape — the identical fail-closed default RunQuery
//!             already enforces (ADR-031's `Or`/`Not` wildcard).
//!
//! Driving port: gRPC :8080 `RunAggregationQuery` (via `SecurityRulesFullContext`
//! + this feature's own `run_count_aggregation` helper — Pillar 3, real
//! Postgres, real gRPC, no mocks).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, run_count_aggregation, SecurityRulesFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-02 — THE single most security-relevant property in this slice: a
// filter on the correct field but bound to a value OTHER than the caller's
// own verified uid is rejected before the count is computed. Mirrors
// security-rules-query-path's own AC-17-51.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trip_entries` has a READ rule requiring
///          `request.auth.uid == resource.data.owner_id`
///   And:   Dana Kim holds a verified identity (uid "dana-kim")
///   When:  Dana runs a COUNT aggregation filtered to `owner_id ==
///          "maria-santos"` — Maria's uid, not Dana's own
///   Then:  the request is rejected as a permission denial, never returning a
///          count derived from Maria's data
///
/// AC-01-02
///
/// @error @driving_port @real-io @US-01 @AC-01-02 @security-critical
#[tokio::test]
async fn counting_another_users_documents_is_rejected_as_permission_denial() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-agg01-deny").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("trip_entries", "request.auth.uid == resource.data.owner_id")
        .await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let result = run_count_aggregation(
        &ctx,
        "trip_entries",
        false,
        &[("owner_id", "maria-santos")],
        Some(&danas_token),
    )
    .await;

    let err = result.expect_err(
        "AC-01-02: a COUNT aggregation attempting to count another user's documents must be \
         rejected, not admitted",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-01-02: denial must be attributable to the rule (PermissionDenied), got {:?}: {}",
        err.code(),
        err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-05: an access rule using an undecidable Condition shape (Or) rejects
// every aggregation against that collection, regardless of filter shape —
// mirrors security-rules-query-path's own AC-17-65.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trip_comments` has an Or-shaped READ rule combining ownership
///          and public-visibility
///   And:   Maria Santos holds a verified identity
///   When:  Maria runs a COUNT aggregation with a filter that WOULD satisfy
///          the rule's ownership half in isolation
///   Then:  the aggregation is rejected outright — the `||` makes the entire
///          rule undecidable for aggregation enforcement, identical to
///          RunQuery's own fail-closed default
///
/// AC-01-05
///
/// @error @driving_port @real-io @US-01 @AC-01-05 @security-critical
#[tokio::test]
async fn an_undecidable_rule_shape_rejects_the_aggregation_outright() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-agg01-or").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "trip_comments",
        "request.auth.uid == resource.data.owner_id || resource.data.public != null",
    )
    .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let result = run_count_aggregation(
        &ctx,
        "trip_comments",
        false,
        &[("owner_id", "maria-santos")],
        Some(&marias_token),
    )
    .await;

    let err = result.expect_err(
        "AC-01-05: an Or-shaped (undecidable) rule must reject the aggregation outright, \
         regardless of filter shape",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-01-05: rejection must be attributable to the rule (PermissionDenied), got {:?}: {}",
        err.code(),
        err.message()
    );
    assert!(
        err.message().contains("[UNSUPPORTED_RULE_SHAPE]"),
        "AC-01-05: rejection message must name the whole-rule-undecidable reason, got: {}",
        err.message()
    );
}
