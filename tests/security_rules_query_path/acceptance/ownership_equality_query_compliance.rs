//! Slice 01 (US-01, ADR-031) — Ownership-Equality Query Compliance (Walking
//! Skeleton).
//!
//! Acceptance criteria verified here (feature-delta.md US-01):
//!   AC-17-49: a RunQuery whose filter includes an equality constraint on
//!             the rule's referenced field, bound to the caller's own
//!             verified `request.auth.uid`, is admitted and executes.
//!   AC-17-50: additional filters beyond the required one do not affect
//!             compliance.
//!   AC-17-51: THE single most security-critical property in this feature —
//!             a filter on the correct field but bound to a value OTHER
//!             than the caller's own verified uid is rejected before
//!             execution.
//!   AC-17-52: field-reference matching is exact-string, case-sensitive.
//!   (regression, not separately numbered) a RunQuery against a collection
//!             with NO read rule defined succeeds exactly as before this
//!             feature existed.
//!
//! Driving port: gRPC :8080 `RunQuery` (via `SecurityRulesFullContext` +
//! this feature's own `run_query`/`create_document` helpers — Pillar 3,
//! real Postgres, real gRPC, no mocks).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, mint_client_identity_token, now_unix, run_query, string_field,
    SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-49: a RunQuery whose filter includes an equality constraint on the
// rule's referenced field, bound to the caller's own verified
// `request.auth.uid`, is admitted and executes.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has a READ rule requiring
///          `request.auth.uid == resource.data.owner_id`
///   And:   Maria Santos holds a verified identity and owns a seeded
///          document (`owner_id: "maria-santos"`)
///   When:  Maria calls `RunQuery` with filter `owner_id == "maria-santos"`
///          (her own uid)
///   Then:  the query is admitted and returns her own document
///
/// AC-17-49
///
/// @driving_port @real-io @US-01 @AC-17-49
#[tokio::test]
async fn a_query_filter_binding_owner_id_to_the_callers_own_uid_is_admitted_and_executes() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp01-allow").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let docs = run_query(
        &ctx,
        "journal_entries",
        &[("owner_id", "maria-santos")],
        Some(&marias_token),
    )
    .await
    .expect("AC-17-49: a query filter binding owner_id to the caller's own uid must be admitted");

    assert_eq!(
        docs.len(),
        1,
        "expected exactly Maria's own seeded document, got {} documents",
        docs.len()
    );
    assert!(
        docs[0].name.ends_with("-maria-doc"),
        "expected Maria's own document, got {}",
        docs[0].name
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-50: additional filters beyond the required one do not affect
// compliance — the query can have MORE filters than strictly required and
// still be admitted.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trip_logs` has a READ rule requiring
///          `request.auth.uid == resource.data.owner_id`
///   And:   Maria Santos holds a verified identity and owns a document with
///          BOTH `owner_id` and `status` fields
///   When:  Maria calls `RunQuery` with filters `owner_id == "maria-santos"`
///          AND `status == "active"` (an extra filter beyond what the rule
///          requires)
///   Then:  the query is admitted and executes, returning her document
///
/// AC-17-50
///
/// @driving_port @real-io @US-01 @AC-17-50
#[tokio::test]
async fn additional_filters_beyond_the_required_one_do_not_prevent_admission() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp01-extra").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("trip_logs", "request.auth.uid == resource.data.owner_id")
        .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let mut fields = std::collections::HashMap::new();
    fields.insert("owner_id".to_string(), string_field("maria-santos"));
    fields.insert("status".to_string(), string_field("active"));
    create_document(&ctx, "trip_logs", "sqp01-extra-doc", fields, Some(&marias_token))
        .await
        .expect("seed trip_logs document");

    let docs = run_query(
        &ctx,
        "trip_logs",
        &[("owner_id", "maria-santos"), ("status", "active")],
        Some(&marias_token),
    )
    .await
    .expect("AC-17-50: extra filters beyond the required one must not prevent admission");

    assert_eq!(docs.len(), 1, "expected exactly 1 document, got {}", docs.len());
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-51 — THE single most security-critical property in this feature: a
// filter on the correct field but bound to a value OTHER than the caller's
// own verified uid is rejected before execution.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has a READ rule requiring
///          `request.auth.uid == resource.data.owner_id`
///   And:   Dana Kim holds a verified identity (uid "dana-kim")
///   When:  Dana calls `RunQuery` with filter `owner_id == "maria-santos"`
///          — Maria's uid, NOT Dana's own uid — syntactically on the right
///          field, with the right operator
///   Then:  the query is REJECTED before execution, never returning any of
///          Maria's data. A compliance check that only verifies "a filter
///          exists on the right field" would wrongly admit this and let
///          Dana enumerate Maria's data by writing Maria's id as the
///          literal filter value.
///
/// AC-17-51
///
/// @error @driving_port @real-io @US-01 @AC-17-51 @security-critical
#[tokio::test]
async fn a_filter_on_the_right_field_bound_to_someone_elses_uid_is_rejected() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp01-deny").await;
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

    let result = run_query(
        &ctx,
        "journal_entries",
        &[("owner_id", "maria-santos")],
        Some(&danas_token),
    )
    .await;

    let err = result.expect_err(
        "AC-17-51 (security-critical): a filter on the right field bound to ANOTHER caller's \
         uid must be REJECTED, not admitted — field-name presence alone is never proof of \
         entitlement",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-51: denial must be attributable to the rule (PermissionDenied), got {:?}: {}",
        err.code(),
        err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-52: field-reference matching is exact-string, case-sensitive — no
// fuzzy or partial matching.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has a READ rule requiring
///          `request.auth.uid == resource.data.owner_id` (lowercase field)
///   And:   Maria Santos holds a verified identity
///   When:  Maria calls `RunQuery` with filter `Owner_Id == "maria-santos"`
///          (different case, otherwise identical value)
///   Then:  the query is REJECTED — the rule's `owner_id` reference is not
///          satisfied by a differently-cased filter field
///
/// AC-17-52
///
/// @error @driving_port @real-io @US-01 @AC-17-52
#[tokio::test]
async fn field_reference_matching_is_exact_string_case_sensitive() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp01-case").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let result = run_query(
        &ctx,
        "journal_entries",
        &[("Owner_Id", "maria-santos")],
        Some(&marias_token),
    )
    .await;

    let err = result.expect_err(
        "AC-17-52: a differently-cased filter field must NOT satisfy the rule's exact-case \
         field reference",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-52: denial must be attributable to the rule (PermissionDenied), got {:?}: {}",
        err.code(),
        err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Regression guardrail (not one of the 4 numbered ACs above, but essential
// coverage for this slice, proven immediately rather than deferred — mirrors
// every prior slice's own "no rule = unaffected" case): a RunQuery against a
// collection with NO read rule defined succeeds exactly as before this
// feature existed.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trail_guides` has NO read rule defined at all
///   When:  an UNAUTHENTICATED `RunQuery` (no client-identity token) is made
///          against it
///   Then:  the query succeeds — the structural no-rule-defined guardrail
///          means `get_access_rule` returns `None` and
///          `check_query_compliance` is never invoked at all
///
/// (regression guardrail, no dedicated AC number)
///
/// @driving_port @real-io @US-01
#[tokio::test]
async fn a_query_against_a_collection_with_no_read_rule_succeeds_unchanged() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp01-norule").await;

    let docs = run_query(&ctx, "trail_guides", &[], None)
        .await
        .expect("a query against a collection with no read rule defined must succeed unchanged");

    assert_eq!(
        docs.len(),
        1,
        "expected the default seeded trail_guides document, got {} documents",
        docs.len()
    );
}
