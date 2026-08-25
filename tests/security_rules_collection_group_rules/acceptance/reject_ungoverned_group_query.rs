//! security-rules-collection-group-rules Slice 04 (US-04, ADR-032) — A
//! Collection-Group Query Against an Ungoverned Collection ID Is Rejected
//! Outright.
//!
//! Acceptance criteria verified here (feature-delta.md
//! `security-rules-collection-group-rules`):
//!   AC-17-89: a collection-group query (`all_descendants = true`) against a
//!             collection id with no group rule is rejected outright, before
//!             touching Postgres — regardless of whether an exact-path rule
//!             exists for that same collection id anywhere.
//!   AC-17-90: this is a new default distinct from — and does not reopen —
//!             `security-rules`'s Resolution 2 ("no exact-path rule ⇒
//!             unrestricted" for GetDocument/non-group RunQuery remains
//!             completely unchanged).
//!   AC-17-91: the "no group rule defined" default is decided by a single
//!             indexed lookup against `group_access_rules` alone — proven
//!             structurally by identical rejection behavior whether or not
//!             a same-named exact-path rule exists (a real SQL-query-count
//!             assertion is not practical at this driving-port boundary).
//!   AC-17-92: the "no group rule defined" rejection is distinguishable from
//!             the "missing required filter"/"unsatisfied conjunct"/
//!             "unsupported rule shape" rejections (Slice 02/03) by reason
//!             code.
//!
//! Driving port: gRPC :8080 `RunQuery` (`SecurityRulesFullContext`, real
//! production composition root — `RunQuery` requires the full gRPC stack,
//! not just the admin HTTP context Slice 01 used).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, run_query, SecurityRulesFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-89 + AC-17-91 + AC-17-92: a collection-group query is rejected
// outright, identically, whether the collection id has a same-named
// exact-path rule (`journal_entries`) or no rule of any kind (`app_config`)
// — proving the reject decision comes from `group_access_rules` alone, and
// carries a distinguishable reason code.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has an active exact-path rule (from
///          `security-rules`) and NO group rule; `app_config` has never had
///          any rule of any kind
///   And:   Maria Santos holds a verified identity
///   When:  Maria issues a `collectionGroup('journal_entries')` query with a
///          filter that WOULD satisfy the exact-path rule, and separately a
///          `collectionGroup('app_config')` query with no filter at all
///   Then:  BOTH are rejected outright, before execution, with the SAME
///          distinguishable `[GROUP_RULE_NOT_DEFINED]` reason — proving the
///          exact-path rule's existence has zero bearing on the group-query
///          decision, and that the reason is distinguishable from the
///          decidable-shape rejection vocabulary
///
/// AC-17-89, AC-17-91, AC-17-92
///
/// @error @driving_port @real-io @US-04 @AC-17-89 @AC-17-91 @AC-17-92 @security-critical
#[tokio::test]
async fn collection_group_query_rejected_outright_regardless_of_exact_path_rule_existence() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-scgr04-ungoverned").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    // journal_entries: exact-path rule exists (Option A/B's tempting but
    // rejected fallback target), no group rule.
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

    // A filter that WOULD satisfy journal_entries' own exact-path rule —
    // proves this is a genuine reject-by-default, not an accidental allow
    // masked by an unrelated missing filter.
    let journal_result = run_query(
        &ctx,
        "journal_entries",
        true,
        &[("owner_id", "maria-santos")],
        Some(&marias_token),
    )
    .await;
    let journal_err = journal_result.expect_err(
        "AC-17-89: a collection-group query against journal_entries must be rejected outright \
         even though a same-named exact-path rule exists and this filter would satisfy it",
    );
    assert_eq!(
        journal_err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-89: rejection must be attributable to the missing group rule (PermissionDenied), \
         got {:?}: {}",
        journal_err.code(),
        journal_err.message()
    );
    assert!(
        journal_err.message().contains("[GROUP_RULE_NOT_DEFINED]"),
        "AC-17-92: rejection message must name the distinguishable no-group-rule reason, got: {}",
        journal_err.message()
    );

    // app_config: no rule of any kind, ever.
    let app_config_result = run_query(&ctx, "app_config", true, &[], Some(&marias_token)).await;
    let app_config_err = app_config_result.expect_err(
        "AC-17-89: a collection-group query against app_config (no rule of any kind) must also \
         be rejected outright under this feature's universal default",
    );
    assert_eq!(
        app_config_err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-89: rejection must be attributable to the missing group rule (PermissionDenied), \
         got {:?}: {}",
        app_config_err.code(),
        app_config_err.message()
    );
    assert!(
        app_config_err.message().contains("[GROUP_RULE_NOT_DEFINED]"),
        "AC-17-92: rejection message must name the distinguishable no-group-rule reason, got: {}",
        app_config_err.message()
    );

    // AC-17-91: identical rejection reason regardless of exact-path rule
    // existence — the structural proof that the decision comes from
    // group_access_rules alone, not from any conditional check against
    // access_rules.
    assert_eq!(
        journal_err.message(),
        app_config_err.message(),
        "AC-17-91: the rejection must be byte-identical whether or not a same-named exact-path \
         rule exists — proving the decision never consults access_rules"
    );

    // AC-17-92: distinguishable from the decidable-shape rejection
    // vocabulary Slice 02/03 will introduce.
    assert!(
        !journal_err.message().contains("[OWNERSHIP_FILTER_MISSING]")
            && !journal_err.message().contains("[UNSUPPORTED_RULE_SHAPE]")
            && !journal_err.message().contains("[AUTH_REQUIRED]")
            && !journal_err.message().contains("[RULE_DENIES_ALL]"),
        "AC-17-92: the no-group-rule rejection must never carry any decidable-shape reason code, \
         got: {}",
        journal_err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-90: GetDocument and non-group RunQuery against the SAME ungoverned
// collection id remain exactly as unrestricted as `security-rules`'s own
// Resolution 2 already left them — this feature's single most important
// regression-safety proof.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `app_config` has never had any rule of any kind (exact-path or
///          group)
///   When:  an anonymous caller issues a plain `GetDocument` on the seeded
///          `app_config` document, a non-group `RunQuery` against
///          `app_config`, AND a `collectionGroup('app_config')` query — all
///          three against the SAME collection id
///   Then:  GetDocument and the non-group query both succeed, unrestricted
///          (unchanged `security-rules` Resolution 2 default), while the
///          collection-group query alone is rejected — proving this
///          feature's new default is additive, never a reopening of the
///          existing "no rule ⇒ unrestricted" behavior
///
/// AC-17-90
///
/// @driving_port @real-io @US-04 @AC-17-90 @security-critical
#[tokio::test]
async fn non_group_query_and_get_document_remain_unrestricted_on_the_same_ungoverned_collection()
{
    let ctx = SecurityRulesFullContext::new("trailmark-prod-scgr04-regression").await;

    let get_doc_result = ctx
        .get_document(
            &ctx.document_resource_name("app_config", "config-doc"),
            None,
        )
        .await;
    assert!(
        get_doc_result.is_ok(),
        "AC-17-90: GetDocument against an ungoverned collection must remain unrestricted \
         (unchanged security-rules Resolution 2), got: {:?}",
        get_doc_result.err()
    );

    let non_group_result = run_query(&ctx, "app_config", false, &[], None).await;
    assert!(
        non_group_result.is_ok(),
        "AC-17-90: a non-group RunQuery against an ungoverned collection must remain \
         unrestricted (unchanged security-rules-query-path default), got: {:?}",
        non_group_result.err()
    );
    assert_eq!(
        non_group_result.unwrap().len(),
        1,
        "AC-17-90: the unrestricted non-group query must return the seeded app_config document"
    );

    let group_result = run_query(&ctx, "app_config", true, &[], None).await;
    let group_err = group_result.expect_err(
        "AC-17-90: the SAME collection id's collection-group query must still be rejected — \
         this feature's new default applies ONLY to all_descendants=true",
    );
    assert_eq!(group_err.code(), tonic::Code::PermissionDenied);
    assert!(group_err.message().contains("[GROUP_RULE_NOT_DEFINED]"));
}
