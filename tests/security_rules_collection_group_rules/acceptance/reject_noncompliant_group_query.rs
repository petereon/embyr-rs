//! security-rules-collection-group-rules Slice 03 (US-03, ADR-032) — A
//! Non-Compliant Collection-Group Query Is Rejected Before Execution, Not
//! Filtered After.
//!
//! Production code for this slice already exists: Slice 04 (commit
//! `a570a09`) built the FULL `all_descendants` branch atomically — the
//! `Some(group_rule_row)` arm in `handle_run_query` calls
//! `check_query_compliance()` and, on any non-`Admitted` outcome, returns
//! `Err(query_compliance_rejection(&outcome))` (`grpc/handler.rs`
//! lines ~1272-1279) — the EXACT SAME mechanism
//! `security-rules-query-path` already built (ADR-031) and confirmed by
//! Slice 02 (commit `675738f`) for the admit path. This slice adds
//! acceptance-test coverage proving that already-shipped rejection
//! mechanism composes correctly for the group-rule call site's reject arm,
//! mirroring how `security-rules-query-path`'s own Slice 02
//! (`noncompliant_query_rejected_preexecution.rs`) added zero new
//! production code on top of its own Slice 01.
//!
//! Acceptance criteria verified here (feature-delta.md
//! `security-rules-collection-group-rules`, US-03):
//!   AC-17-85: a collection-group query with NO filter, against a group
//!             rule requiring a matching equality conjunct, is rejected
//!             before any document is fetched.
//!   AC-17-86: a collection-group query whose filters exist but omit the
//!             required conjunct is rejected — other valid filters present
//!             do not substitute.
//!   AC-17-87: a different operator (`!=`) on the group rule's referenced
//!             field does not satisfy an equality group rule.
//!   AC-17-88: each rejection is distinguishable from Slice 04's "no group
//!             rule defined" rejection (`[GROUP_RULE_NOT_DEFINED]`) — folded
//!             into each of the three tests below as an additional
//!             assertion rather than a fourth, separate test, since it is a
//!             property of each rejection outcome, not an independent
//!             behavior (Behavior-First Test Budget: 3 behaviors, not 4).
//!
//! Zero-Postgres-row-fetch-I/O verification (AC-17-85): identical reasoning
//! to `security_rules_query_path`'s own precedent — `handle_run_query`'s
//! group-rule arm calls `check_query_compliance` and returns on
//! non-`Admitted` outcomes strictly BEFORE `adapter.run_query()` is ever
//! reached (confirmed by direct code read of `grpc/handler.rs`
//! lines ~1272-1279, ~1321+).
//!
//! Driving port: gRPC :8080 `RunQuery(all_descendants = true)`
//! (`SecurityRulesFullContext`, real production composition root).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    mint_client_identity_token, not_equal_filter, now_unix, run_query, run_query_raw,
    seed_group_access_rule_full, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-85 + AC-17-88: a collection-group query with no filter at all,
