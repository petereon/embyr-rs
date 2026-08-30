//! aggregation-queries Slice 04 (US-04, ADR-040) — AVG Aggregation,
//! Postgres-Family Backend Modes.
//!
//! Acceptance criteria verified here (slice-04-average.md):
//!   AC-01-17: an AVG aggregation on a numeric field, filtered to a caller's
//!             own documents, returns the correct average.
//!   AC-01-18: a document missing the averaged field, or holding a
//!             non-numeric value for it, is silently excluded from both the
//!             numerator and denominator — mirrors Slice 03's own AC-01-12.
//!   AC-01-19: averaging across zero matching documents returns an
//!             absent/null average — never `0`, never a divide-by-zero
//!             error. This is the slice's single highest-consequence
//!             scenario (slice-04-average.md's own "Note") — the assertion
//!             below distinguishes the exact `NullValue` wire variant from
//!             a `DoubleValue(0.0)`, not merely "not equal to some wrong
//!             number".
//!   AC-01-20: AVG is governed by the identical `check_query_compliance()`
//!             mechanism as COUNT/SUM — the same unauthorized scenario is
//!             rejected identically.
//!
//! AC-01-21 (zero regression on COUNT/SUM) is verified by re-running Slices
//! 01/02/03's own COUNT/SUM test targets unmodified — no new test needed
//! here.
//!
//! Driving port: gRPC :8080 `RunAggregationQuery` (via `SecurityRulesFullContext`
//! + this feature's own `run_avg_aggregation` helper — Pillar 3, real
//! Postgres, real gRPC, no mocks).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, integer_field, mint_client_identity_token, now_unix, run_avg_aggregation,
    string_field, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-17: an AVG aggregation on a numeric field, filtered to the caller's
// own documents, returns the correct average across all matching documents.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trip_expenses` has a READ rule requiring
///          `request.auth.uid == resource.data.owner_id`
///   And:   Maria Santos owns 3 seeded `trip_expenses` documents with
///          `amount_cents` 1000, 2000, 300
///   When:  Alex's app runs an AVG(amount_cents) aggregation filtered to
///          `owner_id == "maria-santos"`
///   Then:  the response carries `avg: 1100.0` ((1000+2000+300)/3)
///
/// AC-01-17
///
/// @driving_port @real-io @US-04 @AC-01-17
#[tokio::test]
async fn avg_aggregation_over_callers_own_documents_returns_correct_average() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-agg04-avg").await;
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

    for (i, amount) in [1000i64, 2000, 300].into_iter().enumerate() {
        let mut fields = HashMap::new();
        fields.insert("owner_id".to_string(), string_field("maria-santos"));
        fields.insert("amount_cents".to_string(), integer_field(amount));
        create_document(&ctx, "trip_expenses", &format!("agg04-expense-{i}"), fields, Some(&marias_token))
            .await
            .expect("seed trip_expenses document");
    }

    let avg = run_avg_aggregation(
        &ctx,
        "trip_expenses",
        false,
        "amount_cents",
        &[("owner_id", "maria-santos")],
        Some(&marias_token),
    )
    .await
    .expect("AC-01-17: an AVG aggregation over the caller's own documents must be admitted");

    assert_eq!(avg, Some(1100.0), "expected avg Some(1100.0), got {avg:?}");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-18: a document missing the averaged field, or holding a non-numeric
