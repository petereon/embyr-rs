//! Slice 02 (US-02, ADR-031) — Non-Compliant Query Rejected Before Execution.
//!
//! Acceptance criteria verified here (feature-delta.md US-02):
//!   AC-17-53: a RunQuery with NO filter, against a rule requiring a
//!             matching equality conjunct, is rejected before any document
//!             is fetched.
//!   AC-17-54: a RunQuery whose filters exist but omit the required
//!             conjunct is rejected — other valid filters present do not
//!             substitute for the missing one.
//!   AC-17-55: a different operator (`!=`) on the rule's referenced field
//!             does not satisfy an equality (`==`) rule.
//!   AC-17-56: the rejection response names the specific missing
//!             constraint, distinguishable from `authenticate()`-level
//!             rejections (e.g. bad api_key) and composite-index
//!             rejections.
//!
//! Slice 01 (`ownership_equality_query_compliance.rs`) already proved the
//! CORE matching algorithm (`check_query_compliance`/`decompose_decidable`)
//! is correct — its own placeholder rejection message just said "N
//! unsatisfied filter requirement(s)". This slice finishes the rejection
//! RESPONSE quality: the message now embeds a stable `[REASON_CODE]` token
//! (`UnsatisfiedConjunct::reason_code()`, ADR-031 § Decision — Rejection
//! Response Shape) plus the specific missing field, so a caller-facing
//! (and, at Release 2, simulation-facing) consumer can programmatically
//! distinguish "which constraint failed" from a bare pass/fail.
//!
//! Driving port: gRPC :8080 `RunQuery` (via `SecurityRulesFullContext` +
//! this feature's own `run_query`/`run_query_raw`/`not_equal_filter`
//! helpers — Pillar 3, real Postgres, real gRPC, no mocks).
//!
//! Zero-Postgres-row-fetch-I/O verification (AC-17-53): this codebase has no
//! query-counter/mock instrumentation at the adapter boundary for RunQuery
//! (confirmed: no such helper exists in `tests/security_rules_query_path/`,
//! `tests/security_rules/`, or `tests/security_rules_write_path/` — the
//! established convention here is response-assertion plus code-path
//! reasoning). `grpc/handler.rs::handle_run_query` calls
//! `check_query_compliance` and returns on non-`Admitted` outcomes (line
//! ~1266-1268) strictly BEFORE `adapter.run_query()` is ever called (line
//! ~1285) — a rejected query never reaches the adapter call at all, proven
//! by direct code read, the same convention Slice 01's own tests rely on.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    mint_client_identity_token, not_equal_filter, now_unix, run_query, run_query_raw,
    SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-53: a RunQuery with NO filter, against a rule requiring a matching
