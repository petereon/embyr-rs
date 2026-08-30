//! aggregation-queries Slice 03 (US-03, ADR-040) — SUM Aggregation,
//! Postgres-Family Backend Modes.
//!
//! Acceptance criteria verified here (slice-03-sum.md):
//!   AC-01-11: a SUM aggregation on a numeric field, filtered to a caller's
//!             own documents, returns the correct total.
//!   AC-01-12: a document missing the summed field, or holding a
//!             non-numeric value for it, is silently excluded — never an
//!             error.
//!   AC-01-13: summing zero matching documents returns `sum: 0`, never an
//!             error.
//!   AC-01-14: SUM is governed by the identical `check_query_compliance()`
//!             mechanism as COUNT — the same unauthorized scenario is
//!             rejected identically.
//!   AC-01-15: an invalid field path for the summed field is rejected with
//!             InvalidArgument before any query executes.
//!
//! AC-01-16 (zero regression on COUNT) is verified by re-running Slices
//! 01/02's own COUNT test targets unmodified — no new test needed here.
//!
//! Driving port: gRPC :8080 `RunAggregationQuery` (via `SecurityRulesFullContext`
//! + this feature's own `run_sum_aggregation` helper — Pillar 3, real
//! Postgres, real gRPC, no mocks).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, integer_field, mint_client_identity_token, now_unix, run_sum_aggregation,
    string_field, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-11: a SUM aggregation on a numeric field, filtered to the caller's
// own documents, returns the correct total across all matching documents.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trip_expenses` has a READ rule requiring
///          `request.auth.uid == resource.data.owner_id`
///   And:   Maria Santos owns 3 seeded `trip_expenses` documents with
///          `amount_cents` 1000, 2000, 500
///   When:  Alex's app runs a SUM(amount_cents) aggregation filtered to
///          `owner_id == "maria-santos"`
///   Then:  the response carries `sum: 3500.0`
///
/// AC-01-11
///
/// @driving_port @real-io @US-03 @AC-01-11
#[tokio::test]
async fn sum_aggregation_over_callers_own_documents_returns_correct_total() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-agg03-sum").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("trip_expenses", "request.auth.uid == resource.data.owner_id")
        .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    for (i, amount) in [1000i64, 2000, 500].into_iter().enumerate() {
        let mut fields = HashMap::new();
        fields.insert("owner_id".to_string(), string_field("maria-santos"));
        fields.insert("amount_cents".to_string(), integer_field(amount));
        create_document(&ctx, "trip_expenses", &format!("agg03-expense-{i}"), fields, Some(&marias_token))
            .await
            .expect("seed trip_expenses document");
    }

    let sum = run_sum_aggregation(
        &ctx,
        "trip_expenses",
        false,
        "amount_cents",
        &[("owner_id", "maria-santos")],
        Some(&marias_token),
    )
    .await
    .expect("AC-01-11: a SUM aggregation over the caller's own documents must be admitted");

    assert_eq!(sum, 3500.0, "expected sum 3500.0, got {sum}");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-12: a document missing the summed field, or holding a non-numeric