// value for it, is silently excluded from both the numerator and
// denominator — mirrors Slice 03's own AC-01-12.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trip_expenses` has the same ownership rule as AC-01-17
///   And:   Maria owns 3 documents: one with `amount_cents: 1000` (valid),
///          one with NO `amount_cents` field at all, one with
///          `amount_cents: "not-a-number"` (string, not numeric)
///   When:  Alex's app runs an AVG(amount_cents) aggregation over Maria's
///          docs
///   Then:  the response carries `avg: 1000.0` — the excluded documents
///          count toward NEITHER the numerator NOR the denominator (an avg
///          of `500.0` would mean they were counted in the denominator only,
///          which this asserts against)
///
/// AC-01-18
///
/// @driving_port @real-io @US-04 @AC-01-18
#[tokio::test]
async fn avg_aggregation_silently_excludes_missing_and_non_numeric_values() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-agg04-mixed").await;
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
    create_document(&ctx, "trip_expenses", "agg04-mixed-valid", valid, Some(&marias_token))
        .await
        .expect("seed valid document");

    let mut missing = HashMap::new();
    missing.insert("owner_id".to_string(), string_field("maria-santos"));
    create_document(&ctx, "trip_expenses", "agg04-mixed-missing", missing, Some(&marias_token))
        .await
        .expect("seed document missing amount_cents");

    let mut non_numeric = HashMap::new();
    non_numeric.insert("owner_id".to_string(), string_field("maria-santos"));
    non_numeric.insert("amount_cents".to_string(), string_field("not-a-number"));
    create_document(&ctx, "trip_expenses", "agg04-mixed-string", non_numeric, Some(&marias_token))
        .await
        .expect("seed document with non-numeric amount_cents");

    let avg = run_avg_aggregation(
        &ctx,
        "trip_expenses",
        false,
        "amount_cents",
        &[("owner_id", "maria-santos")],
        Some(&marias_token),
    )
    .await
    .expect("AC-01-18: AVG over a mix of valid/missing/non-numeric values must be admitted");

    assert_eq!(
        avg,
        Some(1000.0),
        "expected avg Some(1000.0) (only the valid numeric document counted in numerator AND \
         denominator), got {avg:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-19: averaging across zero matching documents returns an absent/null
// average — never a divide-by-zero error, never reported as `0`. This is
// the slice's single highest-consequence scenario.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trip_expenses` has the same ownership rule as AC-01-17, but
///          this fresh project has NO documents seeded at all
///   When:  Maria runs an AVG(amount_cents) aggregation filtered to her own
///          uid
///   Then:  the response carries the alias key present in `aggregate_fields`
///          with a value that is EXACTLY the `NullValue` proto variant — not
///          absent from the map, not `DoubleValue(0.0)`. `run_avg_aggregation`
///          itself enforces this distinction structurally: its match arms
///          only accept `DoubleValue` (-> `Some`) or `NullValue` (-> `None`)
///          and panic on any other wire shape (including an absent field,
///          which panics at `.expect(...)` on the map lookup) — so a `None`
///          returned here is proof the wire value was specifically
///          `NullValue`, not proof-by-absence-of-a-wrong-number.
///
/// AC-01-19
///
/// @driving_port @real-io @US-04 @AC-01-19 @correctness-critical
#[tokio::test]
async fn avg_aggregation_with_zero_matching_documents_returns_null_not_zero() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-agg04-zero").await;
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

    let avg = run_avg_aggregation(
        &ctx,
        "trip_expenses",
        false,
        "amount_cents",
        &[("owner_id", "maria-santos")],
        Some(&marias_token),
    )
    .await
    .expect("AC-01-19: zero matching documents must return a response, not an error");

    assert_eq!(
        avg, None,
        "AC-01-19: expected the alias field present with a NullValue (decoded here as `None`), \
         never a DoubleValue(0.0) or an absent field — got {avg:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-20: AVG aggregation is governed by the identical
// `check_query_compliance()` mechanism as COUNT/SUM — the same unauthorized
// scenario is rejected identically. Mirrors
// sum_aggregation::sum_aggregation_over_another_users_documents_is_rejected_as_permission_denial
// exactly, with AVG substituted for SUM.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trip_expenses` has a READ rule requiring
///          `request.auth.uid == resource.data.owner_id`
///   And:   Dana Kim holds a verified identity (uid "dana-kim")
///   When:  Dana runs an AVG(amount_cents) aggregation filtered to
///          `owner_id == "maria-santos"` — Maria's uid, not Dana's own
///   Then:  the request is rejected as a permission denial, never returning
///          an average derived from Maria's data
///
/// AC-01-20
///
/// @error @driving_port @real-io @US-04 @AC-01-20 @security-critical
#[tokio::test]
async fn avg_aggregation_over_another_users_documents_is_rejected_as_permission_denial() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-agg04-deny").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("trip_expenses", "request.auth.uid == resource.data.owner_id")
        .await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let result = run_avg_aggregation(
        &ctx,
        "trip_expenses",
        false,
        "amount_cents",
        &[("owner_id", "maria-santos")],
        Some(&danas_token),
    )
    .await;

    let err = result.expect_err(
        "AC-01-20: an AVG aggregation attempting to average another user's documents must be \
         rejected, not admitted",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-01-20: denial must be attributable to the rule (PermissionDenied), got {:?}: {}",
        err.code(),
        err.message()
    );
}
