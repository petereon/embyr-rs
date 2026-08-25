//! Slice 06 (US-06, ADR-031) — No-Rule and Sibling-Collection Regression
//! Guardrail.
//!
//! This is this feature's own AC-17-14/15/16-equivalent regression
//! discipline (feature-delta.md US-06 Technical Notes: "mirrors
//! `security-rules`'/`security-rules-write-path`'s own US-04/US-06
//! discipline exactly, now applied to `RunQuery`") — primarily a PROOF
//! obligation over Slices 01-05's real behavior, not new production logic.
//! `handle_run_query`'s structural no-rule-defined guardrail (`handler.rs`,
//! `get_access_rule() -> None` short-circuit, identical shape to
//! `handle_get_document`'s own AC-17-14/15/16 mechanism) was already built
//! correctly by Slices 01-05 — this slice's own DELIVER confirms that
//! structurally-correct-by-construction claim with real gRPC calls AND by
//! actually re-running the full cumulative regression suite.
//!
//! Acceptance criteria verified here (feature-delta.md US-06):
//!   AC-17-69: a `RunQuery` on a collection with no rule defined succeeds
//!             unfiltered, gated only by the existing `api_key`.
//!   AC-17-70: a rule on one collection has zero observable effect on
//!             `RunQuery` behavior for any other collection without its own
//!             rule (sibling-collection isolation, mirrors AC-17-15/AC-17-44).
//!   AC-17-71: the full cumulative pre-existing regression suite (113
//!             `embyr-rs`/`client-auth` + 20 `security-rules` + 45
//!             `security-rules-write-path` + 20 `security-rules-query-path`
//!             Slices 01-05 = 198 scenarios) passes unmodified — a proof
//!             obligation satisfied by DELIVER's own GREEN-phase re-run of
//!             the full suite, NOT by a self-contained Rust assertion — see
//!             the marker test below.
//!   AC-17-72: `GetDocument`'s existing rule-enforcement behavior
//!             (`security-rules`) is unmodified by this feature — confirmed
//!             here by a real `GetDocument` allow/deny round-trip against an
//!             ownership rule (the SAME `access_rules` row shape
//!             `security-rules-query-path`'s own `RunQuery` compliance check
//!             reads), AND separately by a direct `git diff` read confirming
//!             `handle_get_document` has zero line changes across this
//!             entire feature's commits (reported out-of-band, not
//!             expressible as a Rust assertion).
//!
//! Driving port: gRPC :8080 `RunQuery` (AC-17-69/70) and `GetDocument`
//! (AC-17-72) via `SecurityRulesFullContext` + this feature's own
//! `run_query`/`get_document` helpers (Pillar 3, real Postgres, real gRPC,
//! no mocks); the full pre-existing acceptance suite (subprocess
//! `cargo test`, out-of-band) for AC-17-71.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, run_query, SecurityRulesFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-69: a RunQuery on a collection with no rule defined succeeds
// unfiltered, gated only by the existing api_key.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `app_config` has never had a read rule defined (the same
///          "definitely no rule" collection `security-rules`' own AC-17-14
///          and `security-rules-write-path`'s own AC-17-42 use)
///   When:  an unauthenticated caller (no client-identity token) calls
///          `RunQuery` on `app_config` with no filter, using only the
///          existing project `api_key`
///   Then:  the query succeeds and returns the seeded document — the
///          structural no-rule-defined guardrail means `get_access_rule`
///          returns `None` and `check_query_compliance` is never invoked
///
/// AC-17-69
///
/// @driving_port @real-io @US-06 @AC-17-69
#[tokio::test]
async fn a_run_query_on_a_collection_with_no_rule_defined_succeeds_unfiltered_gated_only_by_api_key(
) {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp06-norule").await;
    // Deliberately: zero rows in access_rules for this project at all.

    let docs = run_query(&ctx, "app_config", &[], None)
        .await
        .expect("AC-17-69: a RunQuery against a collection with no rule defined must succeed, \
                 gated only by the existing api_key");

    assert_eq!(
        docs.len(),
        1,
        "expected the default seeded app_config document, got {} documents",
        docs.len()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-70: a rule on one collection has zero observable effect on RunQuery
// behavior for any other collection without its own rule (sibling-collection
// isolation).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has an active read rule
///          (`request.auth.uid == resource.data.owner_id`) that would reject
///          an unauthenticated, unfiltered query outright; `trail_guides` in
///          the same project has no rule of its own
///   When:  a caller with NO client-identity token at all — who would be
///          rejected by `journal_entries`'s own compliance check — calls
///          `RunQuery` on `trail_guides` with no filter
///   Then:  the query succeeds, completely unaffected by `journal_entries`'s
///          rule
///
/// AC-17-70
///
/// @error @driving_port @real-io @US-06 @AC-17-70
#[tokio::test]
async fn a_rule_on_one_collection_has_zero_effect_on_a_sibling_collections_run_query() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp06-sibling-isolation").await;
    ctx.seed_access_rule(
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;
    // trail_guides deliberately has no rule of its own.

    let docs = run_query(&ctx, "trail_guides", &[], None)
        .await
        .expect(
            "AC-17-70: journal_entries's rule must have ZERO effect on trail_guides's \
             RunQuery, which has no rule of its own",
        );

    assert_eq!(
        docs.len(),
        1,
        "expected the default seeded trail_guides document, got {} documents",
        docs.len()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-72: GetDocument's existing rule-enforcement behavior (security-rules)
// is unmodified by this feature — confirmed here by a real allow/deny
// round-trip; separately confirmed by a direct git-diff read (see final
// DELIVER report, not expressible as a Rust assertion).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has the SAME ownership read rule
///          `security-rules-query-path`'s own RunQuery compliance check
///          reads from the identical `access_rules` row
///   When:  (a) Maria Santos — who owns the seeded document — calls
///          `GetDocument` on her own document, and (b) Dana Kim — who does
///          NOT own it — calls `GetDocument` on the same document
///   Then:  Maria's call succeeds and Dana's call is denied with
///          `PermissionDenied` — the exact allow/deny shape `security-rules`
///          established, proving `handle_get_document`'s rule-enforcement
///          path is unaffected by this feature having been layered on top
///          of the same `access_rules` table
///
/// AC-17-72
///
/// @driving_port @real-io @US-06 @AC-17-72
#[tokio::test]
async fn get_documents_existing_rule_enforcement_behavior_is_unmodified_by_this_feature() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sqp06-getdoc-regression").await;
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
    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let marias_result = ctx
        .get_document(
            &ctx.document_resource_name("journal_entries", "maria-doc"),
            Some(&marias_token),
        )
        .await;
    assert!(
        marias_result.is_ok(),
        "AC-17-72: GetDocument's owner-allow behavior must be unmodified: {:?}",
        marias_result.err()
    );

    let danas_result = ctx
        .get_document(
            &ctx.document_resource_name("journal_entries", "maria-doc"),
            Some(&danas_token),
        )
        .await;
    let err = danas_result.expect_err(
        "AC-17-72: GetDocument's non-owner-deny behavior must be unmodified — a non-owner \
         must still be rejected",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-72: denial must still be attributable to the rule (PermissionDenied), got {:?}: {}",
        err.code(),
        err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-71: full cumulative pre-existing regression suite (proof obligation —
// marker/documentation test, NOT a self-contained assertion)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-71 is satisfied by literally re-running the full cumulative
/// pre-existing regression suite (113 `embyr-rs`/`client-auth` scenarios +
/// 20 `security-rules` acceptance scenarios + 45 `security-rules-write-path`
/// acceptance scenarios [7 files] + 20 `security-rules-query-path`
/// acceptance scenarios [Slices 01-05, 5 files] = 198 scenarios) UNMODIFIED,
/// alongside this slice's own new 3 executable scenarios above, against a
/// build that includes this feature's changes — NOT by any assertion this
/// function's body could make. This mirrors `security-rules`' own `sr04`
/// (`full_113_scenario_regression_suite_passes_unmodified`) and
/// `security-rules-write-path`'s own `read_write_independence_and_regression`
/// (`full_133_scenario_regression_suite_passes_unmodified`) discipline
/// exactly, extended to this feature's own additional query-path targets.
///
/// This function is a structural MARKER — it exists so `AC-17-71` is
/// discoverable in the test suite (`cargo test -- --list` surfaces it,
/// traceable to this AC) without pretending a 198+-scenario regression run
/// can be expressed as one in-process assertion. It is `#[ignore]`d
/// UNCONDITIONALLY — DELIVER's own GREEN phase for this slice satisfies
/// AC-17-71 by re-running the command below, not by unskipping and passing
/// this function.
///
/// Command (from workspace root — the 19 non-`us_10_aws_secrets`
/// `embyr-rs`/`client-auth` targets, `security-rules`' own 5
/// `security_rules_sr0N` targets, all SEVEN
/// `security_rules_write_path_*` targets, and all SIX
/// `security_rules_query_path_*` targets built so far, Slices 01-06;
/// `us_10_aws_secrets` is excluded — confirmed pre-existing environmental
/// failure, unrelated to this feature):
/// ```text
/// cargo test -p embyr-server \
///   --test us_01_configure_sdk --test us_02_write_document --test us_03_read_document \
///   --test us_04_query_collection --test us_05_listen_realtime --test us_06_transactions \
///   --test us_07_provision_project --test us_08_monitor_project --test us_09_suspend_project \
///   --test us_11_gcp_secrets --test us_12_agent_backend --test us_13_browser_transport \
///   --test us_14_rate_limiting --test walking_skeleton \
///   --test client_auth_ca01_register_verification_credential \
///   --test client_auth_ca02_signin_and_reject_invalid_tokens \
///   --test client_auth_ca03_rotate_verification_credential \
///   --test client_auth_ca04_standalone_verify_debug_check \
///   --test client_auth_ca05_algorithm_confusion_defense \
///   --test security_rules_sr01_define_and_redefine_rule \
///   --test security_rules_sr02_signed_in_read_gated_by_rule \
///   --test security_rules_sr03_anonymous_session_evaluated_as_null_auth \
///   --test security_rules_sr04_untouched_collections_and_regression_suite_unaffected \
///   --test security_rules_sr05_simulate_rule_before_publish \
///   --test security_rules_write_path_define_write_rule \
///   --test security_rules_write_path_create_gated_by_write_rule \
///   --test security_rules_write_path_update_gated_by_both_resource_states \
///   --test security_rules_write_path_delete_gated_by_resource \
///   --test security_rules_write_path_anonymous_write_sessions \
///   --test security_rules_write_path_read_write_independence_and_regression \
///   --test security_rules_write_path_simulate_write_rule \
///   --test security_rules_query_path_ownership_equality_query_compliance \
///   --test security_rules_query_path_noncompliant_query_rejected_preexecution \
///   --test security_rules_query_path_auth_presence_and_public_query_compliance \
///   --test security_rules_query_path_undecidable_rule_shape_rejects_outright \
///   --test security_rules_query_path_and_composed_query_compliance \
///   --test security_rules_query_path_no_rule_and_sibling_collection_regression_guardrail
/// ```
///
/// AC-17-71
///
/// @driving_port @real-io @US-06 @AC-17-71 @regression-proof-obligation
#[test]
#[ignore = "structural marker, not an executable assertion — AC-17-71 is satisfied by \
            re-running the full 198-scenario cumulative pre-existing regression suite (plus \
            this slice's own 3 new query-path targets) unmodified, per this function's own \
            doc comment"]
fn full_198_scenario_regression_suite_passes_unmodified() {
    unreachable!(
        "AC-17-71 is a proof obligation over EXTERNAL test binaries, not an in-process \
         assertion this function's body could make — see its doc comment for the exact \
         command DELIVER runs at GREEN for this slice."
    );
}