// value for it, is silently excluded from the sum — never an error.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trip_expenses` has the same ownership rule as AC-01-11
///   And:   Maria owns 3 documents: one with `amount_cents: 1000` (valid),
///          one with NO `amount_cents` field at all, one with
///          `amount_cents: "not-a-number"` (string, not numeric)
///   When:  Alex's app runs a SUM(amount_cents) aggregation over Maria's docs
///   Then:  the response carries `sum: 1000.0` — only the valid numeric
///          document contributes
///
/// AC-01-12
///
/// @driving_port @real-io @US-03 @AC-01-12
#[tokio::test]
async fn sum_aggregation_silently_excludes_missing_and_non_numeric_values() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-agg03-mixed").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("trip_expenses", "request.auth.uid == resource.data.owner_id")
        .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let mut valid = HashMap::new();
    valid.insert("owner_id".to_string(), string_field("maria-santos"));
    valid.insert("amount_cents".to_string(), integer_field(1000));
    create_document(&ctx, "trip_expenses", "agg03-mixed-valid", valid, Some(&marias_token))
        .await
        .expect("seed valid document");

    let mut missing = HashMap::new();
    missing.insert("owner_id".to_string(), string_field("maria-santos"));
    create_document(&ctx, "trip_expenses", "agg03-mixed-missing", missing, Some(&marias_token))
        .await
        .expect("seed document missing amount_cents");

    let mut non_numeric = HashMap::new();
    non_numeric.insert("owner_id".to_string(), string_field("maria-santos"));
    non_numeric.insert("amount_cents".to_string(), string_field("not-a-number"));
    create_document(&ctx, "trip_expenses", "agg03-mixed-string", non_numeric, Some(&marias_token))
        .await
        .expect("seed document with non-numeric amount_cents");

    let sum = run_sum_aggregation(
        &ctx,
        "trip_expenses",
        false,
        "amount_cents",
        &[("owner_id", "maria-santos")],
        Some(&marias_token),
    )
    .await
    .expect("AC-01-12: SUM over a mix of valid/missing/non-numeric values must be admitted");

    assert_eq!(
        sum, 1000.0,
        "expected sum 1000.0 (only the valid numeric document), got {sum}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-13: summing zero matching documents returns `sum: 0`, never an
// error.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trip_expenses` has the same ownership rule as AC-01-11, but
///          this fresh project has NO documents seeded at all
///   When:  Maria runs a SUM(amount_cents) aggregation filtered to her own
///          uid
///   Then:  the response carries `sum: 0.0`, not an error
///
/// AC-01-13
///
/// @driving_port @real-io @US-03 @AC-01-13
#[tokio::test]
async fn sum_aggregation_with_zero_matching_documents_returns_zero() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-agg03-zero").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("trip_expenses", "request.auth.uid == resource.data.owner_id")
        .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let sum = run_sum_aggregation(
        &ctx,
        "trip_expenses",
        false,
        "amount_cents",
        &[("owner_id", "maria-santos")],
        Some(&marias_token),
    )
    .await
    .expect("AC-01-13: zero matching documents must return a sum, not an error");

    assert_eq!(sum, 0.0, "expected sum 0.0 for zero matching documents, got {sum}");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-14: SUM aggregation is governed by the identical
// `check_query_compliance()` mechanism as COUNT — the same unauthorized
// scenario is rejected identically. Mirrors
// count_aggregation_rejection::counting_another_users_documents_is_rejected_as_permission_denial
// exactly, with SUM substituted for COUNT.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trip_expenses` has a READ rule requiring
///          `request.auth.uid == resource.data.owner_id`
///   And:   Dana Kim holds a verified identity (uid "dana-kim")
///   When:  Dana runs a SUM(amount_cents) aggregation filtered to
///          `owner_id == "maria-santos"` — Maria's uid, not Dana's own
///   Then:  the request is rejected as a permission denial, never returning
///          a sum derived from Maria's data
///
/// AC-01-14
///
/// @error @driving_port @real-io @US-03 @AC-01-14 @security-critical
#[tokio::test]
async fn sum_aggregation_over_another_users_documents_is_rejected_as_permission_denial() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-agg03-deny").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("trip_expenses", "request.auth.uid == resource.data.owner_id")
        .await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let result = run_sum_aggregation(
        &ctx,
        "trip_expenses",
        false,
        "amount_cents",
        &[("owner_id", "maria-santos")],
        Some(&danas_token),
    )
    .await;

    let err = result.expect_err(
        "AC-01-14: a SUM aggregation attempting to sum another user's documents must be \
         rejected, not admitted",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-01-14: denial must be attributable to the rule (PermissionDenied), got {:?}: {}",
        err.code(),
        err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-15: an invalid field path for the summed field is rejected with
// InvalidArgument before any query executes.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `daily_logs` has NO access rule defined (irrelevant — rejection
///          must happen before any query executes, per ADR-040 § 1)
///   When:  a caller runs a SUM aggregation on a field path shaped like a
///          SQL-injection payload
///   Then:  the request is rejected with InvalidArgument, never reaching
///          the adapter
///
/// AC-01-15
///
/// @error @driving_port @real-io @US-03 @AC-01-15 @security-critical
#[tokio::test]
async fn sum_aggregation_with_invalid_field_path_is_rejected_before_execution() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-agg03-badpath").await;

    let result = run_sum_aggregation(
        &ctx,
        "daily_logs",
        false,
        "amount'); DROP TABLE documents; --",
        &[],
        None,
    )
    .await;

    let err = result.expect_err(
        "AC-01-15: a non-compliant field path must be rejected before any query executes",
    );
    assert_eq!(
        err.code(),
        tonic::Code::InvalidArgument,
        "AC-01-15: rejection must be InvalidArgument, got {:?}: {}",
        err.code(),
        err.message()
    );
}
