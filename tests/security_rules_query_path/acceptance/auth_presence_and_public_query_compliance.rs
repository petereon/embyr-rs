//! Slice 03 (US-03, ADR-031) — Auth-Presence-Only and Public Query Compliance.
//!
//! Acceptance criteria verified here (feature-delta.md US-03):
//!   AC-17-59: a RunQuery against `allow read: if true` is admitted
//!             regardless of caller identity or filter shape.
//!   AC-17-57: a signed-in caller's query is admitted against a rule
//!             requiring only `request.auth != null`, with or without a
//!             filter.
//!   AC-17-58: a never-signed-in caller's query is rejected against the
//!             same `request.auth != null` rule.
//!   AC-17-60: an invalid identity header (malformed/expired/wrong-project)
//!             is evaluated identically to no header at all — reusing
//!             `client-auth`'s existing ADR-026 "attach nothing" semantics
//!             unchanged, no new rejection class introduced.
//!
//! `check_query_compliance`'s core matching algorithm for `Literal(bool)`/
//! `AuthRequired` (Slices 01/02's own `OwnershipEquality` shape untouched)
//! is unit-tested in `crates/embyr-core/src/access_control/mod.rs`. This
//! file proves the SAME two new atoms end-to-end through the real
//! `handle_run_query` composition (ADR-031 § Decision — Composition, an
//! already-wired call site this slice does not modify).
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
// AC-17-59: a RunQuery against `allow read: if true` is admitted regardless
// of caller identity or filter shape.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `public_notices` has a READ rule of bare `true` (public read)
///   And:   a document is seeded into it
///   When:  an anonymous caller runs `RunQuery` with no filter, an anonymous
///          caller runs `RunQuery` WITH a filter, and a signed-in caller
///          runs `RunQuery` with no filter
///   Then:  all three are admitted and return the seeded document — no auth
///          dependency, no filter requirement at all
///
/// AC-17-59
///
/// @driving_port @real-io @US-03 @AC-17-59
#[tokio::test]
async fn a_bare_true_rule_admits_regardless_of_caller_identity_or_filter_shape() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp03-public").await;
    ctx.seed_access_rule("public_notices", "true").await;

    let mut fields = std::collections::HashMap::new();
    fields.insert("headline".to_string(), string_field("trail-closure"));
    create_document(&ctx, "public_notices", "sqp03-public-doc", fields, None)
        .await
        .expect("seed public_notices document");

    let anon_no_filter = run_query(&ctx, "public_notices", &[], None)
        .await
        .expect("AC-17-59: a bare `true` rule must admit an anonymous caller with no filter");
    assert_eq!(anon_no_filter.len(), 1, "expected the seeded document");

    let anon_with_filter = run_query(&ctx, "public_notices", &[("headline", "trail-closure")], None)
        .await
        .expect("AC-17-59: a bare `true` rule must admit an anonymous caller even with a filter");
    assert_eq!(anon_with_filter.len(), 1, "expected the seeded document");

    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );
    let signed_in_no_filter = run_query(&ctx, "public_notices", &[], Some(&marias_token))
        .await
        .expect("AC-17-59: a bare `true` rule must also admit a signed-in caller");
    assert_eq!(signed_in_no_filter.len(), 1, "expected the seeded document");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-57: a signed-in caller's query is admitted against a rule requiring
// only `request.auth != null`, with or without a filter.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `shared_bulletins` has a READ rule requiring `request.auth !=
///          null`
///   And:   Maria Santos holds a verified identity and owns a seeded
///          document
///   When:  Maria calls `RunQuery` with NO filter, and again WITH an
///          arbitrary filter
///   Then:  both are admitted — the rule has no filter requirement at all,
///          only auth presence
///
/// AC-17-57
///
/// @driving_port @real-io @US-03 @AC-17-57
#[tokio::test]
async fn an_auth_required_rule_admits_a_signed_in_callers_query_with_or_without_a_filter() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp03-authreq").await;
    ctx.seed_access_rule("shared_bulletins", "request.auth != null")
        .await;

    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let mut fields = std::collections::HashMap::new();
    fields.insert("topic".to_string(), string_field("trail-update"));
    create_document(&ctx, "shared_bulletins", "sqp03-bulletin-doc", fields, Some(&marias_token))
        .await
        .expect("seed shared_bulletins document");

    let no_filter = run_query(&ctx, "shared_bulletins", &[], Some(&marias_token))
        .await
        .expect("AC-17-57: a signed-in caller must be admitted with NO filter requirement");
    assert_eq!(no_filter.len(), 1, "expected the seeded document");

    let with_filter = run_query(
        &ctx,
        "shared_bulletins",
        &[("topic", "trail-update")],
        Some(&marias_token),
    )
    .await
    .expect("AC-17-57: a signed-in caller must also be admitted when an unrelated filter is present");
    assert_eq!(with_filter.len(), 1, "expected the seeded document");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-58: a never-signed-in caller's query is rejected against the same
