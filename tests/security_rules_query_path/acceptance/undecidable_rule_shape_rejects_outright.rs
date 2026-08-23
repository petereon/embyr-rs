//! Slice 05 (US-05, ADR-031) — Undecidable Rule Shape Rejects Every Query
//! Outright.
//!
//! Acceptance criteria verified here (feature-delta.md US-05):
//!   AC-17-65: a RunQuery against a collection whose rule contains
//!             `Condition::Or` anywhere in its tree is rejected, regardless
//!             of filter shape.
//!   AC-17-66: a RunQuery against a collection whose rule contains
//!             `Condition::Not` anywhere in its tree is rejected.
//!   AC-17-67: a RunQuery against a collection whose read rule references
//!             `Operand::RequestResourceField` is rejected, never crashes.
//!   AC-17-68: the "unsupported rule shape" rejection is distinguishable
//!             from Slice 02's "missing required filter" rejection.
//!
//! Per direct code read (confirmed BEFORE writing this file, not assumed):
//! `decompose_decidable`'s wildcard arm (`_ => Err(Undecidable)`,
//! `crates/embyr-core/src/access_control/mod.rs`) is ALREADY the sole path
//! to `RejectedUnsupportedRuleShape` for `Or`/`Not`/`RequestResourceField`
//! — built as an explicit wildcard from Slice 01 onward, not an omission.
//! `grpc::handler::query_compliance_rejection` ALREADY embeds a distinct
//! `[UNSUPPORTED_RULE_SHAPE]` token (never `[OWNERSHIP_FILTER_MISSING]`) for
//! this outcome. This slice's job is therefore acceptance-test coverage
//! proving these already-correct behaviors end-to-end through the real
//! `handle_run_query` composition — see the per-test doc comments below for
//! the honest RED/GREEN status of each.
//!
//! `resource.data.public == true` (a literal boolean field comparison) is
//! NOT expressible in the current grammar: `word_to_operand` never returns
//! `Operand::BoolLiteral` (only `parse_primary`'s bare `true`/`false`
//! literal path does, and that is not a comparable operand). The
//! trip_comments domain example below therefore uses the grammar-legal
//! `resource.data.public != null` idiom ("a public-visibility field is
//! present") to express the same "ownership OR public-visibility" intent
//! via `||` — an adaptation of the slice brief's suggested literal example,
//! not a deviation from its intent.
//!
//! Driving port: gRPC :8080 `RunQuery` (via `SecurityRulesFullContext` +
//! this feature's own `run_query`/`run_query_raw` helpers — Pillar 3, real
//! Postgres, real gRPC, no mocks).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, run_query, SecurityRulesFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-65: a RunQuery against a collection whose rule contains
// `Condition::Or` anywhere in its tree is rejected, regardless of filter
// shape. Domain-grounded in `trip_comments` (Trailmark), combining ownership
// and public-visibility via `||`, per this slice's own brief.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trip_comments` has a READ rule combining ownership and
///          public-visibility via `||`:
///          `request.auth.uid == resource.data.owner_id || resource.data.public != null`
///   And:   Maria Santos holds a verified identity
///   When:  Maria calls `RunQuery` with NO filter, and again WITH a filter
///          that would satisfy the rule's OWNERSHIP half in isolation
///          (`owner_id == "maria-santos"`, her own uid)
///   Then:  BOTH calls are rejected outright — the `||` makes the entire
///          rule undecidable for query enforcement; a filter that would
///          satisfy one disjunct is NOT sufficient, because this mechanism
///          never attempts partial `Or` enforcement (explicitly out of
///          scope, ADR-031 § Consequences — Negative)
///
/// AC-17-65
///
/// @error @driving_port @real-io @US-05 @AC-17-65 @security-critical
#[tokio::test]
async fn an_or_shaped_rule_rejects_every_query_regardless_of_filter_shape() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp05-or").await;
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

    let no_filter_result = run_query(&ctx, "trip_comments", &[], Some(&marias_token)).await;
    let err_no_filter = no_filter_result.expect_err(
        "AC-17-65: an unfiltered query against an Or-shaped rule must be rejected outright",
    );
    assert_eq!(
        err_no_filter.code(),
        tonic::Code::PermissionDenied,
        "AC-17-65: rejection must be attributable to the rule (PermissionDenied), got {:?}: {}",
        err_no_filter.code(),
        err_no_filter.message()
    );
    assert!(
        err_no_filter.message().contains("[UNSUPPORTED_RULE_SHAPE]"),
        "AC-17-65: rejection message must name the whole-rule-undecidable reason, got: {}",
        err_no_filter.message()
    );

    let owner_filter_result = run_query(
        &ctx,
        "trip_comments",
        &[("owner_id", "maria-santos")],
        Some(&marias_token),
    )
    .await;
    let err_owner_filter = owner_filter_result.expect_err(
        "AC-17-65: a filter that would satisfy only the rule's ownership disjunct must still be \
         rejected outright — no partial Or enforcement",
    );
    assert_eq!(
        err_owner_filter.code(),
        tonic::Code::PermissionDenied,
        "AC-17-65: rejection must be attributable to the rule (PermissionDenied), got {:?}: {}",
        err_owner_filter.code(),
        err_owner_filter.message()
    );
    assert!(
        err_owner_filter.message().contains("[UNSUPPORTED_RULE_SHAPE]"),
        "AC-17-65: rejection message must name the whole-rule-undecidable reason even when a \
         filter satisfying one disjunct is present, got: {}",
        err_owner_filter.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-66: a RunQuery against a collection whose rule contains
// `Condition::Not` anywhere in its tree is rejected.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `archived_notes` has a READ rule negating an ownership check:
///          `!(request.auth.uid == resource.data.owner_id)`
///   And:   Maria Santos holds a verified identity
///   When:  Maria calls `RunQuery` with no filter
///   Then:  the query is rejected outright — `Not` is permanently out of
///          scope for query enforcement (ADR-031), regardless of what the
///          negated condition would otherwise evaluate to
///
/// AC-17-66
///
/// @error @driving_port @real-io @US-05 @AC-17-66
#[tokio::test]
async fn a_not_shaped_rule_rejects_every_query() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp05-not").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "archived_notes",
        "!(request.auth.uid == resource.data.owner_id)",
    )
    .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let result = run_query(&ctx, "archived_notes", &[], Some(&marias_token)).await;

    let err = result.expect_err(
        "AC-17-66: a Not-shaped rule must reject every query outright, regardless of filter",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-66: rejection must be attributable to the rule (PermissionDenied), got {:?}: {}",
        err.code(),
        err.message()
    );
    assert!(
        err.message().contains("[UNSUPPORTED_RULE_SHAPE]"),
        "AC-17-66: rejection message must name the whole-rule-undecidable reason, got: {}",
        err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-67: a RunQuery against a collection whose read rule references
// `Operand::RequestResourceField` is rejected, never crashes.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `draft_submissions` has a READ rule that (degenerately)
///          references the write-only `request.resource.data.<field>`
///          operand: `request.auth.uid == request.resource.data.owner_id`
///          — grammar-legal (`parse_condition` accepts this syntax
///          unconditionally, security-rules-write-path Slice 02) but
///          semantically nonsensical as a READ rule, since no "proposed new
///          document" exists at query-planning time
///   And:   Maria Santos holds a verified identity
///   When:  Maria calls `RunQuery` with a filter that would satisfy the
///          rule if `RequestResourceField` were (incorrectly) treated as
///          `ResourceField`
///   Then:  the query is rejected gracefully (a `Status::permission_denied`
///          gRPC response, never a panic, never a dropped connection) — the
///          same `Undecidable` fallthrough as `Or`/`Not`, no separate
///          crash-prone code path
///
/// AC-17-67
///
/// @error @driving_port @real-io @US-05 @AC-17-67
#[tokio::test]
async fn a_read_rule_referencing_request_resource_field_is_rejected_never_crashes() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp05-reqresfield").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "draft_submissions",
        "request.auth.uid == request.resource.data.owner_id",
    )
    .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    // A filter that WOULD satisfy the rule if `RequestResourceField` were
    // mistakenly matched like `ResourceField` — proves this is a genuine
    // reject, not an accidental allow masked by an unrelated missing filter.
    let result = run_query(
        &ctx,
        "draft_submissions",
        &[("owner_id", "maria-santos")],
        Some(&marias_token),
    )
    .await;

    let err = result.expect_err(
        "AC-17-67: a read rule referencing request.resource.data.<field> must be rejected, \
         never silently allowed",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-67: rejection must be a graceful gRPC response (PermissionDenied), never a \
         crash — got {:?}: {}",
        err.code(),
        err.message()
    );
    assert!(
        err.message().contains("[UNSUPPORTED_RULE_SHAPE]"),
        "AC-17-67: rejection message must name the whole-rule-undecidable reason, got: {}",
        err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-68: the "unsupported rule shape" rejection is distinguishable from
// Slice 02's "missing required filter" rejection.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: TWO collections — `undecidable_feed` with an Or-shaped
///          (undecidable) rule, and `decidable_feed` with a plain
///          ownership-equality rule
///   When:  Maria runs an unfiltered `RunQuery` against EACH
///   Then:  both are rejected as `PermissionDenied`, but the messages carry
///          DIFFERENT reason-code tokens — `[UNSUPPORTED_RULE_SHAPE]` for
///          the undecidable rule, `[OWNERSHIP_FILTER_MISSING]` for the
///          decidable-but-unsatisfied rule — so a caller (or a future
///          DISTILL-level test) can tell "your query is fine but doesn't
///          have the right filter" apart from "your rule itself can't be
///          enforced for queries at all"
///
/// AC-17-68
///
/// @error @driving_port @real-io @US-05 @AC-17-68
#[tokio::test]
async fn unsupported_rule_shape_rejection_is_distinguishable_from_missing_filter_rejection() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp05-distinct").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "undecidable_feed",
        "request.auth.uid == resource.data.owner_id || resource.data.public != null",
    )
    .await;
    ctx.seed_access_rule(
        "decidable_feed",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let unsupported_shape_err = run_query(&ctx, "undecidable_feed", &[], Some(&marias_token))
        .await
        .expect_err("expected an UNSUPPORTED_RULE_SHAPE rejection for the Or-shaped rule");
    let missing_filter_err = run_query(&ctx, "decidable_feed", &[], Some(&marias_token))
        .await
        .expect_err("expected an OWNERSHIP_FILTER_MISSING rejection for the unfiltered query");

    assert_eq!(
        unsupported_shape_err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-68: the unsupported-shape rejection must be PermissionDenied, got {:?}",
        unsupported_shape_err.code()
    );
    assert_eq!(
        missing_filter_err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-68: the missing-filter rejection must be PermissionDenied, got {:?}",
        missing_filter_err.code()
    );

    assert!(
        unsupported_shape_err.message().contains("[UNSUPPORTED_RULE_SHAPE]"),
        "AC-17-68: the undecidable-rule rejection must carry the UNSUPPORTED_RULE_SHAPE token, \
         got: {}",
        unsupported_shape_err.message()
    );
    assert!(
        !unsupported_shape_err.message().contains("[OWNERSHIP_FILTER_MISSING]"),
        "AC-17-68: the undecidable-rule rejection must NEVER carry the missing-filter token, \
         got: {}",
        unsupported_shape_err.message()
    );

    assert!(
        missing_filter_err.message().contains("[OWNERSHIP_FILTER_MISSING]"),
        "AC-17-68: the missing-filter rejection must carry the OWNERSHIP_FILTER_MISSING token, \
         got: {}",
        missing_filter_err.message()
    );
    assert!(
        !missing_filter_err.message().contains("[UNSUPPORTED_RULE_SHAPE]"),
        "AC-17-68: the missing-filter rejection must NEVER carry the unsupported-shape token, \
         got: {}",
        missing_filter_err.message()
    );

    assert_ne!(
        unsupported_shape_err.message(),
        missing_filter_err.message(),
        "AC-17-68: the two rejection messages must be textually distinguishable"
    );
}
