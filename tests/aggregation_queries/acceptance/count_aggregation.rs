//! aggregation-queries Slice 01 (US-01, ADR-038/039/040) — COUNT Aggregation
//! Happy Paths (Walking Skeleton).
//!
//! Acceptance criteria verified here (feature-delta.md US-01):
//!   AC-01-01: a COUNT aggregation filtered to a caller's own documents (per
//!             an ownership-equality access rule) returns the exact count of
//!             matching documents, carrying no document field data.
//!   AC-01-04: a collection with no access rule defined runs the aggregation
//!             unrestricted — zero behavior change from pre-feature RunQuery.
//!   AC-01-06: zero matching documents returns `count: 0`, never an error.
//!
//! Driving port: gRPC :8080 `RunAggregationQuery` (via `SecurityRulesFullContext`
//! + this feature's own `run_count_aggregation` helper — Pillar 3, real
//! Postgres, real gRPC, no mocks).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, mint_client_identity_token, now_unix, run_count_aggregation, string_field,
    SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-01: a COUNT aggregation filtered to a caller's own documents (per an
// ownership-equality access rule) returns the exact count of matching
// documents — the response carries only a computed number (AggregationResult
// has no `document` field at all — a type-level, not runtime, guarantee of
// "no document field data").
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trip_entries` has a READ rule requiring
///          `request.auth.uid == resource.data.owner_id`
///   And:   Maria Santos holds a verified identity and owns 3 seeded
///          `trip_entries` documents
///   When:  Alex's app runs a COUNT aggregation filtered to
///          `owner_id == "maria-santos"`
///   Then:  the response carries `count: 3`
///
/// AC-01-01
///
/// @driving_port @real-io @US-01 @AC-01-01
#[tokio::test]
async fn count_aggregation_over_callers_own_documents_returns_exact_count() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-agg01-count").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("trip_entries", "request.auth.uid == resource.data.owner_id")
        .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    for i in 0..3 {
        let mut fields = HashMap::new();
        fields.insert("owner_id".to_string(), string_field("maria-santos"));
        create_document(&ctx, "trip_entries", &format!("agg01-entry-{i}"), fields, Some(&marias_token))
            .await
            .expect("seed trip_entries document");
    }

    let count = run_count_aggregation(
        &ctx,
        "trip_entries",
        false,
        &[("owner_id", "maria-santos")],
        Some(&marias_token),
    )
    .await
    .expect("AC-01-01: a COUNT aggregation over the caller's own documents must be admitted");

    assert_eq!(count, 3, "expected exactly 3 matching documents, got {count}");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-04: a collection with no access rule defined runs the aggregation
// unrestricted — zero behavior change from pre-feature RunQuery on the same
// collection.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `daily_logs` has NO access rule defined at all
///   When:  an UNAUTHENTICATED caller (no client-identity token) runs a
///          COUNT aggregation against it
///   Then:  the aggregation succeeds unrestricted, returning the exact count
///
/// AC-01-04
///
/// @driving_port @real-io @US-01 @AC-01-04
#[tokio::test]
async fn count_aggregation_against_collection_with_no_access_rule_runs_unrestricted() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-agg01-norule").await;

    for i in 0..2 {
        let mut fields = HashMap::new();
        fields.insert("kind".to_string(), string_field("log"));
        create_document(&ctx, "daily_logs", &format!("agg01-log-{i}"), fields, None)
            .await
            .expect("seed daily_logs document");
    }

    let count = run_count_aggregation(&ctx, "daily_logs", false, &[], None)
        .await
        .expect("AC-01-04: a collection with no access rule defined must run the aggregation unrestricted");

    assert_eq!(count, 2, "expected exactly 2 documents, got {count}");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-06: zero matching documents returns a count of zero, not an error.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trip_entries` has the same ownership rule as AC-01-01, but this
///          fresh project has NO documents seeded at all
///   When:  Maria runs a COUNT aggregation filtered to her own uid
///   Then:  the response carries `count: 0`, not an error
///
/// AC-01-06
///
/// @driving_port @real-io @US-01 @AC-01-06
#[tokio::test]
async fn count_aggregation_with_zero_matching_documents_returns_zero() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-agg01-zero").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("trip_entries", "request.auth.uid == resource.data.owner_id")
        .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let count = run_count_aggregation(
        &ctx,
        "trip_entries",
        false,
        &[("owner_id", "maria-santos")],
        Some(&marias_token),
    )
    .await
    .expect("AC-01-06: zero matching documents must return a count, not an error");

    assert_eq!(count, 0, "expected count 0 for zero matching documents, got {count}");
}