// equality conjunct, is rejected before any document is fetched.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `field_notes` has a READ rule requiring
///          `request.auth.uid == resource.data.owner_id`
///   And:   Maria Santos holds a verified identity
///   When:  Maria calls `RunQuery` with NO filter at all
///   Then:  the query is rejected before execution, and the rejection names
///          the specific missing constraint (`OWNERSHIP_FILTER_MISSING` on
///          `owner_id`) — not a generic denial
///
/// AC-17-53
///
/// @error @driving_port @real-io @US-02 @AC-17-53
#[tokio::test]
async fn an_unfiltered_query_against_an_ownership_rule_is_rejected_naming_the_missing_constraint()
{
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp02-nofilter").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("field_notes", "request.auth.uid == resource.data.owner_id")
        .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let result = run_query(&ctx, "field_notes", &[], Some(&marias_token)).await;

    let err = result.expect_err(
        "AC-17-53: an unfiltered RunQuery against a rule requiring ownership equality must be \
         rejected before execution",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-53: rejection must be attributable to the rule (PermissionDenied), got {:?}: {}",
        err.code(),
        err.message()
    );
    assert!(
        err.message().contains("[OWNERSHIP_FILTER_MISSING]"),
        "AC-17-53/AC-17-56: rejection message must name the specific missing constraint via a \
         stable reason code, got: {}",
        err.message()
    );
    assert!(
        err.message().contains("owner_id"),
        "AC-17-53: rejection message must name the specific missing field, got: {}",
        err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-54: a RunQuery whose filters exist but omit the required conjunct is
// rejected — other valid filters present do not substitute for the missing
// one.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `expense_reports` has a READ rule requiring
///          `request.auth.uid == resource.data.owner_id`
///   And:   Maria Santos holds a verified identity
///   When:  Maria calls `RunQuery` with a filter on an UNRELATED field
///          (`status == "active"`) — present, valid, but not the required
///          conjunct
///   Then:  the query is rejected — the unrelated filter does not
///          substitute for the missing ownership-equality filter
///
/// AC-17-54
///
/// @error @driving_port @real-io @US-02 @AC-17-54
#[tokio::test]
async fn a_query_with_only_unrelated_filters_is_rejected_the_unrelated_filter_does_not_substitute()
{
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp02-unrelated").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "expense_reports",
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
        "expense_reports",
        &[("status", "active")],
        Some(&marias_token),
    )
    .await;

    let err = result.expect_err(
        "AC-17-54: a filter on an unrelated field must NOT substitute for the required \
         ownership-equality filter",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-54: rejection must be attributable to the rule (PermissionDenied), got {:?}: {}",
        err.code(),
        err.message()
    );
    assert!(
        err.message().contains("[OWNERSHIP_FILTER_MISSING]") && err.message().contains("owner_id"),
        "AC-17-54: rejection message must still name 'owner_id' as the missing constraint, even \
         though an unrelated filter was present, got: {}",
        err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-55: a different operator (`!=`) on the rule's referenced field does
// not satisfy an equality (`==`) rule.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `budget_entries` has a READ rule requiring
///          `request.auth.uid == resource.data.owner_id`
///   And:   Maria Santos holds a verified identity
///   When:  Maria calls `RunQuery` with `owner_id != "maria-santos"` —
///          syntactically on the right field, but the WRONG operator
///   Then:  the query is rejected — a filter's field-name presence with the
///          wrong operator is never proof of entitlement
///
/// AC-17-55
///
/// @error @driving_port @real-io @US-02 @AC-17-55
#[tokio::test]
async fn a_not_equal_filter_on_the_referenced_field_does_not_satisfy_an_equality_rule() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp02-wrongop").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("budget_entries", "request.auth.uid == resource.data.owner_id")
        .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let result = run_query_raw(
        &ctx,
        "budget_entries",
        Some(not_equal_filter("owner_id", "maria-santos")),
        Some(&marias_token),
        &ctx.api_key,
    )
    .await;

    let err = result.expect_err(
        "AC-17-55: a '!=' filter on the rule's referenced field must NOT satisfy an '==' \
         ownership-equality rule",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-55: rejection must be attributable to the rule (PermissionDenied), got {:?}: {}",
        err.code(),
        err.message()
    );
    assert!(
        err.message().contains("[OWNERSHIP_FILTER_MISSING]"),
        "AC-17-55: rejection message must name the missing constraint even though a filter on \
         the right field was present with the wrong operator, got: {}",
        err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-56: the rejection response names the specific missing constraint,
// distinguishable from `authenticate()`-level rejections (e.g. bad api_key).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `daily_logs` has a READ rule requiring
///          `request.auth.uid == resource.data.owner_id`
///   When:  (a) a verified caller runs an unfiltered `RunQuery` (a
///          compliance rejection) and (b) a caller presents a wrong
///          `api_key` against the SAME collection (an `authenticate()`-level
///          rejection)
///   Then:  the two rejections are distinguishable both by gRPC status CODE
///          (`PermissionDenied` vs `Unauthenticated`) and by message
///          content — the compliance rejection carries the
///          `[OWNERSHIP_FILTER_MISSING]` reason-code token, the
///          authentication rejection never does. (The THIRD rejection
///          class, composite-index `FAILED_PRECONDITION`, is proven
///          distinguishable by construction: it is produced by a different,
///          untouched code path — `Self::requires_composite_index`,
///          `grpc/handler.rs` line ~1273 — using the unrelated static
///          message "query requires a composite index; create the index
///          before running this query", a different gRPC status code, and
///          it never executes at all for a query already rejected by
///          compliance, per ADR-031 § OQ-SRQ-03 Resolution: compliance
///          rejection runs strictly BEFORE the composite-index check.)
///
/// AC-17-56
///
/// @error @driving_port @real-io @US-02 @AC-17-56
#[tokio::test]
async fn the_compliance_rejection_reason_is_distinguishable_from_an_authenticate_level_rejection()
{
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp02-distinct").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("daily_logs", "request.auth.uid == resource.data.owner_id")
        .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let compliance_err = run_query(&ctx, "daily_logs", &[], Some(&marias_token))
        .await
        .expect_err("expected a compliance rejection for an unfiltered query");

    let auth_err = run_query_raw(
        &ctx,
        "daily_logs",
        None,
        Some(&marias_token),
        "wrong-api-key-totally-invalid",
    )
    .await
    .expect_err("expected an authenticate()-level rejection for a wrong api_key");

    assert_eq!(
        compliance_err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-56: compliance rejection must be PermissionDenied, got {:?}",
        compliance_err.code()
    );
    assert_eq!(
        auth_err.code(),
        tonic::Code::Unauthenticated,
        "AC-17-56: authenticate()-level rejection must be Unauthenticated, got {:?}",
        auth_err.code()
    );
    assert_ne!(
        compliance_err.code(),
        auth_err.code(),
        "AC-17-56: the two rejection classes must use different gRPC status codes"
    );

    assert!(
        compliance_err.message().contains("[OWNERSHIP_FILTER_MISSING]"),
        "AC-17-56: the compliance rejection must carry the reason-code token, got: {}",
        compliance_err.message()
    );
    assert!(
        !auth_err.message().contains("[OWNERSHIP_FILTER_MISSING]"),
        "AC-17-56: the authenticate()-level rejection must NEVER carry the compliance \
         reason-code token, got: {}",
        auth_err.message()
    );
    assert_ne!(
        compliance_err.message(),
        auth_err.message(),
        "AC-17-56: the two rejection messages must be textually distinguishable"
    );
}