// `request.auth != null` rule.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `private_diaries` has a READ rule requiring `request.auth !=
///          null`
///   When:  an anonymous caller (no client-identity token at all) calls
///          `RunQuery`
///   Then:  the query is rejected before execution, naming the
///          `AUTH_REQUIRED` constraint
///
/// AC-17-58
///
/// @error @driving_port @real-io @US-03 @AC-17-58
#[tokio::test]
async fn an_auth_required_rule_rejects_a_never_signed_in_callers_query() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp03-anondeny").await;
    ctx.seed_access_rule("private_diaries", "request.auth != null")
        .await;

    let result = run_query(&ctx, "private_diaries", &[], None).await;

    let err = result.expect_err(
        "AC-17-58: a never-signed-in caller must be rejected by a rule requiring request.auth != null",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-58: rejection must be attributable to the rule (PermissionDenied), got {:?}: {}",
        err.code(),
        err.message()
    );
    assert!(
        err.message().contains("[AUTH_REQUIRED]"),
        "AC-17-58: rejection message must name the specific missing constraint via the stable \
         reason code, got: {}",
        err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-60: an invalid identity header (malformed/expired/wrong-project) is
// evaluated identically to no header at all — reusing client-auth's
// existing ADR-026 "attach nothing" semantics unchanged, no new rejection
// class introduced.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (mirrors `security_rules::sr03_anonymous_session_evaluated_as_null_auth`'s
/// own AC-17-13 pattern, the equivalent proof for `GetDocument`):
///   Given: `gated_reports` has a READ rule requiring `request.auth != null`
///   And:   Dana Kim holds a client-identity credential, but her token is
///          EXPIRED (one of the three DISCUSS-named equivalent invalid
///          shapes: malformed/expired/wrong-project)
///   When:  (a) a session with NO client-identity header calls `RunQuery`
///          and (b) a session presenting the EXPIRED header calls the SAME
///          query
///   Then:  both are rejected with the EXACT same gRPC status code and the
///          EXACT same message — `attach_client_identity_if_present`
///          attaches nothing for an invalid header, per ADR-026 DDD-CA-5,
///          unchanged, so `check_query_compliance` sees `auth: None` in
///          both cases and introduces no new rejection class
///
/// AC-17-60
///
/// @error @driving_port @real-io @US-03 @AC-17-60
#[tokio::test]
async fn an_invalid_client_identity_header_is_evaluated_identically_to_no_header_at_all_for_queries()
{
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp03-invalidheader").await;
    ctx.seed_access_rule("gated_reports", "request.auth != null")
        .await;

    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    // Expired 1h ago — `attach_client_identity_if_present` attaches nothing
    // for this case (client-auth ADR-026 DDD-CA-5, unchanged).
    let expired_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() - 3600);

    let resp_no_header = run_query(&ctx, "gated_reports", &[], None).await;
    let resp_expired_header = run_query(&ctx, "gated_reports", &[], Some(&expired_token)).await;

    let err_no_header =
        resp_no_header.expect_err("AC-17-60: no-header query must be rejected by the auth-required rule");
    let err_expired_header = resp_expired_header
        .expect_err("AC-17-60: expired-header query must be rejected IDENTICALLY to no-header");

    // Anchor assertion (checked BEFORE the cross-comparison below, per this
    // feature's own established convention — see sr03's AC-17-13 scenario):
    // pins both sides to the concrete expected status so the scenario is
    // meaningful, not vacuously true from two unrelated failures comparing
    // equal.
    assert_eq!(
        err_no_header.code(),
        tonic::Code::PermissionDenied,
        "AC-17-60: the no-header query must be denied specifically as PermissionDenied \
         (attributable to the rule), got {:?}",
        err_no_header.code()
    );

    assert_eq!(
        err_no_header.code(),
        err_expired_header.code(),
        "AC-17-60: no new rejection class — an invalid client-identity header must be denied \
         with the EXACT same gRPC status as no header at all"
    );
    assert_eq!(
        err_no_header.message(),
        err_expired_header.message(),
        "AC-17-60: no new rejection class — identical message too, per ADR-026 DDD-CA-5"
    );
}