// against a collection id whose group rule requires a matching ownership
// equality conjunct, is rejected before any document is fetched — and the
// rejection is distinguishable from Slice 04's "no group rule defined"
// rejection.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has an active collection-group rule requiring
///          `request.auth.uid == resource.data.owner_id`
///   And:   Maria Santos holds a verified identity
///   When:  Maria issues a `collectionGroup('journal_entries')` query with
///          NO filter at all
///   Then:  the query is rejected before execution, naming the specific
///          missing constraint (`[OWNERSHIP_FILTER_MISSING]` on
///          `owner_id`) — never the "no group rule defined" reason, since a
///          group rule genuinely IS defined here
///
/// AC-17-85, AC-17-88
///
/// @error @driving_port @real-io @US-03 @AC-17-85 @AC-17-88
#[tokio::test]
async fn an_unfiltered_group_query_against_an_ownership_rule_is_rejected_naming_the_missing_constraint()
{
    let ctx = SecurityRulesFullContext::new("trailmark-prod-scgr03-nofilter").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    seed_group_access_rule_full(
        &ctx,
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

    let result = run_query(&ctx, "journal_entries", true, &[], Some(&marias_token)).await;

    let err = result.expect_err(
        "AC-17-85: an unfiltered collection-group query against a group rule requiring \
         ownership equality must be rejected before execution",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-85: rejection must be attributable to the group rule (PermissionDenied), got \
         {:?}: {}",
        err.code(),
        err.message()
    );
    assert!(
        err.message().contains("[OWNERSHIP_FILTER_MISSING]"),
        "AC-17-85: rejection message must name the specific missing constraint via a stable \
         reason code, got: {}",
        err.message()
    );
    assert!(
        err.message().contains("owner_id"),
        "AC-17-85: rejection message must name the specific missing field, got: {}",
        err.message()
    );
    assert!(
        !err.message().contains("[GROUP_RULE_NOT_DEFINED]"),
        "AC-17-88: this rejection must be distinguishable from Slice 04's 'no group rule \
         defined' rejection — a group rule genuinely IS defined here, got: {}",
        err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-86 + AC-17-88: a collection-group query whose filters exist but omit
// the required conjunct is rejected — other valid filters present do not
// substitute for the missing one.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has an active collection-group rule requiring
///          `request.auth.uid == resource.data.owner_id`
///   And:   Maria Santos holds a verified identity
///   When:  Maria issues a `collectionGroup('journal_entries')` query
///          filtered only on an UNRELATED field (`archived == false`) —
///          present, valid, but not the required conjunct
///   Then:  the query is rejected — the unrelated filter does not
///          substitute for the missing ownership-equality filter, and the
///          rejection is distinguishable from Slice 04's "no group rule
///          defined" rejection
///
/// AC-17-86, AC-17-88
///
/// @error @driving_port @real-io @US-03 @AC-17-86 @AC-17-88
#[tokio::test]
async fn a_group_query_with_only_unrelated_filters_is_rejected_the_unrelated_filter_does_not_substitute()
{
    let ctx = SecurityRulesFullContext::new("trailmark-prod-scgr03-unrelated").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    seed_group_access_rule_full(
        &ctx,
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
        true,
        &[("archived", "false")],
        Some(&marias_token),
    )
    .await;

    let err = result.expect_err(
        "AC-17-86: a filter on an unrelated field must NOT substitute for the required \
         ownership-equality filter in a collection-group query",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-86: rejection must be attributable to the group rule (PermissionDenied), got \
         {:?}: {}",
        err.code(),
        err.message()
    );
    assert!(
        err.message().contains("[OWNERSHIP_FILTER_MISSING]") && err.message().contains("owner_id"),
        "AC-17-86: rejection message must still name 'owner_id' as the missing constraint, even \
         though an unrelated filter was present, got: {}",
        err.message()
    );
    assert!(
        !err.message().contains("[GROUP_RULE_NOT_DEFINED]"),
        "AC-17-88: this rejection must be distinguishable from Slice 04's 'no group rule \
         defined' rejection — a group rule genuinely IS defined here, got: {}",
        err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-87 + AC-17-88: a different operator (`!=`) on the group rule's
// referenced field does not satisfy an equality (`==`) group rule.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has an active collection-group rule requiring
///          `request.auth.uid == resource.data.owner_id`
///   And:   Maria Santos holds a verified identity
///   When:  Maria issues a `collectionGroup('journal_entries')` query with
///          `owner_id != "maria-santos"` — syntactically on the right
///          field, but the WRONG operator
///   Then:  the query is rejected — a filter's field-name presence with the
///          wrong operator is never proof of entitlement, and the
///          rejection is distinguishable from Slice 04's "no group rule
///          defined" rejection
///
/// AC-17-87, AC-17-88
///
/// @error @driving_port @real-io @US-03 @AC-17-87 @AC-17-88
#[tokio::test]
async fn a_not_equal_filter_on_the_referenced_field_does_not_satisfy_an_equality_group_rule() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-scgr03-wrongop").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    seed_group_access_rule_full(
        &ctx,
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

    let result = run_query_raw(
        &ctx,
        "journal_entries",
        true,
        Some(not_equal_filter("owner_id", "maria-santos")),
        Some(&marias_token),
    )
    .await;

    let err = result.expect_err(
        "AC-17-87: a '!=' filter on the group rule's referenced field must NOT satisfy an '==' \
         ownership-equality group rule",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-87: rejection must be attributable to the group rule (PermissionDenied), got \
         {:?}: {}",
        err.code(),
        err.message()
    );
    assert!(
        err.message().contains("[OWNERSHIP_FILTER_MISSING]"),
        "AC-17-87: rejection message must name the missing constraint even though a filter on \
         the right field was present with the wrong operator, got: {}",
        err.message()
    );
    assert!(
        !err.message().contains("[GROUP_RULE_NOT_DEFINED]"),
        "AC-17-88: this rejection must be distinguishable from Slice 04's 'no group rule \
         defined' rejection — a group rule genuinely IS defined here, got: {}",
        err.message()
    );
}
